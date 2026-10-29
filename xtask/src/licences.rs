//! `cargo xtask licences`: every crate and npm package that ends up in Observer for a target, with
//! the licence texts each ships, for the About page and the release archives.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, bail};
use observer_domain::about::{Ecosystem, LicenceFile, ThirdPartyLicences, ThirdPartyPackage};
use serde_json::Value;
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

/// The binary whose dependencies ship.
const ROOT: &str = "observer";
/// Build tools whose output lands in the panel's bundle, so their licences travel with it:
/// Tailwind's base styles are in the CSS, Vite's module preload helper in the JavaScript.
const BUNDLED_BUILD_PACKAGES: [&str; 2] = ["node_modules/tailwindcss", "node_modules/vite"];
/// The names a licence file starts with.
const NAMES: [&str; 6] = [
    "licence",
    "license",
    "copying",
    "notice",
    "copyright",
    "unlicense",
];
/// Extensions of code and data, never of a licence text: `copyright.mjs` is an icon. Licence
/// files themselves take any other (`LICENSE.APACHE`, `LICENSE.httprouter`, `LICENSE.spdx`).
const CODE: [&str; 16] = [
    "c", "cjs", "css", "cts", "h", "js", "json", "jsx", "map", "mjs", "mts", "py", "rs", "ts", "tsx", "wasm",
];
/// Licence files under names of their own: egui's fonts ship theirs next to each font.
const NAMED_FILES: [(&str, &[&str]); 1] = [(
    "epaint_default_fonts",
    &[
        "fonts/Hack-Regular.txt",
        "fonts/OFL.txt",
        "fonts/UFL.txt",
        "fonts/emoji-icon-font-mit-license.txt",
    ],
)];

/// Where the list the API serves goes: beside the panel, never in `web/dist`, whose every file
/// is public.
pub const PRIVATE: &str = "web/private";

/// Writes `web/private/licences.json` and `target/licences/<target>.txt`, and says where the
/// text went.
pub fn write(root: &Path, target: &str) -> anyhow::Result<PathBuf> {
    let licences = collect(root, target)?;
    let private = root.join(PRIVATE);
    // The compressed copies of an older list must not outlive it.
    if private.exists() {
        fs::remove_dir_all(&private).with_context(|| format!("cannot empty {}", private.display()))?;
    }
    fs::create_dir_all(&private).with_context(|| format!("cannot create {}", private.display()))?;
    let json = private.join("licences.json");
    fs::write(&json, serde_json::to_vec(&licences)?)
        .with_context(|| format!("cannot write {}", json.display()))?;
    let folder = root.join("target/licences");
    fs::create_dir_all(&folder).with_context(|| format!("cannot create {}", folder.display()))?;
    let text = folder.join(format!("{target}.txt"));
    fs::write(&text, plain_text(&licences)).with_context(|| format!("cannot write {}", text.display()))?;
    let count = |ecosystem| {
        licences
            .packages
            .iter()
            .filter(|package| package.ecosystem == ecosystem)
            .count()
    };
    println!(
        "licences: {} crates and {} npm packages for {target}",
        count(Ecosystem::Crate),
        count(Ecosystem::Npm)
    );
    Ok(text)
}

/// The host's target triple, as `rustc -vV` names it.
pub fn host() -> anyhow::Result<String> {
    let output = Command::new("rustc")
        .arg("-vV")
        .output()
        .context("cannot run rustc")?;
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .map(str::to_owned)
        .context("rustc -vV names no host")
}

fn collect(root: &Path, target: &str) -> anyhow::Result<ThirdPartyLicences> {
    let mut texts = Texts::default();
    let spdx = root.join("xtask/licences");
    let mut packages = Vec::new();
    for found in crates(root, target)?.into_iter().chain(npm(root)?) {
        packages.push(found.into_package(&mut texts, &spdx)?);
    }
    packages.sort_by(|a, b| (a.ecosystem, &a.name, &a.version).cmp(&(b.ecosystem, &b.name, &b.version)));
    Ok(ThirdPartyLicences {
        target: target.to_owned(),
        packages,
        texts: texts.0,
    })
}

