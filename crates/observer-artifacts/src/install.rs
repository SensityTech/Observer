use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::Duration;

use jiff::Timestamp;
use reqwest::Url;
use tokio_util::sync::CancellationToken;

use crate::download::{self, Downloaded, RETRY_DELAYS};
use crate::error::{Error, io};
use crate::extract::{self, Track};
use crate::http::{ARCHIVE_LIMIT, Head, Http, MIB, too_large};
use crate::layout::{self, MANIFEST_FORMAT, Manifest, Origin};
use crate::{Branch, CommitSha, Platform, disk};

pub(crate) const STAGING: &str = ".staging";
/// Far longer than an install takes: three download attempts of two hours at most, then seconds.
const STALE_STAGING: Duration = Duration::from_hours(24);
const HEAD_TIMEOUT: Duration = Duration::from_secs(30);
/// Room for the download and its extraction, checked before anything is fetched.
const WINDOWS_SPACE: u64 = 350 * MIB;
const LINUX_SPACE: u64 = 450 * MIB;
/// An xz archive does not say how much it unpacks to; FXServer's expand a little over four times.
const XZ_EXPANSION: u64 = 5;
const XZ_MAGIC: [u8; 6] = [0xFD, b'7', b'z', b'X', b'Z', 0x00];
/// Who signs FXServer.exe. Linux builds carry no signature at all.
#[cfg(windows)]
const SIGNER: &str = "Rockstar Games, Inc.";

/// A build to install, as the catalog describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallRequest {
    pub build: u32,
    pub commit: CommitSha,
    pub branch: Branch,
    pub platform: Platform,
    pub url: Url,
    /// The SHA-256, in hexadecimal, of the archive the first time this build was installed.
    /// Cfx.re publishes no checksum, so a later download that differs is refused rather than
    /// trusted.
    pub fingerprint: Option<String>,
    /// The user the manifest names; none for the command line.
    pub installed_by: Option<String>,
}

/// The step an install or an import is at. The counters of `Extracting` and `Copying` move on
/// while the step lasts.
#[derive(Debug, Clone)]
pub enum Progress {
    Checking,
    Downloading {
        received: u64,
        total: u64,
    },
    /// The download's fingerprint, the archive's index or header, and the room to unpack it.
    Verifying,
    /// `total` is the zip's entry count; an xz archive does not say.
    Extracting {
        total: Option<u64>,
        done: Arc<AtomicU64>,
    },
    Copying {
        total: u64,
        done: Arc<AtomicU64>,
    },
    /// The layout, the signature on Windows, the measure and the manifest.
    Validating,
    /// The rename that makes the build appear: past this point a cancellation has no effect.
    Committing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    pub path: PathBuf,
    pub manifest: Manifest,
}

/// Installs builds under one artifacts folder. A build is downloaded and extracted under
/// `.staging/`, verified there, and only then renamed to `<build>/`, so an interrupted install
/// never leaves a build that looks complete.
#[derive(Debug)]
pub struct Installer {
    pub(crate) http: Http,
    pub(crate) root: PathBuf,
}

