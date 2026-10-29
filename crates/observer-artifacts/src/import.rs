//! The import of a build folder the owner already has, by copy into the artifacts folder. The
//! source is only ever read; the copy is checked as an install's extraction is, and the files
//! FXServer and Observer write at runtime stay behind.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::error::{Error, ImportRefusal, io};
use crate::extract::{self, MAX_ENTRIES, MAX_UNPACKED, Refusal, Track};
use crate::install::{Installed, Progress, blocking, finish, stop_if, suffix, validate};
use crate::layout::{self, Origin};
use crate::{Installer, Platform, disk};

/// What the copy creates, in order: folders before what they hold, links last.
#[derive(Debug)]
struct Plan {
    entries: Vec<(PathBuf, Entry)>,
    /// Links by their path inside the build, `/`-separated, with their targets.
    links: HashMap<String, String>,
    bytes: u64,
}

#[derive(Debug)]
enum Entry {
    Folder,
    File,
}

impl Installer {
    /// Copies the build in `source` as build `build`. `cancel` stops the copy at the next entry
    /// until the copy is validated.
    pub async fn import<F>(
        &self,
        source: &Path,
        build: u32,
        installed_by: Option<String>,
        cancel: &CancellationToken,
        mut progress: F,
    ) -> Result<Installed, Error>
    where
        F: FnMut(Progress) + Send,
    {
        progress(Progress::Checking);
        let platform = Platform::host();
        let refuse = |refusal| Error::Import {
            path: source.to_path_buf(),
            refusal,
        };
        if !source.is_absolute() {
            return Err(refuse(ImportRefusal::NotAbsolute));
        }
        let target = self.root.join(build.to_string());
        if tokio::fs::symlink_metadata(&target).await.is_ok() {
            return Err(Error::AlreadyInstalled(build));
        }
        let staging = self.staging().await?;
        let (root, from) = (self.root.clone(), source.to_path_buf());
        let plan = blocking(move || check(&from, &root, platform)).await?;
        stop_if(cancel)?;

        let track = Track {
            cancel: cancel.clone(),
            done: Arc::default(),
        };
        progress(Progress::Copying {
            total: plan.entries.len() as u64 + plan.links.len() as u64,
            done: Arc::clone(&track.done),
        });
        let tree = staging.join(format!("{build}-{}", suffix()?));
        let (from, copy) = (source.to_path_buf(), tree.clone());
        let copied = blocking(move || copy_plan(&from, &copy, &plan, &track)).await;
        let staged = match copied.and_then(|()| stop_if(cancel)) {
            Ok(()) => {
                progress(Progress::Validating);
                let (checked, folder) = (tree.clone(), source.to_path_buf());
                blocking(move || validate(&checked, build, platform, installed_by, Origin::Import { folder }))
                    .await
            }
            Err(error) => Err(error),
        };
        finish(staged, tree, target, build, &mut progress).await
    }
}

/// The checks before anything is copied, and the list of what to copy.
fn check(source: &Path, root: &Path, platform: Platform) -> Result<Plan, Error> {
    let refuse = |refusal| Error::Import {
        path: source.to_path_buf(),
        refusal,
    };
    let metadata = fs::symlink_metadata(source).map_err(|_| refuse(ImportRefusal::NotAFolder))?;
    if metadata.is_symlink() || reparse_point(&metadata) {
        return Err(refuse(ImportRefusal::Link));
    }
    if !metadata.is_dir() {
        return Err(refuse(ImportRefusal::NotAFolder));
    }
    let canonical = fs::canonicalize(source).map_err(io(source))?;
    let artifacts = fs::canonicalize(root).map_err(io(root))?;
    // A folder holding the artifacts folder would copy the copy being made.
    if canonical.starts_with(&artifacts) || artifacts.starts_with(&canonical) {
        return Err(refuse(ImportRefusal::InsideArtifacts));
    }
    layout::validate(source, platform)?;
    let plan = list(source, platform)?;
    disk::require_space(root, plan.bytes + disk::SPACE_MARGIN)?;
    Ok(plan)
}

