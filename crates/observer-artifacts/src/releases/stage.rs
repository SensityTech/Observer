use std::fs::File;
use std::io::{self, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use observer_domain::updates::{self, SIGNATURE, SUMS};
use reqwest::header::HeaderMap;
use reqwest::{Method, Url};
use tokio_util::sync::CancellationToken;

use super::{Published, Releases, key};
use crate::disk;
use crate::download::{self, RETRY_DELAYS};
use crate::error::{Error, io};
use crate::http::{self, MIB, too_large};

/// `SHA256SUMS` and its signature are a few hundred bytes.
const SMALL_LIMIT: u64 = MIB;
const SMALL_TIMEOUT: Duration = Duration::from_secs(60);
/// Observer's archives weigh tens of mebibytes.
const ARCHIVE_LIMIT: u64 = 256 * MIB;
const EXECUTABLE_LIMIT: u64 = 512 * MIB;
/// Room for the archive and the executable, which deflate shrinks to about half.
const SPACE_FACTOR: u64 = 4;
const RUN_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageProgress {
    Downloading {
        received: u64,
        total: u64,
    },
    /// The archive's checksums, then its executable.
    Verifying,
}

/// A release's executable, unpacked from an archive whose checksum the release key signed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Staged {
    pub executable: PathBuf,
    pub version: semver::Version,
    /// Of the archive, in hexadecimal.
    pub sha256: String,
}

impl Releases {
    /// Downloads `release`'s archive for this platform into `dir`, a folder of its own, and
    /// unpacks its executable there. Nothing large is fetched before the signature of the
    /// checksums is verified; the archive must then match both the signed checksum and the digest
    /// GitHub computed. `cancel` stops it at the next chunk.
    pub async fn stage(
        &self,
        release: &Published,
        dir: &Path,
        cancel: &CancellationToken,
        progress: &mut (dyn FnMut(StageProgress) + Send),
    ) -> Result<Staged, Error> {
        let key = self.key.ok_or(Error::NoReleaseKey)?;
        let archive = release.archive().ok_or_else(|| Error::NoArchive {
            name: updates::archive_name(
                &release.version.to_string(),
                std::env::consts::OS,
                std::env::consts::ARCH,
            ),
        })?;
        if archive.size > ARCHIVE_LIMIT {
            return Err(too_large(&archive.url, ARCHIVE_LIMIT));
        }
        let signature = release.asset(SIGNATURE).ok_or(Error::Unsigned)?;
        let not_listed = || Error::NotListed {
            name: archive.name.clone(),
        };
        let sums = release.asset(SUMS).ok_or_else(not_listed)?;
        let sums = self.small(&sums.url, cancel).await?;
        let signature = self.small(&signature.url, cancel).await?;
        if !key.verifies(&sums, &signature) {
            return Err(Error::BadSignature);
        }
        let expected =
            key::checksum_of(&String::from_utf8_lossy(&sums), &archive.name).ok_or_else(not_listed)?;

        let needed = archive.size.saturating_mul(SPACE_FACTOR);
        let available = observer_platform::available_space(dir).map_err(io(dir))?;
        if available < needed {
            return Err(Error::DiskSpace {
                path: dir.to_path_buf(),
                needed_mib: needed.div_ceil(MIB),
                available_mib: available / MIB,
            });
        }
        let path = dir.join(&archive.name);
        let downloaded = download::download(
            &self.http,
            &archive.url,
            archive.size,
            &path,
            &RETRY_DELAYS,
            cancel,
            &mut |received, total| progress(StageProgress::Downloading { received, total }),
        )
        .await?;
        progress(StageProgress::Verifying);
        if downloaded.sha256 != expected {
            return Err(Error::ChecksumMismatch {
                name: archive.name.clone(),
                listed: key::hex(&expected),
                downloaded: key::hex(&downloaded.sha256),
            });
        }
        if let Some(digest) = archive.digest
            && digest != downloaded.sha256
        {
            return Err(Error::DigestMismatch {
                name: archive.name.clone(),
                github: key::hex(&digest),
                downloaded: key::hex(&downloaded.sha256),
            });
        }
        let into = dir.to_path_buf();
        let executable = tokio::task::spawn_blocking(move || unpack(&path, &into))
            .await
            .map_err(|error| Error::Damaged(error.to_string()))??;
        Ok(Staged {
            executable,
            version: release.version.clone(),
            sha256: key::hex(&downloaded.sha256),
        })
    }

