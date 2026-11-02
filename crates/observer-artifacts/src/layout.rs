use std::ffi::OsStr;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::error::{Error, LayoutProblem, io};
use crate::{Branch, CommitSha, Platform, disk};

const MANIFEST: &str = ".observer-build.json";
pub(crate) const MANIFEST_FORMAT: u32 = 2;

/// Where FXServer and its `citizen/` folder live inside a build.
const LINUX_SERVER: &str = "alpine/opt/cfx-server";

/// What FXServer writes into a build while it runs, relative to the folder that holds `citizen/`:
/// the crash dumps and the TLS pair it makes at its first start. Besides Observer's bridge (see
/// [`is_bridge_folder`]), everything else is never written again after the install; these are
/// left out of its measure and of an import.
pub const RUNTIME_PATHS: [&str; 3] = ["crashes", "server-tls.crt", "server-tls.key"];

/// Where Observer writes its bridge, relative to the folder that holds `citizen/`.
const SYSTEM_RESOURCES: &str = "citizen/system_resources";
const BRIDGE_PREFIX: &str = "sys_";
const BRIDGE_RANDOM_CHARS: usize = 12;

/// What Observer recorded about a build when it installed it, kept in the build's folder so the disk
/// alone says what is installed, even without the database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub format: u32,
    pub build: u32,
    pub platform: Platform,
    pub installed_at: Timestamp,
    /// Who installed or imported it; none for the command line.
    pub installed_by: Option<String>,
    /// Regular files and their bytes at install time, without the parts that keep changing.
    pub files: u64,
    pub bytes: u64,
    pub origin: Origin,
}

/// Where a build came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Origin {
    Download {
        commit: CommitSha,
        branch: Branch,
        source_url: String,
        archive_bytes: u64,
        archive_sha256: String,
    },
    /// Copied by the owner from a folder of theirs, which is only ever read.
    Import { folder: PathBuf },
}

impl Origin {
    pub fn archive_sha256(&self) -> Option<&str> {
        match self {
            Self::Download { archive_sha256, .. } => Some(archive_sha256),
            Self::Import { .. } => None,
        }
    }
}

/// The manifest Observer wrote before builds could be imported.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FirstManifest {
    build: u32,
    commit: CommitSha,
    branch: Branch,
    platform: Platform,
    source_url: String,
    archive_bytes: u64,
    archive_sha256: String,
    installed_at: Timestamp,
    files: u64,
    bytes: u64,
}

impl Manifest {
    pub fn read(build_dir: &Path) -> Result<Self, Error> {
        let path = build_dir.join(MANIFEST);
        let text = fs::read_to_string(&path).map_err(io(&path))?;
        let invalid = |error: serde_json::Error| Error::Manifest {
            path: path.clone(),
            reason: error.to_string(),
        };
        let value: serde_json::Value = serde_json::from_str(&text).map_err(invalid)?;
        match value.get("format").and_then(serde_json::Value::as_u64) {
            Some(1) => {
                let first: FirstManifest = serde_json::from_value(value).map_err(invalid)?;
                Ok(Self {
                    format: MANIFEST_FORMAT,
                    build: first.build,
                    platform: first.platform,
                    installed_at: first.installed_at,
                    installed_by: None,
                    files: first.files,
                    bytes: first.bytes,
                    origin: Origin::Download {
                        commit: first.commit,
                        branch: first.branch,
                        source_url: first.source_url,
                        archive_bytes: first.archive_bytes,
                        archive_sha256: first.archive_sha256,
                    },
                })
            }
            Some(2) => serde_json::from_value(value).map_err(invalid),
            Some(format) => Err(Error::Manifest {
                path,
                reason: format!("format {format} is newer than this Observer reads"),
            }),
            None => Err(Error::Manifest {
                path,
                reason: "it names no format".to_owned(),
            }),
        }
    }

