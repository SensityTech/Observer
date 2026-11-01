//! The originals of the files Observer changed, kept in Observer's data folder. Each is named by its
//! SHA-256, beside a sidecar `<sha256>.json` that says which folder and file it came from and when,
//! so that a file can be put back by hand even without the database. A backup is written under a
//! temporary name, flushed and renamed, then read back before anything relies on it, and verified
//! again before every restore: a backup that no longer holds its bytes is never written anywhere.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// What a run leaves free on each volume it writes to, by default: the database and the server
/// itself need room too.
pub const DEFAULT_RESERVE: u64 = 64 * 1024 * 1024;
/// The largest backup read back: a `.ytyp` holds kilobytes, a model-info file at most a few hundred.
const MAX_BYTES: u64 = 64 * 1024 * 1024;
/// Antivirus scanners and indexers hold freshly written files for a moment on Windows, which makes a
/// rename or a removal fail; the same call a little later succeeds.
const BUSY_DELAYS_MS: [u64; 5] = [50, 100, 200, 400, 800];
const PARTIAL: &str = ".partial";
const SIDECAR: &str = ".json";

/// Why a backup could not be kept or read.
#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error("no backup {0} is kept")]
    Missing(String),
    #[error("the backup {0} no longer holds the bytes it is named after, so it is not used")]
    Corrupt(String),
    #[error("{}: {source}", path.display())]
    Io { path: PathBuf, source: io::Error },
}

impl BackupError {
    /// Whether the volume of the backups is full.
    pub fn is_storage_full(&self) -> bool {
        matches!(self, Self::Io { source, .. } if super::write::storage_full(source))
    }
}

/// Where a backup came from, as its sidecar says.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Origin {
    folder: String,
    path: String,
    saved_at: Timestamp,
}

/// A backup's sidecar: the first file it was saved for, and any other file found with the same
/// bytes since.
#[derive(Debug, Serialize, Deserialize)]
struct Sidecar {
    #[serde(flatten)]
    first: Origin,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    also: Vec<Origin>,
}

/// The backups folder, `data/tools/backups`, and the room a run leaves free.
#[derive(Debug, Clone)]
pub struct Backups {
    folder: PathBuf,
    reserve: u64,
}

impl Backups {
    pub fn new(folder: impl Into<PathBuf>) -> Self {
        Self {
            folder: folder.into(),
            reserve: DEFAULT_RESERVE,
        }
    }

    /// The bytes a run leaves free on the volume of the backups and on the server folder's: a write
    /// that would take more stops the run as if the volume were full.
    #[must_use]
    pub fn with_reserve(mut self, bytes: u64) -> Self {
        self.reserve = bytes;
        self
    }

    pub fn folder(&self) -> &Path {
        &self.folder
    }

    pub fn reserve(&self) -> u64 {
        self.reserve
    }

    /// The file that holds the backup named `sha256`, kept or not.
    pub fn file(&self, sha256: &[u8; 32]) -> PathBuf {
        self.folder.join(hex(sha256))
    }

    /// Creates the folder if it is not there yet.
    pub(super) fn prepare(&self) -> Result<(), BackupError> {
        fs::create_dir_all(&self.folder).map_err(|source| io_error(&self.folder, source))
    }

    /// Keeps `bytes`, the original of the file at `path` (relative to the server folder `folder`,
    /// as shown to the owner), and answers its SHA-256 once it reads back. A backup already kept
    /// with these bytes is kept as it is; one that no longer holds them is written again.
    pub fn put(&self, bytes: &[u8], folder: &str, path: &str) -> Result<[u8; 32], BackupError> {
        let sha256: [u8; 32] = Sha256::digest(bytes).into();
        self.prepare()?;
        let name = hex(&sha256);
        if self.get(&sha256).is_err() {
            self.write_atomically(&name, bytes)?;
            if let Err(error) = self.get(&sha256) {
                let _ = retry(|| fs::remove_file(self.folder.join(&name)));
                return Err(error);
            }
        }
        self.note_origin(&name, folder, path)?;
        Ok(sha256)
    }

    /// The backup named `sha256`, once its bytes are checked against the name.
    pub fn get(&self, sha256: &[u8; 32]) -> Result<Vec<u8>, BackupError> {
        let name = hex(sha256);
        let path = self.folder.join(&name);
        let mut bytes = Vec::new();
        match File::open(&path) {
            Ok(file) => file
                .take(MAX_BYTES + 1)
                .read_to_end(&mut bytes)
                .map_err(|source| io_error(&path, source))?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Err(BackupError::Missing(name)),
            Err(source) => return Err(io_error(&path, source)),
        };
        let read: [u8; 32] = Sha256::digest(&bytes).into();
        if read == *sha256 {
            Ok(bytes)
        } else {
            Err(BackupError::Corrupt(name))
        }
    }

