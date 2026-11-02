//! The host's folders as the setup's folder step shows them: the folders of one folder, each with
//! what it looks like from outside, and the servers found in the usual places. Both read names,
//! the walk of a server's `resources/` and what the folder check reads; the search also reads each
//! server's cfg as the cfg check does, and from txAdmin's data the server folder each profile
//! names. Both stop at a count of entries and at a deadline, and neither follows a link or a
//! junction it meets on the way.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use observer_domain::launch::{self, OneSync, Platform};
use observer_domain::server_data::{
    FolderEntry, FolderKind, FolderListing, FolderProblem, FolderQuery, FoundServer, PortHolder,
    ResourceSummary,
};
use serde::Deserialize;

use crate::cfg::{self, Context};
use crate::server_data::{self, SKIPPED};

/// Folders listed from one folder.
const MAX_LISTED: usize = 200;
/// Entries read from one folder, files included.
const MAX_READ: usize = 5000;
/// Folders counted inside a folder listed.
const MAX_COUNTED: u32 = 200;
const MAX_INSIDE: usize = 5;
/// Looking into the folders listed stops after this; the rest are listed by name alone.
const LOOK_BUDGET: Duration = Duration::from_millis(1500);
const MAX_SERVERS: usize = 24;
/// Entries the search reads across every folder, files included.
const MAX_SEARCHED: usize = 60_000;
/// A txAdmin profile's `config.json` is a few kilobytes.
const MAX_PROFILE_BYTES: u64 = 1024 * 1024;
const MAX_PROFILES: usize = 20;
/// Folders the search never goes into, on top of the walk's: program folders, an FXServer build's
/// own and the per-user application data.
const PASSED_OVER: [&str; 5] = ["appdata", "citizen", "alpine", "resources", "$recycle.bin"];
/// The system's own folders at the root of a drive or of the file system. `Users` is searched
/// through the home folder alone.
const SYSTEM_ROOTS: &[&str] = if cfg!(windows) {
    &[
        "windows",
        "program files",
        "program files (x86)",
        "programdata",
        "perflogs",
        "recovery",
        "system volume information",
        "users",
    ]
} else {
    &[
        "proc",
        "sys",
        "dev",
        "run",
        "boot",
        "usr",
        "lib",
        "lib32",
        "lib64",
        "libx32",
        "bin",
        "sbin",
        "etc",
        "snap",
        "tmp",
        "var",
        "lost+found",
    ]
};

/// Lists the folders in `query.path` whose name starts with its prefix, then looks into each
/// while time allows. `observer` is Observer's data folder: nothing in it is a server.
pub fn list(query: &FolderQuery, observer: &Path) -> FolderListing {
    let mut listing = FolderListing {
        path: query.path.clone(),
        parent: None,
        problem: None,
        path_problem: launch::folder_problem(&query.path, Platform::host()),
        entries: Vec::new(),
        truncated: false,
    };
    if listing.path_problem.is_some() {
        listing.problem = Some(FolderProblem::InvalidPath);
        return listing;
    }
    let folder = Path::new(&query.path);
    listing.parent = folder.parent().and_then(Path::to_str).map(str::to_owned);
    let read = match fs::read_dir(folder) {
        Ok(read) => read,
        Err(error) => {
            listing.problem = Some(match error.kind() {
                io::ErrorKind::NotFound => FolderProblem::Missing,
                _ if folder.is_file() => FolderProblem::NotAFolder,
                _ => FolderProblem::Unreadable,
            });
            return listing;
        }
    };
    let prefix = query.prefix.to_lowercase();
    let mut found: Vec<(String, PathBuf, bool)> = read
        .take(MAX_READ)
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            if !name.to_lowercase().starts_with(&prefix) || (prefix.is_empty() && hidden(&entry, &name)) {
                return None;
            }
            let link = folder_link(&entry)?;
            Some((name, entry.path(), link))
        })
        .collect();
    found.sort_by_cached_key(|(name, _, _)| name.to_lowercase());
    listing.truncated = found.len() > MAX_LISTED;
    found.truncate(MAX_LISTED);
    let observer = fs::canonicalize(observer).ok();
    let deadline = Instant::now() + LOOK_BUDGET;
    listing.entries = found
        .into_iter()
        .filter_map(|(name, path, link)| {
            let path_text = path.to_str()?.to_owned();
            let mut entry = FolderEntry {
                name,
                path: path_text,
                kind: FolderKind::Folder,
                link,
                resources: None,
                cfg: None,
                folders: None,
                inside: Vec::new(),
            };
            if Instant::now() < deadline {
                look(&mut entry, &path, observer.as_deref(), Some(deadline));
            }
            Some(entry)
        })
        .collect();
    listing
}