    /// Written under a temporary name first, so a manifest is never half there.
    pub fn write(&self, build_dir: &Path) -> Result<(), Error> {
        let path = build_dir.join(MANIFEST);
        let partial = build_dir.join(format!("{MANIFEST}.partial"));
        let text = serde_json::to_string_pretty(self).expect("a manifest serializes");
        let mut file = fs::File::create_new(&partial).map_err(io(&partial))?;
        file.write_all(text.as_bytes()).map_err(io(&partial))?;
        file.sync_all().map_err(io(&partial))?;
        drop(file);
        disk::retry(|| fs::rename(&partial, &path)).map_err(io(&path))
    }
}

/// Whether a file at a build's root is the manifest, or one being written.
pub(crate) fn is_manifest(name: &OsStr) -> bool {
    name.to_str().is_some_and(|name| {
        name.strip_prefix(MANIFEST)
            .is_some_and(|rest| rest.is_empty() || rest == ".partial")
    })
}

#[derive(Debug, Clone, Copy)]
enum Expected {
    File,
    Executable,
    Folder,
}

/// Checks that a build has what FXServer needs to start, and reports the first thing missing.
pub(crate) fn validate(build_dir: &Path, platform: Platform) -> Result<(), Error> {
    let expected: &[(&'static str, Expected)] = match platform {
        Platform::Windows => &[("FXServer.exe", Expected::File), ("citizen", Expected::Folder)],
        Platform::Linux => &[
            ("run.sh", Expected::Executable),
            ("alpine/opt/cfx-server/FXServer", Expected::Executable),
            ("alpine/opt/cfx-server/ld-musl-x86_64.so.1", Expected::Executable),
            ("alpine/opt/cfx-server/citizen", Expected::Folder),
        ],
    };
    for &(path, expected) in expected {
        let refuse = |problem| Error::Layout { path, problem };
        // A link could point anywhere on the host; the build must hold the real thing.
        let metadata =
            fs::symlink_metadata(build_dir.join(path)).map_err(|_| refuse(LayoutProblem::Missing))?;
        let found = match expected {
            Expected::File | Expected::Executable => metadata.is_file(),
            Expected::Folder => metadata.is_dir(),
        };
        if !found {
            return Err(refuse(LayoutProblem::NotPlain));
        }
        if matches!(expected, Expected::Executable) && !executable(&metadata) {
            return Err(refuse(LayoutProblem::NotExecutable));
        }
    }
    Ok(())
}

#[cfg(unix)]
fn executable(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn executable(_metadata: &fs::Metadata) -> bool {
    true
}

/// Whether a folder of `system_resources` is a bridge Observer wrote: under the name drawn for a
/// start of FXServer, or `observer`, the one it had before. The fxserver crate draws the names.
pub fn is_bridge_folder(name: &str) -> bool {
    name == "observer"
        || name.strip_prefix(BRIDGE_PREFIX).is_some_and(|random| {
            random.len() == BRIDGE_RANDOM_CHARS
                && random
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || (b'2'..=b'7').contains(&byte))
        })
}

/// The [`RUNTIME_PATHS`] of a build, and the bridge folders it holds.
pub(crate) fn runtime_paths(build_dir: &Path, platform: Platform) -> Vec<PathBuf> {
    let server = match platform {
        Platform::Windows => build_dir.to_path_buf(),
        Platform::Linux => build_dir.join(LINUX_SERVER),
    };
    let mut paths: Vec<PathBuf> = RUNTIME_PATHS.iter().map(|path| server.join(path)).collect();
    // A folder that cannot be read fails the walk that follows, with its path.
    if let Ok(entries) = fs::read_dir(server.join(SYSTEM_RESOURCES)) {
        paths.extend(
            entries
                .flatten()
                .filter(|entry| entry.file_name().to_str().is_some_and(is_bridge_folder))
                .map(|entry| entry.path()),
        );
    }
    paths
}

/// Counts the regular files of a build and their bytes, without following links and without the
/// parts that keep changing.
pub(crate) fn measure(build_dir: &Path, platform: Platform) -> Result<(u64, u64), Error> {
    let skipped = runtime_paths(build_dir, platform);
    let mut totals = (0, 0);
    let mut pending = vec![build_dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).map_err(io(&dir))? {
            let entry = entry.map_err(io(&dir))?;
            let path = entry.path();
            if skipped.contains(&path) || (dir == build_dir && is_manifest(&entry.file_name())) {
                continue;
            }
            let kind = entry.file_type().map_err(io(&path))?;
            if kind.is_dir() {
                pending.push(path);
            } else if kind.is_file() {
                totals.0 += 1;
                totals.1 += entry.metadata().map_err(io(&path))?.len();
            }
        }
    }
    Ok(totals)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> Manifest {
        Manifest {
            format: MANIFEST_FORMAT,
            build: 37872,
            platform: Platform::Windows,
            installed_at: "2026-10-28T10:00:00Z".parse().unwrap(),
            installed_by: Some("ada".to_owned()),
            files: 563,
            bytes: 196_620_014,
            origin: Origin::Download {
                commit: "0cfcdb5ead57a4a26fcb6ebad98adbcd0273e607".parse().unwrap(),
                branch: Branch::master(),
                source_url: "https://runtime.fivem.net/artifacts/fivem/build_server_windows/master/37872-0cfcdb5ead57a4a26fcb6ebad98adbcd0273e607/server.zip".to_owned(),
                archive_bytes: 66_631_791,
                archive_sha256: "a".repeat(64),
            },
        }
    }

    #[test]
    fn the_manifest_reads_back_as_written() {
        let dir = tempfile::tempdir().unwrap();

        manifest().write(dir.path()).unwrap();

        assert_eq!(Manifest::read(dir.path()).unwrap(), manifest());
        assert!(!dir.path().join(".observer-build.json.partial").exists());
        let text = fs::read_to_string(dir.path().join(MANIFEST)).unwrap();
        assert!(text.contains("\"archiveSha256\""), "{text}");
        assert!(text.contains("\"kind\": \"download\""), "{text}");
        assert!(text.contains("\"platform\": \"windows\""), "{text}");
    }

    #[test]
    fn an_imported_build_names_the_folder_it_was_copied_from() {
        let dir = tempfile::tempdir().unwrap();
        let imported = Manifest {
            origin: Origin::Import {
                folder: PathBuf::from(r"C:\FXServer\build"),
            },
            ..manifest()
        };

        imported.write(dir.path()).unwrap();

        let back = Manifest::read(dir.path()).unwrap();
        assert_eq!(back, imported);
        assert_eq!(back.origin.archive_sha256(), None);
    }

    #[test]
    fn a_manifest_of_the_first_format_reads_as_a_download_by_the_command_line() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join(MANIFEST),
            r#"{
                "format": 1,
                "build": 37872,
                "commit": "0cfcdb5ead57a4a26fcb6ebad98adbcd0273e607",
                "branch": "master",
                "platform": "linux",
                "sourceUrl": "https://runtime.fivem.net/artifacts/fivem/build_proot_linux/master/37872-0cfcdb5ead57a4a26fcb6ebad98adbcd0273e607/fx.tar.xz",
                "archiveBytes": 39000000,
                "archiveSha256": "bbbb",
                "installedAt": "2026-10-28T10:00:00Z",
                "files": 900,
                "bytes": 170000000
            }"#,
        )
        .unwrap();

        let manifest = Manifest::read(dir.path()).unwrap();

        assert_eq!(manifest.installed_by, None);
        assert_eq!(manifest.platform, Platform::Linux);
        assert_eq!(manifest.origin.archive_sha256(), Some("bbbb"));
        assert_eq!(manifest.format, MANIFEST_FORMAT);
    }

    #[test]
    fn a_manifest_from_a_newer_observer_is_not_misread() {
        let dir = tempfile::tempdir().unwrap();
        let newer = Manifest {
            format: 3,
            ..manifest()
        };
        newer.write(dir.path()).unwrap();

        assert!(matches!(Manifest::read(dir.path()), Err(Error::Manifest { .. })));
    }

    #[test]
    fn a_windows_build_needs_fxserver_and_citizen() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("FXServer.exe"), b"MZ").unwrap();

        let error = validate(dir.path(), Platform::Windows).unwrap_err();
        assert!(
            matches!(
                error,
                Error::Layout {
                    path: "citizen",
                    problem: LayoutProblem::Missing
                }
            ),
            "{error}"
        );

        fs::create_dir(dir.path().join("citizen")).unwrap();
        validate(dir.path(), Platform::Windows).unwrap();
    }

    #[test]
    fn the_first_missing_piece_of_a_linux_build_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("alpine/opt/cfx-server/FXServer")).unwrap();

        let error = validate(dir.path(), Platform::Linux).unwrap_err();
        assert_eq!(error.to_string(), "the build is incomplete: run.sh is missing");

        fs::write(dir.path().join("run.sh"), "#!/bin/bash").unwrap();
        set_mode(&dir.path().join("run.sh"), 0o755);
        let error = validate(dir.path(), Platform::Linux).unwrap_err();
        assert!(
            matches!(
                error,
                Error::Layout {
                    path: "alpine/opt/cfx-server/FXServer",
                    problem: LayoutProblem::NotPlain
                }
            ),
            "{error}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_linux_launcher_without_its_executable_bit_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("run.sh"), "#!/bin/bash").unwrap();
        set_mode(&dir.path().join("run.sh"), 0o644);

        let error = validate(dir.path(), Platform::Linux).unwrap_err();

        assert!(
            matches!(
                error,
                Error::Layout {
                    path: "run.sh",
                    problem: LayoutProblem::NotExecutable
                }
            ),
            "{error}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_link_in_place_of_a_required_file_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink("/bin/sh", dir.path().join("run.sh")).unwrap();

        let error = validate(dir.path(), Platform::Linux).unwrap_err();

        assert!(
            matches!(
                error,
                Error::Layout {
                    path: "run.sh",
                    problem: LayoutProblem::NotPlain
                }
            ),
            "{error}"
        );
    }

    #[test]
    fn the_measure_leaves_out_what_fxserver_and_observer_write_at_runtime() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("crashes")).unwrap();
        fs::write(root.join("FXServer.exe"), [0; 10]).unwrap();
        fs::create_dir_all(root.join("citizen/system_resources")).unwrap();
        fs::write(root.join("citizen/system_resources/chat.lua"), [0; 5]).unwrap();
        for bridge in ["observer", "sys_k3jx7qmw2pza", "sys_234567abcdef"] {
            let folder = root.join("citizen/system_resources").join(bridge);
            fs::create_dir_all(&folder).unwrap();
            fs::write(folder.join("fxmanifest.lua"), [0; 100]).unwrap();
        }
        fs::write(root.join("crashes/dump.dmp"), [0; 1000]).unwrap();
        fs::write(root.join("server-tls.crt"), [0; 300]).unwrap();
        fs::write(root.join("server-tls.key"), [0; 200]).unwrap();
        manifest().write(root).unwrap();

        assert_eq!(measure(root, Platform::Windows).unwrap(), (2, 15));
    }

    #[test]
    fn only_drawn_names_and_the_old_fixed_one_are_bridge_folders() {
        for name in ["observer", "sys_abcdefghijkl", "sys_234567zzzzzz"] {
            assert!(is_bridge_folder(name), "{name}");
        }
        for name in [
            "monitor",
            "chat",
            "Observer",
            "sys_",
            "sys_abcdefghijk",
            "sys_abcdefghijklm",
            "sys_abcdefghijk1",
            "sys_ABCDEFGHIJKL",
            "xys_abcdefghijkl",
        ] {
            assert!(!is_bridge_folder(name), "{name}");
        }
    }

    #[cfg(unix)]
    fn set_mode(path: &Path, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    }

    #[cfg(not(unix))]
    fn set_mode(_path: &Path, _mode: u32) {}
}
