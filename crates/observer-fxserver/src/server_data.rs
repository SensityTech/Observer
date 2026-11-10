//! The server data folder as FXServer reads it: the resources it finds under `resources/`, and the
//! setup's checks of a folder and of its cfg, which read and never write. Observer writes into the
//! folder only SFTP uploads and the fixes of the server tools the owner runs or turns on.

use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use observer_domain::convars::CfgConvars;
use observer_domain::launch::{self, LaunchSettings, Platform};
use observer_domain::server_data::{CfgFileProblem, CfgReport, FolderProblem, FolderReport, ResourceSummary};

use crate::cfg::{self, Context, ReadError};

const MAX_CATEGORIES: u32 = 250;
/// Entries the walk reads, files included: telling a folder from a file costs a look at each.
const MAX_WALKED: usize = 5000;
/// Resources without a manifest named in a summary.
const MAX_UNMANIFESTED: usize = 20;
/// Folders FXServer's walk passes over, as txAdmin's does.
pub(crate) const SKIPPED: [&str; 6] = ["cache", "db", "node_modules", ".git", ".idea", ".vscode"];
const MANIFESTS: [&str; 2] = ["fxmanifest.lua", "__resource.lua"];
/// Entries read from one folder when looking around a refused folder or for cfg files.
const MAX_ENTRIES: usize = 200;
const MAX_CFG_FILES: usize = 50;
/// txAdmin's guesses at the cfg's name, in its order.
const CFG_GUESSES: [&str; 5] = [
    "server.cfg",
    "server.cfg.txt",
    "server.cfg.cfg",
    "server.txt",
    "server",
];

/// What the walk of `resources/` found. Names are kept in lower case: a cfg that starts `Chat` for
/// a folder `chat` is left to FXServer to judge, not reported.
#[derive(Debug, Clone, Default)]
pub struct Resources {
    names: HashSet<String>,
    categories: HashSet<String>,
    /// Resources with their manifest.
    manifested: u32,
    /// The categories each resource with a manifest lies under, outermost first: FXServer starts
    /// those and no other folder. Of two folders of one name, the first found.
    under: HashMap<String, Vec<String>>,
    pub summary: ResourceSummary,
}

impl Resources {
    /// Whether `ensure <name>` finds a resource or a category of the folder.
    pub fn holds(&self, name: &str) -> bool {
        let name = name.to_ascii_lowercase();
        self.names.contains(&name) || self.categories.contains(&name)
    }

    /// The resources `ensure <target>` starts, in lower case and in name order: the one of that
    /// name, or every one under the category `[target]`. None when the folder holds no such
    /// resource, as for one of the build's own.
    pub fn started_by(&self, target: &str) -> Vec<String> {
        let target = target.to_ascii_lowercase();
        let mut started: Vec<String> = if target.starts_with('[') && target.ends_with(']') {
            self.under
                .iter()
                .filter(|(_, categories)| categories.contains(&target))
                .map(|(name, _)| name.clone())
                .collect()
        } else {
            self.under
                .get_key_value(&target)
                .map(|(name, _)| name.clone())
                .into_iter()
                .collect()
        };
        started.sort_unstable();
        started
    }

    /// How many resources hold `fxmanifest.lua` or `__resource.lua`.
    pub fn manifested(&self) -> u32 {
        self.manifested
    }
}

