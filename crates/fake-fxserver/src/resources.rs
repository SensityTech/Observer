//! The resources the fake knows: the folders with a manifest under the server folder's `resources`
//! and the build's `system_resources`, as `ServerResourceList.cpp` scans them, with what each
//! manifest says of its version, of what it depends on and of the names it provides. A name no
//! folder holds is a resource all the same, which the fake always let start.

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

const MANIFESTS: [&str; 2] = ["fxmanifest.lua", "__resource.lua"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resource {
    pub name: String,
    /// As `GetResourcePath` gives it: the root, `/`, then the folders down to the resource's own.
    pub path: String,
    pub version: Option<String>,
    pub needs: Vec<String>,
    pub provides: Vec<String>,
}

/// What a `refresh` found: the resources it did not know, those whose manifest changed, and how
/// many it read again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Refreshed {
    pub added: usize,
    pub changed: usize,
    pub read: usize,
}

#[derive(Debug, Default)]
pub struct Registry {
    known: Vec<Resource>,
}

impl Registry {
    /// Scans the two roots, the server's first: of two folders of one name, the first found is
    /// the resource.
    pub fn scan(server: &Path, system: Option<&Path>) -> Self {
        let mut known = Vec::new();
        for root in [Some(server.join("resources")), system.map(Path::to_path_buf)]
            .into_iter()
            .flatten()
        {
            let shown = format!("{}/", root.to_string_lossy().replace('\\', "/"));
            walk(&root, &shown, &mut known);
        }
        Self { known }
    }

    /// Scans again, as `refresh` does. A resource whose folder is gone stays known.
    pub fn refresh(&mut self, server: &Path, system: Option<&Path>) -> Refreshed {
        let mut refreshed = Refreshed {
            added: 0,
            changed: 0,
            read: 0,
        };
        for found in Self::scan(server, system).known {
            if let Some(known) = self.known.iter_mut().find(|known| known.name == found.name) {
                refreshed.read += 1;
                refreshed.changed += usize::from(*known != found);
                *known = found;
            } else {
                refreshed.added += 1;
                self.known.push(found);
            }
        }
        refreshed
    }

    fn find(&self, name: &str) -> Option<&Resource> {
        self.known.iter().find(|known| known.name == name)
    }

