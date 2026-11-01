//! Applying a plan, healing it and undoing it. Observer changes a file of the server folder only
//! through a journal kept ahead of the disk, so that whatever stops it midway, every file it changed
//! is known, its original is kept, and each entry it took out can be put back on its own.
//!
//! - **One open record per file** of a server folder. Its backup is the file as it was before
//!   Observer first touched it; its removals pile up across runs, each with the run that made it,
//!   as positions in that original. A new fix of a file already changed rebuilds it from the
//!   original with the old removals and the new ones together, so that earlier runs can still be
//!   healed or undone one by one.
//! - **Write ahead.** Per file: read it once through the confined tree, hard links accepted
//!   (FXServer links every stream file into its cache), check that it is the file the plan was made
//!   on, build the new bytes from the original and read them back; save the backup; record the write
//!   as pending, with the name of its temporary file, *before* creating that file; write and flush
//!   it, and rename it over the target only while the target is still the file read, holding the
//!   bytes read. The record is applied once the rename landed. A write that fails is settled from
//!   what the file holds, since a rename can land before the error; whatever stops Observer in
//!   between, [`reconcile`] tells from the file's SHA-256 whether the rename landed, and removes the
//!   temporary file.
//! - **Heal** puts back every removal whose copy kept no longer stands in for it, and closes the
//!   records of files someone else changed since. When unsure, it restores: a duplicate warning that
//!   comes back is harmless, a missing archetype is not.
//! - **Undo** puts back the removals of one run, of some files or of all, only while the file still
//!   holds what Observer wrote.
//!
//! The analysis reads the files as they are, so a plan's positions are positions in the current,
//! perhaps already changed, file; they are turned into positions in the original before they are
//! recorded. Every write first checks that both volumes it touches keep a reserve free, and a run
//! stops at the first full volume. Nothing here takes a lock: the caller holds the tools' writer
//! lock and keeps FXServer stopped. A replaced file keeps its mode; its ACLs and extended attributes
//! are not carried over.

mod backups;
mod run;
mod write;

use std::io;
use std::path::{Path, PathBuf};

use observer_domain::files::UPLOAD_PREFIX;
use observer_platform::{ConfinedTree, TreeError};
use serde::{Deserialize, Serialize};

use crate::edit::{Format, Removal};

pub use backups::{BackupError, Backups, DEFAULT_RESERVE};
pub use run::{apply, heal, reconcile, undo};

/// A run of a tool, as the store numbers it.
pub type RunId = i64;
/// A record of the journal, as the store numbers it.
pub type RecordId = i64;

/// A server folder's identity: a record applies to the folder it was made in, whatever path leads
/// there later, and never to another folder reached under the same path.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FolderId {
    /// The root's final path, as [`ConfinedTree::final_path`] gives it (the NT form on Windows).
    pub path: String,
    /// The root folder's [`FileId`](observer_platform::FileId), as text: what finds the records.
    /// Only its inode on Unix: Linux numbers a device each time it mounts it, and a btrfs, ZFS or
    /// LVM volume may come back under another number after a reboot, which would leave every
    /// record behind. The inode still tells the folder from another made under the same path; only
    /// the root of another file system, whose inode is the same on every file system of its kind,
    /// could be taken for it, and every write checks first that the file holds what the record says.
    pub id: String,
}

/// The text of a root folder's identity that finds its records: the volume's serial number and the
/// folder's index where the volume keeps its number (`volume_stays`, Windows), the index alone
/// where it may be numbered anew at the next mount.
fn folder_key(volume: u64, index: u128, volume_stays: bool) -> String {
    if volume_stays {
        format!("{volume:016x}-{index:032x}")
    } else {
        format!("inode-{index:032x}")
    }
}

/// One entry taken out of a file, and the run that took it out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recorded {
    /// Its `index` is its position in the original file, before Observer touched it.
    pub removal: Removal,
    pub run: RunId,
}

