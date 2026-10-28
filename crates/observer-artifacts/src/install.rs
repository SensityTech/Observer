use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use jiff::Timestamp;
use reqwest::Url;

use crate::download::{self, Downloaded, RETRY_DELAYS};
use crate::error::{Error, io};
use crate::http::{ARCHIVE_LIMIT, Head, Http, MIB, too_large};
use crate::layout::{self, MANIFEST_FORMAT, Manifest};
use crate::{Branch, CommitSha, Platform, disk, extract};

const STAGING: &str = ".staging";
/// Far longer than an install takes: three download attempts of two hours at most, then seconds.
const STALE_STAGING: Duration = Duration::from_hours(24);
const HEAD_TIMEOUT: Duration = Duration::from_secs(30);
/// Room for the download and its extraction, checked before anything is fetched.
const WINDOWS_SPACE: u64 = 350 * MIB;
const LINUX_SPACE: u64 = 450 * MIB;
/// An xz archive does not say how much it unpacks to; FXServer's expand a little over four times.
const XZ_EXPANSION: u64 = 5;
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    Downloading { received: u64, total: u64 },
    Extracting,
    Verifying,
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
    http: Http,
    root: PathBuf,
    /// The SHA-256, in hexadecimal, of each archive the first time it was installed. Cfx.re
    /// publishes no checksum, so a later download that differs is refused rather than trusted. In
    /// memory for now.
    first_seen: Mutex<HashMap<(Platform, u32), String>>,
}

impl Installer {
    pub fn new(http: Http, root: PathBuf) -> Self {
        Self {
            http,
            root,
            first_seen: Mutex::default(),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The length of the artifacts folder when, on Windows, it leaves FXServer too little room for
    /// the paths of its deepest files.
    pub fn long_path(&self) -> Option<usize> {
        disk::long_path(&self.root)
    }

    pub async fn install<F>(&self, request: &InstallRequest, mut progress: F) -> Result<Installed, Error>
    where
        F: FnMut(Progress) + Send,
    {
        if request.platform != Platform::host() {
            return Err(Error::WrongPlatform(request.platform));
        }
        let target = self.root.join(request.build.to_string());
        if tokio::fs::symlink_metadata(&target).await.is_ok() {
            return Err(Error::AlreadyInstalled(request.build));
        }
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
            sweep(&swept);
            Ok(())
        })
        .await?;

        // A `HEAD` first: the docs page only links `server.7z`, so this proves `server.zip` is there.
        let head = self.head(&request.url).await?;
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
        let staged = self.stage(request, length, &archive, &tree, &mut progress).await;
        if let Err(error) = blocking(move || disk::remove_file(&archive).map_err(io(&archive))).await {
            tracing::warn!(%error, "a downloaded archive is left in the staging folder");
        }
        let result = match staged {
            Ok(manifest) => commit(tree.clone(), target, request.build)
                .await
                .map(|path| Installed { path, manifest }),
            Err(error) => Err(error),
        };
        if let Ok(installed) = &result {
            self.remember(request, &installed.manifest.archive_sha256);
        } else if let Err(error) = blocking(move || remove_tree(&tree).map_err(io(&tree))).await {
            tracing::warn!(%error, "a staged build is left in the staging folder");
        }
        result
    }

