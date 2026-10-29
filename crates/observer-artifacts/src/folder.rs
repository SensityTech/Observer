//! What the artifacts folder holds: the builds a scan finds there, and their deletion.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::archive::build_number;
use crate::error::{Error, LayoutProblem, io};
use crate::install::{STAGING, blocking, remove_tree, suffix, sweep};
use crate::layout::{self, Manifest};
use crate::{Installer, Platform, disk};

/// Where a deleted build goes first, on the same volume, so that it leaves its name at once even
/// when a scanner holds one of its files.
const TRASH: &str = ".trash";
const STALE_STAGING: Duration = Duration::from_hours(24);

/// A build folder as a scan found it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scanned {
    pub build: u32,
    pub path: PathBuf,
    pub manifest: Option<Manifest>,
    pub health: Health,
    /// What the manifest counted, or what is on disk for a folder without one.
    pub bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    Valid,
    Broken(Broken),
}

/// Why FXServer cannot start from a build folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Broken {
    NoManifest,
    /// The manifest cannot be read, or names another build or platform.
    BadManifest,
    /// The folder is a link.
    Link,
    Layout {
        path: &'static str,
        problem: LayoutProblem,
    },
}

/// The manifest of the build in `build_dir`, and whether FXServer can start from it on
/// `platform`. The signature is checked once, at install or import: what is on disk after that
/// is the owner's host.
pub fn inspect(build_dir: &Path, build: u32, platform: Platform) -> (Option<Manifest>, Health) {
    match fs::symlink_metadata(build_dir) {
        Ok(metadata) if metadata.is_symlink() => return (None, Health::Broken(Broken::Link)),
        Ok(_) => {}
        Err(_) => return (None, Health::Broken(Broken::NoManifest)),
    }
    let manifest = match Manifest::read(build_dir) {
        Ok(manifest) => manifest,
        Err(Error::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            return (None, Health::Broken(Broken::NoManifest));
        }
        Err(_) => return (None, Health::Broken(Broken::BadManifest)),
    };
    if manifest.build != build || manifest.platform != platform {
        return (Some(manifest), Health::Broken(Broken::BadManifest));
    }
    let health = match layout::validate(build_dir, platform) {
        Ok(()) => Health::Valid,
        Err(Error::Layout { path, problem }) => Health::Broken(Broken::Layout { path, problem }),
        Err(_) => Health::Broken(Broken::Layout {
            path: "",
            problem: LayoutProblem::Missing,
        }),
    };
    (Some(manifest), health)
}

impl Installer {
    /// Every `<build>/` folder of the artifacts folder, newest number first. Folders with other
    /// names, `.staging` and `.trash` among them, are not builds.
    pub async fn installed(&self) -> Result<Vec<Scanned>, Error> {
        let root = self.root.clone();
        blocking(move || scan(&root, Platform::host())).await
    }

    /// Removes a build, and what earlier deletions left behind.
    pub async fn delete(&self, build: u32) -> Result<u64, Error> {
        let root = self.root.clone();
        blocking(move || delete(&root, build)).await
    }

    /// Removes what earlier jobs and deletions left: everything in `.trash`, and in `.staging`
    /// what is older than any job could be.
    pub async fn sweep(&self) {
        let root = self.root.clone();
        let swept = blocking(move || {
            sweep(&root.join(TRASH), Duration::ZERO);
            sweep(&root.join(STAGING), STALE_STAGING);
            Ok(())
        })
        .await;
        if let Err(error) = swept {
            tracing::warn!(%error, "cannot sweep the artifacts folder");
        }
    }
}

fn scan(root: &Path, platform: Platform) -> Result<Vec<Scanned>, Error> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(io(root)(error)),
    };
    let mut builds = Vec::new();
    for entry in entries {
        let entry = entry.map_err(io(root))?;
        let Some(build) = entry.file_name().to_str().and_then(build_number) else {
            continue;
        };
        let path = entry.path();
        let kind = entry.file_type().map_err(io(&path))?;
        if !kind.is_dir() && !kind.is_symlink() {
            continue;
        }
        let (manifest, health) = inspect(&path, build, platform);
        let bytes = match &manifest {
            Some(manifest) => manifest.bytes,
            None if kind.is_dir() => layout::measure(&path, platform).map_or(0, |(_, bytes)| bytes),
            None => 0,
        };
        builds.push(Scanned {
            build,
            path,
            manifest,
            health,
            bytes,
        });
    }
    builds.sort_by_key(|scanned| std::cmp::Reverse(scanned.build));
    Ok(builds)
}