/// Walks `resources/` of `data` the way FXServer does: a folder named `[x]` is a category, read in
/// turn, and any other folder a resource. It stops at 250 categories or 5,000 entries; a folder it
/// cannot read holds nothing.
pub fn walk(data: &Path) -> Resources {
    let mut resources = Resources::default();
    let mut queue = VecDeque::from([(data.join("resources"), "resources".to_owned())]);
    let mut walked = 0;
    'walk: while let Some((folder, relative)) = queue.pop_front() {
        let Ok(entries) = fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            walked += 1;
            if walked > MAX_WALKED {
                resources.summary.truncated = true;
                break 'walk;
            }
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if SKIPPED.contains(&name.as_str()) {
                continue;
            }
            let lower = name.to_ascii_lowercase();
            if name.starts_with('[') && name.ends_with(']') {
                if resources.summary.categories == MAX_CATEGORIES {
                    resources.summary.truncated = true;
                    continue;
                }
                queue.push_back((path, format!("{relative}/{name}")));
                resources.categories.insert(lower);
                resources.summary.categories += 1;
                continue;
            }
            resources.summary.resources += 1;
            let manifest = MANIFESTS.iter().any(|manifest| path.join(manifest).is_file());
            if manifest {
                resources.manifested += 1;
                resources
                    .under
                    .entry(lower.clone())
                    .or_insert_with(|| relative.split('/').skip(1).map(str::to_ascii_lowercase).collect());
            }
            if !manifest && resources.summary.without_manifest.len() < MAX_UNMANIFESTED {
                resources.summary.without_manifest.push(name.clone());
            }
            resources.names.insert(lower);
        }
    }
    resources
}

/// Checks `path` as the folder of a server. A refused folder comes with a folder nearby that would
/// do, when there is one; an accepted one with its resources and cfg files. `observer` is
/// Observer's data folder, which FXServer must stay out of.
pub fn check_folder(path: &str, observer: &Path) -> FolderReport {
    let mut report = FolderReport {
        problem: None,
        path_problem: launch::folder_problem(path, Platform::host()),
        suggestion: None,
        resources: None,
        cfg_files: Vec::new(),
        detected_cfg: None,
    };
    if report.path_problem.is_some() {
        report.problem = Some(FolderProblem::InvalidPath);
        return report;
    }
    let observer = fs::canonicalize(observer).ok();
    let folder = Path::new(path);
    match inspect(folder, observer.as_deref()) {
        Ok(resources) => {
            let (cfg_files, detected) = cfg_files(folder);
            report.resources = Some(resources.summary);
            report.cfg_files = cfg_files;
            report.detected_cfg = detected;
        }
        Err(problem) => {
            report.problem = Some(problem);
            report.suggestion = suggest(folder, observer.as_deref());
        }
    }
    report
}

/// Checks the cfg of a server folder, after the folder itself, and every file it runs.
pub fn check_cfg(data_path: &str, cfg_file: &str, observer: &Path, context: &Context) -> CfgReport {
    let mut report = CfgReport {
        folder: None,
        problem: None,
        path_problem: None,
        files: Vec::new(),
        facts: None,
        findings: Vec::new(),
    };
    let platform = Platform::host();
    let observer = fs::canonicalize(observer).ok();
    let data = Path::new(data_path);
    let inspected = match launch::folder_problem(data_path, platform) {
        Some(_) => Err(FolderProblem::InvalidPath),
        None => inspect(data, observer.as_deref()),
    };
    let resources = match inspected {
        Ok(resources) => resources,
        Err(problem) => {
            report.folder = Some(problem);
            return report;
        }
    };
    report.path_problem = launch::cfg_problem(cfg_file, platform);
    if report.path_problem.is_some() {
        report.problem = Some(CfgFileProblem::InvalidPath);
        return report;
    }
    let text = match cfg::read(&data.join(cfg_file)) {
        Ok(text) => text,
        Err(error) => {
            report.problem = Some(read_problem(&error));
            return report;
        }
    };
    let scan = cfg::scan(data, cfg_file, &text, &resources, context);
    report.files = scan.files;
    report.facts = Some(scan.facts);
    report.findings = scan.findings;
    report
}

