//! The npm packages of a `node_modules` folder, by name and version: only each package's own
//! `package.json` is read, `node_modules/<name>/` and `node_modules/@<scope>/<name>/`, never what
//! lies deeper.

use std::fs::{self, File};
use std::io::Read;
use std::path::Path;

use observer_domain::changes::Package;

const MAX_MANIFEST: u64 = 64 * 1024;
const MAX_NAME: usize = 214;
const MAX_VERSION: usize = 64;
/// Entries of a `node_modules` folder or of a scope looked at.
const MAX_ENTRIES: usize = 5000;

/// Adds the packages of `node_modules` to `found`, which holds at most `limit`.
pub fn read(node_modules: &Path, found: &mut Vec<Package>, limit: usize) {
    for (name, path) in folders(node_modules) {
        if found.len() >= limit {
            return;
        }
        if name.starts_with('@') {
            for (_, package) in folders(&path) {
                if found.len() >= limit {
                    return;
                }
                found.extend(package_json(&package));
            }
        } else if !name.starts_with('.') {
            found.extend(package_json(&path));
        }
    }
}

fn folders(path: &Path) -> impl Iterator<Item = (String, std::path::PathBuf)> {
    fs::read_dir(path)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .take(MAX_ENTRIES)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            // pnpm links its packages, so a link is followed to see whether it is a folder.
            fs::metadata(entry.path())
                .is_ok_and(|metadata| metadata.is_dir())
                .then(|| (name, entry.path()))
        })
}

fn package_json(folder: &Path) -> Option<Package> {
    let mut bytes = Vec::new();
    File::open(folder.join("package.json"))
        .ok()?
        .take(MAX_MANIFEST + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if u64::try_from(bytes.len()).ok()? > MAX_MANIFEST {
        return None;
    }
    let manifest: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let name = manifest.get("name")?.as_str()?;
    if name.is_empty() || name.len() > MAX_NAME || name.chars().any(char::is_control) {
        return None;
    }
    let version = manifest
        .get("version")
        .and_then(serde_json::Value::as_str)
        .filter(|version| version.len() <= MAX_VERSION && !version.chars().any(char::is_control))
        .map(str::to_owned);
    Some(Package {
        name: name.to_owned(),
        version,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(root: &Path, folder: &str, json: &str) {
        let path = root.join(folder);
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("package.json"), json).unwrap();
    }

    #[test]
    fn packages_and_scoped_packages_are_read_by_name_and_version() {
        let dir = tempfile::tempdir().unwrap();
        package(dir.path(), "axios", r#"{ "name": "axios", "version": "1.7.2" }"#);
        package(
            dir.path(),
            "@types/node",
            r#"{ "name": "@types/node", "version": "22.0.0" }"#,
        );
        package(
            dir.path(),
            "axios/node_modules/deep",
            r#"{ "name": "deep", "version": "1.0.0" }"#,
        );
        package(dir.path(), ".bin", r#"{ "name": "bin" }"#);
        package(dir.path(), "broken", "{ not json");

        let mut found = Vec::new();
        read(dir.path(), &mut found, 100);
        found.sort();

        assert_eq!(
            found,
            [
                Package {
                    name: "@types/node".to_owned(),
                    version: Some("22.0.0".to_owned())
                },
                Package {
                    name: "axios".to_owned(),
                    version: Some("1.7.2".to_owned())
                },
            ]
        );
    }

    #[test]
    fn the_limit_stops_the_read() {
        let dir = tempfile::tempdir().unwrap();
        for index in 0..5 {
            package(
                dir.path(),
                &format!("p{index}"),
                &format!(r#"{{ "name": "p{index}" }}"#),
            );
        }

        let mut found = Vec::new();
        read(dir.path(), &mut found, 3);

        assert_eq!(found.len(), 3);
    }
}