    async fn stage(
        &self,
        request: &InstallRequest,
        length: u64,
        archive: &Path,
        tree: &Path,
        progress: &mut (dyn FnMut(Progress) + Send),
    ) -> Result<Manifest, Error> {
        let downloaded = download::download(
            &self.http,
            &request.url,
            length,
            archive,
            &RETRY_DELAYS,
            &mut |received, total| {
                progress(Progress::Downloading { received, total });
            },
        )
        .await?;
        self.check_first_seen(request, &downloaded)?;

        progress(Progress::Extracting);
        let platform = request.platform;
        let (root, archive, tree) = (self.root.clone(), archive.to_path_buf(), tree.to_path_buf());
        let extracted = tree.clone();
        blocking(move || extract(platform, &root, &archive, downloaded.bytes, &extracted)).await?;

        progress(Progress::Verifying);
        let request = request.clone();
        blocking(move || {
            verify(&tree, platform)?;
            let (files, bytes) = layout::measure(&tree, platform)?;
            let manifest = Manifest {
                format: MANIFEST_FORMAT,
                build: request.build,
                commit: request.commit,
                branch: request.branch,
                platform,
                source_url: request.url.to_string(),
                archive_bytes: downloaded.bytes,
                archive_sha256: hex(&downloaded.sha256),
                installed_at: Timestamp::now(),
                files,
                bytes,
            };
            // Last, so that a build with a manifest is a build that passed every check.
            manifest.write(&tree)?;
            Ok(manifest)
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

    fn check_first_seen(&self, request: &InstallRequest, downloaded: &Downloaded) -> Result<(), Error> {
        let now = hex(&downloaded.sha256);
        match self.first_seen().get(&(request.platform, request.build)) {
            Some(first) if *first != now => Err(Error::ArchiveChanged {
                build: request.build,
                first: first.clone(),
                now,
            }),
            _ => Ok(()),
        }
    }

    /// Only an archive that passed every check is remembered: a download refused on the way, a
    /// maintenance page or a damaged file, would otherwise stand in for the real archive.
    fn remember(&self, request: &InstallRequest, sha256: &str) {
        self.first_seen()
            .entry((request.platform, request.build))
            .or_insert_with(|| sha256.to_owned());
    }

    fn first_seen(&self) -> MutexGuard<'_, HashMap<(Platform, u32), String>> {
        self.first_seen
            .lock()
            .expect("the fingerprint lock is never poisoned")
    }
}

fn extract(
    platform: Platform,
    root: &Path,
    archive: &Path,
    archive_bytes: u64,
    tree: &Path,
) -> Result<(), Error> {
    match platform {
        Platform::Windows => {
            let plan = extract::plan_zip(archive)?;
            disk::require_space(root, plan.unpacked + disk::SPACE_MARGIN)?;
            extract::extract_zip(archive, &plan, tree)
        }
        Platform::Linux => {
            let space = archive_bytes.saturating_mul(XZ_EXPANSION);
            disk::require_space(root, space + disk::SPACE_MARGIN)?;
            extract::extract_tar_xz(archive, tree, space)
        }
    }
}

/// The layout first, so that a missing FXServer.exe is reported as missing rather than unsigned.
fn verify(tree: &Path, platform: Platform) -> Result<(), Error> {
    layout::validate(tree, platform)?;
    // Only Windows builds are signed, and only Windows can check them; the platforms always match.
    #[cfg(windows)]
    {
        let server = tree.join("FXServer.exe");
        // Opened first, so that a file a scanner holds is reported as such, not as a bad signature.
        fs::File::open(&server).map_err(io(&server))?;
        observer_platform::verify_signature(&server, SIGNER).map_err(Error::Signature)?;
    }
    Ok(())
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

fn remove_tree(tree: &Path) -> io::Result<()> {
    match disk::retry(|| fs::remove_dir_all(tree)) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

/// Removes what earlier installs left in the staging folder, a tree a scanner held while it was
/// being removed or the download of an install that crashed, once it is too old for any install
/// still to be writing it.
fn sweep(staging: &Path) {
    let Ok(entries) = fs::read_dir(staging) else {
        return;
    };
    for entry in entries.flatten() {
        let stale = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .is_ok_and(|modified| modified.elapsed().is_ok_and(|age| age > STALE_STAGING));
        if !stale {
            continue;
        }
        let path = entry.path();
        let removed = if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            remove_tree(&path)
        } else {
            disk::remove_file(&path)
        };
        if let Err(error) = removed {
            tracing::warn!(path = %path.display(), %error, "cannot remove what an earlier install left");
        }
    }
}

async fn blocking<T, F>(function: F) -> Result<T, Error>
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

/// Keeps two installs of the same build, or the leftovers of a crashed one, apart in `.staging/`.
fn suffix() -> Result<String, Error> {
    let mut bytes = [0; 4];
    getrandom::fill(&mut bytes).map_err(Error::Random)?;
    Ok(hex(&bytes))
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    bytes
        .iter()
        .flat_map(|byte| [DIGITS[usize::from(byte >> 4)], DIGITS[usize::from(byte & 0xf)]])
        .map(char::from)
        .collect()
}

#[cfg(test)]
mod tests {
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
            }
        }

        fn staging_is_empty(&self) -> bool {
            fs::read_dir(self.installer.root().join(STAGING))
                .map_or(true, |mut entries| entries.next().is_none())
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_linux_build_is_installed_with_its_links_confined_and_its_modes_kept() {
        use std::os::unix::fs::PermissionsExt;

        let archive = host_build();
        let lab = Lab::new(vec![archive.clone()]).await;
        let mut downloaded = 0;

        let installed = lab
            .installer
            .install(&lab.request("/fx.tar.xz"), |progress| {
                if let Progress::Downloading { received, .. } = progress {
                    downloaded = received;
                }
            })
            .await
            .unwrap();

        let build = lab.installer.root().join("37872");
        assert_eq!(installed.path, build);
        assert_eq!(downloaded, archive.len() as u64);
        assert_eq!(
            fs::read_link(build.join("alpine/bin/kill")).unwrap(),
            Path::new("busybox")
        );
        assert_eq!(
            fs::metadata(build.join("run.sh")).unwrap().permissions().mode() & 0o777,
            0o755
        );
        assert_eq!(Manifest::read(&build).unwrap(), installed.manifest);
        assert_eq!(installed.manifest.archive_sha256, hex(&Sha256::digest(&archive)));
        assert_eq!(installed.manifest.files, 5);
        assert!(lab.staging_is_empty());
        assert_eq!(
            lab.installer.first_seen().get(&(Platform::Linux, 37872)),
            Some(&installed.manifest.archive_sha256)
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn an_fxserver_without_the_rockstar_signature_is_refused_and_leaves_nothing_behind() {
        let lab = Lab::new(vec![host_build()]).await;

        let error = lab
            .installer
            .install(&lab.request("/server.zip"), |_| {})
            .await
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "FXServer.exe is not signed by Rockstar Games: it is not a Windows executable"
        );
        assert!(!lab.installer.root().join("37872").exists());
        assert!(lab.staging_is_empty());
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn a_windows_build_without_fxserver_is_reported_incomplete_rather_than_unsigned() {
        use crate::samples::{ZipEntry, zip_archive};

        let lab = Lab::new(vec![zip_archive(&[ZipEntry::Deflated(
            "citizen/scripting/lua/scheduler.lua",
            b"-- scheduler",
        )])])
        .await;

        let error = lab
            .installer
            .install(&lab.request("/server.zip"), |_| {})
            .await
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "the extracted build is incomplete: FXServer.exe is missing"
        );
    }

    #[tokio::test]
    async fn an_archive_that_changed_since_its_first_install_is_refused() {
        let mut changed = host_build();
        changed.push(0);
        let lab = Lab::new(vec![changed]).await;
        let request = lab.request("/archive");
        // What an install of the genuine archive leaves; on Windows the sample cannot get that far,
        // its FXServer.exe carries no signature.
        lab.installer
            .remember(&request, &hex(&Sha256::digest(host_build())));

        let error = lab.installer.install(&request, |_| {}).await.unwrap_err();

        assert!(
            matches!(error, Error::ArchiveChanged { build: 37872, .. }),
            "{error}"
        );
        assert!(!lab.installer.root().join("37872").exists());
        assert!(lab.staging_is_empty());
    }

    #[tokio::test]
    async fn a_refused_first_download_does_not_stand_in_for_the_archive() {
        let lab = Lab::new(vec![b"<html>maintenance</html>".to_vec(), host_build()]).await;
        let request = lab.request("/archive");
        let first = lab.installer.install(&request, |_| {}).await.unwrap_err();
        assert!(matches!(first, Error::Damaged(_)), "{first}");

        let second = lab.installer.install(&request, |_| {}).await;

        if cfg!(windows) {
            assert!(matches!(second, Err(Error::Signature(_))), "{second:?}");
        } else {
            second.unwrap();
        }
    }

    #[tokio::test]
    async fn an_installed_build_is_never_downloaded_again() {
        let lab = Lab::new(vec![host_build()]).await;
        fs::create_dir_all(lab.installer.root().join("37872")).unwrap();

        let error = lab
            .installer
            .install(&lab.request("/archive"), |_| {})
            .await
            .unwrap_err();

        assert!(matches!(error, Error::AlreadyInstalled(37872)), "{error}");
        assert!(lab.server.requests().is_empty());
    }

    #[tokio::test]
    async fn a_missing_archive_is_found_out_before_downloading() {
        let lab = Lab::new(vec![host_build()]).await;

        let error = lab
            .installer
            .install(&lab.request("/missing"), |_| {})
            .await
            .unwrap_err();

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

        let result = lab.installer.install(&lab.request("/archive"), |_| {}).await;

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

        let _ = lab.installer.install(&lab.request("/archive"), |_| {}).await;

        assert!(!old_tree.exists());
        assert!(!old_part.exists());
        assert!(recent.exists());
    }

    fn set_modified(path: &Path, at: std::time::SystemTime) {
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

        let error = lab.installer.install(&request, |_| {}).await.unwrap_err();

        assert!(matches!(error, Error::WrongPlatform(_)), "{error}");
    }
}