/// What the journal knows of one file Observer changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub id: RecordId,
    pub folder: FolderId,
    /// Relative to the server folder, components joined by `/`.
    pub path: String,
    pub resource: String,
    pub format: Format,
    /// The SHA-256 of the file before Observer touched it, which names its backup.
    pub original_sha256: [u8; 32],
    /// What the file holds while the record is applied; `None` while its first write is pending.
    pub patched_sha256: Option<[u8; 32]>,
    /// The removals in effect, every run's.
    pub removals: Vec<Recorded>,
    /// The write under way, from [`Journal::begin`] to [`Journal::applied`] or [`Journal::abandon`].
    pub pending: Option<Pending>,
}

/// A write recorded before it happens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    /// The temporary file's name, in the folder of the file it replaces: [`UPLOAD_PREFIX`] and 26
    /// base32 characters, no extension, so that the start sweep removes a leftover and FXServer
    /// never streams one.
    pub temporary: String,
    /// What the file holds once the rename lands.
    pub expected_sha256: [u8; 32],
    /// Every removal in effect once it lands.
    pub removals: Vec<Recorded>,
}

/// Why a record was closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CloseReason {
    /// Every removal was undone, and the original written back.
    Undone,
    /// Every removal was healed, and the original written back.
    Healed,
    /// The file holds neither what Observer wrote nor its original: someone else changed it, and the
    /// record no longer describes it.
    Stale,
    /// The file holds its original again, put back by someone else or by a restore Observer was
    /// interrupted in.
    Restored,
    /// The record's first write never landed.
    Abandoned,
}

impl CloseReason {
    pub const ALL: [Self; 5] = [
        Self::Undone,
        Self::Healed,
        Self::Stale,
        Self::Restored,
        Self::Abandoned,
    ];

    /// The name on the wire and in the store.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Undone => "undone",
            Self::Healed => "healed",
            Self::Stale => "stale",
            Self::Restored => "restored",
            Self::Abandoned => "abandoned",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|reason| reason.as_str() == name)
    }
}

