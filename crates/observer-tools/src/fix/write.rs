//! One file's write: reading it through the confined tree, building its new bytes from the original
//! and checking them entry by entry, and the write-ahead sequence that replaces it.

use std::collections::BTreeSet;
use std::io::{self, Read, Write as _};

use observer_domain::files::{UPLOAD_PREFIX, built_in_refusal};
use observer_gamefiles::{model_info, ytyp};
use observer_platform::{FileId, RelativePath, TreeError};
use sha2::{Digest, Sha256};

use super::{
    Backups, CloseReason, Error, Failure, Journal, Outcome, Pending, RecordId, Recorded, RunId, ServerFolder,
    Skip,
};
use crate::edit::Format;

/// The largest file read: a streamed `.ytyp` holds kilobytes, a model-info file at most a few
/// hundred.
const MAX_BYTES: u64 = 64 * 1024 * 1024;
/// What a backup's sidecar takes, at most, beside the backup.
const SIDECAR_BYTES: u64 = 64 * 1024;

/// Whether an error means a volume is full, or the user's quota on it.
pub(super) fn storage_full(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded
    )
}

pub(super) fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// The components of `path` (relative to the server folder, with `/`) when Observer may write there:
/// under `resources/`, neither a cfg, nor FXServer's cache, nor one of Observer's own paths in
/// `folder`'s list or in `also`.
pub(super) fn writable(path: &str, folder: &ServerFolder, also: &[Vec<String>]) -> Option<RelativePath> {
    let components: Vec<String> = path.split('/').map(str::to_owned).collect();
    let refused = components.first().map(String::as_str) != Some("resources")
        || built_in_refusal(&components, folder.refused())
        || built_in_refusal(&components, also);
    if refused {
        return None;
    }
    RelativePath::new(components).ok()
}

/// A file as it was read: its identity and the SHA-256 of its bytes. A write renames over it only
/// while its name still leads to that file and the file still holds those bytes: a file rewritten
/// in place keeps its identity, and only its bytes tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Seen {
    pub id: FileId,
    pub sha256: [u8; 32],
}

/// Reads the file at `at`, hard links accepted, with what tells it apart. The handle is closed
/// before this returns: on Windows an open handle would block the rename over it.
pub(super) fn read(folder: &ServerFolder, at: &RelativePath) -> Result<(Vec<u8>, Seen), TreeError> {
    let (file, id) = folder.tree().open_file_shared(at)?;
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(TreeError::Io)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(TreeError::Io(io::Error::other(format!(
            "larger than {} MiB",
            MAX_BYTES >> 20
        ))));
    }
    let seen = Seen {
        id,
        sha256: sha256(&bytes),
    };
    Ok((bytes, seen))
}

/// The position in the original of the entry at `current` in a file that holds the original's
/// entries without those at the positions `removed`, in order.
pub(super) fn original_position(current: usize, removed: &BTreeSet<usize>) -> usize {
    let mut position = current;
    for &gone in removed {
        if gone > position {
            break;
        }
        position += 1;
    }
    position
}

/// The hash each entry of a file registers, by position: the archetypes of a `.ytyp`, entries of
/// unknown types counted, or the `Item`s of a model-info file, unnamed ones counted. `None` for an
/// entry that registers nothing.
pub(super) fn entries(format: Format, bytes: &[u8]) -> Result<Vec<Option<u32>>, String> {
    match format {
        Format::Ytyp => {
            let map = ytyp::read(bytes).map_err(|error| error.to_string())?;
            let mut entries = vec![None; map.archetypes.len() + map.skipped.len()];
            for archetype in &map.archetypes {
                *entries
                    .get_mut(archetype.index)
                    .ok_or("an archetype lies past the end of the list")? = Some(archetype.name);
            }
            Ok(entries)
        }
        Format::ModelInfo => {
            let file = model_info::read(bytes)
                .map_err(|error| error.to_string())?
                .ok_or("not a model-info file")?;
            let mut entries = vec![None; file.item_count()];
            for item in &file.items {
                *entries
                    .get_mut(item.ordinal)
                    .ok_or("an item lies past the end of the list")? = Some(item.hash);
            }
            Ok(entries)
        }
    }
}