/// What `entry` is, and for a plain folder what it holds, if `deadline` is given and not past.
fn look(entry: &mut FolderEntry, path: &Path, observer: Option<&Path>, deadline: Option<Instant>) {
    let Ok(read) = fs::read_dir(path) else {
        entry.kind = FolderKind::Unreadable;
        return;
    };
    if tx_admin_data(path) {
        entry.kind = FolderKind::TxAdminData;
    } else if let Some(resources) = server(path, observer) {
        entry.kind = FolderKind::Server;
        entry.resources = Some(resources.summary.resources);
        entry.cfg = server_data::cfg_files(path).1;
        return;
    }
    let Some(deadline) = deadline else {
        return;
    };
    let mut folders = 0;
    for child in read.take(MAX_READ).flatten() {
        let Some(name) = child.file_name().into_string().ok() else {
            continue;
        };
        if hidden(&child, &name) || folder_link(&child).is_none() {
            continue;
        }
        folders += 1;
        if entry.inside.len() < MAX_INSIDE && Instant::now() < deadline {
            let path = child.path();
            let tx_admin = tx_admin_data(&path);
            let resources = if tx_admin { None } else { server(&path, observer) };
            if (tx_admin || resources.is_some())
                && let Some(text) = path.to_str()
            {
                entry.inside.push(FolderEntry {
                    name,
                    path: text.to_owned(),
                    kind: if tx_admin {
                        FolderKind::TxAdminData
                    } else {
                        FolderKind::Server
                    },
                    link: false,
                    resources: resources.as_ref().map(|found| found.summary.resources),
                    cfg: resources
                        .is_some()
                        .then(|| server_data::cfg_files(&path).1)
                        .flatten(),
                    folders: None,
                    inside: Vec::new(),
                });
            }
        }
        if folders == MAX_COUNTED {
            break;
        }
    }
    entry.folders = Some(folders);
}

/// For an entry that leads to a folder, whether it does through a link or a junction.
fn folder_link(entry: &fs::DirEntry) -> Option<bool> {
    let kind = entry.file_type().ok()?;
    if kind.is_symlink() {
        return fs::metadata(entry.path())
            .is_ok_and(|metadata| metadata.is_dir())
            .then_some(true);
    }
    kind.is_dir().then_some(false)
}

/// Folders a file browser hides: dotted ones, and on Windows the hidden ones.
fn hidden(entry: &fs::DirEntry, name: &str) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
        name.starts_with('.')
            || entry
                .metadata()
                .is_ok_and(|metadata| metadata.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0)
    }
    #[cfg(unix)]
    {
        _ = entry;
        name.starts_with('.')
    }
}

/// The resources of a folder FXServer can run from, outside Observer's data folder.
fn server(path: &Path, observer: Option<&Path>) -> Option<server_data::Resources> {
    if !path.join("resources").is_dir() {
        return None;
    }
    server_data::inspect(path, observer).ok()
}

/// txAdmin's data folder: its `admins.json`, or the `config.json` of its default profile.
fn tx_admin_data(path: &Path) -> bool {
    path.join("admins.json").is_file() || path.join("default").join("config.json").is_file()
}

/// Where the search looks: each root down to its depth, the first ones first, and Observer's
/// data folder it leaves out.
#[derive(Debug, Clone)]
pub struct Search {
    /// A folder and how many folders down to look: the home folder deeper than a drive.
    pub roots: Vec<(PathBuf, usize)>,
    pub observer: PathBuf,
    /// Observer's own FXServer, whose port is taken from nobody.
    pub own_pids: Vec<u32>,
    pub budget: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub servers: Vec<FoundServer>,
    /// The search stopped at a limit before it was through.
    pub truncated: bool,
}