    async fn small(&self, url: &Url, cancel: &CancellationToken) -> Result<Vec<u8>, Error> {
        let response = tokio::select! {
            response = self.http.send(Method::GET, url, HeaderMap::new(), SMALL_TIMEOUT) => response?,
            () = cancel.cancelled() => return Err(Error::Cancelled),
        };
        http::expect_ok(&response)?;
        tokio::select! {
            body = http::body(response, SMALL_LIMIT) => body,
            () = cancel.cancelled() => Err(Error::Cancelled),
        }
    }
}

impl Staged {
    /// Runs the executable once, as `--version`, which must name the release; and on Windows,
    /// when `current` carries an Authenticode signature, checks the new one carries one by the same
    /// signer. Only an executable whose archive was verified is ever run.
    pub async fn check(&self, current: &Path) -> Result<(), Error> {
        let version = version_of(&self.executable).await?;
        if version != self.version {
            return Err(Error::WontRun(format!(
                "it says it is {version}, not {}",
                self.version
            )));
        }
        same_signer(current, &self.executable)
    }
}

/// The version an Observer executable reports with `--version`: `observer 0.2.0 (…)`.
pub async fn version_of(executable: &Path) -> Result<semver::Version, Error> {
    let mut command = tokio::process::Command::new(executable);
    command
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let output = tokio::time::timeout(RUN_TIMEOUT, command.output())
        .await
        .map_err(|_| Error::WontRun("it did not answer within 30 seconds".to_owned()))?
        .map_err(|error| Error::WontRun(error.to_string()))?;
    if !output.status.success() {
        return Err(Error::WontRun(format!("it exited with {}", output.status)));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    text.trim()
        .strip_prefix("observer ")
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|version| semver::Version::parse(version).ok())
        .ok_or_else(|| Error::WontRun("it does not name its version".to_owned()))
}

#[cfg(windows)]
fn same_signer(current: &Path, new: &Path) -> Result<(), Error> {
    match observer_platform::signer_of(current) {
        Ok(signer) => observer_platform::verify_signature(new, &signer).map_err(Error::ExecutableSignature),
        // An unsigned executable has no signer to keep.
        Err(_) => Ok(()),
    }
}

#[cfg(not(windows))]
#[expect(clippy::unnecessary_wraps, reason = "Windows checks the signature here")]
fn same_signer(_current: &Path, _new: &Path) -> Result<(), Error> {
    Ok(())
}

/// The executable of the archive at `archive`, written into `into`: the one entry of its name at
/// the archive's root, a plain file. The archive is read as hostile.
fn unpack(archive: &Path, into: &Path) -> Result<PathBuf, Error> {
    let name = updates::executable_name(std::env::consts::OS);
    let target = into.join(name);
    let file = File::open(archive).map_err(io(archive))?;
    let archive_name = archive
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let unpacked = if is_zip(archive_name) {
        from_zip(file, name, &target)
    } else if archive_name.to_ascii_lowercase().ends_with(".tar.gz") {
        from_tar_gz(file, name, &target)
    } else {
        Err(Error::Damaged(format!(
            "{archive_name} is neither a zip nor a gzipped tar"
        )))
    };
    if unpacked.is_err() {
        let _ = disk::remove_file(&target);
    }
    unpacked.map(|()| target)
}

/// Whether an archive is a zip, by its name.
pub(crate) fn is_zip(name: &str) -> bool {
    Path::new(name)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
}

fn from_zip(file: File, name: &str, target: &Path) -> Result<(), Error> {
    let damaged = |error: zip::result::ZipError| Error::Damaged(error.to_string());
    let mut zip = zip::ZipArchive::new(BufReader::new(file)).map_err(damaged)?;
    if zip
        .file_names_raw()
        .filter(|listed| *listed == name.as_bytes())
        .count()
        != 1
    {
        return Err(Error::Damaged(format!("the archive holds no single {name}")));
    }
    let mut entry = zip.by_name(name).map_err(damaged)?;
    if !entry.is_file() {
        return Err(Error::Damaged(format!("{name} is not a plain file")));
    }
    let size = entry.size();
    write_limited(&mut entry, size, target)
}