/// The original without the entries `removals` name, checked: each of them registers the hash its
/// removal names, and the new file lists every other entry of the original, in order. No removal
/// gives back the original's very bytes.
pub(super) fn rebuild(format: Format, original: &[u8], removals: &[Recorded]) -> Result<Vec<u8>, String> {
    if removals.is_empty() {
        return Ok(original.to_vec());
    }
    let before = entries(format, original)?;
    for recorded in removals {
        let removal = &recorded.removal;
        if before.get(removal.index) != Some(&Some(removal.hash)) {
            return Err(format!(
                "entry {} of the original does not register {:08X}",
                removal.index, removal.hash
            ));
        }
    }
    let gone: BTreeSet<usize> = removals.iter().map(|recorded| recorded.removal.index).collect();
    let indices: Vec<usize> = gone.iter().copied().collect();
    if indices.len() != removals.len() {
        return Err("an entry is taken out twice".to_owned());
    }
    let bytes = match format {
        Format::Ytyp => ytyp::remove_archetypes(original, &indices).map_err(|error| error.to_string())?,
        Format::ModelInfo => {
            model_info::remove_items(original, &indices).map_err(|error| error.to_string())?
        }
    };
    let expected: Vec<Option<u32>> = before
        .iter()
        .enumerate()
        .filter(|(position, _)| !gone.contains(position))
        .map(|(_, hash)| *hash)
        .collect();
    if entries(format, &bytes)? != expected {
        return Err("the new file does not list the original's entries minus those taken out".to_owned());
    }
    Ok(bytes)
}

/// A write about to happen: the file read, what replaces it, and what the journal records.
pub(super) struct Planned<'a> {
    pub path: &'a str,
    pub at: &'a RelativePath,
    pub resource: &'a str,
    pub format: Format,
    /// The file read: the rename happens only while the name still leads to it, unchanged.
    pub read: Seen,
    pub original_sha256: [u8; 32],
    /// The original, to back up first: for a file without a record yet.
    pub backup: Option<&'a [u8]>,
    pub existing: Option<RecordId>,
    pub bytes: Vec<u8>,
    pub removals: Vec<Recorded>,
    /// Closes the record once the write lands, with this reason, instead of applying it: for the
    /// original written back.
    pub close: Option<CloseReason>,
}

/// Replaces a file by the write-ahead sequence: space checked, backup kept, the write recorded as
/// pending, the temporary file written, flushed and renamed over the file read, the record applied
/// (or closed). Answers the record, or the outcome of a write that did not land, in which case the
/// record is as it was. Only a journal failure is an error: the run must stop there.
///
/// A failed write is settled from what the file holds, not from the error: a commit may fail after
/// its rename landed (the folder's flush, on Unix). A file that holds the new bytes is recorded as
/// written, whatever the error said; one that holds what was read, or met a refusal before the
/// rename, has its record put back; a file that cannot tell after such an error keeps its record
/// pending, for [`reconcile`](super::reconcile) to settle at the next run.
pub(super) fn write_ahead<J: Journal + ?Sized>(
    journal: &mut J,
    folder: &ServerFolder,
    backups: &Backups,
    run: RunId,
    planned: Planned<'_>,
) -> Result<Result<RecordId, Outcome>, Error> {
    let backup_bytes = planned
        .backup
        .map_or(0, |bytes| bytes.len() as u64 + SIDECAR_BYTES);
    if let Err(outcome) = space(folder, backups, planned.bytes.len() as u64 + backup_bytes) {
        return Ok(Err(outcome));
    }
    if let Some(original) = planned.backup {
        let shown = folder.path().display().to_string();
        match backups.put(original, &shown, planned.path) {
            Ok(sha256) if sha256 == planned.original_sha256 => {}
            Ok(_) => {
                return Ok(Err(failed_backup(
                    "the backup is not of the file read".to_owned(),
                )));
            }
            Err(error) if error.is_storage_full() => {
                return Ok(Err(Outcome::Failed {
                    why: Failure::StorageFull {
                        error: error.to_string(),
                    },
                }));
            }
            Err(error) => return Ok(Err(failed_backup(error.to_string()))),
        }
    }
    let pending = Pending {
        temporary: super::temporary_name()?,
        expected_sha256: sha256(&planned.bytes),
        removals: planned.removals,
    };
    let id = journal.begin(
        run,
        folder.id(),
        planned.path,
        planned.resource,
        planned.format,
        planned.original_sha256,
        &pending,
        planned.existing,
    )?;
    let (error, unsure) = match replace(
        folder,
        planned.at,
        &pending.temporary,
        &planned.bytes,
        planned.read,
    ) {
        Ok(()) => {
            settle_landed(journal, run, id, planned.close)?;
            return Ok(Ok(id));
        }
        Err(Unlanded::Refused(error)) => (error, false),
        Err(Unlanded::Unsure(error)) => (error, true),
    };
    let held = read(folder, planned.at).ok().map(|(_, now)| now.sha256);
    match left(held, pending.expected_sha256, planned.read.sha256, unsure) {
        Left::Landed => {
            tracing::warn!(%error, path = planned.path, "a fix's write failed, yet its file holds the new bytes");
            settle_landed(journal, run, id, planned.close)?;
            Ok(Ok(id))
        }
        Left::Untouched => {
            journal.abandon(id)?;
            Ok(Err(tree_outcome(error)))
        }
        Left::Unknown => {
            tracing::warn!(
                %error,
                path = planned.path,
                "whether a fix's write landed is unknown: its record stays pending until the next run"
            );
            Ok(Err(tree_outcome(error)))
        }
    }
}