/// Searches the roots for server folders and txAdmin's data, then reads each server's cfg and who
/// listens on its port. It stops at 24 servers, 60,000 entries or the budget.
pub fn discover(search: &Search) -> Found {
    let deadline = Instant::now() + search.budget;
    let observer = fs::canonicalize(&search.observer).ok();
    let Swept {
        mut servers,
        named,
        mut truncated,
    } = sweep(&search.roots, observer.as_deref(), deadline);
    let mut used: HashSet<PathBuf> = HashSet::new();
    let mut known: HashSet<PathBuf> = servers.iter().filter_map(|(path, _)| canonical(path)).collect();
    for path in named {
        let Some(resolved) = canonical(&path) else {
            continue;
        };
        used.insert(resolved.clone());
        if known.contains(&resolved) {
            continue;
        }
        if servers.len() >= MAX_SERVERS {
            truncated = true;
            break;
        }
        if let Some(resources) = server(&path, observer.as_deref()) {
            known.insert(resolved);
            servers.push((path, resources.summary));
        }
    }
    let mut holders: HashMap<u16, Option<PortHolder>> = HashMap::new();
    let mut found: Vec<FoundServer> = servers
        .into_iter()
        .filter_map(|(path, resources)| {
            let text = path.to_str()?.to_owned();
            if launch::folder_problem(&text, Platform::host()).is_some() {
                return None;
            }
            let (_, cfg) = server_data::cfg_files(&path);
            let scan = cfg.as_deref().and_then(|cfg| facts(&path, cfg));
            let port = scan.and_then(|(_, port)| port);
            let tx_admin = canonical(&path).is_some_and(|path| used.contains(&path));
            let port_holder = port.and_then(|port| {
                holders
                    .entry(port)
                    .or_insert_with(|| holder(port, &search.own_pids))
                    .clone()
            });
            Some(FoundServer {
                path: text,
                resources: resources.resources,
                resources_truncated: resources.truncated,
                cfg,
                onesync: scan.and_then(|(onesync, _)| onesync),
                tx_admin,
                port,
                port_holder,
            })
        })
        .collect();
    found.sort_by_cached_key(|server| (!server.tx_admin, server.path.to_lowercase()));
    Found {
        servers: found,
        truncated,
    }
}

struct Swept {
    servers: Vec<(PathBuf, ResourceSummary)>,
    /// The server folders txAdmin's profiles name.
    named: Vec<PathBuf>,
    truncated: bool,
}

/// Goes through the roots breadth first, the first root's folders before the next root's at each
/// depth, and stops going down at a server folder.
fn sweep(roots: &[(PathBuf, usize)], observer: Option<&Path>, deadline: Instant) -> Swept {
    let mut queue: VecDeque<(PathBuf, usize, usize)> = VecDeque::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    for (root, depth) in roots {
        if seen.insert(folded(root)) {
            queue.push_back((root.clone(), 0, *depth));
        }
    }
    let mut swept = Swept {
        servers: Vec::new(),
        named: Vec::new(),
        truncated: false,
    };
    let mut read = 0;
    'sweep: while let Some((folder, depth, max)) = queue.pop_front() {
        if Instant::now() >= deadline {
            swept.truncated = true;
            break;
        }
        let Ok(entries) = fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.take(MAX_READ).flatten() {
            read += 1;
            if read > MAX_SEARCHED {
                swept.truncated = true;
                break 'sweep;
            }
            // Links and junctions are not followed: they loop, or lead where the search goes anyway.
            if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                continue;
            }
            let Some(name) = entry.file_name().into_string().ok() else {
                continue;
            };
            let lower = name.to_lowercase();
            if hidden(&entry, &name)
                || SKIPPED.contains(&lower.as_str())
                || PASSED_OVER.contains(&lower.as_str())
                || (depth == 0 && folder.parent().is_none() && SYSTEM_ROOTS.contains(&lower.as_str()))
            {
                continue;
            }
            let path = entry.path();
            if !seen.insert(folded(&path)) {
                continue;
            }
            if path.join("resources").is_dir() {
                if let Some(resources) = server(&path, observer) {
                    if swept.servers.len() == MAX_SERVERS {
                        swept.truncated = true;
                        break 'sweep;
                    }
                    swept.servers.push((path, resources.summary));
                }
                continue;
            }
            if tx_admin_data(&path) {
                swept.named.extend(tx_admin_servers(&path));
            }
            if depth + 1 < max {
                queue.push_back((path, depth + 1, max));
            }
        }
    }
    swept
}

