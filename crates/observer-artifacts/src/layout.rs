use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::error::{Error, io};
use crate::{Branch, CommitSha, Platform};

const MANIFEST: &str = ".observer-build.json";
pub(crate) const MANIFEST_FORMAT: u32 = 1;

/// Where FXServer and its `citizen/` folder live inside a build.
const LINUX_SERVER: &str = "alpine/opt/cfx-server";

/// The parts of an installed build that keep changing, relative to its server folder: the bridge
/// resource Observer adds, and the crash dumps FXServer writes next to itself. Everything else is
/// never written again after the install.
const MUTABLE: [&str; 2] = ["citizen/system_resources/observer", "crashes"];

/// What Observer recorded about a build when it installed it, kept in the build's folder so the disk
/// alone says what is installed, even without the database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub format: u32,
    pub build: u32,
    pub commit: CommitSha,
    pub branch: Branch,
    pub platform: Platform,
    pub source_url: String,
    pub archive_bytes: u64,
    pub archive_sha256: String,
    pub installed_at: Timestamp,
    /// Regular files and their bytes at install time, without the parts that keep changing.
    pub files: u64,
    pub bytes: u64,
}

impl Manifest {
    pub fn read(build_dir: &Path) -> Result<Self, Error> {
        let path = build_dir.join(MANIFEST);
        let text = fs::read_to_string(&path).map_err(io(&path))?;
        let manifest: Self = serde_json::from_str(&text).map_err(|error| Error::Manifest {
            path: path.clone(),
            reason: error.to_string(),
        })?;
        if manifest.format != MANIFEST_FORMAT {
            return Err(Error::Manifest {
                path,
                reason: format!("format {} is newer than this Observer reads", manifest.format),
            });
        }
        Ok(manifest)
    }

    /// Written under a temporary name first, so a manifest is never half there.
    pub(crate) fn write(&self, build_dir: &Path) -> Result<(), Error> {
        let path = build_dir.join(MANIFEST);
        let partial = build_dir.join(format!("{MANIFEST}.partial"));
        let text = serde_json::to_string_pretty(self).expect("a manifest serializes");
        let mut file = fs::File::create_new(&partial).map_err(io(&partial))?;
        file.write_all(text.as_bytes()).map_err(io(&partial))?;
        file.sync_all().map_err(io(&partial))?;
        drop(file);
        fs::rename(&partial, &path).map_err(io(&path))
    }
}

#[derive(Debug, Clone, Copy)]
enum Expected {
    File,
    Executable,
    Folder,
}

/// Checks that a build has what FXServer needs to start, and reports the first thing missing.
pub(crate) fn validate(build_dir: &Path, platform: Platform) -> Result<(), Error> {
    let expected: &[(&str, Expected)] = match platform {
        Platform::Windows => &[("FXServer.exe", Expected::File), ("citizen", Expected::Folder)],
        Platform::Linux => &[
            ("run.sh", Expected::Executable),
            ("alpine/opt/cfx-server/FXServer", Expected::Executable),
            ("alpine/opt/cfx-server/ld-musl-x86_64.so.1", Expected::Executable),
            ("alpine/opt/cfx-server/citizen", Expected::Folder),
        ],
    };
    for (path, expected) in expected {
        // A link could point anywhere on the host; the build must hold the real thing.
        let metadata = fs::symlink_metadata(build_dir.join(path))
            .map_err(|_| Error::Layout(format!("{path} is missing")))?;
        let found = match expected {
            Expected::File | Expected::Executable => metadata.is_file(),
            Expected::Folder => metadata.is_dir(),
        };
        if !found {
            return Err(Error::Layout(format!(
                "{path} is not a plain {}",
                noun(*expected)
            )));
        }
        if matches!(expected, Expected::Executable) && !executable(&metadata) {
            return Err(Error::Layout(format!("{path} is not executable")));
        }
    }
    Ok(())
}

fn noun(expected: Expected) -> &'static str {
    match expected {
        Expected::File | Expected::Executable => "file",
        Expected::Folder => "folder",
    }
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