fn delete(root: &Path, build: u32) -> Result<u64, Error> {
    let path = root.join(build.to_string());
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Err(Error::NotInstalled(build)),
        Err(error) => return Err(io(&path)(error)),
    };
    let bytes = match Manifest::read(&path) {
        Ok(manifest) => manifest.bytes,
        Err(_) if metadata.is_dir() => layout::measure(&path, Platform::host()).map_or(0, |(_, bytes)| bytes),
        Err(_) => 0,
    };
    let trash = root.join(TRASH);
    fs::create_dir_all(&trash).map_err(io(&trash))?;
    if !fs::symlink_metadata(&trash).map_err(io(&trash))?.is_dir() {
        return Err(Error::StagingLink(trash));
    }
    let held = trash.join(format!("{build}-{}", suffix()?));
    match disk::retry(|| fs::rename(&path, &held)) {
        Ok(()) => {}
        Err(error) if disk::busy(&error) => return Err(Error::Busy(path)),
        Err(error) => return Err(io(&path)(error)),
    }
    if let Err(error) = remove_tree(&held) {
        tracing::warn!(path = %held.display(), %error, "a deleted build is left in the trash for now");
    }
    sweep(&trash, Duration::ZERO);
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{MANIFEST_FORMAT, Origin};

    /// A build of the host's platform that passes the layout check, with its manifest.
    pub(crate) fn place(root: &Path, build: u32) -> PathBuf {
        let dir = root.join(build.to_string());
        match Platform::host() {
            Platform::Windows => {
                fs::create_dir_all(dir.join("citizen")).unwrap();
                fs::write(dir.join("FXServer.exe"), b"MZ").unwrap();
            }
            Platform::Linux => {
                let server = dir.join("alpine/opt/cfx-server");
                fs::create_dir_all(server.join("citizen")).unwrap();
                for file in [
                    dir.join("run.sh"),
                    server.join("FXServer"),
                    server.join("ld-musl-x86_64.so.1"),
                ] {
                    fs::write(&file, b"#!").unwrap();
                    executable(&file);
                }
            }
        }
        Manifest {
            format: MANIFEST_FORMAT,
            build,
            platform: Platform::host(),
            installed_at: "2026-10-28T10:00:00Z".parse().unwrap(),
            installed_by: None,
            files: 3,
            bytes: 1234,
            origin: Origin::Import {
                folder: PathBuf::from("/srv/fxserver"),
            },
        }
        .write(&dir)
        .unwrap();
        dir
    }

    #[cfg(unix)]
    fn executable(path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[cfg(not(unix))]
    fn executable(_path: &Path) {}

    #[test]
    fn a_scan_lists_the_build_folders_newest_first_and_says_which_are_broken() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        place(root, 35245);
        place(root, 37872);
        fs::create_dir_all(root.join("36000/citizen")).unwrap();
        let other = place(root, 36500);
        fs::rename(other, root.join("36501")).unwrap();
        fs::create_dir_all(root.join(".staging/37900-0badf00d")).unwrap();
        fs::create_dir_all(root.join("notes")).unwrap();
        fs::write(root.join("38000"), b"a file").unwrap();

        let scanned = scan(root, Platform::host()).unwrap();

        let found: Vec<(u32, Health)> = scanned.iter().map(|build| (build.build, build.health)).collect();
        assert_eq!(
            found,
            [
                (37872, Health::Valid),
                (36501, Health::Broken(Broken::BadManifest)),
                (36000, Health::Broken(Broken::NoManifest)),
                (35245, Health::Valid),
            ]
        );
        assert_eq!(scanned[0].bytes, 1234);
    }

    #[test]
    fn a_build_missing_a_part_is_broken_with_what_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        let build = place(dir.path(), 37872);
        let required = match Platform::host() {
            Platform::Windows => build.join("FXServer.exe"),
            Platform::Linux => build.join("run.sh"),
        };
        fs::remove_file(required).unwrap();

        let (_, health) = inspect(&build, 37872, Platform::host());

        assert!(
            matches!(
                health,
                Health::Broken(Broken::Layout {
                    problem: LayoutProblem::Missing,
                    ..
                })
            ),
            "{health:?}"
        );
    }

    #[test]
    fn a_deleted_build_leaves_its_name_at_once_and_nothing_in_the_trash() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        place(root, 37872);
        fs::create_dir_all(root.join(".trash/37000-0badf00d/citizen")).unwrap();

        let freed = delete(root, 37872).unwrap();

        assert_eq!(freed, 1234);
        assert!(!root.join("37872").exists());
        assert_eq!(fs::read_dir(root.join(".trash")).unwrap().count(), 0);
        assert!(matches!(delete(root, 37872), Err(Error::NotInstalled(37872))));
    }

    /// Opens a file the way an antivirus scanner does, without letting it be deleted or renamed.
    #[cfg(windows)]
    fn hold(path: &Path) -> fs::File {
        use std::os::windows::fs::OpenOptionsExt;
        fs::File::options().read(true).share_mode(1).open(path).unwrap()
    }

    #[cfg(windows)]
    #[test]
    fn a_build_a_scanner_holds_for_a_moment_is_deleted_once_it_lets_go() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let build = place(root, 37872);
        let held = hold(&build.join("FXServer.exe"));
        let scanner = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            drop(held);
        });

        let freed = delete(root, 37872).unwrap();

        scanner.join().unwrap();
        assert_eq!(freed, 1234);
        assert!(!root.join("37872").exists());
        assert_eq!(fs::read_dir(root.join(".trash")).unwrap().count(), 0);
    }

    #[cfg(windows)]
    #[test]
    fn a_build_a_scanner_keeps_holding_is_busy_and_stays() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let build = place(root, 37872);
        let _held = hold(&build.join("FXServer.exe"));

        let error = delete(root, 37872).unwrap_err();

        assert!(matches!(error, Error::Busy(_)), "{error}");
        assert!(build.join("FXServer.exe").exists());
    }

    /// A folder outside the artifacts folder, with a file and a folder a day old.
    fn elsewhere(dir: &Path) -> PathBuf {
        let folder = dir.join("photos");
        fs::create_dir_all(folder.join("2026")).unwrap();
        fs::write(folder.join("notes.txt"), b"keep me").unwrap();
        let two_days_ago = std::time::SystemTime::now() - Duration::from_hours(48);
        for old in [folder.join("notes.txt"), folder.join("2026")] {
            crate::install::tests::set_modified(&old, two_days_ago);
        }
        folder
    }

    /// A link to a folder any account may make: a symbolic link on Unix, a junction on Windows.
    fn link_folder(target: &Path, link: &Path) {
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, link).unwrap();
        #[cfg(windows)]
        {
            let made = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(link)
                .arg(target)
                .output()
                .unwrap();
            assert!(made.status.success(), "{made:?}");
        }
    }

    #[tokio::test]
    async fn the_start_sweep_leaves_alone_what_a_trash_or_staging_link_leads_to() {
        for holding in [TRASH, STAGING] {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().join("artifacts");
            fs::create_dir_all(&root).unwrap();
            let target = elsewhere(dir.path());
            link_folder(&target, &root.join(holding));

            Installer::new(crate::Http::new().unwrap(), root).sweep().await;

            assert!(target.join("notes.txt").exists(), "{holding}");
            assert!(target.join("2026").exists(), "{holding}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_build_observer_may_not_remove_fails_at_once_rather_than_as_busy() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("artifacts");
        place(&root, 37872);
        fs::create_dir_all(root.join(TRASH)).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o555)).unwrap();
        if fs::write(root.join("probe"), b"").is_ok() {
            // Root writes anywhere: there is nothing to refuse.
            fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
            return;
        }

        let started = std::time::Instant::now();
        let error = delete(&root, 37872).unwrap_err();
        let took = started.elapsed();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();

        assert!(matches!(error, Error::Io { .. }), "{error}");
        assert!(took < Duration::from_secs(1), "{took:?}");
        assert!(root.join("37872").exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_link_in_place_of_a_build_is_broken_and_its_deletion_leaves_the_target() {
        let dir = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let target = place(elsewhere.path(), 37872);
        std::os::unix::fs::symlink(&target, dir.path().join("37872")).unwrap();

        let scanned = scan(dir.path(), Platform::host()).unwrap();
        delete(dir.path(), 37872).unwrap();

        assert_eq!(scanned[0].health, Health::Broken(Broken::Link));
        assert!(target.join("run.sh").exists());
    }
}