/// Records a write that landed: applied, or closed for `close`.
fn settle_landed<J: Journal + ?Sized>(
    journal: &mut J,
    run: RunId,
    id: RecordId,
    close: Option<CloseReason>,
) -> Result<(), Error> {
    match close {
        Some(reason) => journal.close(id, run, reason)?,
        None => journal.applied(id)?,
    }
    Ok(())
}

/// What a failed write left in its file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Left {
    /// The new bytes: the rename landed.
    Landed,
    /// The file as it was read, or anything after a refusal that came before the rename.
    Untouched,
    /// Neither, after an error that may have come after the rename, or nothing readable.
    Unknown,
}

/// What a failed write left, from the SHA-256 its file holds now (`None` when it cannot be read),
/// the one it was to hold (`expected`) and the one it held when read (`read`). `unsure` when the
/// error may have come after the rename.
fn left(held: Option<[u8; 32]>, expected: [u8; 32], read: [u8; 32], unsure: bool) -> Left {
    match held {
        Some(held) if held == expected => Left::Landed,
        Some(held) if held == read => Left::Untouched,
        _ if unsure => Left::Unknown,
        _ => Left::Untouched,
    }
}

/// Why a write did not land, or may not have.
enum Unlanded {
    /// Refused or failed before the rename: whatever the file holds, the write did not put it
    /// there.
    Refused(TreeError),
    /// The commit failed with an error of the system, which may come after its rename landed: only
    /// the file can tell. The checks before the rename refuse with errors of their own.
    Unsure(TreeError),
}

/// Writes `bytes` to the temporary file `temporary` beside the file at `at`, flushes it and renames
/// it over that file while it is still the file `seen`, holding the same bytes. The file is read
/// again once the temporary file is flushed, so that only a writer racing the rename itself can
/// slip in. Any refusal removes the temporary file.
fn replace(
    folder: &ServerFolder,
    at: &RelativePath,
    temporary: &str,
    bytes: &[u8],
    seen: Seen,
) -> Result<(), Unlanded> {
    let tree = folder.tree();
    let (parent, name) = at.split_last().ok_or(Unlanded::Refused(TreeError::IsAFolder))?;
    let parent = tree.open_folder(&parent).map_err(Unlanded::Refused)?;
    let mut file = tree
        .create_temporary(&parent, temporary)
        .map_err(Unlanded::Refused)?;
    let written = file.file().write_all(bytes).and_then(|()| file.file().sync_all());
    if let Err(error) = written {
        let _ = tree.discard(&parent, file);
        return Err(Unlanded::Refused(TreeError::Io(error)));
    }
    let unchanged = match read(folder, at) {
        Ok((_, now)) if now == seen => Ok(()),
        Ok(_) => Err(TreeError::Changed),
        Err(error) => Err(error),
    };
    if let Err(error) = unchanged {
        let _ = tree.discard(&parent, file);
        return Err(Unlanded::Refused(error));
    }
    tree.commit_over_shared(&parent, file, name, Some(seen.id))
        .map_err(|error| match error {
            TreeError::Io(_) => Unlanded::Unsure(error),
            error => Unlanded::Refused(error),
        })
}

/// Removes the temporary file `temporary` a write to `path` may have left. A file already gone is
/// no error; any other failure is only logged, and the record is settled all the same: a record left
/// pending would keep heal and undo away from its file.
pub(super) fn remove_temporary(folder: &ServerFolder, path: &str, temporary: &str) {
    if !temporary.starts_with(UPLOAD_PREFIX) || temporary.contains(['/', '\\']) {
        return;
    }
    let Ok(at) = RelativePath::new(path.split('/')) else {
        return;
    };
    let Some((parent, _)) = at.split_last() else {
        return;
    };
    let tree = folder.tree();
    match tree
        .open_folder(&parent)
        .and_then(|parent| tree.remove_file(&parent, temporary))
    {
        Ok(()) | Err(TreeError::NotFound) => {}
        Err(error) => {
            tracing::warn!(%error, path, temporary, "a temporary file of a fix could not be removed");
        }
    }
}