/// What the launch's cfg and the files it runs set, for the launch settings. Only files inside the
/// data folder are read, as the setup reads them; a setting in another one is not listed.
pub fn cfg_convars(settings: &LaunchSettings) -> CfgConvars {
    let platform = Platform::host();
    let mut convars = CfgConvars {
        cfg_file: settings.cfg_file.clone(),
        problem: None,
        convars: Vec::new(),
        complete: false,
    };
    let refused = launch::folder_problem(&settings.data_path, platform).is_some()
        || launch::cfg_problem(&settings.cfg_file, platform).is_some();
    if refused {
        convars.problem = Some(CfgFileProblem::InvalidPath);
        return convars;
    }
    let data = Path::new(&settings.data_path);
    let text = match cfg::read(&data.join(&settings.cfg_file)) {
        Ok(text) => text,
        Err(error) => {
            convars.problem = Some(read_problem(&error));
            return convars;
        }
    };
    let context = Context {
        panel: None,
        onesync: settings.onesync,
        probe_ports: false,
        system_resources: None,
        follow_outside: false,
        convars: Some(
            settings
                .extra_arguments
                .iter()
                .map(|argument| argument.name.clone())
                .collect(),
        ),
    };
    let scan = cfg::scan(data, &settings.cfg_file, &text, &Resources::default(), &context);
    convars.convars = scan.convars;
    convars.complete = scan.convars_complete;
    convars
}

fn read_problem(error: &ReadError) -> CfgFileProblem {
    match error {
        ReadError::Io(error) if error.kind() == io::ErrorKind::NotFound => CfgFileProblem::Missing,
        ReadError::Io(_) => CfgFileProblem::Unreadable,
        ReadError::NotAFile => CfgFileProblem::NotAFile,
        ReadError::TooLarge => CfgFileProblem::TooLarge,
    }
}

/// The first problem of a folder whose path the rules accept, in the order they are checked, or
/// else the resources it holds.
pub(crate) fn inspect(folder: &Path, observer: Option<&Path>) -> Result<Resources, FolderProblem> {
    let metadata = fs::metadata(folder).map_err(|error| match error.kind() {
        io::ErrorKind::NotFound => FolderProblem::Missing,
        _ => FolderProblem::Unreadable,
    })?;
    if !metadata.is_dir() {
        return Err(FolderProblem::NotAFolder);
    }
    fs::read_dir(folder).map_err(|_| FolderProblem::Unreadable)?;
    let resolved = fs::canonicalize(folder).map_err(|_| FolderProblem::Unreadable)?;
    if observer.is_some_and(|observer| resolved.starts_with(observer)) {
        return Err(FolderProblem::InsideObserver);
    }
    let in_server = folder.file_name().is_some_and(is_resources)
        && folder
            .parent()
            .is_some_and(|parent| walk(parent).summary.resources > 0);
    if in_server {
        return Err(FolderProblem::IsResources);
    }
    if !folder.join("resources").is_dir() {
        return Err(FolderProblem::NoResources);
    }
    let resources = walk(folder);
    if resources.summary.resources == 0 {
        return Err(FolderProblem::EmptyResources);
    }
    Ok(resources)
}

/// A folder near a refused one that passes every check and holds a server, a resource with its
/// manifest and a cfg: its parent, the parent's other children, the path cut before a `resources`
/// component, then its own children. A web app's `resources` folder is no server.
fn suggest(folder: &Path, observer: Option<&Path>) -> Option<String> {
    let parent = folder.parent().filter(|parent| parent.file_name().is_some());
    let siblings = parent
        .into_iter()
        .flat_map(children)
        .filter(|sibling| sibling != folder);
    parent
        .map(Path::to_path_buf)
        .into_iter()
        .chain(siblings)
        .chain(before_resources(folder))
        .chain(children(folder))
        .filter_map(|candidate| candidate.to_str().map(str::to_owned))
        .find(|candidate| {
            let folder = Path::new(candidate);
            launch::folder_problem(candidate, Platform::host()).is_none()
                && inspect(folder, observer).is_ok_and(|resources| resources.manifested() > 0)
                && !cfg_files(folder).0.is_empty()
        })
}