/// A package as found, before its files are read.
struct Found {
    ecosystem: Ecosystem,
    name: String,
    version: String,
    licence: Option<String>,
    repository: Option<String>,
    folder: PathBuf,
    /// Files named elsewhere: Cargo's `license_file`, the panel's own NOTICE copies.
    extra: Vec<(String, PathBuf)>,
}

impl Found {
    fn into_package(self, texts: &mut Texts, spdx: &Path) -> anyhow::Result<ThirdPartyPackage> {
        let mut paths = licence_files(&self.folder, self.ecosystem == Ecosystem::Npm);
        for (shown, path) in self.extra {
            if path.is_file() && !paths.iter().any(|(_, known)| *known == path) {
                paths.push((shown, path));
            }
        }
        let mut files = Vec::new();
        for (shown, path) in paths {
            let text = read_text(&path)?;
            files.push(LicenceFile {
                path: shown,
                text: texts.add(text),
            });
        }
        let from_spdx = files.is_empty();
        if from_spdx {
            let expression = self.licence.as_deref().unwrap_or_default();
            let identifiers = identifiers(expression);
            if identifiers.is_empty() {
                bail!(
                    "{} {} ships no licence file and declares no licence; look in {}",
                    self.name,
                    self.version,
                    self.folder.display()
                );
            }
            for identifier in identifiers {
                let path = spdx.join(format!("{identifier}.txt"));
                let Ok(text) = read_text(&path) else {
                    bail!(
                        "{} {} ships no licence file and {} has no text for {identifier} (its licence: {expression}); \
                         looked in {}. A new licence is a decision: add its SPDX text there once it is accepted",
                        self.name,
                        self.version,
                        spdx.display(),
                        self.folder.display()
                    );
                };
                files.push(LicenceFile {
                    path: format!("{identifier} (SPDX)"),
                    text: texts.add(text),
                });
            }
        }
        Ok(ThirdPartyPackage {
            ecosystem: self.ecosystem,
            name: self.name,
            version: self.version,
            licence: self.licence,
            repository: self.repository,
            files,
            from_spdx,
        })
    }
}

/// Each text once, by the first 16 hex digits of its SHA-256.
#[derive(Default)]
struct Texts(BTreeMap<String, String>);

impl Texts {
    fn add(&mut self, text: String) -> String {
        let digest = Sha256::digest(text.as_bytes());
        let mut key = String::with_capacity(16);
        for byte in &digest[..8] {
            let _ = write!(key, "{byte:02x}");
        }
        self.0.entry(key.clone()).or_insert(text);
        key
    }
}