/// The OneSync the cfg sets and the port of its first TCP endpoint, as the cfg check reads them.
fn facts(data: &Path, cfg_file: &str) -> Option<(Option<OneSync>, Option<u16>)> {
    if launch::cfg_problem(cfg_file, Platform::host()).is_some() {
        return None;
    }
    let text = cfg::read(&data.join(cfg_file)).ok()?;
    let context = Context {
        panel: None,
        onesync: OneSync::On,
        probe_ports: false,
        system_resources: None,
        follow_outside: false,
    };
    let scan = cfg::scan(data, cfg_file, &text, &server_data::walk(data), &context);
    Some((scan.facts.onesync, scan.endpoint.map(|endpoint| endpoint.port())))
}

/// Who listens on `port`, Observer's own FXServer aside. Reading the system's tables binds nothing.
fn holder(port: u16, ours: &[u32]) -> Option<PortHolder> {
    let listener = observer_platform::tcp_listeners(port)
        .ok()?
        .into_iter()
        .find(|listener| listener.pid.is_none_or(|pid| !ours.contains(&pid)))?;
    let program = listener
        .executable
        .as_deref()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .map(str::to_owned);
    Some(PortHolder {
        fx_server: listener.executable.as_deref().is_some_and(fx_server),
        program,
    })
}

/// FXServer's executable, or on Linux the loader of its build that runs it.
fn fx_server(executable: &Path) -> bool {
    let name = executable
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    name.eq_ignore_ascii_case("FXServer.exe")
        || name == "FXServer"
        || (name.starts_with("ld-musl-")
            && executable
                .parent()
                .and_then(Path::file_name)
                .is_some_and(|parent| parent == "cfx-server"))
}

#[derive(Deserialize)]
struct Profile {
    server: Option<ProfileServer>,
    /// Before txAdmin 8.
    #[serde(rename = "fxRunner")]
    fx_runner: Option<OldRunner>,
}

#[derive(Deserialize)]
struct ProfileServer {
    #[serde(rename = "dataPath")]
    data_path: Option<String>,
}

#[derive(Deserialize)]
struct OldRunner {
    #[serde(rename = "serverDataPath")]
    server_data_path: Option<String>,
}

/// The server folder each profile of txAdmin's data names in its `config.json`. Nothing else of
/// the file is kept: it holds txAdmin's secrets.
fn tx_admin_servers(folder: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(folder) else {
        return Vec::new();
    };
    entries
        .take(MAX_READ)
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| {
            let config = entry.path().join("config.json");
            let metadata = fs::metadata(&config).ok()?;
            if !metadata.is_file() || metadata.len() > MAX_PROFILE_BYTES {
                return None;
            }
            let profile: Profile = serde_json::from_slice(&fs::read(&config).ok()?).ok()?;
            let path = profile
                .server
                .and_then(|server| server.data_path)
                .or_else(|| profile.fx_runner.and_then(|runner| runner.server_data_path))?;
            // txAdmin writes `C:/x` as often as `C:\x`; the components give the host's own form.
            let path: PathBuf = Path::new(&path).components().collect();
            path.is_absolute().then_some(path)
        })
        .take(MAX_PROFILES)
        .collect()
}

/// The path the file system resolves, as Windows compares it.
fn canonical(path: &Path) -> Option<PathBuf> {
    fs::canonicalize(path).ok().map(|path| folded(&path))
}