/// The folders among the first entries of `folder`.
fn children(folder: &Path) -> impl Iterator<Item = PathBuf> {
    fs::read_dir(folder)
        .into_iter()
        .flatten()
        .take(MAX_ENTRIES)
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
}

/// The server folder above a path that goes into its `resources`.
fn before_resources(folder: &Path) -> Option<PathBuf> {
    let mut cut = PathBuf::new();
    for component in folder.components() {
        if let Component::Normal(name) = component
            && is_resources(name)
        {
            return cut.file_name().is_some().then_some(cut);
        }
        cut.push(component);
    }
    None
}

fn is_resources(name: &OsStr) -> bool {
    name.to_str().is_some_and(|name| {
        if cfg!(windows) {
            name.eq_ignore_ascii_case("resources")
        } else {
            name == "resources"
        }
    })
}

/// The cfg files of a server folder, at its root and one folder down, with txAdmin's guesses at
/// the root, and the one detected: the first guess that is a file, else the only cfg at the root.
pub(crate) fn cfg_files(folder: &Path) -> (Vec<String>, Option<String>) {
    let fits = |name: &str| launch::cfg_problem(name, Platform::host()).is_none();
    let mut root = Vec::new();
    let mut nested = Vec::new();
    for (name, path) in entries(folder) {
        if path.is_file() && is_cfg(&name) && fits(&name) {
            root.push(name);
        } else if path.is_dir() && !passed_over(&name) {
            nested.extend(
                entries(&path)
                    .filter(|(file, path)| is_cfg(file) && path.is_file())
                    .map(|(file, _)| format!("{name}/{file}"))
                    .filter(|file| fits(file)),
            );
        }
    }
    root.sort();
    nested.sort();
    // Windows finds `server.cfg` as `Server.cfg` too: the folder's own spelling names it.
    let same = |name: &str, guess: &str| {
        if cfg!(windows) {
            name.eq_ignore_ascii_case(guess)
        } else {
            name == guess
        }
    };
    let detected = CFG_GUESSES
        .into_iter()
        .find(|guess| folder.join(guess).is_file())
        .map(|guess| {
            root.iter()
                .find(|name| same(name, guess))
                .map_or_else(|| guess.to_owned(), Clone::clone)
        })
        .or_else(|| match &root[..] {
            [only] => Some(only.clone()),
            _ => None,
        });
    let guesses = CFG_GUESSES
        .into_iter()
        .filter(|guess| !is_cfg(guess) && folder.join(guess).is_file())
        .map(str::to_owned);
    let mut files: Vec<String> = detected.clone().into_iter().collect();
    for file in root.into_iter().chain(guesses).chain(nested) {
        if files.len() == MAX_CFG_FILES {
            break;
        }
        if !files.contains(&file) {
            files.push(file);
        }
    }
    (files, detected)
}

/// The names and paths of the first entries of `folder`, those named in UTF-8.
fn entries(folder: &Path) -> impl Iterator<Item = (String, PathBuf)> {
    fs::read_dir(folder)
        .into_iter()
        .flatten()
        .take(MAX_ENTRIES)
        .flatten()
        .filter_map(|entry| Some((entry.file_name().into_string().ok()?, entry.path())))
}

fn is_cfg(name: &str) -> bool {
    name.len() > ".cfg".len()
        && name
            .get(name.len() - ".cfg".len()..)
            .is_some_and(|extension| extension.eq_ignore_ascii_case(".cfg"))
}

