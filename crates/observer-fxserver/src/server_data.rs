//! The server data folder as FXServer reads it: the resources it finds under `resources/`.

use std::collections::{HashSet, VecDeque};
use std::fs;
use std::path::Path;

use observer_domain::server_data::ResourceSummary;

const MAX_CATEGORIES: u32 = 250;
const MAX_FOLDERS: usize = 5000;
/// Resources without a manifest named in a summary.
const MAX_UNMANIFESTED: usize = 20;
/// Folders FXServer's walk passes over, as txAdmin's does.
const SKIPPED: [&str; 6] = ["cache", "db", "node_modules", ".git", ".idea", ".vscode"];
const MANIFESTS: [&str; 2] = ["fxmanifest.lua", "__resource.lua"];

/// What the walk of `resources/` found. Names are kept in lower case: a cfg that starts `Chat` for
/// a folder `chat` is left to FXServer to judge, not reported.
#[derive(Debug, Clone, Default)]
pub struct Resources {
    names: HashSet<String>,
    categories: HashSet<String>,
    /// Relative to the data folder with `/`.
    named_observer: Vec<String>,
    pub summary: ResourceSummary,
}

impl Resources {
    /// Whether `ensure <name>` finds a resource or a category of the folder.
    pub fn holds(&self, name: &str) -> bool {
        let name = name.to_ascii_lowercase();
        self.names.contains(&name) || self.categories.contains(&name)
    }

    /// The resource folders named like Observer's bridge.
    pub fn named_observer(&self) -> &[String] {
        &self.named_observer
    }
}

/// Walks `resources/` of `data` the way FXServer does: a folder named `[x]` is a category, read in
/// turn, and any other folder a resource. It stops at 250 categories or 5,000 folders; a folder it
/// cannot read holds nothing.
pub fn walk(data: &Path) -> Resources {
    let mut resources = Resources::default();
    let mut queue = VecDeque::from([(data.join("resources"), "resources".to_owned())]);
    let mut folders = 0;
    'walk: while let Some((folder, relative)) = queue.pop_front() {
        let Ok(entries) = fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            folders += 1;
            if folders > MAX_FOLDERS {
                resources.summary.truncated = true;
                break 'walk;
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
            if !manifest && resources.summary.without_manifest.len() < MAX_UNMANIFESTED {
                resources.summary.without_manifest.push(name.clone());
            }
            if lower == crate::bridge::NAME {
                resources.named_observer.push(format!("{relative}/{name}"));
            }
            resources.names.insert(lower);
        }
    }
    resources
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
        assert_eq!(resources.named_observer(), ["resources/[local]/observer"]);
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
        let folders: Vec<String> = (0..=MAX_FOLDERS).map(|index| format!("r{index}!")).collect();
        let dir = data(&folders.iter().map(String::as_str).collect::<Vec<_>>());

        let resources = walk(dir.path());

        assert_eq!(resources.summary.resources, 5000);
        assert_eq!(resources.summary.without_manifest.len(), 20);
        assert!(resources.summary.truncated);
    }

    #[test]
    fn a_folder_without_resources_holds_none() {
        let dir = tempfile::tempdir().unwrap();

        assert_eq!(walk(dir.path()).summary, ResourceSummary::default());
    }
}