/// Why the journal could not be read or written. A run stops at the first: the journal is what
/// tells later which files Observer changed.
#[derive(Debug, thiserror::Error)]
#[error("the journal of changes cannot be read or written: {0}")]
pub struct JournalError(#[source] Box<dyn std::error::Error + Send + Sync>);

impl JournalError {
    pub fn new(error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> Self {
        Self(error.into())
    }
}

/// The records of the files Observer changed, kept by the store; one open record (pending or
/// applied) per server folder and file.
pub trait Journal {
    /// The open records, pending or applied, of this folder for this tool.
    fn open(&mut self, folder: &FolderId) -> Result<Vec<Record>, JournalError>;

    /// Records the write about to happen, *before* its temporary file is created: inserts a record
    /// for a file without one, or moves its open record (`existing`) to pending, and in the same
    /// transaction records the temporary file as an upload (its folder, relative to the server
    /// folder, and its name), so that the start sweep removes a leftover.
    #[allow(clippy::too_many_arguments)]
    fn begin(
        &mut self,
        run: RunId,
        folder: &FolderId,
        path: &str,
        resource: &str,
        format: Format,
        original_sha256: [u8; 32],
        pending: &Pending,
        existing: Option<RecordId>,
    ) -> Result<RecordId, JournalError>;

    /// The rename landed: the record holds the pending SHA-256 and removals, its pending write is
    /// cleared and its upload forgotten.
    fn applied(&mut self, id: RecordId) -> Result<(), JournalError>;

    /// The write did not happen: the record goes back to what it was, or is closed as
    /// [`CloseReason::Abandoned`] when it was new, and its upload is forgotten.
    fn abandon(&mut self, id: RecordId) -> Result<(), JournalError>;

    /// Closes an open record, pending or applied, for `reason`; a pending write's upload is
    /// forgotten.
    fn close(&mut self, id: RecordId, run: RunId, reason: CloseReason) -> Result<(), JournalError>;
}

/// Why a run could not go on. What a single file meets is in its [`FileResult`] instead.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the server folder {} cannot be opened: {source}", path.display())]
    Folder { path: PathBuf, source: io::Error },
    #[error("the identity of the server folder cannot be read: {0}")]
    Identity(#[source] TreeError),
    #[error(transparent)]
    Journal(#[from] JournalError),
    #[error("no random bytes for a temporary file's name: {0}")]
    Random(getrandom::Error),
}

/// The server folder the fixes write into, reached only through a confined tree, with the paths of
/// Observer's own files and folders inside it, which are never written.
#[derive(Debug)]
pub struct ServerFolder {
    path: PathBuf,
    tree: ConfinedTree,
    id: FolderId,
    refused: Vec<Vec<String>>,
}

impl ServerFolder {
    pub fn open(path: &Path) -> Result<Self, Error> {
        let tree = ConfinedTree::open(path).map_err(|source| Error::Folder {
            path: path.to_path_buf(),
            source,
        })?;
        let root = tree.root_id().map_err(Error::Identity)?;
        let id = FolderId {
            path: tree.final_path().to_owned(),
            id: folder_key(root.volume(), root.index(), cfg!(windows)),
        };
        Ok(Self {
            path: path.to_path_buf(),
            tree,
            id,
            refused: Vec::new(),
        })
    }

    /// The paths of Observer's own files and folders inside the server folder, as components: the
    /// list SFTP and the index take. None by default.
    #[must_use]
    pub fn with_refused(mut self, refused: Vec<Vec<String>>) -> Self {
        self.refused = refused;
        self
    }

    /// The folder as it was opened, which the free space is asked of.
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn tree(&self) -> &ConfinedTree {
        &self.tree
    }

    pub fn id(&self) -> &FolderId {
        &self.id
    }

    pub fn refused(&self) -> &[Vec<String>] {
        &self.refused
    }
}

/// Which removals an undo puts back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    /// Every removal of every file.
    All,
    /// The removals one run made, in whichever files.
    Run(RunId),
    /// Every removal of these files, relative to the server folder with `/`.
    Paths(Vec<String>),
}

impl Selection {
    /// Whether the removal `recorded` of the file at `path` is put back.
    fn takes(&self, path: &str, recorded: &Recorded) -> bool {
        match self {
            Self::All => true,
            Self::Run(run) => recorded.run == *run,
            Self::Paths(paths) => paths.iter().any(|selected| selected == path),
        }
    }
}

/// What a run did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// The records settled before the run: interrupted writes, and records that no longer
    /// described their file.
    pub reconciled: Vec<FileResult>,
    /// One per file the run looked at, in order. A heal leaves out the files whose copies kept all
    /// stand, and counts them in `unchanged`.
    pub files: Vec<FileResult>,
    pub unchanged: u32,
    /// The cancel flag stopped the run: the files left are [`Skip::NotTried`].
    pub cancelled: bool,
    /// A full volume stopped the run: the files left are [`Skip::NotTried`].
    pub storage_full: bool,
}

impl Report {
    /// The files this run wrote.
    pub fn written(&self) -> impl Iterator<Item = &FileResult> {
        self.files.iter().filter(|file| {
            matches!(
                file.outcome,
                Outcome::Written { .. }
                    | Outcome::Closed {
                        reason: CloseReason::Healed | CloseReason::Undone
                    }
            )
        })
    }
}

/// What a run did to one file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileResult {
    /// Relative to the server folder, with `/`.
    pub path: String,
    pub resource: String,
    pub outcome: Outcome,
    /// The entries put back, by a heal or an undo.
    pub restored: Vec<Restored>,
}

/// An entry put back into a file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Restored {
    /// Its position in the original file.
    pub index: usize,
    pub hash: u32,
    /// The run that had taken it out.
    pub run: RunId,
    /// For a heal, why the copy kept no longer stands in for it.
    pub why: Option<String>,
}