fn from_tar_gz(file: File, name: &str, target: &Path) -> Result<(), Error> {
    let damaged = |error: io::Error| Error::Damaged(error.to_string());
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(BufReader::new(file)));
    let mut found = false;
    for entry in archive.entries().map_err(damaged)? {
        let mut entry = entry.map_err(damaged)?;
        if entry.path().map_err(damaged)? != Path::new(name) {
            continue;
        }
        if found {
            return Err(Error::Damaged(format!("the archive holds {name} twice")));
        }
        if entry.header().entry_type() != tar::EntryType::Regular {
            return Err(Error::Damaged(format!("{name} is not a plain file")));
        }
        let size = entry.size();
        write_limited(&mut entry, size, target)?;
        found = true;
    }
    if found {
        Ok(())
    } else {
        Err(Error::Damaged(format!("the archive holds no {name}")))
    }
}

/// Copies exactly `size` bytes into a new file at `target`, made executable.
fn write_limited(reader: &mut dyn Read, size: u64, target: &Path) -> Result<(), Error> {
    if size > EXECUTABLE_LIMIT {
        return Err(Error::Damaged(format!(
            "the executable is larger than {} MiB",
            EXECUTABLE_LIMIT / MIB
        )));
    }
    let mut file = executable_file(target).map_err(io(target))?;
    let copied = io::copy(&mut reader.take(size + 1), &mut file).map_err(|error| {
        if error.kind() == io::ErrorKind::InvalidData {
            Error::Damaged(error.to_string())
        } else {
            Error::Io {
                path: target.to_path_buf(),
                source: error,
            }
        }
    })?;
    if copied != size {
        return Err(Error::Damaged(format!(
            "the executable is {copied} bytes long, its header says {size}"
        )));
    }
    file.flush().map_err(io(target))?;
    file.sync_all().map_err(io(target))
}

fn executable_file(path: &Path) -> io::Result<File> {
    let mut options = File::options();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o755);
    options.open(path)
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::releases::fake;

    /// The archive `bytes` as a file of the temporary folder, opened to read.
    fn opened(dir: &Path, bytes: &[u8]) -> File {
        let path = dir.join("archive");
        File::create(&path).unwrap().write_all(bytes).unwrap();
        File::open(path).unwrap()
    }

    #[test]
    fn both_archive_formats_give_up_their_executable_whatever_the_host() {
        for (name, program) in [("o.zip", "observer.exe"), ("o.tar.gz", "observer")] {
            let dir = tempfile::tempdir().unwrap();
            let file = opened(dir.path(), &fake::archive(name, program, b"the new Observer"));
            let target = dir.path().join("unpacked");

            if is_zip(name) {
                from_zip(file, program, &target).unwrap();
            } else {
                from_tar_gz(file, program, &target).unwrap();
            }

            assert_eq!(std::fs::read(&target).unwrap(), b"the new Observer", "{name}");
        }
    }

    #[test]
    fn an_archive_without_the_executable_gives_nothing() {
        for (name, program) in [("o.zip", "observer.exe"), ("o.tar.gz", "observer")] {
            let dir = tempfile::tempdir().unwrap();
            let file = opened(dir.path(), &fake::archive(name, "server", b"something else"));
            let target = dir.path().join("unpacked");

            let error = if is_zip(name) {
                from_zip(file, program, &target).unwrap_err()
            } else {
                from_tar_gz(file, program, &target).unwrap_err()
            };

            assert!(matches!(error, Error::Damaged(_)), "{name}: {error}");
        }
    }

    #[tokio::test]
    async fn an_executable_that_does_not_run_is_refused() {
        let dir = tempfile::tempdir().unwrap();

        let error = version_of(&dir.path().join("missing-observer"))
            .await
            .unwrap_err();

        assert!(matches!(error, Error::WontRun(_)), "{error}");
    }
}