    /// The resources `name` depends on, constraints such as `/onesync` left out.
    pub fn needs(&self, name: &str) -> Vec<String> {
        self.find(name)
            .map(|known| {
                known
                    .needs
                    .iter()
                    .filter(|need| !need.starts_with('/'))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The resource a dependency named `need` stands for, as `ResourceManager::GetResource` finds
    /// it: the one of that name unless it is stopped and a started one provides the name.
    pub fn resolve(&self, need: &str, started: &[String]) -> String {
        let up = |name: &str| started.iter().any(|started| started == name);
        if up(need) {
            return need.to_owned();
        }
        self.known
            .iter()
            .filter(|known| known.provides.iter().any(|provided| provided == need))
            .find(|known| up(&known.name) || self.find(need).is_none())
            .map_or_else(|| need.to_owned(), |provider| provider.name.clone())
    }

    /// The started resources whose manifest depends on `name`.
    pub fn dependants(&self, name: &str, started: &[String]) -> Vec<String> {
        started
            .iter()
            .filter(|other| other.as_str() != name)
            .filter(|other| {
                self.needs(other)
                    .iter()
                    .any(|need| self.resolve(need, started) == name)
            })
            .cloned()
            .collect()
    }

    /// Every resource as the bridge lists it, in name order: the known ones, and those started
    /// that no folder holds.
    pub fn listed(&self, started: &[String]) -> Vec<Value> {
        let state = |name: &str| {
            if started.iter().any(|started| started == name) {
                "started"
            } else {
                "stopped"
            }
        };
        let mut entries: Vec<(String, Value)> = self
            .known
            .iter()
            .map(|known| {
                let mut entry = json!({
                    "name": known.name,
                    "state": state(&known.name),
                    "path": known.path,
                });
                if let Some(version) = &known.version {
                    entry["version"] = json!(version);
                }
                if !known.needs.is_empty() {
                    entry["needs"] = json!(known.needs);
                }
                if !known.provides.is_empty() {
                    entry["provides"] = json!(known.provides);
                }
                (known.name.clone(), entry)
            })
            .collect();
        for name in started {
            if self.find(name).is_none() {
                entries.push((name.clone(), json!({ "name": name, "state": "started" })));
            }
        }
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        entries.into_iter().map(|(_, entry)| entry).collect()
    }
}

/// Reads `folder`: a folder named `[x]` is a category, read in turn, and any other that holds a
/// manifest is a resource.
fn walk(folder: &Path, shown: &str, known: &mut Vec<Resource>) {
    let Ok(entries) = fs::read_dir(folder) else {
        return;
    };
    let mut folders: Vec<(String, std::path::PathBuf)> = entries
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| Some((entry.file_name().to_str()?.to_owned(), entry.path())))
        .collect();
    folders.sort();
    for (name, path) in folders {
        let shown = format!("{shown}/{name}");
        if name.starts_with('[') && name.ends_with(']') {
            walk(&path, &shown, known);
            continue;
        }
        let Some(manifest) = MANIFESTS
            .iter()
            .find_map(|manifest| fs::read_to_string(path.join(manifest)).ok())
        else {
            continue;
        };
        if name.starts_with('.') || known.iter().any(|known| known.name == name) {
            continue;
        }
        let mut resource = Resource {
            name,
            path: shown,
            version: None,
            needs: Vec::new(),
            provides: Vec::new(),
        };
        read_manifest(&manifest, &mut resource);
        known.push(resource);
    }
}

/// What a manifest says of its version, its dependencies and the names it provides: the keyword
/// at the start of a line, then one quoted value, or several between braces over any lines.
fn read_manifest(manifest: &str, resource: &mut Resource) {
    let mut lines = manifest
        .lines()
        .map(|line| line.split("--").next().unwrap_or_default());
    while let Some(line) = lines.next() {
        let line = line.trim_start();
        let Some(keyword) = ["version", "dependencies", "dependency", "provides", "provide"]
            .into_iter()
            .find(|keyword| {
                line.strip_prefix(keyword)
                    .is_some_and(|rest| rest.starts_with([' ', '\t', '\'', '"', '{', '(']))
            })
        else {
            continue;
        };
        let mut statement = line[keyword.len()..].to_owned();
        while statement.contains('{') && !statement.contains('}') {
            let Some(more) = lines.next() else {
                break;
            };
            statement.push(' ');
            statement.push_str(more);
        }
        let values = quoted(&statement);
        match keyword {
            "version" => resource.version = values.into_iter().next(),
            "dependency" | "dependencies" => resource.needs.extend(values),
            _ => resource.provides.extend(values),
        }
    }
}

/// The values between single or double quotes of `text`.
fn quoted(text: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find(['\'', '"']) {
        let quote = rest[start..].chars().next().unwrap_or('\'');
        let inside = &rest[start + 1..];
        let Some(end) = inside.find(quote) else {
            break;
        };
        values.push(inside[..end].to_owned());
        rest = &inside[end + 1..];
    }
    values
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resource(root: &Path, folder: &str, manifest: &str) {
        let path = root.join(folder);
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("fxmanifest.lua"), manifest).unwrap();
    }

    fn started(names: &[&str]) -> Vec<String> {
        names.iter().map(|&name| name.to_owned()).collect()
    }

    #[test]
    fn the_two_roots_are_scanned_for_folders_with_a_manifest_under_their_categories() {
        let dir = tempfile::tempdir().unwrap();
        let (server, system) = (dir.path().join("data"), dir.path().join("system_resources"));
        resource(
            &server,
            "resources/[core]/[ox]/ox_inventory",
            "fx_version 'cerulean'\nversion '2.44.1' -- the release\ndependencies {\n    '/onesync',\n    'ox_lib', -- first\n    \"oxmysql\",\n}\nprovide 'inventory'\n",
        );
        resource(
            &server,
            "resources/ox_lib",
            "version \"3.30.6\"\ndependency 'unused' -- kept\n",
        );
        resource(&server, "resources/[core]/ox_lib", "version 'a copy'\n");
        resource(&server, "resources/.hidden", "version '1'\n");
        fs::create_dir_all(server.join("resources/no_manifest")).unwrap();
        resource(&system, "monitor", "provides { 'txAdmin', 'tx' }\n");

        let registry = Registry::scan(&server, Some(&system));

        let root = server.join("resources").to_string_lossy().replace('\\', "/");
        let names: Vec<&str> = registry.known.iter().map(|known| known.name.as_str()).collect();
        assert_eq!(names, ["ox_inventory", "ox_lib", "monitor"]);
        let inventory = registry.find("ox_inventory").unwrap();
        assert_eq!(inventory.path, format!("{root}//[core]/[ox]/ox_inventory"));
        assert_eq!(inventory.version.as_deref(), Some("2.44.1"));
        assert_eq!(inventory.needs, ["/onesync", "ox_lib", "oxmysql"]);
        assert_eq!(inventory.provides, ["inventory"]);
        assert_eq!(registry.needs("ox_inventory"), ["ox_lib", "oxmysql"]);
        assert_eq!(
            registry.find("ox_lib").unwrap().version.as_deref(),
            Some("a copy"),
            "the first folder found, in name order"
        );
        assert_eq!(registry.find("monitor").unwrap().provides, ["txAdmin", "tx"]);
    }

    #[test]
    fn a_dependency_stands_for_the_started_resource_that_provides_its_name() {
        let dir = tempfile::tempdir().unwrap();
        let server = dir.path();
        resource(server, "resources/oxmysql", "provide 'mysql-async'\n");
        resource(server, "resources/mysql-async", "version '3'\n");
        resource(server, "resources/garage", "dependency 'mysql-async'\n");
        resource(server, "resources/bank", "dependencies { 'ghost', 'garage' }\n");
        let registry = Registry::scan(server, None);

        let up = started(&["oxmysql", "garage", "bank"]);
        assert_eq!(registry.resolve("mysql-async", &up), "oxmysql");
        assert_eq!(
            registry.resolve("mysql-async", &started(&["mysql-async"])),
            "mysql-async"
        );
        assert_eq!(registry.resolve("mysql-async", &[]), "mysql-async");
        assert_eq!(registry.resolve("ghost", &up), "ghost");
        assert_eq!(registry.dependants("oxmysql", &up), ["garage"]);
        assert_eq!(registry.dependants("garage", &up), ["bank"]);
        assert_eq!(registry.dependants("bank", &up), Vec::<String>::new());
    }

    #[test]
    fn a_refresh_counts_what_is_new_and_what_changed_and_forgets_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let server = dir.path();
        resource(server, "resources/a", "version '1'\n");
        resource(server, "resources/b", "version '1'\n");
        resource(server, "resources/gone", "version '1'\n");
        let mut registry = Registry::scan(server, None);

        resource(server, "resources/b", "version '2'\n");
        resource(server, "resources/[new]/c", "version '1'\n");
        fs::remove_dir_all(server.join("resources/gone")).unwrap();
        let refreshed = registry.refresh(server, None);

        assert_eq!(
            refreshed,
            Refreshed {
                added: 1,
                changed: 1,
                read: 2
            }
        );
        assert_eq!(registry.known.len(), 4);
        assert_eq!(registry.find("b").unwrap().version.as_deref(), Some("2"));
    }

    #[test]
    fn the_list_holds_what_is_known_and_what_started_without_a_folder() {
        let dir = tempfile::tempdir().unwrap();
        resource(dir.path(), "resources/b", "version '1'\ndependency 'a'\n");
        let registry = Registry::scan(dir.path(), None);

        let listed = registry.listed(&started(&["ghost", "b"]));

        let root = dir.path().join("resources").to_string_lossy().replace('\\', "/");
        assert_eq!(
            listed,
            [
                json!({ "name": "b", "state": "started", "path": format!("{root}//b"), "version": "1", "needs": ["a"] }),
                json!({ "name": "ghost", "state": "started" }),
            ]
        );
        assert_eq!(registry.listed(&[])[0]["state"], "stopped");
    }
}