/// What became of a file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Outcome {
    /// The file holds its original without these entries (positions in the original): written by
    /// this run, or, among the records reconciled, by the write that was interrupted.
    Written { removed: Vec<usize> },
    /// Its record is closed. For [`CloseReason::Healed`] and [`CloseReason::Undone`] this run wrote
    /// the original back; otherwise nothing was written.
    Closed { reason: CloseReason },
    /// An interrupted write that never landed: the record goes on as it was before it.
    Kept,
    /// Nothing was written.
    Skipped { why: Skip },
    /// Nothing was written, and the record is as it was.
    Failed { why: Failure },
}

/// Why a file was left as it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Skip {
    /// The file is not the one the plan was made on, no longer holds what Observer wrote, or was
    /// replaced or rewritten between its read and the rename.
    Changed,
    /// The file is gone.
    Missing,
    /// A path Observer never writes: outside `resources/`, a cfg, or Observer's own files.
    Refused,
    /// An earlier write to the file is still pending.
    Unresolved,
    /// Nothing to take out or put back.
    NothingToDo,
    /// The run stopped before this file.
    NotTried,
}

/// Why a write failed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Failure {
    /// A volume is full, or would fall below its reserve: the run stops here.
    StorageFull { error: String },
    /// The file or its folder could not be read or written.
    Io { error: String },
    /// The new bytes could not be built, or did not read back as planned.
    Patch { error: String },
    /// The backup could not be written, or no longer holds the original.
    Backup { error: String },
}

/// A new name for a temporary file: [`UPLOAD_PREFIX`] and 26 base32 characters (130 random bits),
/// as SFTP names its uploads.
pub fn temporary_name() -> Result<String, Error> {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut bytes = [0u8; 26];
    getrandom::fill(&mut bytes).map_err(Error::Random)?;
    let suffix: String = bytes
        .iter()
        .map(|byte| char::from(ALPHABET[usize::from(byte & 31)]))
        .collect();
    Ok(format!("{UPLOAD_PREFIX}{suffix}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_temporary_name_is_an_upload_name_without_extension() {
        let name = temporary_name().unwrap();
        let suffix = name.strip_prefix(UPLOAD_PREFIX).unwrap();

        assert_eq!(suffix.len(), 26);
        assert!(
            suffix
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || (b'2'..=b'7').contains(&byte))
        );
        assert!(!name.contains('.') || name.rfind('.') == Some(0));
        assert_ne!(temporary_name().unwrap(), name);
    }

    #[test]
    fn a_folder_numbered_anew_at_its_next_mount_on_unix_still_finds_its_records() {
        // A device number that changed after a reboot: the inode alone finds the records.
        assert_eq!(folder_key(0x803, 42, false), folder_key(0x2f, 42, false));
        assert_ne!(folder_key(0x803, 42, false), folder_key(0x803, 43, false));
        // A Windows volume keeps its serial number, which still tells it apart.
        assert_ne!(folder_key(0x803, 42, true), folder_key(0x2f, 42, true));
    }

    #[test]
    fn a_server_folder_is_known_by_its_roots_identity_on_windows_and_by_its_inode_elsewhere() {
        let dir = tempfile::tempdir().unwrap();
        let root = ConfinedTree::open(dir.path()).unwrap().root_id().unwrap();

        let folder = ServerFolder::open(dir.path()).unwrap();

        let expected = if cfg!(windows) {
            root.to_string()
        } else {
            format!("inode-{:032x}", root.index())
        };
        assert_eq!(folder.id().id, expected);
        assert_eq!(ServerFolder::open(dir.path()).unwrap().id(), folder.id());
    }

    #[test]
    fn close_reasons_are_stored_by_the_names_the_store_checks() {
        for reason in CloseReason::ALL {
            assert_eq!(serde_json::to_value(reason).unwrap(), reason.as_str());
            assert_eq!(CloseReason::from_name(reason.as_str()), Some(reason));
        }
        assert_eq!(CloseReason::from_name("closed"), None);
    }
}
