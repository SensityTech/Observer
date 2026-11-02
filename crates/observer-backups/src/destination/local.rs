//! A folder of the host, or a share mounted on it. A copy is written next to its final name,
//! synced, checked against the file's digest, then renamed: a folder never holds half a backup
//! under a backup's name.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use observer_domain::backups::BackupError;
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

use crate::Failure;

const BUFFER: usize = 256 * 1024;

/// The copy's path in `folder`.
pub fn location(folder: &str, name: &str) -> String {
    Path::new(folder).join(name).to_string_lossy().into_owned()
}

/// Copies `file` to `location`, whose folder is made if it is missing, and returns the bytes
/// copied once they match `sha256`.
pub async fn upload(
    file: &Path,
    location: &str,
    sha256: [u8; 32],
    progress: Arc<AtomicU64>,
    cancel: CancellationToken,
) -> Result<u64, Failure> {
    let (file, location) = (file.to_path_buf(), PathBuf::from(location));
    tokio::task::spawn_blocking(move || copy(&file, &location, sha256, &progress, &cancel))
        .await
        .map_err(io::Error::other)?
}

fn copy(
    file: &Path,
    location: &Path,
    sha256: [u8; 32],
    progress: &AtomicU64,
    cancel: &CancellationToken,
) -> Result<u64, Failure> {
    if let Some(folder) = location.parent() {
        fs::create_dir_all(folder)?;
    }
    let mut partial_name = location.as_os_str().to_owned();
    partial_name.push(".partial");
    let partial = PathBuf::from(partial_name);
    let copied = write_partial(file, &partial, sha256, progress, cancel);
    if copied.is_err() {
        let _ = fs::remove_file(&partial);
        return copied;
    }
    if let Err(error) = fs::rename(&partial, location) {
        let _ = fs::remove_file(&partial);
        return Err(error.into());
    }
    copied
}

fn write_partial(
    file: &Path,
    partial: &Path,
    sha256: [u8; 32],
    progress: &AtomicU64,
    cancel: &CancellationToken,
) -> Result<u64, Failure> {
    let mut source = BufReader::with_capacity(BUFFER, File::open(file)?);
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut target = BufWriter::with_capacity(BUFFER, options.open(partial)?);
    let mut hash = Sha256::new();
    let mut copied = 0u64;
    let mut buffer = vec![0; BUFFER];
    loop {
        if cancel.is_cancelled() {
            return Err(Failure::new(BackupError::Cancelled));
        }
        let read = source.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        target.write_all(&buffer[..read])?;
        hash.update(&buffer[..read]);
        copied += read as u64;
        progress.store(copied, Ordering::Relaxed);
    }
    let target = target.into_inner().map_err(io::IntoInnerError::into_error)?;
    target.sync_all()?;
    if <[u8; 32]>::from(hash.finalize()) != sha256 {
        return Err(Failure::with(
            BackupError::VerifyFailed,
            "the copy differs from the backup",
        ));
    }
    Ok(copied)
}

/// Deletes a copy; one already gone counts as deleted.
pub async fn delete(location: &str) -> Result<(), Failure> {
    match tokio::fs::remove_file(location).await {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error.into()),
        _ => Ok(()),
    }
}

pub async fn open(location: &str) -> Result<(tokio::fs::File, u64), Failure> {
    let file = tokio::fs::File::open(location).await?;
    let bytes = file.metadata().await?.len();
    Ok((file, bytes))
}

/// Whether `folder` is `outer` or lies under it, once both are resolved as far as they exist:
/// links and `..` cannot hide a folder inside another.
pub fn inside(folder: &Path, outer: &Path) -> bool {
    let (folder, outer) = (resolved(folder), resolved(outer));
    let same = |a: &Component<'_>, b: &Component<'_>| {
        if cfg!(windows) {
            a.as_os_str().eq_ignore_ascii_case(b.as_os_str())
        } else {
            a == b
        }
    };
    let (folder, outer): (Vec<_>, Vec<_>) = (folder.components().collect(), outer.components().collect());
    folder.len() >= outer.len() && outer.iter().zip(&folder).all(|(a, b)| same(a, b))
}

/// The path with its longest existing start canonicalised and the rest added as written.
fn resolved(path: &Path) -> PathBuf {
    let mut rest = Vec::new();
    let mut current = path;
    loop {
        if let Ok(found) = fs::canonicalize(current) {
            return rest.into_iter().rev().fold(found, |path, part| path.join(part));
        }
        match (current.parent(), current.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_owned());
                current = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_with(dir: &Path, bytes: &[u8]) -> (PathBuf, [u8; 32]) {
        let path = dir.join("backup.sql.gz");
        fs::write(&path, bytes).unwrap();
        (path, Sha256::digest(bytes).into())
    }

    #[tokio::test]
    async fn a_copy_lands_whole_under_its_name_in_a_folder_made_for_it() {
        let dir = tempfile::tempdir().unwrap();
        let (file, sha256) = file_with(dir.path(), b"gzipped dump");
        let folder = dir.path().join("copies").join("es_extended");
        let location = location(&folder.to_string_lossy(), "es-1.sql.gz");
        let progress = Arc::new(AtomicU64::new(0));

        let copied = upload(
            &file,
            &location,
            sha256,
            progress.clone(),
            CancellationToken::new(),
        )
        .await
        .unwrap();

        assert_eq!(copied, 12);
        assert_eq!(progress.load(Ordering::Relaxed), 12);
        assert_eq!(fs::read(&location).unwrap(), b"gzipped dump");
        assert_eq!(fs::read_dir(&folder).unwrap().count(), 1);
    }

    #[tokio::test]
    async fn a_copy_that_does_not_match_leaves_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        let (file, _) = file_with(dir.path(), b"gzipped dump");
        let location = location(&dir.path().join("copies").to_string_lossy(), "es-1.sql.gz");

        let failure = upload(
            &file,
            &location,
            [0; 32],
            Arc::default(),
            CancellationToken::new(),
        )
        .await
        .unwrap_err();

        assert_eq!(failure.error, BackupError::VerifyFailed);
        assert_eq!(fs::read_dir(dir.path().join("copies")).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn a_copy_already_gone_counts_as_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let (file, _) = file_with(dir.path(), b"x");

        delete(&file.to_string_lossy()).await.unwrap();
        delete(&file.to_string_lossy()).await.unwrap();

        assert!(!file.exists());
    }

    #[test]
    fn a_folder_under_another_is_inside_it_even_before_it_exists() {
        let dir = tempfile::tempdir().unwrap();
        let server = dir.path().join("server-data");
        fs::create_dir(&server).unwrap();

        assert!(inside(&server.join("resources").join("dumps"), &server));
        assert!(inside(&server, &server));
        assert!(inside(
            &dir.path()
                .join("server-data")
                .join("..")
                .join("server-data")
                .join("x"),
            &server
        ));
        assert!(!inside(&dir.path().join("server-data-backups"), &server));
        assert!(!inside(dir.path(), &server));
    }
}
