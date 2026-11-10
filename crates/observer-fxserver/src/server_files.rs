//! The setup's view into a server folder: the entries of one of its folders, and a cfg's text with
//! its secrets left out. Both only read, never go up out of the folder, and never list through a
//! link; what holds secrets or belongs to FXServer or Observer is named and never opened.

use std::fs;
use std::io;
use std::ops::Range;
use std::path::{Path, PathBuf};

use observer_domain::files::refused_by_name;
use observer_domain::launch::{self, Platform};
use observer_domain::server_data::{
    CfgFileProblem, CfgSpan, CfgText, FileEntry, FileEntryKind, FileListing, FolderProblem,
};

use crate::cfg::{self, ReadError};
use crate::cfg_edit::merged;
use crate::changes::mask;

const MAX_ENTRIES: usize = 500;
const MAX_LINES: usize = 2000;

/// The entries of `relative`, a folder of the server folder `data_path`, folders first.
pub fn list(data_path: &str, relative: &str, observer: &Path) -> FileListing {
    let mut listing = FileListing {
        path: relative.to_owned(),
        problem: None,
        entries: Vec::new(),
        truncated: false,
    };
    let observer = fs::canonicalize(observer).ok();
    let folder = match data_folder(data_path, observer.as_deref()).and_then(|data| {
        let parts = components(relative).ok_or(FolderProblem::InvalidPath)?;
        let folder = descend(&data, &parts)?;
        Ok((folder, parts))
    }) {
        Ok(found) => found,
        Err(problem) => {
            listing.problem = Some(problem);
            return listing;
        }
    };
    let (folder, parts) = folder;
    let Ok(read) = fs::read_dir(&folder) else {
        listing.problem = Some(FolderProblem::Unreadable);
        return listing;
    };
    for entry in read.flatten() {
        if listing.entries.len() == MAX_ENTRIES {
            listing.truncated = true;
            break;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let link = kind.is_symlink();
        let mut path = parts.clone();
        path.push(name.clone());
        let is_folder = if link {
            fs::metadata(entry.path()).is_ok_and(|metadata| metadata.is_dir())
        } else {
            kind.is_dir()
        };
        let own = observer.as_deref().is_some_and(|observer| {
            fs::canonicalize(entry.path()).is_ok_and(|real| real.starts_with(observer))
        });
        let kind = if withheld(&path) || own {
            FileEntryKind::Withheld
        } else if is_folder {
            FileEntryKind::Folder
        } else if is_cfg(&name) {
            FileEntryKind::Cfg
        } else {
            FileEntryKind::File
        };
        listing.entries.push(FileEntry {
            name,
            path: path.join("/"),
            kind,
            link,
        });
    }
    listing.entries.sort_by_cached_key(|entry| {
        let folder = entry.kind == FileEntryKind::Folder
            || (entry.kind == FileEntryKind::Withheld && folder.join(&entry.name).is_dir());
        (!folder, entry.name.to_lowercase())
    });
    listing
}

/// The cfg `cfg_file` of the server folder `data_path`, line by line, its secrets left out.
pub fn cfg_text(data_path: &str, cfg_file: &str, observer: &Path) -> CfgText {
    let mut shown = CfgText {
        folder: None,
        problem: None,
        path_problem: None,
        lines: Vec::new(),
        truncated: false,
    };
    let observer = fs::canonicalize(observer).ok();
    let data = match data_folder(data_path, observer.as_deref()) {
        Ok(data) => data,
        Err(problem) => {
            shown.folder = Some(problem);
            return shown;
        }
    };
    shown.path_problem = launch::cfg_problem(cfg_file, Platform::host());
    if shown.path_problem.is_some() {
        shown.problem = Some(CfgFileProblem::InvalidPath);
        return shown;
    }
    let parts: Vec<String> = cfg_file.split(['/', '\\']).map(str::to_owned).collect();
    if withheld(&parts) {
        shown.problem = Some(CfgFileProblem::Unreadable);
        return shown;
    }
    let text = match cfg::read(&data.join(cfg_file)) {
        Ok(text) => text,
        Err(error) => {
            shown.problem = Some(match error {
                ReadError::Io(error) if error.kind() == io::ErrorKind::NotFound => CfgFileProblem::Missing,
                ReadError::Io(_) => CfgFileProblem::Unreadable,
                ReadError::NotAFile => CfgFileProblem::NotAFile,
                ReadError::TooLarge => CfgFileProblem::TooLarge,
            });
            return shown;
        }
    };
    let secrets = merged(mask::secrets(&text, cfg_file, true));
    let mut start = 0;
    for (number, line) in text.split('\n').enumerate() {
        if number == MAX_LINES {
            shown.truncated = true;
            break;
        }
        let end = start + line.len();
        let content = line.strip_suffix('\r').map_or(end, |kept| start + kept.len());
        shown.lines.push(spans(&text, start..content, &secrets));
        start = end + 1;
    }
    // A file that ends with a line break has no line after it.
    if !shown.truncated && text.ends_with('\n') {
        shown.lines.pop();
    }
    shown
}

/// The server folder, once the path rules accept it and it is a folder outside Observer's own.
fn data_folder(data_path: &str, observer: Option<&Path>) -> Result<PathBuf, FolderProblem> {
    if launch::folder_problem(data_path, Platform::host()).is_some() {
        return Err(FolderProblem::InvalidPath);
    }
    let data = PathBuf::from(data_path);
    let metadata = fs::metadata(&data).map_err(|error| match error.kind() {
        io::ErrorKind::NotFound => FolderProblem::Missing,
        _ => FolderProblem::Unreadable,
    })?;
    if !metadata.is_dir() {
        return Err(FolderProblem::NotAFolder);
    }
    let real = fs::canonicalize(&data).map_err(|_| FolderProblem::Unreadable)?;
    if observer.is_some_and(|observer| real.starts_with(observer)) {
        return Err(FolderProblem::InsideObserver);
    }
    Ok(data)
}

/// The names of a path written with `/`, none of them empty, `.`, `..` or holding what a path
/// could reach elsewhere with. The empty path is the folder itself.
fn components(relative: &str) -> Option<Vec<String>> {
    if relative.is_empty() {
        return Some(Vec::new());
    }
    relative
        .split('/')
        .map(|part| {
            let refused = part.is_empty()
                || part == "."
                || part == ".."
                || part.contains(['\\', ':', '\0'])
                || part.chars().all(|c| c == '.' || c == ' ');
            (!refused).then(|| part.to_owned())
        })
        .collect()
}

/// Walks down `parts` from `data`, refusing a link, a file, and a folder Observer never opens.
fn descend(data: &Path, parts: &[String]) -> Result<PathBuf, FolderProblem> {
    let mut folder = data.to_path_buf();
    for (depth, part) in parts.iter().enumerate() {
        folder.push(part);
        if withheld(&parts[..=depth]) {
            return Err(FolderProblem::Unreadable);
        }
        let metadata = fs::symlink_metadata(&folder).map_err(|error| match error.kind() {
            io::ErrorKind::NotFound => FolderProblem::Missing,
            _ => FolderProblem::Unreadable,
        })?;
        if metadata.file_type().is_symlink() {
            return Err(FolderProblem::Unreadable);
        }
        if !metadata.is_dir() {
            return Err(FolderProblem::NotAFolder);
        }
    }
    Ok(folder)
}

/// What the setup names and never opens: names that hold secrets anywhere, as file access and file
/// changes refuse them, and FXServer's `cache` at the root.
fn withheld(path: &[String]) -> bool {
    refused_by_name(path)
        || path
            .first()
            .is_some_and(|first| first.eq_ignore_ascii_case("cache"))
}

fn is_cfg(name: &str) -> bool {
    name.len() > 4
        && name
            .get(name.len() - 4..)
            .is_some_and(|end| end.eq_ignore_ascii_case(".cfg"))
}

/// The line `line` of `text`, cut at each secret: text between them, and an empty span where each
/// one was.
fn spans(text: &str, line: Range<usize>, secrets: &[Range<usize>]) -> Vec<CfgSpan> {
    let mut out = Vec::new();
    let mut at = line.start;
    for secret in secrets
        .iter()
        .filter(|secret| secret.start < line.end && secret.end > line.start)
    {
        let from = secret.start.max(line.start);
        if from > at {
            out.push(CfgSpan {
                text: text[at..from].to_owned(),
                secret: false,
            });
        }
        out.push(CfgSpan {
            text: String::new(),
            secret: true,
        });
        at = secret.end.min(line.end);
    }
    if at < line.end || out.is_empty() {
        out.push(CfgSpan {
            text: text[at..line.end].to_owned(),
            secret: false,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A server folder in a temporary directory, and an Observer data folder beside it.
    fn server() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("server-data");
        let observer = root.path().join("observer-data");
        fs::create_dir_all(data.join("resources/[local]/hello")).unwrap();
        fs::create_dir_all(data.join("configs")).unwrap();
        fs::create_dir_all(data.join("cache/files")).unwrap();
        fs::create_dir_all(data.join("txData/default")).unwrap();
        fs::create_dir_all(&observer).unwrap();
        fs::write(data.join("server.cfg"), "sv_hostname \"Lab\"\n").unwrap();
        fs::write(
            data.join("configs/perms.cfg"),
            "add_ace group.admin command allow\n",
        )
        .unwrap();
        fs::write(data.join(".env"), "TOKEN=1\n").unwrap();
        fs::write(data.join("README.md"), "hello\n").unwrap();
        (root, data, observer)
    }

    fn names(listing: &FileListing) -> Vec<(&str, FileEntryKind)> {
        listing
            .entries
            .iter()
            .map(|entry| (entry.name.as_str(), entry.kind))
            .collect()
    }

    #[test]
    fn a_folder_lists_its_folders_first_and_names_what_it_never_opens() {
        let (_root, data, observer) = server();

        let listing = list(data.to_str().unwrap(), "", &observer);

        assert_eq!(listing.problem, None);
        assert_eq!(
            names(&listing),
            [
                ("cache", FileEntryKind::Withheld),
                ("configs", FileEntryKind::Folder),
                ("resources", FileEntryKind::Folder),
                ("txData", FileEntryKind::Withheld),
                (".env", FileEntryKind::Withheld),
                ("README.md", FileEntryKind::File),
                ("server.cfg", FileEntryKind::Cfg),
            ]
        );
    }

    #[test]
    fn a_subfolder_lists_its_entries_by_their_path_from_the_server_folder() {
        let (_root, data, observer) = server();

        let listing = list(data.to_str().unwrap(), "configs", &observer);

        assert_eq!(listing.entries.len(), 1);
        assert_eq!(listing.entries[0].path, "configs/perms.cfg");
        assert_eq!(listing.entries[0].kind, FileEntryKind::Cfg);
    }

    #[test]
    fn a_path_that_climbs_out_or_into_a_withheld_folder_is_refused() {
        let (_root, data, observer) = server();
        let data = data.to_str().unwrap();

        for path in [
            "..",
            "configs/..",
            "/configs",
            "configs//x",
            "C:\\Windows",
            "a\\..\\b",
        ] {
            assert_eq!(
                list(data, path, &observer).problem,
                Some(FolderProblem::InvalidPath),
                "{path}"
            );
        }
        assert_eq!(
            list(data, "cache", &observer).problem,
            Some(FolderProblem::Unreadable)
        );
        assert_eq!(
            list(data, "txData/default", &observer).problem,
            Some(FolderProblem::Unreadable)
        );
        assert_eq!(
            list(data, "server.cfg", &observer).problem,
            Some(FolderProblem::NotAFolder)
        );
        assert_eq!(
            list(data, "missing", &observer).problem,
            Some(FolderProblem::Missing)
        );
    }

    #[test]
    fn observers_own_folder_is_neither_listed_into_nor_a_server_folder() {
        let (_root, data, observer) = server();
        let inside = observer.join("server");
        fs::create_dir_all(&inside).unwrap();

        let listing = list(inside.to_str().unwrap(), "", &observer);

        assert_eq!(listing.problem, Some(FolderProblem::InsideObserver));
        assert_eq!(
            list(data.to_str().unwrap(), "", &data).problem,
            Some(FolderProblem::InsideObserver)
        );
    }

    #[test]
    fn a_cfg_comes_line_by_line_with_its_secrets_left_out() {
        let (_root, data, observer) = server();
        fs::write(
            data.join("server.cfg"),
            "sv_hostname \"Lab\"\r\nsv_licenseKey \"cfxk_abcdefghijklmnopqrstu_12345\"\nset mysql_connection_string \"mysql://root:hunter2@localhost/db\"\n",
        )
        .unwrap();

        let shown = cfg_text(data.to_str().unwrap(), "server.cfg", &observer);

        assert_eq!(shown.problem, None);
        assert_eq!(shown.lines.len(), 3);
        assert_eq!(
            shown.lines[0],
            [CfgSpan {
                text: "sv_hostname \"Lab\"".into(),
                secret: false
            }]
        );
        let joined: String = shown
            .lines
            .iter()
            .flatten()
            .map(|span| span.text.as_str())
            .collect();
        assert!(!joined.contains("cfxk_"), "{joined}");
        assert!(!joined.contains("hunter2"), "{joined}");
        assert!(shown.lines[1].iter().any(|span| span.secret));
        assert!(shown.lines[2].iter().any(|span| span.secret));
    }

    #[test]
    fn a_cfg_the_rules_refuse_or_that_is_withheld_is_not_read() {
        let (_root, data, observer) = server();
        let data = data.to_str().unwrap();
        fs::write(Path::new(data).join("cache/files/x.cfg"), "sv_licenseKey x\n").unwrap();

        assert_eq!(
            cfg_text(data, "../server.cfg", &observer).problem,
            Some(CfgFileProblem::InvalidPath)
        );
        assert_eq!(
            cfg_text(data, "cache/files/x.cfg", &observer).problem,
            Some(CfgFileProblem::Unreadable)
        );
        assert_eq!(
            cfg_text(data, "missing.cfg", &observer).problem,
            Some(CfgFileProblem::Missing)
        );
        assert_eq!(
            cfg_text(data, "configs", &observer).problem,
            Some(CfgFileProblem::NotAFile)
        );
    }

    #[test]
    fn spans_cut_a_line_at_each_secret_and_keep_what_is_around() {
        let text = "set a \"x\" b \"y\"";
        let secrets = merged(vec![12..15, 6..9]);

        let out = spans(text, 0..text.len(), &secrets);

        assert_eq!(
            out.iter()
                .map(|span| (span.text.as_str(), span.secret))
                .collect::<Vec<_>>(),
            [("set a ", false), ("", true), (" b ", false), ("", true)]
        );
    }
}