    /// Deletes every backup, with its sidecar, whose SHA-256 is not in `keep`, and every file a put
    /// was interrupted in. Other files are left alone. Answers how many backups went.
    pub fn retain(&self, keep: &HashSet<[u8; 32]>) -> Result<usize, BackupError> {
        let entries = match fs::read_dir(&self.folder) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
            Err(source) => return Err(io_error(&self.folder, source)),
        };
        let mut deleted = 0;
        for entry in entries {
            let entry = entry.map_err(|source| io_error(&self.folder, source))?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let (stem, partial) = match name.strip_suffix(PARTIAL) {
                Some(stem) => (stem, true),
                None => (name, false),
            };
            let (stem, sidecar) = match stem.strip_suffix(SIDECAR) {
                Some(stem) => (stem, true),
                None => (stem, false),
            };
            let Some(sha256) = unhex(stem) else { continue };
            if partial || !keep.contains(&sha256) {
                let path = entry.path();
                match retry(|| fs::remove_file(&path)) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                    Err(source) => return Err(io_error(&path, source)),
                }
                if !partial && !sidecar {
                    deleted += 1;
                }
            }
        }
        Ok(deleted)
    }

    /// Writes `bytes` to `<name>.partial`, flushes it, renames it to `name` and flushes the folder.
    fn write_atomically(&self, name: &str, bytes: &[u8]) -> Result<(), BackupError> {
        let partial = self.folder.join(format!("{name}{PARTIAL}"));
        let target = self.folder.join(name);
        let written = (|| {
            let mut file = File::create(&partial)?;
            file.write_all(bytes)?;
            file.sync_all()?;
            drop(file);
            retry(|| fs::rename(&partial, &target))?;
            sync_folder(&self.folder)
        })();
        written.map_err(|source| {
            let _ = retry(|| fs::remove_file(&partial));
            io_error(&target, source)
        })
    }

    /// Adds `folder` and `path` to the backup's sidecar, or writes one.
    fn note_origin(&self, name: &str, folder: &str, path: &str) -> Result<(), BackupError> {
        let file = format!("{name}{SIDECAR}");
        let origin = Origin {
            folder: folder.to_owned(),
            path: path.to_owned(),
            saved_at: Timestamp::now(),
        };
        let same = |kept: &Origin| kept.folder == origin.folder && kept.path == origin.path;
        let sidecar = match fs::read(self.folder.join(&file))
            .ok()
            .and_then(|text| serde_json::from_slice::<Sidecar>(&text).ok())
        {
            Some(kept) if same(&kept.first) || kept.also.iter().any(same) => return Ok(()),
            Some(mut kept) => {
                kept.also.push(origin);
                kept
            }
            None => Sidecar {
                first: origin,
                also: Vec::new(),
            },
        };
        let text = serde_json::to_vec_pretty(&sidecar)
            .map_err(|error| io_error(&self.folder.join(&file), error.into()))?;
        self.write_atomically(&file, &text)
    }
}