impl Installer {
    pub fn new(http: Http, root: PathBuf) -> Self {
        Self { http, root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The length of the artifacts folder when, on Windows, it leaves FXServer too little room for
    /// the paths of its deepest files.
    pub fn long_path(&self) -> Option<usize> {
        disk::long_path(&self.root)
    }

    /// Downloads, verifies and extracts a build. `cancel` stops it at the next chunk or entry until
    /// the build is validated; the commit that follows runs whatever it says.
    pub async fn install<F>(
        &self,
        request: &InstallRequest,
        cancel: &CancellationToken,
        mut progress: F,
    ) -> Result<Installed, Error>
    where
        F: FnMut(Progress) + Send,
    {
        progress(Progress::Checking);
        if request.platform != Platform::host() {
            return Err(Error::WrongPlatform(request.platform));
        }
        let target = self.root.join(request.build.to_string());
        if tokio::fs::symlink_metadata(&target).await.is_ok() {
            return Err(Error::AlreadyInstalled(request.build));
        }
        let staging = self.staging().await?;

        // A `HEAD` first: the docs page only links `server.7z`, so this proves `server.zip` is there.
        let head = tokio::select! {
            head = self.head(&request.url) => head?,
            () = cancel.cancelled() => return Err(Error::Cancelled),
        };
        let length = match head.length {
            None => {
                return Err(Error::LengthMissing {
                    url: request.url.to_string(),
                });
            }
            Some(length) if length > ARCHIVE_LIMIT => return Err(too_large(&request.url, ARCHIVE_LIMIT)),
            Some(length) => length,
        };
        let space = match request.platform {
            Platform::Windows => WINDOWS_SPACE,
            Platform::Linux => LINUX_SPACE,
        };
        let root = self.root.clone();
        blocking(move || disk::require_space(&root, space)).await?;

        let name = format!("{}-{}", request.build, suffix()?);
        let archive = staging.join(format!("{name}.part"));
        let tree = staging.join(&name);
        let staged = self
            .stage(request, length, &archive, &tree, cancel, &mut progress)
            .await;
        if let Err(error) = blocking(move || disk::remove_file(&archive).map_err(io(&archive))).await {
            tracing::warn!(%error, "a downloaded archive is left in the staging folder");
        }
        finish(staged, tree, target, request.build, &mut progress).await
    }

    /// The staging folder, created if need be, once what earlier jobs left there is swept.
    pub(crate) async fn staging(&self) -> Result<PathBuf, Error> {
        let staging = self.root.join(STAGING);
        tokio::fs::create_dir_all(&staging).await.map_err(io(&staging))?;
        // The final rename is only atomic within one volume, which a link could leave.
        let metadata = tokio::fs::symlink_metadata(&staging)
            .await
            .map_err(io(&staging))?;
        if !metadata.is_dir() {
            return Err(Error::StagingLink(staging));
        }
        let swept = staging.clone();
        blocking(move || {
            sweep(&swept, STALE_STAGING);
            Ok(())
        })
        .await?;
        Ok(staging)
    }

    async fn stage(
        &self,
        request: &InstallRequest,
        length: u64,
        archive: &Path,
        tree: &Path,
        cancel: &CancellationToken,
        progress: &mut (dyn FnMut(Progress) + Send),
    ) -> Result<Manifest, Error> {
        let downloaded = download::download(
            &self.http,
            &request.url,
            length,
            archive,
            &RETRY_DELAYS,
            cancel,
            &mut |received, total| {
                progress(Progress::Downloading { received, total });
            },
        )
        .await?;

        progress(Progress::Verifying);
        check_fingerprint(request, &downloaded)?;
        let platform = request.platform;
        let (root, archive, tree) = (self.root.clone(), archive.to_path_buf(), tree.to_path_buf());
        let unpacking = archive.clone();
        let unpack = blocking(move || prepare(platform, &root, &unpacking, downloaded.bytes)).await?;
        stop_if(cancel)?;

        let track = Track {
            cancel: cancel.clone(),
            done: Arc::default(),
        };
        progress(Progress::Extracting {
            total: unpack.entries(),
            done: Arc::clone(&track.done),
        });
        let extracted = tree.clone();
        blocking(move || unpack.into(&archive, &extracted, &track)).await?;
        stop_if(cancel)?;

        progress(Progress::Validating);
        let request = request.clone();
        blocking(move || {
            let origin = Origin::Download {
                commit: request.commit,
                branch: request.branch,
                source_url: request.url.to_string(),
                archive_bytes: downloaded.bytes,
                archive_sha256: hex(&downloaded.sha256),
            };
            validate(&tree, request.build, platform, request.installed_by, origin)
        })
        .await
    }

    /// Gets as many attempts as the download after it.
    async fn head(&self, url: &Url) -> Result<Head, Error> {
        let mut delays = RETRY_DELAYS.iter();
        loop {
            match self.http.head(url, HEAD_TIMEOUT).await {
                Err(error) if error.is_transient() => {
                    let Some(delay) = delays.next() else {
                        return Err(error);
                    };
                    tracing::warn!(%url, %error, "the archive did not answer, trying again");
                    tokio::time::sleep(*delay).await;
                }
                result => return result,
            }
        }
    }
}

/// Commits a staged build, or removes what is left of it.
pub(crate) async fn finish(
    staged: Result<Manifest, Error>,
    tree: PathBuf,
    target: PathBuf,
    build: u32,
    progress: &mut (dyn FnMut(Progress) + Send),
) -> Result<Installed, Error> {
    let result = match staged {
        Ok(manifest) => {
            progress(Progress::Committing);
            commit(tree.clone(), target, build)
                .await
                .map(|path| Installed { path, manifest })
        }
        Err(error) => Err(error),
    };
    if result.is_err()
        && let Err(error) = blocking(move || remove_tree(&tree).map_err(io(&tree))).await
    {
        tracing::warn!(%error, "a staged build is left in the staging folder");
    }
    result
}

fn check_fingerprint(request: &InstallRequest, downloaded: &Downloaded) -> Result<(), Error> {
    let now = hex(&downloaded.sha256);
    match &request.fingerprint {
        Some(first) if *first != now => Err(Error::ArchiveChanged {
            build: request.build,
            first: first.clone(),
            now,
        }),
        _ => Ok(()),
    }
}

pub(crate) fn stop_if(cancel: &CancellationToken) -> Result<(), Error> {
    if cancel.is_cancelled() {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}

/// How an archive is unpacked, once its index or header was read and the room for it checked.
enum Unpack {
    Zip(extract::Plan),
    TarXz { space: u64 },
}

impl Unpack {
    fn entries(&self) -> Option<u64> {
        match self {
            Self::Zip(plan) => Some(plan.entries() as u64),
            Self::TarXz { .. } => None,
        }
    }

    fn into(self, archive: &Path, tree: &Path, track: &Track) -> Result<(), Error> {
        match self {
            Self::Zip(plan) => extract::extract_zip(archive, &plan, tree, track),
            Self::TarXz { space } => extract::extract_tar_xz(archive, tree, space, track),
        }
    }
}

fn prepare(platform: Platform, root: &Path, archive: &Path, archive_bytes: u64) -> Result<Unpack, Error> {
    match platform {
        Platform::Windows => {
            let plan = extract::plan_zip(archive)?;
            disk::require_space(root, plan.unpacked + disk::SPACE_MARGIN)?;
            Ok(Unpack::Zip(plan))
        }
        Platform::Linux => {
            let mut magic = [0; 6];
            let read = fs::File::open(archive).and_then(|mut file| file.read_exact(&mut magic));
            if read.is_err() || magic != XZ_MAGIC {
                return Err(Error::Damaged("it is not an xz archive".to_owned()));
            }
            let space = archive_bytes.saturating_mul(XZ_EXPANSION);
            disk::require_space(root, space + disk::SPACE_MARGIN)?;
            Ok(Unpack::TarXz { space })
        }
    }
}

/// Checks a staged build and writes its manifest. The layout first, so that a missing
/// FXServer.exe is reported as missing rather than unsigned.
pub(crate) fn validate(
    tree: &Path,
    build: u32,
    platform: Platform,
    installed_by: Option<String>,
    origin: Origin,
) -> Result<Manifest, Error> {
    layout::validate(tree, platform)?;
    // Only Windows builds are signed, and only Windows can check them; the platforms always match.
    #[cfg(windows)]
    {
        let server = tree.join("FXServer.exe");
        // Opened first, so that a file a scanner holds is reported as such, not as a bad signature.
        fs::File::open(&server).map_err(io(&server))?;
        observer_platform::verify_signature(&server, SIGNER).map_err(Error::Signature)?;
    }
    let (files, bytes) = layout::measure(tree, platform)?;
    let manifest = Manifest {
        format: MANIFEST_FORMAT,
        build,
        platform,
        installed_at: Timestamp::now(),
        installed_by,
        files,
        bytes,
        origin,
    };
    manifest.write(tree)?;
    Ok(manifest)
}

async fn commit(tree: PathBuf, target: PathBuf, build: u32) -> Result<PathBuf, Error> {
    blocking(move || {
        if fs::symlink_metadata(&target).is_ok() {
            return Err(Error::AlreadyInstalled(build));
        }
        // The staged tree is what a scanner holds open, not the target that does not exist yet.
        disk::retry(|| fs::rename(&tree, &target)).map_err(io(&tree))?;
        Ok(target)
    })
    .await
}

/// Removes a tree without following its links.
pub(crate) fn remove_tree(tree: &Path) -> io::Result<()> {
    match disk::retry(|| fs::remove_dir_all(tree)) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

/// Removes what earlier jobs left in a holding folder, a tree a scanner held while it was being
/// removed or the download of an install that crashed, once it is older than `stale`. A holding
/// folder that is a link, a junction included, is left alone: what it leads to is not Observer's.
pub(crate) fn sweep(holding: &Path, stale: Duration) {
    match fs::symlink_metadata(holding) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => {
            tracing::warn!(
                path = %holding.display(),
                "a holding folder of the artifacts folder is a link or a file: nothing in it is removed"
            );
            return;
        }
        Err(_) => return,
    }
    let Ok(entries) = fs::read_dir(holding) else {
        return;
    };
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .is_ok_and(|modified| modified.elapsed().is_ok_and(|age| age >= stale));
        if !old {
            continue;
        }
        let path = entry.path();
        let removed = if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            remove_tree(&path)
        } else {
            disk::remove_file(&path)
        };
        if let Err(error) = removed {
            tracing::warn!(path = %path.display(), %error, "cannot remove what an earlier job left");
        }
    }
}

pub(crate) async fn blocking<T, F>(function: F) -> Result<T, Error>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, Error> + Send + 'static,
{
    match tokio::task::spawn_blocking(function).await {
        Ok(result) => result,
        Err(error) if error.is_panic() => std::panic::resume_unwind(error.into_panic()),
        Err(_) => Err(Error::Interrupted),
    }
}

/// Keeps two jobs on the same build, or the leftovers of a crashed one, apart.
pub(crate) fn suffix() -> Result<String, Error> {
    let mut bytes = [0; 4];
    getrandom::fill(&mut bytes).map_err(Error::Random)?;
    Ok(hex(&bytes))
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    bytes
        .iter()
        .flat_map(|byte| [DIGITS[usize::from(byte >> 4)], DIGITS[usize::from(byte & 0xf)]])
        .map(char::from)
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use sha2::{Digest, Sha256};

    use super::*;
    use crate::samples::host_build;
    use crate::test_server::{Reply, Request, TestServer};

    const COMMIT: &str = "0cfcdb5ead57a4a26fcb6ebad98adbcd0273e607";

    struct Lab {
        _dir: tempfile::TempDir,
        installer: Installer,
        server: TestServer,
    }

    impl Lab {
        /// Serves `archives[n]` to the n-th download, and the last one to any download after.
        async fn new(archives: Vec<Vec<u8>>) -> Self {
            let downloads = AtomicUsize::new(0);
            Self::serving(move |request, _| {
                if request.path == "/missing" {
                    return Reply::status(404);
                }
                let served = downloads.load(Ordering::SeqCst).min(archives.len() - 1);
                if request.method == "GET" {
                    downloads.fetch_add(1, Ordering::SeqCst);
                }
                Reply::ok(archives[served].clone())
            })
            .await
        }

        async fn serving(handler: impl Fn(&Request, usize) -> Reply + Send + Sync + 'static) -> Self {
            let server = TestServer::start(handler).await;
            let dir = tempfile::tempdir().unwrap();
            let installer = Installer::new(Http::local(server.port), dir.path().join("artifacts"));
            Self {
                _dir: dir,
                installer,
                server,
            }
        }

        fn request(&self, path: &str) -> InstallRequest {
            InstallRequest {
                build: 37872,
                commit: COMMIT.parse().unwrap(),
                branch: Branch::master(),
                platform: Platform::host(),
                url: self.server.url(path),
                fingerprint: None,
                installed_by: Some("ada".to_owned()),
            }
        }

        async fn install(&self, request: &InstallRequest) -> Result<Installed, Error> {
            self.installer
                .install(request, &CancellationToken::new(), |_| {})
                .await
        }

        fn staging_is_empty(&self) -> bool {
            fs::read_dir(self.installer.root().join(STAGING))
                .map_or(true, |mut entries| entries.next().is_none())
        }
    }

    /// The steps an install went through, each once, in order.
    fn steps(seen: &[Progress]) -> Vec<&'static str> {
        let mut steps: Vec<&'static str> = Vec::new();
        for progress in seen {
            let step = match progress {
                Progress::Checking => "checking",
                Progress::Downloading { .. } => "downloading",
                Progress::Verifying => "verifying",
                Progress::Extracting { .. } => "extracting",
                Progress::Copying { .. } => "copying",
                Progress::Validating => "validating",
                Progress::Committing => "committing",
            };
            if steps.last() != Some(&step) {
                steps.push(step);
            }
        }
        steps
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_linux_build_is_installed_with_its_links_confined_and_its_modes_kept() {
        use std::os::unix::fs::PermissionsExt;

        let archive = host_build();
        let lab = Lab::new(vec![archive.clone()]).await;
        let mut seen = Vec::new();

        let installed = lab
            .installer
            .install(
                &lab.request("/fx.tar.xz"),
                &CancellationToken::new(),
                |progress| {
                    seen.push(progress);
                },
            )
            .await
            .unwrap();

        let build = lab.installer.root().join("37872");
        assert_eq!(installed.path, build);
        assert_eq!(
            steps(&seen),
            [
                "checking",
                "downloading",
                "verifying",
                "extracting",
                "validating",
                "committing"
            ]
        );
        let extracted = seen.iter().find_map(|progress| match progress {
            Progress::Extracting { total, done } => Some((*total, done.load(Ordering::Relaxed))),
            _ => None,
        });
        assert_eq!(extracted, Some((None, 6)));
        assert_eq!(
            fs::read_link(build.join("alpine/bin/kill")).unwrap(),
            Path::new("busybox")
        );
        assert_eq!(
            fs::metadata(build.join("run.sh")).unwrap().permissions().mode() & 0o777,
            0o755
        );
        assert_eq!(Manifest::read(&build).unwrap(), installed.manifest);
        assert_eq!(
            installed.manifest.origin.archive_sha256(),
            Some(hex(&Sha256::digest(&archive)).as_str())
        );
        assert_eq!(installed.manifest.installed_by.as_deref(), Some("ada"));
        assert_eq!(installed.manifest.files, 5);
        assert!(lab.staging_is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_cancel_once_the_build_is_validated_is_too_late() {
        let lab = Lab::new(vec![host_build()]).await;
        let cancel = CancellationToken::new();

        let installed = lab
            .installer
            .install(&lab.request("/fx.tar.xz"), &cancel, |progress| {
                if matches!(progress, Progress::Committing) {
                    cancel.cancel();
                }
            })
            .await;

        assert!(installed.is_ok(), "{installed:?}");
        assert!(lab.installer.root().join("37872").is_dir());
    }

    #[tokio::test]
    async fn a_cancelled_extraction_leaves_nothing_behind() {
        let lab = Lab::new(vec![host_build()]).await;
        let cancel = CancellationToken::new();

        let error = lab
            .installer
            .install(&lab.request("/archive"), &cancel, |progress| {
                if matches!(progress, Progress::Extracting { .. }) {
                    cancel.cancel();
                }
            })
            .await
            .unwrap_err();

        assert!(matches!(error, Error::Cancelled), "{error}");
        assert!(!lab.installer.root().join("37872").exists());
        assert!(lab.staging_is_empty());
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn an_fxserver_without_the_rockstar_signature_fails_its_validation_and_leaves_nothing_behind() {
        let lab = Lab::new(vec![host_build()]).await;
        let mut seen = Vec::new();

        let error = lab
            .installer
            .install(
                &lab.request("/server.zip"),
                &CancellationToken::new(),
                |progress| {
                    seen.push(progress);
                },
            )
            .await
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "FXServer.exe is not signed by Rockstar Games: it is not a Windows executable"
        );
        assert_eq!(
            steps(&seen),
            ["checking", "downloading", "verifying", "extracting", "validating"]
        );
        let extracted = seen.iter().find_map(|progress| match progress {
            Progress::Extracting { total, done } => Some((*total, done.load(Ordering::Relaxed))),
            _ => None,
        });
        assert_eq!(extracted, Some((Some(3), 3)));
        assert!(!lab.installer.root().join("37872").exists());
        assert!(lab.staging_is_empty());
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn a_windows_build_without_fxserver_is_reported_incomplete_rather_than_unsigned() {
        use crate::LayoutProblem;
        use crate::samples::{ZipEntry, zip_archive};

        let lab = Lab::new(vec![zip_archive(&[ZipEntry::Deflated(
            "citizen/scripting/lua/scheduler.lua",
            b"-- scheduler",
        )])])
        .await;

        let error = lab.install(&lab.request("/server.zip")).await.unwrap_err();

        assert!(
            matches!(
                error,
                Error::Layout {
                    path: "FXServer.exe",
                    problem: LayoutProblem::Missing
                }
            ),
            "{error}"
        );
    }

    #[tokio::test]
    async fn an_archive_that_changed_since_its_first_install_is_refused() {
        let mut changed = host_build();
        changed.push(0);
        let lab = Lab::new(vec![changed]).await;
        let request = InstallRequest {
            fingerprint: Some(hex(&Sha256::digest(host_build()))),
            ..lab.request("/archive")
        };

        let error = lab.install(&request).await.unwrap_err();

        assert!(
            matches!(error, Error::ArchiveChanged { build: 37872, .. }),
            "{error}"
        );
        assert!(!lab.installer.root().join("37872").exists());
        assert!(lab.staging_is_empty());
    }

    #[tokio::test]
    async fn an_installed_build_is_never_downloaded_again() {
        let lab = Lab::new(vec![host_build()]).await;
        fs::create_dir_all(lab.installer.root().join("37872")).unwrap();

        let error = lab.install(&lab.request("/archive")).await.unwrap_err();

        assert!(matches!(error, Error::AlreadyInstalled(37872)), "{error}");
        assert!(lab.server.requests().is_empty());
    }

    #[tokio::test]
    async fn a_missing_archive_is_found_out_before_downloading() {
        let lab = Lab::new(vec![host_build()]).await;

        let error = lab.install(&lab.request("/missing")).await.unwrap_err();

        assert!(matches!(error, Error::Status { status: 404, .. }), "{error}");
        let methods: Vec<String> = lab
            .server
            .requests()
            .into_iter()
            .map(|request| request.method)
            .collect();
        assert_eq!(methods, ["HEAD"]);
    }

    #[tokio::test]
    async fn a_head_that_fails_once_is_tried_again_like_the_download() {
        let archive = host_build();
        let lab = Lab::serving(move |_, index| {
            if index == 0 {
                Reply::status(503)
            } else {
                Reply::ok(archive.clone())
            }
        })
        .await;

        let result = lab.install(&lab.request("/archive")).await;

        assert!(!matches!(result, Err(Error::Status { .. })), "{result:?}");
        let methods: Vec<String> = lab
            .server
            .requests()
            .into_iter()
            .map(|request| request.method)
            .collect();
        assert_eq!(methods, ["HEAD", "HEAD", "GET"]);
    }

    #[tokio::test]
    async fn what_an_earlier_install_left_is_removed_once_it_is_a_day_old() {
        let lab = Lab::new(vec![host_build()]).await;
        let staging = lab.installer.root().join(STAGING);
        let old_tree = staging.join("37000-0badf00d");
        let old_part = staging.join("37000-0badf00d.part");
        let recent = staging.join("37001-cafebabe.part");
        fs::create_dir_all(old_tree.join("citizen")).unwrap();
        fs::write(&old_part, b"PK").unwrap();
        fs::write(&recent, b"PK").unwrap();
        let two_days_ago = std::time::SystemTime::now() - Duration::from_hours(48);
        for old in [&old_part, &old_tree] {
            set_modified(old, two_days_ago);
        }

        let _ = lab.install(&lab.request("/archive")).await;

        assert!(!old_tree.exists());
        assert!(!old_part.exists());
        assert!(recent.exists());
    }

    pub(crate) fn set_modified(path: &Path, at: std::time::SystemTime) {
        let mut options = fs::File::options();
        options.read(true);
        // Windows needs the right to write attributes, and opens a folder only with backup semantics.
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.access_mode(0x100).custom_flags(0x0200_0000);
        }
        options.open(path).unwrap().set_modified(at).unwrap();
    }

    #[tokio::test]
    async fn a_build_for_the_other_platform_is_refused() {
        let lab = Lab::new(vec![host_build()]).await;
        let request = InstallRequest {
            platform: Platform::host().other(),
            ..lab.request("/archive")
        };

        let error = lab.install(&request).await.unwrap_err();

        assert!(matches!(error, Error::WrongPlatform(_)), "{error}");
    }
}