/// The folders whose cfg files belong to no server: the resources', the cache's and the tools'.
fn passed_over(name: &str) -> bool {
    ["resources", "cache"]
        .into_iter()
        .chain(SKIPPED)
        .any(|skipped| skipped.eq_ignore_ascii_case(name))
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    /// A data folder with these folders under `resources/`, each holding a manifest unless its
    /// path ends in `!`.
    fn data(folders: &[&str]) -> TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("resources")).unwrap();
        for folder in folders {
            let (folder, manifest) = match folder.strip_suffix('!') {
                Some(folder) => (folder, false),
                None => (*folder, true),
            };
            let path = dir.path().join("resources").join(folder);
            fs::create_dir_all(&path).unwrap();
            if manifest {
                fs::write(path.join("fxmanifest.lua"), "fx_version 'cerulean'\n").unwrap();
            }
        }
        dir
    }

    #[test]
    fn categories_are_walked_and_other_folders_are_resources() {
        let dir = data(&[
            "chat",
            "[cfx-default]/[system]/spawnmanager",
            "[cfx-default]/sessionmanager",
            "[local]/garage!",
            "[local]/old!",
            "[local]/observer",
        ]);
        fs::write(dir.path().join("resources/[local]/old/__resource.lua"), "").unwrap();
        fs::write(dir.path().join("resources/README.md"), "not a resource").unwrap();

        let resources = walk(dir.path());

        assert_eq!(resources.summary.resources, 6);
        assert_eq!(resources.summary.categories, 3);
        assert_eq!(resources.summary.without_manifest, ["garage"]);
        assert_eq!(resources.manifested(), 5);
        assert!(!resources.summary.truncated);
        for name in [
            "chat",
            "SpawnManager",
            "sessionmanager",
            "old",
            "[local]",
            "[system]",
        ] {
            assert!(resources.holds(name), "{name}");
        }
        assert!(!resources.holds("local"));
        assert!(!resources.holds("README.md"));
    }

    #[test]
    fn a_command_starts_the_resource_of_its_name_or_every_one_under_its_category() {
        let dir = data(&[
            "chat",
            "[Maps]/docks",
            "[Maps]/[interiors]/Bank",
            "[Maps]/drafts!",
            "[jobs]/police",
        ]);

        let resources = walk(dir.path());

        assert_eq!(resources.started_by("[maps]"), ["bank", "docks"]);
        assert_eq!(resources.started_by("[Interiors]"), ["bank"]);
        assert_eq!(resources.started_by("Chat"), ["chat"]);
        assert_eq!(
            resources.started_by("drafts"),
            Vec::<String>::new(),
            "no manifest"
        );
        assert_eq!(resources.started_by("sessionmanager"), Vec::<String>::new());
        assert_eq!(resources.started_by("[gone]"), Vec::<String>::new());
    }

    #[test]
    fn caches_and_tool_folders_are_passed_over() {
        let dir = data(&[
            "cache!",
            "db!",
            "node_modules!",
            ".git!",
            ".idea!",
            ".vscode!",
            "[tools]/node_modules!",
            "[tools]/builder",
        ]);

        let resources = walk(dir.path());

        assert_eq!(resources.summary.resources, 1);
        assert_eq!(resources.summary.without_manifest, Vec::<String>::new());
        assert!(!resources.holds("cache"));
    }

    #[test]
    fn the_walk_stops_at_250_categories() {
        let folders: Vec<String> = (0..=MAX_CATEGORIES)
            .map(|index| format!("[c{index}]/r{index}"))
            .collect();
        let dir = data(&folders.iter().map(String::as_str).collect::<Vec<_>>());

        let resources = walk(dir.path());

        assert_eq!(resources.summary.categories, 250);
        assert_eq!(resources.summary.resources, 250);
        assert!(resources.summary.truncated);
    }

    #[test]
    fn the_walk_stops_at_5000_folders() {
        let folders: Vec<String> = (0..=MAX_WALKED).map(|index| format!("r{index}!")).collect();
        let dir = data(&folders.iter().map(String::as_str).collect::<Vec<_>>());

        let resources = walk(dir.path());

        assert_eq!(resources.summary.resources, 5000);
        assert_eq!(resources.summary.without_manifest.len(), 20);
        assert!(resources.summary.truncated);
    }

    #[test]
    fn loose_files_count_toward_the_walks_limit_too() {
        let dir = data(&[]);
        for index in 0..=MAX_WALKED {
            fs::write(dir.path().join(format!("resources/notes-{index}.txt")), "").unwrap();
        }

        let resources = walk(dir.path());

        assert_eq!(resources.summary.resources, 0);
        assert!(resources.summary.truncated);
    }

    #[test]
    fn a_folder_without_resources_holds_none() {
        let dir = tempfile::tempdir().unwrap();

        assert_eq!(walk(dir.path()).summary, ResourceSummary::default());
    }

    /// `parts` joined under `root`, one component at a time, as a path the panel would send.
    fn path(root: &Path, parts: &[&str]) -> PathBuf {
        parts
            .iter()
            .fold(root.to_path_buf(), |path, part| path.join(part))
    }

    fn text(path: &Path) -> String {
        path.to_str().unwrap().to_owned()
    }

    /// A server folder holding `chat` and a cfg.
    fn server(folder: &Path) {
        let chat = folder.join("resources").join("chat");
        fs::create_dir_all(&chat).unwrap();
        fs::write(chat.join("fxmanifest.lua"), "fx_version 'cerulean'\n").unwrap();
        fs::write(folder.join("server.cfg"), "endpoint_add_tcp \"0.0.0.0:30140\"\n").unwrap();
    }

    fn context() -> Context {
        Context {
            panel: None,
            onesync: launch::OneSync::On,
            probe_ports: false,
            system_resources: None,
            follow_outside: false,
            convars: None,
        }
    }

    #[test]
    fn a_folder_is_refused_for_its_first_problem_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let observer = root.join("observer-data");
        server(&observer.join("server-data"));
        server(&root.join("live"));
        fs::write(root.join("notes.txt"), "not a folder").unwrap();
        fs::create_dir_all(root.join("bare")).unwrap();
        fs::create_dir_all(path(root, &["empty", "resources", "[local]", "node_modules"])).unwrap();
        let problem = |folder: String| check_folder(&folder, &observer).problem;

        let relative = check_folder("server-data", &observer);
        assert_eq!(relative.problem, Some(FolderProblem::InvalidPath));
        assert_eq!(relative.path_problem, Some(launch::LaunchProblem::NotAbsolute));
        assert_eq!(relative.suggestion, None);
        for (folder, expected) in [
            (root.join("nowhere"), FolderProblem::Missing),
            (root.join("notes.txt"), FolderProblem::NotAFolder),
            (observer.clone(), FolderProblem::InsideObserver),
            (observer.join("server-data"), FolderProblem::InsideObserver),
            (path(root, &["live", "resources"]), FolderProblem::IsResources),
            (root.join("bare"), FolderProblem::NoResources),
            (root.join("empty"), FolderProblem::EmptyResources),
        ] {
            assert_eq!(problem(text(&folder)), Some(expected), "{}", folder.display());
        }
        assert_eq!(problem(text(&root.join("live"))), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_that_cannot_be_listed_is_unreadable() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("locked");
        server(&folder);
        fs::set_permissions(&folder, fs::Permissions::from_mode(0o000)).unwrap();
        let listed = fs::read_dir(&folder).is_ok();

        let report = check_folder(&text(&folder), &dir.path().join("observer-data"));

        fs::set_permissions(&folder, fs::Permissions::from_mode(0o755)).unwrap();
        // Root lists any folder.
        if !listed {
            assert_eq!(report.problem, Some(FolderProblem::Unreadable));
        }
    }

    #[test]
    fn an_accepted_folder_names_its_resources_and_cfg_files() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("server-data");
        server(&folder);
        for file in [
            "extra.cfg",
            "my server.cfg",
            "server.txt",
            "README.md",
            "configs/permissions.cfg",
            "configs/notes.txt",
            "bad dir/x.cfg",
            "cache/files/old.cfg",
            "cache/old.cfg",
            "resources/chat/chat.cfg",
            "node_modules/x/x.cfg",
            "node_modules/x.cfg",
        ] {
            let file = folder.join(file);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, "").unwrap();
        }

        let report = check_folder(&text(&folder), &dir.path().join("observer-data"));

        assert_eq!(report.problem, None);
        assert_eq!(
            report.resources,
            Some(ResourceSummary {
                resources: 1,
                categories: 0,
                without_manifest: Vec::new(),
                truncated: false,
            })
        );
        assert_eq!(
            report.cfg_files,
            ["server.cfg", "extra.cfg", "server.txt", "configs/permissions.cfg"]
        );
        assert_eq!(report.detected_cfg.as_deref(), Some("server.cfg"));
    }

    #[test]
    fn the_cfg_detected_is_txadmins_first_guess_or_the_only_one_at_the_root() {
        let detected = |files: &[&str]| {
            let dir = tempfile::tempdir().unwrap();
            server(dir.path());
            fs::remove_file(dir.path().join("server.cfg")).unwrap();
            for file in files {
                fs::write(dir.path().join(file), "").unwrap();
            }
            cfg_files(dir.path()).1
        };

        assert_eq!(
            detected(&["server.txt", "server.cfg.txt"]).as_deref(),
            Some("server.cfg.txt")
        );
        assert_eq!(detected(&["live.cfg"]).as_deref(), Some("live.cfg"));
        assert_eq!(detected(&["live.cfg", "test.cfg"]), None);
        assert_eq!(detected(&["my live.cfg"]), None, "a name the cfg rules refuse");
        assert_eq!(detected(&[]), None);
    }

    #[test]
    fn a_cfg_is_listed_once_under_the_name_the_folder_gives_it() {
        let dir = tempfile::tempdir().unwrap();
        server(dir.path());
        fs::remove_file(dir.path().join("server.cfg")).unwrap();
        fs::write(dir.path().join("Server.cfg"), "").unwrap();

        let (files, detected) = cfg_files(dir.path());

        assert_eq!(files, ["Server.cfg"]);
        assert_eq!(detected.as_deref(), Some("Server.cfg"));
    }

    #[test]
    fn a_refused_folder_is_offered_the_first_folder_nearby_that_passes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let observer = root.join("observer-data");
        let suggestion = |folder: PathBuf| check_folder(&text(&folder), &observer).suggestion;
        server(&path(root, &["parent", "live"]));
        server(&path(root, &["sibling", "live"]));
        server(&path(root, &["cut", "live"]));
        let garage = path(root, &["cut", "live", "resources", "[local]", "garage"]);
        fs::create_dir_all(&garage).unwrap();
        server(&path(root, &["child", "fxserver", "server-data"]));
        fs::create_dir_all(path(root, &["child", "fxserver", "build"])).unwrap();
        fs::create_dir_all(path(root, &["none", "empty", "resources"])).unwrap();
        fs::create_dir_all(path(root, &["none", "bare"])).unwrap();
        // A web app's resources: FXServer would walk them, but nothing there is a server.
        fs::create_dir_all(path(root, &["none", "webapp", "resources", "views"])).unwrap();
        server(&observer.join("server-data"));

        assert_eq!(
            suggestion(path(root, &["parent", "live", "resources"])),
            Some(text(&path(root, &["parent", "live"])))
        );
        assert_eq!(
            suggestion(path(root, &["sibling", "liev"])),
            Some(text(&path(root, &["sibling", "live"])))
        );
        assert_eq!(suggestion(garage), Some(text(&path(root, &["cut", "live"]))));
        assert_eq!(
            suggestion(path(root, &["child", "fxserver"])),
            Some(text(&path(root, &["child", "fxserver", "server-data"])))
        );
        assert_eq!(suggestion(path(root, &["none", "bare"])), None);
        assert_eq!(
            suggestion(observer.join("server-dat")),
            None,
            "a folder inside Observer's passes no check"
        );
    }

    #[test]
    fn the_cfg_is_checked_once_the_folder_passes_and_the_file_can_be_read() {
        let dir = tempfile::tempdir().unwrap();
        let observer = dir.path().join("observer-data");
        let folder = dir.path().join("server-data");
        server(&folder);
        fs::create_dir_all(folder.join("configs.cfg")).unwrap();
        fs::write(folder.join("large.cfg"), "# padding\n".repeat(110_000)).unwrap();
        let data = text(&folder);
        let check = |data: &str, cfg: &str| check_cfg(data, cfg, &observer, &context());

        assert_eq!(
            check("server-data", "server.cfg").folder,
            Some(FolderProblem::InvalidPath)
        );
        assert_eq!(
            check(&text(&dir.path().join("bare")), "server.cfg").folder,
            Some(FolderProblem::Missing)
        );
        let refused = check(&data, "../server.cfg");
        assert_eq!(
            (refused.problem, refused.path_problem),
            (
                Some(CfgFileProblem::InvalidPath),
                Some(launch::LaunchProblem::InvalidCfgFile)
            )
        );
        for (cfg, problem) in [
            ("missing.cfg", CfgFileProblem::Missing),
            ("configs.cfg", CfgFileProblem::NotAFile),
            ("large.cfg", CfgFileProblem::TooLarge),
        ] {
            let report = check(&data, cfg);
            assert_eq!(report.problem, Some(problem), "{cfg}");
            assert_eq!(
                (report.files.len(), report.facts, report.findings.len()),
                (0, None, 0)
            );
        }
        let read = check(&data, "server.cfg");
        assert_eq!((read.folder, read.problem), (None, None));
        assert_eq!(read.files.len(), 1);
        assert_eq!(read.facts.unwrap().endpoints.len(), 1);
        let kinds: Vec<_> = read.findings.iter().map(|finding| finding.kind).collect();
        assert_eq!(
            kinds,
            [
                observer_domain::server_data::FindingKind::EndpointUnpaired,
                observer_domain::server_data::FindingKind::LicenceMissing,
            ]
        );
    }

    #[test]
    fn the_convars_of_the_launch_cfg_are_read_inside_the_data_folder_with_the_launch_names() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        fs::create_dir_all(&data).unwrap();
        fs::write(
            data.join("server.cfg"),
            "set sv_hostname Lab
lab_convar 2
other_word 3
exec ../shared.cfg
",
        )
        .unwrap();
        fs::write(
            dir.path().join("shared.cfg"),
            "set sv_maxclients 64
",
        )
        .unwrap();
        let settings = LaunchSettings {
            data_path: data.to_str().unwrap().to_owned(),
            cfg_file: "server.cfg".to_owned(),
            onesync: launch::OneSync::On,
            auto_start: false,
            txadmin_events: false,
            extra_arguments: vec![launch::ExtraArgument {
                name: "lab_convar".to_owned(),
                value: "1".to_owned(),
            }],
            hang_detection: true,
            hang_timeout_seconds: launch::DEFAULT_HANG_TIMEOUT_SECONDS,
        };

        let read = cfg_convars(&settings);
        let missing = cfg_convars(&LaunchSettings {
            cfg_file: "missing.cfg".to_owned(),
            ..settings.clone()
        });

        let names: Vec<_> = read
            .convars
            .iter()
            .map(|set| (set.name.as_str(), set.line))
            .collect();
        assert_eq!(names, [("sv_hostname", 1), ("lab_convar", 2)]);
        assert_eq!(read.problem, None);
        assert!(
            !read.complete,
            "shared.cfg, outside the data folder, was not read"
        );
        assert_eq!(missing.problem, Some(CfgFileProblem::Missing));
        assert_eq!(missing.convars, []);
    }
}