/// The crates the binary links for `target`: its normal dependencies, followed through, the
/// workspace's own crates left out. Build and dev dependencies never ship.
fn crates(root: &Path, target: &str) -> anyhow::Result<Vec<Found>> {
    let output = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
        .args([
            "metadata",
            "--format-version",
            "1",
            "--locked",
            "--filter-platform",
            target,
        ])
        .current_dir(root)
        .output()
        .context("cannot run cargo metadata")?;
    if !output.status.success() {
        bail!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let metadata: Value = serde_json::from_slice(&output.stdout)?;
    let packages: HashMap<&str, &Value> = list(&metadata["packages"])
        .iter()
        .filter_map(|package| Some((package["id"].as_str()?, package)))
        .collect();
    let members: BTreeSet<&str> = list(&metadata["workspace_members"])
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let nodes: HashMap<&str, &Value> = list(&metadata["resolve"]["nodes"])
        .iter()
        .filter_map(|node| Some((node["id"].as_str()?, node)))
        .collect();
    let start = members
        .iter()
        .copied()
        .find(|id| packages.get(id).is_some_and(|package| package["name"] == ROOT))
        .context("cargo metadata lists no observer package")?;

    let mut seen = BTreeSet::from([start]);
    let mut queue = VecDeque::from([start]);
    while let Some(id) = queue.pop_front() {
        let Some(node) = nodes.get(id) else { continue };
        for dependency in list(&node["deps"]) {
            let normal = list(&dependency["dep_kinds"])
                .iter()
                .any(|kind| kind["kind"].is_null());
            if let Some(next) = dependency["pkg"].as_str().filter(|_| normal)
                && seen.insert(next)
            {
                queue.push_back(next);
            }
        }
    }

    let mut found = Vec::new();
    for id in seen.into_iter().filter(|id| !members.contains(id)) {
        let package = packages
            .get(id)
            .with_context(|| format!("cargo metadata misses {id}"))?;
        let manifest = PathBuf::from(package["manifest_path"].as_str().unwrap_or_default());
        let folder = manifest.parent().map(Path::to_path_buf).unwrap_or_default();
        let name = string(&package["name"]);
        let named = NAMED_FILES
            .iter()
            .filter(|(package, _)| *package == name)
            .flat_map(|(_, files)| files.iter().copied());
        let extra = package["license_file"]
            .as_str()
            .into_iter()
            .chain(named)
            .map(|file| (file.replace('\\', "/"), folder.join(file)))
            .collect();
        found.push(Found {
            ecosystem: Ecosystem::Crate,
            name,
            version: string(&package["version"]),
            licence: package["license"].as_str().map(str::to_owned),
            repository: package["repository"].as_str().map(str::to_owned),
            folder,
            extra,
        });
    }
    Ok(found)
}

/// The npm packages the panel's bundle is built from.
fn npm(root: &Path) -> anyhow::Result<Vec<Found>> {
    let web = root.join("web");
    let lockfile = web.join("package-lock.json");
    let lock: Value = serde_json::from_slice(
        &fs::read(&lockfile).with_context(|| format!("cannot read {}", lockfile.display()))?,
    )?;
    let notices = web.join("public/licenses");
    let mut found = Vec::new();
    for (key, entry) in shipped(&lock) {
        let folder = web.join(&key);
        let manifest: Value = fs::read(folder.join("package.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or(Value::Null);
        let name = entry["name"]
            .as_str()
            .or_else(|| manifest["name"].as_str())
            .map_or_else(|| package_name(&key).to_owned(), str::to_owned);
        let repository = match &manifest["repository"] {
            Value::String(url) => Some(url.clone()),
            other => other["url"].as_str().map(str::to_owned),
        };
        let notice = notices.join(format!("{}-NOTICE.txt", name.replace('/', "-")));
        found.push(Found {
            ecosystem: Ecosystem::Npm,
            version: string(&entry["version"]),
            licence: entry["license"]
                .as_str()
                .or_else(|| manifest["license"].as_str())
                .map(str::to_owned),
            repository,
            folder,
            extra: vec![("NOTICE".to_owned(), notice)],
            name,
        });
    }
    Ok(found)
}

/// The lockfile's packages that reach the bundle: every installed one that is neither `dev` nor
/// `devOptional`, and the build tools that put code of their own in it.
fn shipped(lock: &Value) -> Vec<(String, &Value)> {
    let Some(packages) = lock["packages"].as_object() else {
        return Vec::new();
    };
    packages
        .iter()
        .filter(|(key, _)| key.starts_with("node_modules/"))
        .filter(|(key, entry)| {
            let dev = entry["dev"] == true || entry["devOptional"] == true;
            !dev || BUNDLED_BUILD_PACKAGES.contains(&key.as_str())
        })
        .map(|(key, entry)| (key.clone(), entry))
        .collect()
}

/// `node_modules/@scope/name/node_modules/other` names `other`.
fn package_name(key: &str) -> &str {
    key.rsplit_once("node_modules/").map_or(key, |(_, name)| name)
}

/// The licence files of a package folder, at any depth (vendored code keeps its own), with the
/// path they show under. Nested `node_modules` are other packages.
fn licence_files(folder: &Path, npm: bool) -> Vec<(String, PathBuf)> {
    let mut files: Vec<(String, PathBuf)> = WalkDir::new(folder)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|entry| !(npm && entry.depth() > 0 && entry.file_name() == "node_modules"))
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file() && is_licence_name(&entry.file_name().to_string_lossy()))
        .map(|entry| {
            let shown = entry
                .path()
                .strip_prefix(folder)
                .unwrap_or(entry.path())
                .to_string_lossy()
                .replace('\\', "/");
            (shown, entry.into_path())
        })
        .collect();
    files.sort();
    files
}

/// `LICENSE`, `license.md`, `COPYING`, `NOTICE.txt`, `LICENSE-MIT`: a licence name alone, or
/// followed by `-`, `.` or `_` and anything, unless it ends as code does.
fn is_licence_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    let code = name
        .rsplit_once('.')
        .is_some_and(|(_, extension)| CODE.contains(&extension));
    !code
        && NAMES.iter().any(|prefix| {
            name.strip_prefix(prefix)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with(['-', '.', '_']))
        })
}