/// Counts the regular files of a build and their bytes, without following links and without the
/// parts that keep changing.
pub(crate) fn measure(build_dir: &Path, platform: Platform) -> Result<(u64, u64), Error> {
    let server = match platform {
        Platform::Windows => build_dir.to_path_buf(),
        Platform::Linux => build_dir.join(LINUX_SERVER),
    };
    let skipped: Vec<PathBuf> = MUTABLE.iter().map(|path| server.join(path)).collect();
    let mut totals = (0, 0);
    let mut pending = vec![build_dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).map_err(io(&dir))? {
            let entry = entry.map_err(io(&dir))?;
            let path = entry.path();
            let kind = entry.file_type().map_err(io(&path))?;
            if kind.is_dir() && !skipped.contains(&path) {
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
            commit: "0cfcdb5ead57a4a26fcb6ebad98adbcd0273e607".parse().unwrap(),
            branch: Branch::master(),
            platform: Platform::Windows,
            source_url: "https://runtime.fivem.net/artifacts/fivem/build_server_windows/master/37872-0cfcdb5ead57a4a26fcb6ebad98adbcd0273e607/server.zip".to_owned(),
            archive_bytes: 66_631_791,
            archive_sha256: "a".repeat(64),
            installed_at: "2026-10-28T10:00:00Z".parse().unwrap(),
            files: 563,
            bytes: 196_620_014,
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
        assert!(text.contains("\"platform\": \"windows\""), "{text}");
    }

    #[test]
    fn a_manifest_from_a_newer_observer_is_not_misread() {
        let dir = tempfile::tempdir().unwrap();
        let newer = Manifest {
            format: 2,
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
        assert_eq!(
            error.to_string(),
            "the extracted build is incomplete: citizen is missing"
        );

        fs::create_dir(dir.path().join("citizen")).unwrap();
        validate(dir.path(), Platform::Windows).unwrap();
    }

    #[test]
    fn the_first_missing_piece_of_a_linux_build_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("alpine/opt/cfx-server/FXServer")).unwrap();

        let error = validate(dir.path(), Platform::Linux).unwrap_err();
        assert_eq!(
            error.to_string(),
            "the extracted build is incomplete: run.sh is missing"
        );

        fs::write(dir.path().join("run.sh"), "#!/bin/bash").unwrap();
        set_mode(&dir.path().join("run.sh"), 0o755);
        let error = validate(dir.path(), Platform::Linux).unwrap_err();
        assert_eq!(
            error.to_string(),
            "the extracted build is incomplete: alpine/opt/cfx-server/FXServer is not a plain file"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_linux_launcher_without_its_executable_bit_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("run.sh"), "#!/bin/bash").unwrap();
        set_mode(&dir.path().join("run.sh"), 0o644);

        let error = validate(dir.path(), Platform::Linux).unwrap_err();

        assert_eq!(
            error.to_string(),
            "the extracted build is incomplete: run.sh is not executable"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_link_in_place_of_a_required_file_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink("/bin/sh", dir.path().join("run.sh")).unwrap();

        let error = validate(dir.path(), Platform::Linux).unwrap_err();

        assert_eq!(
            error.to_string(),
            "the extracted build is incomplete: run.sh is not a plain file"
        );
    }

    #[test]
    fn the_measure_leaves_out_the_folders_that_keep_changing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("citizen/system_resources/observer")).unwrap();
        fs::create_dir_all(root.join("crashes")).unwrap();
        fs::write(root.join("FXServer.exe"), [0; 10]).unwrap();
        fs::write(root.join("citizen/system_resources/chat.lua"), [0; 5]).unwrap();
        fs::write(
            root.join("citizen/system_resources/observer/fxmanifest.lua"),
            [0; 100],
        )
        .unwrap();
        fs::write(root.join("crashes/dump.dmp"), [0; 1000]).unwrap();

        assert_eq!(measure(root, Platform::Windows).unwrap(), (2, 15));
    }

    #[cfg(unix)]
    fn set_mode(path: &Path, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    }

    #[cfg(not(unix))]
    fn set_mode(_path: &Path, _mode: u32) {}
}