/// Walks the source without following links, leaving out what FXServer and Observer write at
/// runtime and an earlier manifest.
fn list(source: &Path, platform: Platform) -> Result<Plan, Error> {
    let skipped = layout::runtime_paths(source, platform);
    let mut plan = Plan {
        entries: Vec::new(),
        links: HashMap::new(),
        bytes: 0,
    };
    let mut pending = vec![PathBuf::new()];
    while let Some(relative) = pending.pop() {
        let dir = source.join(&relative);
        for entry in fs::read_dir(&dir).map_err(io(&dir))? {
            let entry = entry.map_err(io(&dir))?;
            let path = entry.path();
            let name = entry.file_name();
            if skipped.contains(&path) || (relative.as_os_str().is_empty() && layout::is_manifest(&name)) {
                continue;
            }
            let inside = relative.join(&name);
            let Some(shown) = inside.to_str().map(|text| text.replace('\\', "/")) else {
                return Err(extract::refuse(&inside.to_string_lossy(), Refusal::Control));
            };
            let metadata = fs::symlink_metadata(&path).map_err(io(&path))?;
            if plan.entries.len() + plan.links.len() >= MAX_ENTRIES {
                return Err(Error::Import {
                    path: source.to_path_buf(),
                    refusal: ImportRefusal::TooManyFiles,
                });
            }
            if metadata.is_symlink() && cfg!(unix) {
                let target = fs::read_link(&path).map_err(io(&path))?;
                let Some(target) = target.to_str() else {
                    return Err(extract::refuse(&shown, Refusal::Control));
                };
                plan.links.insert(shown, target.to_owned());
            } else if metadata.is_symlink() || reparse_point(&metadata) {
                return Err(Error::Import {
                    path,
                    refusal: ImportRefusal::Link,
                });
            } else if metadata.is_dir() {
                plan.entries.push((inside.clone(), Entry::Folder));
                pending.push(inside);
            } else if metadata.is_file() {
                plan.bytes = plan.bytes.saturating_add(metadata.len());
                if plan.bytes > MAX_UNPACKED {
                    return Err(Error::Import {
                        path: source.to_path_buf(),
                        refusal: ImportRefusal::TooLarge,
                    });
                }
                plan.entries.push((inside, Entry::File));
            } else {
                return Err(extract::refuse(&shown, Refusal::EntryType(special(&metadata))));
            }
        }
    }
    Ok(plan)
}

fn copy_plan(source: &Path, tree: &Path, plan: &Plan, track: &Track) -> Result<(), Error> {
    let canonical = fs::canonicalize(source).map_err(io(source))?;
    fs::create_dir(tree).map_err(io(tree))?;
    for (relative, entry) in &plan.entries {
        track.entry()?;
        let path = tree.join(relative);
        match entry {
            Entry::Folder => fs::create_dir_all(&path).map_err(io(&path))?,
            Entry::File => {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent).map_err(io(parent))?;
                }
                copy_file(source, &canonical, relative, &path)?;
            }
        }
    }
    let links = extract::confine(&plan.links).map_err(|error| match error {
        Error::Unsafe {
            entry,
            refusal: Refusal::LinkEscapes | Refusal::LinkOutsideAlpine | Refusal::LinkLoop,
        } => Error::Import {
            path: source.join(entry),
            refusal: ImportRefusal::Link,
        },
        error => error,
    })?;
    for (path, target) in links {
        track.entry()?;
        extract::create_link(&target, &tree.join(path))?;
    }
    Ok(())
}

/// Copies a file from where the listing found it. The folder can change between the listing and
/// the copy: a file read through a folder swapped for a link, or one that is no longer a plain
/// file, refuses the import rather than bringing in something from elsewhere.
fn copy_file(source: &Path, canonical: &Path, relative: &Path, to: &Path) -> Result<(), Error> {
    let from = source.join(relative);
    let link = || Error::Import {
        path: from.clone(),
        refusal: ImportRefusal::Link,
    };
    let (mut file, resolved) = match observer_platform::open_resolved(&from) {
        Ok(opened) => opened,
        Err(_) if fs::symlink_metadata(&from).is_ok_and(|metadata| metadata.is_symlink()) => {
            return Err(link());
        }
        Err(error) => return Err(io(&from)(error)),
    };
    let metadata = file.metadata().map_err(io(&from))?;
    if resolved != canonical.join(relative) || metadata.is_symlink() || reparse_point(&metadata) {
        return Err(link());
    }
    if !metadata.is_file() {
        let shown = relative.to_string_lossy().replace('\\', "/");
        return Err(extract::refuse(&shown, Refusal::EntryType(special(&metadata))));
    }
    let mut copy = fs::File::create_new(to).map_err(io(to))?;
    std::io::copy(&mut file, &mut copy).map_err(io(&from))?;
    extract::keep_mode(&copy, to, mode(&metadata))
}