/// Windows compares paths without regard to case.
fn folded(path: &Path) -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(path.to_string_lossy().to_lowercase())
    } else {
        path.to_path_buf()
    }
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;

    use observer_domain::launch::LaunchProblem;
    use tempfile::TempDir;

    use super::*;

    /// A server folder holding `chat` and a cfg on `port`.
    fn server(folder: &Path, port: u16) {
        let chat = folder.join("resources").join("chat");
        fs::create_dir_all(&chat).unwrap();
        fs::write(chat.join("fxmanifest.lua"), "fx_version 'cerulean'\n").unwrap();
        fs::write(
            folder.join("server.cfg"),
            format!(
                "endpoint_add_tcp \"0.0.0.0:{port}\"\nendpoint_add_udp \"0.0.0.0:{port}\"\nset onesync on\n"
            ),
        )
        .unwrap();
    }

    /// txAdmin's data with a profile naming `data` as its server, as txAdmin 8 writes it.
    fn tx_data(folder: &Path, data: &str) {
        let profile = folder.join("default");
        fs::create_dir_all(profile.join("data")).unwrap();
        fs::write(folder.join("admins.json"), "[]").unwrap();
        let config = serde_json::json!({
            "version": 2,
            "general": { "serverName": "lab" },
            "server": { "dataPath": data, "cfgPath": "server.cfg" },
            "discordBot": { "token": "not read" },
        });
        fs::write(profile.join("config.json"), config.to_string()).unwrap();
    }

    fn text(path: &Path) -> String {
        path.to_str().unwrap().to_owned()
    }

    /// The folder `path` listed with a trailing separator, as the panel asks for it.
    fn listed(path: &Path, prefix: &str, observer: &Path) -> FolderListing {
        let query = FolderQuery {
            path: format!("{}{}", text(path), std::path::MAIN_SEPARATOR),
            prefix: prefix.to_owned(),
        };
        list(&query, observer)
    }

    fn names(listing: &FolderListing) -> Vec<&str> {
        listing.entries.iter().map(|entry| entry.name.as_str()).collect()
    }

    /// A port nothing listens on: bound, then let go.
    fn free_port() -> u16 {
        TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    #[test]
    fn the_folders_that_start_with_the_prefix_are_listed_by_name_with_what_each_is() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        server(&root.join("Sensity").join("FXServer"), free_port());
        tx_data(&root.join("Sensity").join("txData"), "C:/elsewhere");
        fs::create_dir_all(root.join("Sensity").join("docs")).unwrap();
        fs::create_dir_all(root.join("seed").join("one")).unwrap();
        fs::write(root.join("seed").join("notes.txt"), "").unwrap();
        fs::write(root.join("Sensei.txt"), "a file").unwrap();
        fs::create_dir_all(root.join("other")).unwrap();

        let listing = listed(root, "SE", &root.join("observer-data"));

        assert_eq!(listing.problem, None);
        assert_eq!(listing.parent.as_deref(), root.parent().and_then(Path::to_str));
        assert_eq!(names(&listing), ["seed", "Sensity"]);
        assert!(!listing.truncated);
        let seed = &listing.entries[0];
        assert_eq!(
            (seed.kind, seed.folders, seed.inside.len()),
            (FolderKind::Folder, Some(1), 0)
        );
        let sensity = &listing.entries[1];
        assert_eq!(sensity.path, text(&root.join("Sensity")));
        assert_eq!(sensity.folders, Some(3));
        let inside: Vec<_> = sensity
            .inside
            .iter()
            .map(|entry| {
                (
                    entry.name.as_str(),
                    entry.kind,
                    entry.resources,
                    entry.cfg.as_deref(),
                )
            })
            .collect();
        assert!(inside.contains(&("FXServer", FolderKind::Server, Some(1), Some("server.cfg"))));
        assert!(inside.contains(&("txData", FolderKind::TxAdminData, None, None)));
        assert_eq!(inside.len(), 2);

        let inner = listed(&root.join("Sensity"), "", &root.join("observer-data"));
        let kinds: Vec<_> = inner
            .entries
            .iter()
            .map(|entry| (entry.name.as_str(), entry.kind))
            .collect();
        assert_eq!(
            kinds,
            [
                ("docs", FolderKind::Folder),
                ("FXServer", FolderKind::Server),
                ("txData", FolderKind::TxAdminData)
            ]
        );
    }

    #[test]
    fn hidden_folders_are_listed_only_once_a_prefix_names_them() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".git")).unwrap();
        fs::create_dir_all(dir.path().join("live")).unwrap();
        let observer = dir.path().join("observer-data");

        assert_eq!(names(&listed(dir.path(), "", &observer)), ["live"]);
        assert_eq!(names(&listed(dir.path(), ".", &observer)), [".git"]);
    }

    #[test]
    fn a_server_inside_observers_data_folder_is_no_server() {
        let dir = tempfile::tempdir().unwrap();
        let observer = dir.path().join("observer-data");
        server(&observer.join("server-data"), free_port());

        let listing = listed(&observer, "", &observer);

        assert_eq!(listing.entries[0].kind, FolderKind::Folder);
    }

    #[test]
    fn the_listing_stops_at_200_folders_and_the_count_inside_at_200() {
        let dir = tempfile::tempdir().unwrap();
        for index in 0..=MAX_LISTED {
            fs::create_dir_all(dir.path().join(format!("f{index:03}"))).unwrap();
        }
        for index in 0..=MAX_COUNTED {
            fs::create_dir_all(dir.path().join("f000").join(format!("g{index:03}"))).unwrap();
        }

        let listing = listed(dir.path(), "f", &dir.path().join("observer-data"));

        assert_eq!(listing.entries.len(), 200);
        assert!(listing.truncated);
        assert_eq!(listing.entries[0].folders, Some(200));
        assert_eq!(listing.entries[199].name, "f199");
    }

    #[test]
    fn a_folder_that_cannot_be_listed_says_why_and_a_network_path_is_never_opened() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("notes.txt"), "").unwrap();
        let observer = dir.path().join("observer-data");
        let problem = |path: &str| {
            let listing = list(
                &FolderQuery {
                    path: path.to_owned(),
                    prefix: String::new(),
                },
                &observer,
            );
            (listing.problem, listing.path_problem)
        };

        assert_eq!(
            problem("server-data"),
            (Some(FolderProblem::InvalidPath), Some(LaunchProblem::NotAbsolute))
        );
        assert_eq!(
            problem(&text(&dir.path().join("nowhere"))),
            (Some(FolderProblem::Missing), None)
        );
        assert_eq!(
            problem(&text(&dir.path().join("notes.txt"))),
            (Some(FolderProblem::NotAFolder), None)
        );
        if cfg!(windows) {
            assert_eq!(
                problem(r"\\lab-host\share\"),
                (Some(FolderProblem::InvalidPath), Some(LaunchProblem::NetworkPath))
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_observer_cannot_list_is_unreadable_and_a_link_says_so() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let locked = dir.path().join("locked");
        fs::create_dir_all(&locked).unwrap();
        fs::create_dir_all(dir.path().join("target")).unwrap();
        std::os::unix::fs::symlink(dir.path().join("target"), dir.path().join("shortcut")).unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        let listable = fs::read_dir(&locked).is_ok();

        let listing = listed(dir.path(), "", &dir.path().join("observer-data"));

        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        let entry = |name: &str| listing.entries.iter().find(|entry| entry.name == name).unwrap();
        // Root lists any folder.
        if !listable {
            assert_eq!(entry("locked").kind, FolderKind::Unreadable);
        }
        assert!(entry("shortcut").link);
        assert!(!entry("target").link);
    }

    #[cfg(windows)]
    #[test]
    fn a_junction_is_listed_as_a_link_and_never_searched_through() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        server(&target.join("live"), free_port());
        let junction = dir.path().join("shortcut");
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&junction)
            .arg(&target)
            .output()
            .unwrap();
        assert!(made.status.success(), "{made:?}");

        let listing = listed(dir.path(), "", &dir.path().join("observer-data"));
        let found = discover(&Search {
            roots: vec![(dir.path().to_path_buf(), 3)],
            observer: dir.path().join("observer-data"),
            own_pids: Vec::new(),
            budget: Duration::from_secs(30),
        });

        let links: Vec<_> = listing
            .entries
            .iter()
            .map(|entry| (entry.name.as_str(), entry.link))
            .collect();
        assert_eq!(links, [("shortcut", true), ("target", false)]);
        assert_eq!(paths(&found), [text(&target.join("live"))]);
    }

    struct Machine {
        dir: TempDir,
    }

    impl Machine {
        fn path(&self, parts: &[&str]) -> PathBuf {
            parts
                .iter()
                .fold(self.dir.path().to_path_buf(), |path, part| path.join(part))
        }

        fn search(&self, roots: &[(&[&str], usize)]) -> Search {
            Search {
                roots: roots
                    .iter()
                    .map(|(parts, depth)| (self.path(parts), *depth))
                    .collect(),
                observer: self.path(&["observer-data"]),
                own_pids: Vec::new(),
                budget: Duration::from_secs(30),
            }
        }
    }

    fn machine() -> Machine {
        Machine {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn paths(found: &Found) -> Vec<String> {
        found.servers.iter().map(|server| server.path.clone()).collect()
    }

    #[test]
    fn servers_are_found_down_to_each_roots_depth_and_not_in_the_folders_passed_over() {
        let machine = machine();
        let port = free_port();
        for parts in [
            &["drive", "FiveM", "lab-server"][..],
            &["home", "Documents", "Dev", "Sensity", "FXServer"],
            &["drive", "a", "b", "c", "too-deep"],
            &["drive", "node_modules", "pkg"],
            &["drive", ".cache", "hidden"],
            &["drive", "FiveM", "lab-server", "nested"],
            &["observer-data", "server-data"],
        ] {
            server(&machine.path(parts), port);
        }

        let found = discover(&machine.search(&[(&["home"], 4), (&["drive"], 3), (&["observer-data"], 2)]));

        assert_eq!(
            paths(&found),
            [
                text(&machine.path(&["drive", "FiveM", "lab-server"])),
                text(&machine.path(&["home", "Documents", "Dev", "Sensity", "FXServer"])),
            ]
        );
        assert!(!found.truncated);
        let lab = &found.servers[0];
        assert_eq!((lab.resources, lab.resources_truncated), (1, false));
        assert_eq!(lab.cfg.as_deref(), Some("server.cfg"));
        assert_eq!(lab.onesync, Some(OneSync::On));
        assert_eq!(lab.port, Some(port));
        assert_eq!(lab.port_holder, None);
        assert!(!lab.tx_admin);
    }

    #[test]
    fn the_servers_txadmins_profiles_name_are_found_and_marked_wherever_they_are() {
        let machine = machine();
        server(&machine.path(&["drive", "Sensity", "FXServer"]), free_port());
        server(&machine.path(&["elsewhere", "deep", "live"]), free_port());
        server(&machine.path(&["drive", "Sensity", "lab"]), free_port());
        // txAdmin writes forward slashes on Windows too.
        let named = text(&machine.path(&["drive", "Sensity", "FXServer"])).replace('\\', "/");
        tx_data(&machine.path(&["drive", "Sensity", "txData"]), &named);
        let profile = machine.path(&["drive", "Sensity", "txData", "second"]);
        fs::create_dir_all(&profile).unwrap();
        let old = serde_json::json!({
            "fxRunner": { "serverDataPath": text(&machine.path(&["elsewhere", "deep", "live"])) },
        });
        fs::write(profile.join("config.json"), old.to_string()).unwrap();

        let found = discover(&machine.search(&[(&["drive"], 3)]));

        let marked: Vec<(String, bool)> = found
            .servers
            .iter()
            .map(|server| (server.path.clone(), server.tx_admin))
            .collect();
        assert_eq!(
            marked,
            [
                (text(&machine.path(&["drive", "Sensity", "FXServer"])), true),
                (text(&machine.path(&["elsewhere", "deep", "live"])), true),
                (text(&machine.path(&["drive", "Sensity", "lab"])), false),
            ]
        );
    }

    #[test]
    fn a_program_listening_on_a_servers_port_is_named_unless_it_is_observers_fxserver() {
        let machine = machine();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        server(&machine.path(&["drive", "live"]), port);
        let mut search = machine.search(&[(&["drive"], 2)]);
        let own = std::env::current_exe().unwrap();

        let held = discover(&search);
        search.own_pids = vec![std::process::id()];
        let ours = discover(&search);

        assert_eq!(
            held.servers[0].port_holder,
            Some(PortHolder {
                program: own.file_name().and_then(|name| name.to_str()).map(str::to_owned),
                fx_server: false,
            })
        );
        assert_eq!(ours.servers[0].port_holder, None);
    }

    #[test]
    fn the_search_stops_at_24_servers_and_at_its_budget() {
        let machine = machine();
        for index in 0..=MAX_SERVERS {
            server(&machine.path(&["drive", &format!("s{index:02}")]), free_port());
        }
        let mut search = machine.search(&[(&["drive"], 3)]);

        let found = discover(&search);
        search.budget = Duration::ZERO;
        let none = discover(&search);

        assert_eq!(found.servers.len(), 24);
        assert!(found.truncated);
        assert_eq!((none.servers.len(), none.truncated), (0, true));
    }

    #[test]
    fn fxserver_is_known_by_its_executable_or_its_builds_loader() {
        // A path is read the host's way: Linux takes `C:\x\y` as one name.
        if cfg!(windows) {
            assert!(fx_server(Path::new(r"C:\builds\37900\FXServer.exe")));
            assert!(!fx_server(Path::new(r"C:\Program Files\nodejs\node.exe")));
        } else {
            assert!(fx_server(Path::new(
                "/srv/fx/alpine/opt/cfx-server/ld-musl-x86_64.so.1"
            )));
            assert!(!fx_server(Path::new("/usr/lib/ld-musl-x86_64.so.1")));
            assert!(!fx_server(Path::new("/usr/bin/node")));
        }
    }
}