/// The licence identifiers of an SPDX expression, exceptions included, or of the older `A/B`.
fn identifiers(expression: &str) -> Vec<String> {
    let mut identifiers = Vec::new();
    for word in expression.split(|c: char| c.is_whitespace() || matches!(c, '(' | ')' | '/')) {
        if word.is_empty() || matches!(word, "AND" | "OR" | "WITH") {
            continue;
        }
        let word = word.trim_end_matches('+').to_owned();
        if !identifiers.contains(&word) {
            identifiers.push(word);
        }
    }
    identifiers
}

/// UTF-8 at best, LF line ends, no trailing blank lines.
fn read_text(path: &Path) -> anyhow::Result<String> {
    let bytes = fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
    let text = String::from_utf8_lossy(&bytes).replace("\r\n", "\n");
    Ok(format!("{}\n", text.trim_end()))
}

/// The list as text, for the archives: each package, its repository, then its files.
fn plain_text(licences: &ThirdPartyLicences) -> String {
    let rule = "=".repeat(78);
    let mut text = format!(
        "Third-party software in Observer for {}: {} packages.\n",
        licences.target,
        licences.packages.len()
    );
    for package in &licences.packages {
        let licence = package.licence.as_deref().unwrap_or("no licence declared");
        let kind = match package.ecosystem {
            Ecosystem::Crate => "crate",
            Ecosystem::Npm => "npm",
        };
        let _ = write!(
            text,
            "\n{rule}\n{} {} ({kind}) · {licence}\n",
            package.name, package.version
        );
        if let Some(repository) = &package.repository {
            let _ = writeln!(text, "{repository}");
        }
        for file in &package.files {
            let body = licences.texts.get(&file.text).map_or("", String::as_str);
            let _ = write!(text, "\n--- {}\n\n{body}", file.path);
        }
    }
    text
}

fn list(value: &Value) -> &[Value] {
    value.as_array().map_or(&[], Vec::as_slice)
}