/// Refuses a write of `needed` bytes that would leave either volume below the reserve. Both volumes
/// are asked for the whole: they may be one.
fn space(folder: &ServerFolder, backups: &Backups, needed: u64) -> Result<(), Outcome> {
    backups
        .prepare()
        .map_err(|error| failed_backup(error.to_string()))?;
    let wanted = needed.saturating_add(backups.reserve());
    for dir in [folder.path(), backups.folder()] {
        let available = observer_platform::available_space(dir).map_err(|error| Outcome::Failed {
            why: Failure::Io {
                error: format!("the free space of {} cannot be read: {error}", dir.display()),
            },
        })?;
        if available < wanted {
            return Err(Outcome::Failed {
                why: Failure::StorageFull {
                    error: format!(
                        "{} has {} MiB free, and the write needs {} MiB with the reserve",
                        dir.display(),
                        available >> 20,
                        wanted.div_ceil(1 << 20)
                    ),
                },
            });
        }
    }
    Ok(())
}

/// What a refusal of the confined tree means for the file.
pub(super) fn tree_outcome(error: TreeError) -> Outcome {
    match error {
        TreeError::Changed => Outcome::Skipped { why: Skip::Changed },
        TreeError::NotFound => Outcome::Skipped { why: Skip::Missing },
        TreeError::Io(error) if storage_full(&error) => Outcome::Failed {
            why: Failure::StorageFull {
                error: error.to_string(),
            },
        },
        error => Outcome::Failed {
            why: Failure::Io {
                error: error.to_string(),
            },
        },
    }
}

pub(super) fn failed_backup(error: String) -> Outcome {
    Outcome::Failed {
        why: Failure::Backup { error },
    }
}

pub(super) fn failed_patch(error: String) -> Outcome {
    Outcome::Failed {
        why: Failure::Patch { error },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::{Keeper, Removal};

    #[test]
    fn a_position_in_a_changed_file_is_found_again_in_the_original() {
        let removed = BTreeSet::from([1, 3]);
        // The original's entries 0 to 5 without 1 and 3: 0, 2, 4, 5.
        let found: Vec<usize> = (0..4).map(|at| original_position(at, &removed)).collect();

        assert_eq!(found, [0, 2, 4, 5]);
        assert_eq!(original_position(2, &BTreeSet::new()), 2);
        assert_eq!(original_position(0, &BTreeSet::from([0, 1, 2])), 3);
    }

    #[test]
    fn a_failed_write_is_settled_from_what_its_file_holds_not_from_the_error() {
        let (expected, read) = ([1; 32], [2; 32]);

        // The rename landed, whatever the error that followed it, or whoever else wrote it.
        assert_eq!(left(Some(expected), expected, read, true), Left::Landed);
        assert_eq!(left(Some(expected), expected, read, false), Left::Landed);
        // The file as it was read: the write never happened.
        assert_eq!(left(Some(read), expected, read, true), Left::Untouched);
        // A refusal before the rename leaves the file to whoever changed it.
        assert_eq!(left(Some([3; 32]), expected, read, false), Left::Untouched);
        assert_eq!(left(None, expected, read, false), Left::Untouched);
        // After an error that may follow the rename, a file that tells nothing keeps it pending.
        assert_eq!(left(Some([3; 32]), expected, read, true), Left::Unknown);
        assert_eq!(left(None, expected, read, true), Left::Unknown);
    }

    fn removal(index: usize, name: &str) -> Recorded {
        Recorded {
            removal: Removal {
                index,
                hash: observer_gamefiles::hash::name_hash(name),
                keeper: Keeper {
                    path: "resources/props/stream/a.ytyp".to_owned(),
                    format: Format::Ytyp,
                    index: 0,
                    sha256: [0; 32],
                },
            },
            run: 1,
        }
    }

    #[test]
    fn a_rebuild_takes_out_exactly_the_entries_named_and_refuses_any_other() {
        let original = observer_gamefiles::testing::ytyp("a", &["gift", "bow", "gift", "box"]);

        let rebuilt = rebuild(Format::Ytyp, &original, &[removal(0, "gift"), removal(3, "box")]).unwrap();
        let hashes = entries(Format::Ytyp, &rebuilt).unwrap();
        let named = |name| Some(observer_gamefiles::hash::name_hash(name));
        assert_eq!(hashes, [named("bow"), named("gift")]);

        assert!(rebuild(Format::Ytyp, &original, &[removal(1, "gift")]).is_err());
        assert!(rebuild(Format::Ytyp, &original, &[removal(0, "gift"), removal(0, "gift")]).is_err());
        assert!(rebuild(Format::Ytyp, &original, &[removal(9, "gift")]).is_err());
        assert_eq!(rebuild(Format::Ytyp, &original, &[]).unwrap(), original);
    }
}