#[cfg(unix)]
fn mode(metadata: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode()
}

#[cfg(not(unix))]
fn mode(_metadata: &fs::Metadata) -> u32 {
    0
}

/// A reparse point other than a link, such as a junction or a mount point, which could lead
/// anywhere on the host.
#[cfg(windows)]
fn reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}

#[cfg(unix)]
fn special(metadata: &fs::Metadata) -> String {
    use std::os::unix::fs::FileTypeExt;
    let kind = metadata.file_type();
    if kind.is_fifo() {
        "named pipe"
    } else if kind.is_socket() {
        "socket"
    } else if kind.is_block_device() {
        "block device"
    } else {
        "character device"
    }
    .to_owned()
}

#[cfg(not(unix))]
fn special(_metadata: &fs::Metadata) -> String {
    "device".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Http;
    use crate::install::STAGING;

    struct Lab {
        _dir: tempfile::TempDir,
        source: PathBuf,
        installer: Installer,
    }

    impl Lab {
        /// A build of the host's platform in `source/`, with what FXServer writes at runtime.
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let source = dir.path().join("fxserver");
            let server = match Platform::host() {
                Platform::Windows => {
                    fs::create_dir_all(source.join("citizen/scripting/lua")).unwrap();
                    fs::write(source.join("FXServer.exe"), b"MZ not really").unwrap();
                    source.clone()
                }
                Platform::Linux => {
                    let server = source.join("alpine/opt/cfx-server");
                    fs::create_dir_all(server.join("citizen/scripting/lua")).unwrap();
                    for (file, mode) in [
                        (source.join("run.sh"), 0o755),
                        (server.join("FXServer"), 0o755),
                        (server.join("ld-musl-x86_64.so.1"), 0o755),
                    ] {
                        fs::write(&file, b"#!").unwrap();
                        set_mode(&file, mode);
                    }
                    server
                }
            };
            fs::write(server.join("citizen/scripting/lua/scheduler.lua"), b"--").unwrap();
            fs::create_dir_all(server.join("crashes")).unwrap();
            fs::write(server.join("crashes/dump.dmp"), [0; 64]).unwrap();
            fs::create_dir_all(server.join("citizen/system_resources/observer")).unwrap();
            fs::write(
                server.join("citizen/system_resources/observer/fxmanifest.lua"),
                b"--",
            )
            .unwrap();
            fs::write(server.join("server-tls.crt"), b"cert").unwrap();
            fs::write(server.join("server-tls.key"), b"key").unwrap();
            let installer = Installer::new(Http::new().unwrap(), dir.path().join("data/artifacts"));
            Self {
                _dir: dir,
                source,
                installer,
            }
        }

        #[cfg(unix)]
        async fn import(&self) -> Result<Installed, Error> {
            self.installer
                .import(
                    &self.source,
                    35245,
                    Some("ada".to_owned()),
                    &CancellationToken::new(),
                    |_| {},
                )
                .await
        }

        fn staging_is_empty(&self) -> bool {
            fs::read_dir(self.installer.root().join(STAGING))
                .map_or(true, |mut entries| entries.next().is_none())
        }
    }

    #[cfg(unix)]
    fn set_mode(path: &Path, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    }

    #[cfg(not(unix))]
    fn set_mode(_path: &Path, _mode: u32) {}

    #[cfg(unix)]
    #[tokio::test]
    async fn an_imported_build_is_a_copy_without_what_fxserver_writes_at_runtime() {
        use std::os::unix::fs::PermissionsExt;

        let lab = Lab::new();
        let alpine = lab.source.join("alpine");
        fs::create_dir_all(alpine.join("bin")).unwrap();
        fs::write(alpine.join("bin/busybox"), b"\x7fELF").unwrap();
        std::os::unix::fs::symlink("busybox", alpine.join("bin/kill")).unwrap();
        std::os::unix::fs::symlink("/bin/busybox", alpine.join("bin/sh")).unwrap();

        let installed = lab.import().await.unwrap();

        let build = &installed.path;
        let server = build.join("alpine/opt/cfx-server");
        assert!(server.join("citizen/scripting/lua/scheduler.lua").is_file());
        assert!(!server.join("crashes").exists());
        assert!(!server.join("citizen/system_resources/observer").exists());
        assert!(!server.join("server-tls.crt").exists());
        assert_eq!(
            fs::read_link(build.join("alpine/bin/kill")).unwrap(),
            Path::new("busybox")
        );
        assert_eq!(
            fs::read_link(build.join("alpine/bin/sh")).unwrap(),
            Path::new("busybox")
        );
        assert_eq!(
            fs::metadata(build.join("run.sh")).unwrap().permissions().mode() & 0o777,
            0o755
        );
        assert_eq!(
            installed.manifest.origin,
            Origin::Import {
                folder: lab.source.clone()
            }
        );
        assert!(lab.source.join("alpine/opt/cfx-server/crashes/dump.dmp").exists());
        assert!(lab.staging_is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn an_imported_build_gets_the_modes_an_install_gives() {
        use std::os::unix::fs::PermissionsExt;

        let lab = Lab::new();
        set_mode(&lab.source.join("run.sh"), 0o777);
        set_mode(&lab.source.join("alpine/opt/cfx-server/FXServer"), 0o775);
        set_mode(
            &lab.source
                .join("alpine/opt/cfx-server/citizen/scripting/lua/scheduler.lua"),
            0o466,
        );

        let installed = lab.import().await.unwrap();

        let mode = |path: &str| {
            fs::metadata(installed.path.join(path))
                .unwrap()
                .permissions()
                .mode()
                & 0o7777
        };
        assert_eq!(mode("run.sh"), 0o755);
        assert_eq!(mode("alpine/opt/cfx-server/FXServer"), 0o755);
        assert_eq!(
            mode("alpine/opt/cfx-server/citizen/scripting/lua/scheduler.lua"),
            0o644
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_link_leaving_the_build_refuses_the_import() {
        let lab = Lab::new();
        std::os::unix::fs::symlink("../../../..", lab.source.join("alpine/opt/escape")).unwrap();

        let error = lab.import().await.unwrap_err();

        assert!(
            matches!(
                error,
                Error::Import {
                    refusal: ImportRefusal::Link,
                    ..
                }
            ),
            "{error}"
        );
        assert!(!lab.installer.root().join("35245").exists());
        assert!(lab.staging_is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_named_pipe_in_the_folder_refuses_the_import() {
        let lab = Lab::new();
        let status = std::process::Command::new("mkfifo")
            .arg(lab.source.join("pipe"))
            .status()
            .unwrap();
        assert!(status.success());

        let error = lab.import().await.unwrap_err();

        assert!(
            matches!(
                &error,
                Error::Unsafe {
                    refusal: Refusal::EntryType(kind),
                    ..
                } if kind == "named pipe"
            ),
            "{error}"
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn an_unsigned_fxserver_refuses_the_import_and_leaves_the_source_as_it_was() {
        let lab = Lab::new();
        let mut steps = Vec::new();

        let error = lab
            .installer
            .import(&lab.source, 35245, None, &CancellationToken::new(), |progress| {
                steps.push(progress);
            })
            .await
            .unwrap_err();

        assert!(matches!(error, Error::Signature(_)), "{error}");
        assert!(matches!(steps.last(), Some(Progress::Validating)), "{steps:?}");
        let copied = steps.iter().find_map(|progress| match progress {
            Progress::Copying { total, done } => {
                Some((*total, done.load(std::sync::atomic::Ordering::Relaxed)))
            }
            _ => None,
        });
        // FXServer.exe, then citizen with its folders and scheduler.lua; not what runs left.
        assert_eq!(copied, Some((6, 6)));
        assert!(!lab.installer.root().join("35245").exists());
        assert!(lab.staging_is_empty());
        assert!(lab.source.join("crashes/dump.dmp").exists());
    }

    #[tokio::test]
    async fn a_folder_inside_the_artifacts_folder_or_holding_it_is_refused() {
        let lab = Lab::new();
        let inside = lab.installer.root().join("copy");
        fs::create_dir_all(&inside).unwrap();
        let data = lab.installer.root().parent().unwrap().to_path_buf();

        for folder in [inside, data] {
            let error = lab
                .installer
                .import(&folder, 35245, None, &CancellationToken::new(), |_| {})
                .await
                .unwrap_err();

            assert!(
                matches!(
                    error,
                    Error::Import {
                        refusal: ImportRefusal::InsideArtifacts,
                        ..
                    }
                ),
                "{error}"
            );
        }
    }

    #[tokio::test]
    async fn a_folder_that_is_not_a_build_is_refused_before_anything_is_copied() {
        let lab = Lab::new();
        let empty = lab.source.parent().unwrap().join("empty");
        fs::create_dir_all(&empty).unwrap();
        let file = lab.source.parent().unwrap().join("notes.txt");
        fs::write(&file, b"notes").unwrap();

        let cancel = CancellationToken::new();
        let missing = lab.installer.import(&empty, 35245, None, &cancel, |_| {}).await;
        let not_a_folder = lab.installer.import(&file, 35245, None, &cancel, |_| {}).await;
        let relative = lab
            .installer
            .import(Path::new("fxserver"), 35245, None, &cancel, |_| {})
            .await;

        assert!(matches!(missing, Err(Error::Layout { .. })), "{missing:?}");
        assert!(
            matches!(
                not_a_folder,
                Err(Error::Import {
                    refusal: ImportRefusal::NotAFolder,
                    ..
                })
            ),
            "{not_a_folder:?}"
        );
        assert!(
            matches!(
                relative,
                Err(Error::Import {
                    refusal: ImportRefusal::NotAbsolute,
                    ..
                })
            ),
            "{relative:?}"
        );
        assert!(!lab.installer.root().join("35245").exists());
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
    async fn a_folder_swapped_for_a_link_once_it_was_listed_refuses_the_import() {
        let lab = Lab::new();
        let outside = lab.source.parent().unwrap().join("outside");
        fs::create_dir_all(outside.join("lua")).unwrap();
        fs::write(outside.join("lua/scheduler.lua"), b"not part of the build").unwrap();
        let citizen = match Platform::host() {
            Platform::Windows => lab.source.join("citizen"),
            Platform::Linux => lab.source.join("alpine/opt/cfx-server/citizen"),
        };

        let error = lab
            .installer
            .import(&lab.source, 35245, None, &CancellationToken::new(), |progress| {
                if matches!(progress, Progress::Copying { .. }) {
                    fs::rename(citizen.join("scripting"), citizen.join("scripting-listed")).unwrap();
                    link_folder(&outside, &citizen.join("scripting"));
                }
            })
            .await
            .unwrap_err();

        assert!(
            matches!(
                error,
                Error::Import {
                    refusal: ImportRefusal::Link,
                    ..
                }
            ),
            "{error}"
        );
        assert!(!lab.installer.root().join("35245").exists());
        assert!(lab.staging_is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_file_swapped_for_a_link_once_it_was_listed_refuses_the_import() {
        let lab = Lab::new();
        let outside = lab.source.parent().unwrap().join("outside.lua");
        fs::write(&outside, b"not part of the build").unwrap();
        let swapped = lab
            .source
            .join("alpine/opt/cfx-server/citizen/scripting/lua/scheduler.lua");

        let error = lab
            .installer
            .import(&lab.source, 35245, None, &CancellationToken::new(), |progress| {
                if matches!(progress, Progress::Copying { .. }) {
                    fs::remove_file(&swapped).unwrap();
                    std::os::unix::fs::symlink(&outside, &swapped).unwrap();
                }
            })
            .await
            .unwrap_err();

        assert!(
            matches!(
                error,
                Error::Import {
                    refusal: ImportRefusal::Link,
                    ..
                }
            ),
            "{error}"
        );
        assert!(lab.staging_is_empty());
    }

    #[tokio::test]
    async fn a_cancelled_copy_leaves_nothing_behind() {
        let lab = Lab::new();
        let cancel = CancellationToken::new();

        let error = lab
            .installer
            .import(&lab.source, 35245, None, &cancel, |progress| {
                if matches!(progress, Progress::Copying { .. }) {
                    cancel.cancel();
                }
            })
            .await
            .unwrap_err();

        assert!(matches!(error, Error::Cancelled), "{error}");
        assert!(!lab.installer.root().join("35245").exists());
        assert!(lab.staging_is_empty());
    }
}