fn string(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_owned()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn licence_files_are_known_by_their_names() {
        for name in [
            "LICENSE",
            "LICENSE-APACHE",
            "license.md",
            "COPYING",
            "NOTICE.txt",
            "Licence_MIT",
            "UNLICENSE",
            "LICENSE.APACHE",
            "LICENSE.httprouter",
            "license-apache-2.0",
            "LICENSE.spdx",
        ] {
            assert!(is_licence_name(name), "{name}");
        }
        for name in [
            "licensed.rs",
            "noticeboard.md",
            "README.md",
            "unlicensed.txt",
            "copyright.mjs",
            "copyright.mjs.map",
            "license.rs",
            "notice.js",
            "license.d.ts",
        ] {
            assert!(!is_licence_name(name), "{name}");
        }
    }

    #[test]
    fn an_expression_names_each_of_its_licences_once() {
        assert_eq!(
            identifiers("(MIT OR Apache-2.0) AND Unicode-3.0"),
            ["MIT", "Apache-2.0", "Unicode-3.0"]
        );
        assert_eq!(identifiers("MIT/Apache-2.0"), ["MIT", "Apache-2.0"]);
        assert_eq!(
            identifiers("Apache-2.0 WITH LLVM-exception OR Apache-2.0"),
            ["Apache-2.0", "LLVM-exception"]
        );
    }

    #[test]
    fn only_what_reaches_the_bundle_is_listed_from_the_lockfile() {
        let lock = json!({ "packages": {
            "": { "name": "observer-panel" },
            "node_modules/react": { "version": "19.3.1", "license": "MIT" },
            "node_modules/vitest": { "version": "5.0.5", "dev": true },
            "node_modules/fsevents": { "version": "2.3.3", "devOptional": true },
            "node_modules/vite": { "version": "8.3.5", "dev": true },
            "node_modules/a/node_modules/react": { "version": "18.3.1", "license": "MIT" },
        }});

        let keys: Vec<String> = shipped(&lock).into_iter().map(|(key, _)| key).collect();

        assert_eq!(
            keys,
            [
                "node_modules/a/node_modules/react",
                "node_modules/react",
                "node_modules/vite"
            ]
        );
        assert_eq!(package_name("node_modules/a/node_modules/react"), "react");
        assert_eq!(
            package_name("node_modules/@tanstack/react-query"),
            "@tanstack/react-query"
        );
    }

    fn package(folder: &Path, licence: &str) -> Found {
        Found {
            ecosystem: Ecosystem::Crate,
            name: "quiet".to_owned(),
            version: "1.2.3".to_owned(),
            licence: Some(licence.to_owned()),
            repository: None,
            folder: folder.to_path_buf(),
            extra: Vec::new(),
        }
    }

    #[test]
    fn a_package_with_no_file_and_an_unknown_licence_stops_the_build() {
        let folder = tempfile::tempdir().unwrap();
        let spdx = tempfile::tempdir().unwrap();
        fs::write(spdx.path().join("MIT.txt"), "MIT text\r\n\r\n").unwrap();

        let error = package(folder.path(), "MIT OR Zlib")
            .into_package(&mut Texts::default(), spdx.path())
            .unwrap_err()
            .to_string();
        assert!(error.starts_with("quiet 1.2.3 ships no licence file"), "{error}");
        assert!(error.contains("Zlib") && error.contains("MIT OR Zlib"), "{error}");

        let mut texts = Texts::default();
        let found = package(folder.path(), "MIT")
            .into_package(&mut texts, spdx.path())
            .unwrap();
        assert!(found.from_spdx);
        assert_eq!(texts.0[&found.files[0].text], "MIT text\n");
    }

    #[test]
    fn a_text_two_packages_ship_is_kept_once() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        fs::create_dir_all(second.path().join("vendor/fiat")).unwrap();
        for path in [
            first.path().join("LICENSE"),
            second.path().join("vendor/fiat/LICENSE"),
        ] {
            fs::write(path, "Permission is hereby granted\n").unwrap();
        }
        let mut texts = Texts::default();
        let spdx = Path::new("unused");

        let one = package(first.path(), "MIT")
            .into_package(&mut texts, spdx)
            .unwrap();
        let two = package(second.path(), "MIT")
            .into_package(&mut texts, spdx)
            .unwrap();

        assert_eq!(texts.0.len(), 1);
        assert_eq!(one.files[0].text, two.files[0].text);
        assert_eq!(two.files[0].path, "vendor/fiat/LICENSE");
        assert!(!two.from_spdx);
    }
}