fn io_error(path: &Path, source: io::Error) -> BackupError {
    BackupError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// Runs a rename or a removal again while a scanner holds the file.
fn retry<T>(mut operation: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    for delay in BUSY_DELAYS_MS {
        match operation() {
            Err(error) if busy(&error) => thread::sleep(Duration::from_millis(delay)),
            result => return result,
        }
    }
    operation()
}

fn busy(error: &io::Error) -> bool {
    // A file a scanner holds is denied on Windows, where 32 and 33, its sharing and lock violations,
    // have no portable kind either. Elsewhere a denial means Observer may not write there.
    error.kind() == io::ErrorKind::ResourceBusy
        || (cfg!(windows)
            && (error.kind() == io::ErrorKind::PermissionDenied
                || matches!(error.raw_os_error(), Some(32 | 33))))
}

/// Makes a rename in `folder` durable. NTFS journals the rename itself, and Windows offers no way to
/// flush a folder opened only to read.
#[cfg(unix)]
fn sync_folder(folder: &Path) -> io::Result<()> {
    File::open(folder)?.sync_all()
}

#[cfg(not(unix))]
#[allow(clippy::unnecessary_wraps)]
fn sync_folder(_folder: &Path) -> io::Result<()> {
    Ok(())
}

/// The SHA-256 as 64 lowercase hexadecimal digits: a backup's name.
pub(super) fn hex(sha256: &[u8; 32]) -> String {
    use std::fmt::Write as _;
    sha256.iter().fold(String::with_capacity(64), |mut text, byte| {
        let _ = write!(text, "{byte:02x}");
        text
    })
}

fn unhex(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64
        || !text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return None;
    }
    let mut sha256 = [0u8; 32];
    for (index, byte) in sha256.iter_mut().enumerate() {
        *byte = u8::from_str_radix(text.get(index * 2..index * 2 + 2)?, 16).ok()?;
    }
    Some(sha256)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn backups() -> (tempfile::TempDir, Backups) {
        let dir = tempfile::tempdir().unwrap();
        let backups = Backups::new(dir.path().join("tools").join("backups"));
        (dir, backups)
    }

    fn names(backups: &Backups) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(backups.folder())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_backup_is_named_by_its_sha256_and_says_where_it_came_from() {
        let (_dir, backups) = backups();

        let sha256 = backups
            .put(b"original", "C:/server", "resources/props/stream/a.ytyp")
            .unwrap();

        assert_eq!(sha256, <[u8; 32]>::from(Sha256::digest(b"original")));
        assert_eq!(backups.get(&sha256).unwrap(), b"original");
        let name = hex(&sha256);
        assert_eq!(names(&backups), [name.clone(), format!("{name}.json")]);
        let sidecar: serde_json::Value =
            serde_json::from_slice(&fs::read(backups.folder().join(format!("{name}.json"))).unwrap())
                .unwrap();
        assert_eq!(sidecar["folder"], "C:/server");
        assert_eq!(sidecar["path"], "resources/props/stream/a.ytyp");
        assert!(sidecar["saved_at"].is_string());
        assert!(sidecar.get("also").is_none());
    }

    #[test]
    fn the_same_bytes_from_another_file_share_one_backup_and_both_are_named() {
        let (_dir, backups) = backups();
        let first = backups
            .put(b"same", "C:/server", "resources/a/stream/x.ytyp")
            .unwrap();
        let again = backups
            .put(b"same", "C:/server", "resources/a/stream/x.ytyp")
            .unwrap();
        let other = backups
            .put(b"same", "C:/server", "resources/b/stream/x.ytyp")
            .unwrap();

        assert_eq!((first, again), (other, other));
        let sidecar: serde_json::Value = serde_json::from_slice(
            &fs::read(backups.folder().join(format!("{}.json", hex(&first)))).unwrap(),
        )
        .unwrap();
        assert_eq!(sidecar["path"], "resources/a/stream/x.ytyp");
        assert_eq!(sidecar["also"].as_array().unwrap().len(), 1);
        assert_eq!(sidecar["also"][0]["path"], "resources/b/stream/x.ytyp");
    }

    #[test]
    fn a_backup_that_no_longer_holds_its_bytes_is_refused_and_written_again_by_the_next_put() {
        let (_dir, backups) = backups();
        let sha256 = backups.put(b"original", "C:/server", "a.ytyp").unwrap();
        fs::write(backups.file(&sha256), b"damaged").unwrap();

        assert!(matches!(backups.get(&sha256), Err(BackupError::Corrupt(_))));
        assert!(matches!(backups.get(&[7; 32]), Err(BackupError::Missing(_))));

        backups.put(b"original", "C:/server", "a.ytyp").unwrap();
        assert_eq!(backups.get(&sha256).unwrap(), b"original");
    }

    #[test]
    fn retain_deletes_every_other_backup_and_leftover_but_no_foreign_file() {
        let (_dir, backups) = backups();
        let kept = backups.put(b"kept", "C:/server", "a.ytyp").unwrap();
        let gone = backups.put(b"gone", "C:/server", "b.ytyp").unwrap();
        fs::write(backups.folder().join(format!("{}.partial", hex(&kept))), b"half").unwrap();
        fs::write(backups.folder().join("notes.txt"), b"mine").unwrap();

        let deleted = backups.retain(&HashSet::from([kept])).unwrap();

        assert_eq!(deleted, 1);
        let name = hex(&kept);
        assert_eq!(
            names(&backups),
            [name.clone(), format!("{name}.json"), "notes.txt".to_owned()]
        );
        assert!(matches!(backups.get(&gone), Err(BackupError::Missing(_))));
        assert_eq!(
            Backups::new("/nowhere/at/all").retain(&HashSet::new()).unwrap(),
            0
        );
    }

    #[test]
    fn names_are_read_back_only_when_they_are_a_sha256() {
        let sha256 = <[u8; 32]>::from(Sha256::digest(b"x"));
        assert_eq!(unhex(&hex(&sha256)), Some(sha256));
        assert_eq!(unhex(&hex(&sha256).to_uppercase()), None);
        assert_eq!(unhex("abc"), None);
        assert_eq!(unhex(&"g".repeat(64)), None);
    }
}
