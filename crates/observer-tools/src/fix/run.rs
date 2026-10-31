//! The four things done with the journal: settling the writes an earlier Observer was interrupted
//! in, healing the removals whose copy kept is gone, applying a plan, and undoing removals. Each
//! goes file by file, checks the cancel flag between files, and stops at the first full volume.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};

use observer_domain::tools::ToolSettings;
use observer_platform::{FileId, RelativePath, TreeError};

use super::write::{
    Planned, failed_backup, failed_patch, original_position, read, rebuild, remove_temporary, sha256,
    tree_outcome, writable, write_ahead,
};
use super::{
    Backups, CloseReason, Error, Failure, FileResult, Journal, Outcome, Record, Recorded, Report, Restored,
    RunId, Selection, ServerFolder, Skip,
};
use crate::archetypes::{self, KeeperCheck};
use crate::edit::{Edit, Removal};
use crate::resources::ServerIndex;

/// Settles every pending record of the folder, from what its file holds now: the write landed
/// (applied), never landed (the record goes back to what it was, or is closed as abandoned when it
/// was new), or the file holds its original (closed as restored) or something else (closed as
/// stale). Every leftover temporary file is removed. A file that cannot be read keeps its record
/// pending, and the answer says why. Run at every start of Observer and before every run; the
/// other operations run it first themselves.
pub fn reconcile<J: Journal + ?Sized>(
    journal: &mut J,
    folder: &ServerFolder,
    run: RunId,
) -> Result<Vec<FileResult>, Error> {
    let mut results = Vec::new();
    for record in journal.open(folder.id())? {
        let Some(pending) = &record.pending else {
            continue;
        };
        let held = match RelativePath::new(record.path.split('/')).map(|at| read(folder, &at)) {
            Ok(Ok((bytes, _))) => Some(sha256(&bytes)),
            Ok(Err(TreeError::NotFound)) | Err(_) => None,
            Ok(Err(error)) => {
                results.push(result(&record, tree_outcome(error)));
                continue;
            }
        };
        remove_temporary(folder, &record.path, &pending.temporary);
        let outcome = if held == Some(pending.expected_sha256) {
            journal.applied(record.id)?;
            if pending.removals.is_empty() {
                // An original written back by a heal or an undo that did not get to close it.
                journal.close(record.id, run, CloseReason::Restored)?;
                Outcome::Closed {
                    reason: CloseReason::Restored,
                }
            } else {
                Outcome::Written {
                    removed: positions(&pending.removals),
                }
            }
        } else if held.is_some() && held == record.patched_sha256 {
            journal.abandon(record.id)?;
            Outcome::Kept
        } else if held == Some(record.original_sha256) && record.patched_sha256.is_none() {
            journal.abandon(record.id)?;
            Outcome::Closed {
                reason: CloseReason::Abandoned,
            }
        } else {
            let reason = if held == Some(record.original_sha256) {
                CloseReason::Restored
            } else {
                CloseReason::Stale
            };
            journal.close(record.id, run, reason)?;
            Outcome::Closed { reason }
        };
        results.push(result(&record, outcome));
    }
    Ok(results)
}

/// Heals the folder's open records against a fresh `index`: a record whose file no longer holds
/// what Observer wrote is closed (restored when it holds its original, else stale); a removal whose
/// copy kept is gone ([`archetypes::check_keeper`]) is put back, by rebuilding the file from its
/// original without the removals still valid, or by writing the original back and closing the
/// record as healed when none is left. Reconciles first. Run before every scan and at every start,
/// whatever the tool's mode.
pub fn heal<J: Journal + ?Sized>(
    index: &ServerIndex,
    settings: &ToolSettings,
    journal: &mut J,
    folder: &ServerFolder,
    backups: &Backups,
    run: RunId,
    cancel: &AtomicBool,
) -> Result<Report, Error> {
    let mut report = Report {
        reconciled: reconcile(journal, folder, run)?,
        ..Report::default()
    };
    for record in journal.open(folder.id())? {
        if stopped(&mut report, cancel) {
            report.files.push(result(&record, not_tried()));
            continue;
        }
        match heal_one(index, settings, journal, folder, backups, run, &record)? {
            Some(healed) => push(&mut report, healed),
            None => report.unchanged += 1,
        }
    }
    Ok(report)
}

/// Heals one record; `None` when every copy kept stands.
fn heal_one<J: Journal + ?Sized>(
    index: &ServerIndex,
    settings: &ToolSettings,
    journal: &mut J,
    folder: &ServerFolder,
    backups: &Backups,
    run: RunId,
    record: &Record,
) -> Result<Option<FileResult>, Error> {
    let (at, bytes, read_id) = match read_record(folder, index.refused(), record) {
        Ok(read) => read,
        Err(Outcome::Skipped { why: Skip::Missing }) => {
            journal.close(record.id, run, CloseReason::Stale)?;
            let closed = Outcome::Closed {
                reason: CloseReason::Stale,
            };
            return Ok(Some(result(record, closed)));
        }
        Err(outcome) => return Ok(Some(result(record, outcome))),
    };
    if let Some(closed) = close_if_changed(journal, run, record, &bytes)? {
        return Ok(Some(result(record, closed)));
    }
    let mut kept = Vec::new();
    let mut restored = Vec::new();
    for recorded in &record.removals {
        match archetypes::check_keeper(index, settings, &record.path, &recorded.removal) {
            KeeperCheck::Holds => kept.push(recorded.clone()),
            KeeperCheck::Gone(why) => restored.push(restore(recorded, Some(why.to_string()))),
        }
    }
    if restored.is_empty() {
        return Ok(None);
    }
    let outcome = rewrite(
        journal,
        folder,
        backups,
        run,
        record,
        &at,
        read_id,
        kept,
        CloseReason::Healed,
    )?;
    let restored = if matches!(outcome, Outcome::Written { .. } | Outcome::Closed { .. }) {
        restored
    } else {
        Vec::new()
    };
    Ok(Some(FileResult {
        restored,
        ..result(record, outcome)
    }))
}

/// Applies `edits`, a plan the analysis made on the files as they are now, under `run`. A file with
/// an open record is rebuilt from its original with the record's removals and the new ones; any
/// other is backed up first. A file that is no longer the one the plan was made on is skipped.
/// Reconciles first; a record that no longer describes its file is closed, as a heal would.
pub fn apply<J: Journal + ?Sized>(
    edits: &[Edit],
    index: &ServerIndex,
    journal: &mut J,
    folder: &ServerFolder,
    backups: &Backups,
    run: RunId,
    cancel: &AtomicBool,
) -> Result<Report, Error> {
    let mut report = Report {
        reconciled: reconcile(journal, folder, run)?,
        ..Report::default()
    };
    for edit in edits {
        if stopped(&mut report, cancel) {
            report.files.push(edit_result(edit, not_tried()));
            continue;
        }
        let outcome = apply_one(edit, index, journal, folder, backups, run, &mut report.reconciled)?;
        push(&mut report, edit_result(edit, outcome));
    }
    Ok(report)
}

fn apply_one<J: Journal + ?Sized>(
    edit: &Edit,
    index: &ServerIndex,
    journal: &mut J,
    folder: &ServerFolder,
    backups: &Backups,
    run: RunId,
    reconciled: &mut Vec<FileResult>,
) -> Result<Outcome, Error> {
    if edit.removals.is_empty() {
        return Ok(Outcome::Skipped {
            why: Skip::NothingToDo,
        });
    }
    let Some(at) = writable(&edit.path, folder, index.refused()) else {
        return Ok(refused());
    };
    let existing = journal
        .open(folder.id())?
        .into_iter()
        .find(|record| record.path == edit.path);
    if existing.as_ref().is_some_and(|record| record.pending.is_some()) {
        return Ok(Outcome::Skipped {
            why: Skip::Unresolved,
        });
    }
    let (bytes, read_id) = match read(folder, &at) {
        Ok(read) => read,
        Err(error) => return Ok(tree_outcome(error)),
    };
    if sha256(&bytes) != edit.sha256 {
        return Ok(Outcome::Skipped { why: Skip::Changed });
    }
    let existing = match existing {
        Some(record) => match close_if_changed(journal, run, &record, &bytes)? {
            Some(closed) => {
                reconciled.push(result(&record, closed));
                None
            }
            None => Some(record),
        },
        None => None,
    };
    let original = match &existing {
        Some(record) => match backups.get(&record.original_sha256) {
            Ok(original) => original,
            Err(error) => return Ok(failed_backup(error.to_string())),
        },
        None => bytes.clone(),
    };
    let previous = existing.as_ref().map_or(&[][..], |record| &record.removals[..]);
    let removals = match translate(edit, &bytes, previous, run) {
        Ok(removals) => removals,
        Err(error) => return Ok(failed_patch(error)),
    };
    let new_bytes = match rebuild(edit.format, &original, &removals) {
        Ok(new_bytes) => new_bytes,
        Err(error) => return Ok(failed_patch(error)),
    };
    let removed = positions(&removals);
    let planned = Planned {
        path: &edit.path,
        at: &at,
        resource: &edit.resource,
        format: edit.format,
        read: read_id,
        original_sha256: existing
            .as_ref()
            .map_or(edit.sha256, |record| record.original_sha256),
        backup: existing.is_none().then_some(&original[..]),
        existing: existing.as_ref().map(|record| record.id),
        bytes: new_bytes,
        removals,
        close: None,
    };
    Ok(match write_ahead(journal, folder, backups, run, planned)? {
        Ok(_) => Outcome::Written { removed },
        Err(outcome) => outcome,
    })
}

/// The record's removals and the edit's together, the edit's turned into positions in the original:
/// the file read holds the original without `previous`, in order. Each entry the edit names must
/// register its hash in the file read.
fn translate(edit: &Edit, bytes: &[u8], previous: &[Recorded], run: RunId) -> Result<Vec<Recorded>, String> {
    let current = super::write::entries(edit.format, bytes)?;
    let removed: BTreeSet<usize> = previous.iter().map(|recorded| recorded.removal.index).collect();
    let mut removals = previous.to_vec();
    for removal in &edit.removals {
        if current.get(removal.index) != Some(&Some(removal.hash)) {
            return Err(format!(
                "entry {} of the file does not register {:08X}, as the plan says",
                removal.index, removal.hash
            ));
        }
        removals.push(Recorded {
            removal: Removal {
                index: original_position(removal.index, &removed),
                ..removal.clone()
            },
            run,
        });
    }
    removals.sort_by_key(|recorded| recorded.removal.index);
    Ok(removals)
}

/// Puts back the removals `selection` takes, by rebuilding each file from its original without
/// them, or by writing the original back and closing the record as undone when none is left. Only
/// a file that still holds what Observer wrote is touched; any other is listed with the reason.
/// Reconciles first.
pub fn undo<J: Journal + ?Sized>(
    journal: &mut J,
    folder: &ServerFolder,
    backups: &Backups,
    selection: &Selection,
    run: RunId,
    cancel: &AtomicBool,
) -> Result<Report, Error> {
    let mut report = Report {
        reconciled: reconcile(journal, folder, run)?,
        ..Report::default()
    };
    for record in journal.open(folder.id())? {
        let selected = record
            .removals
            .iter()
            .any(|recorded| selection.takes(&record.path, recorded));
        if !selected {
            continue;
        }
        if stopped(&mut report, cancel) {
            report.files.push(result(&record, not_tried()));
            continue;
        }
        let undone = undo_one(journal, folder, backups, selection, run, &record)?;
        push(&mut report, undone);
    }
    Ok(report)
}

fn undo_one<J: Journal + ?Sized>(
    journal: &mut J,
    folder: &ServerFolder,
    backups: &Backups,
    selection: &Selection,
    run: RunId,
    record: &Record,
) -> Result<FileResult, Error> {
    let (at, bytes, read_id) = match read_record(folder, &[], record) {
        Ok(read) => read,
        Err(outcome) => return Ok(result(record, outcome)),
    };
    if Some(sha256(&bytes)) != record.patched_sha256 {
        return Ok(result(record, Outcome::Skipped { why: Skip::Changed }));
    }
    let (undone, kept): (Vec<&Recorded>, Vec<&Recorded>) = record
        .removals
        .iter()
        .partition(|recorded| selection.takes(&record.path, recorded));
    let restored: Vec<Restored> = undone.iter().map(|recorded| restore(recorded, None)).collect();
    let kept: Vec<Recorded> = kept.into_iter().cloned().collect();
    let outcome = rewrite(
        journal,
        folder,
        backups,
        run,
        record,
        &at,
        read_id,
        kept,
        CloseReason::Undone,
    )?;
    let restored = if matches!(outcome, Outcome::Written { .. } | Outcome::Closed { .. }) {
        restored
    } else {
        Vec::new()
    };
    Ok(FileResult {
        restored,
        ..result(record, outcome)
    })
}

/// Reads a record's file, when it has no write pending and lies where Observer may write: its path,
/// its bytes and its identity, or why it is left.
fn read_record(
    folder: &ServerFolder,
    also: &[Vec<String>],
    record: &Record,
) -> Result<(RelativePath, Vec<u8>, FileId), Outcome> {
    if record.pending.is_some() {
        return Err(Outcome::Skipped {
            why: Skip::Unresolved,
        });
    }
    let at = writable(&record.path, folder, also).ok_or_else(refused)?;
    let (bytes, id) = read(folder, &at).map_err(tree_outcome)?;
    Ok((at, bytes, id))
}

/// Closes `record` when the file `bytes` no longer holds what Observer wrote: as restored when it
/// holds its original, else as stale. Answers the outcome when it did.
fn close_if_changed<J: Journal + ?Sized>(
    journal: &mut J,
    run: RunId,
    record: &Record,
    bytes: &[u8],
) -> Result<Option<Outcome>, Error> {
    let held = sha256(bytes);
    let reason = if Some(held) == record.patched_sha256 {
        if !record.removals.is_empty() {
            return Ok(None);
        }
        // An original written back by a heal or an undo that did not get to close it.
        CloseReason::Restored
    } else if held == record.original_sha256 {
        CloseReason::Restored
    } else {
        CloseReason::Stale
    };
    journal.close(record.id, run, reason)?;
    Ok(Some(Outcome::Closed { reason }))
}

/// Rebuilds a record's file from its original with only `kept`, or writes the original back and
/// closes the record for `reason` when `kept` is empty.
#[allow(clippy::too_many_arguments)]
fn rewrite<J: Journal + ?Sized>(
    journal: &mut J,
    folder: &ServerFolder,
    backups: &Backups,
    run: RunId,
    record: &Record,
    at: &RelativePath,
    read_id: FileId,
    kept: Vec<Recorded>,
    reason: CloseReason,
) -> Result<Outcome, Error> {
    let original = match backups.get(&record.original_sha256) {
        Ok(original) => original,
        Err(error) => return Ok(failed_backup(error.to_string())),
    };
    let bytes = match rebuild(record.format, &original, &kept) {
        Ok(bytes) => bytes,
        Err(error) => return Ok(failed_patch(error)),
    };
    let close = kept.is_empty().then_some(reason);
    let removed = positions(&kept);
    let planned = Planned {
        path: &record.path,
        at,
        resource: &record.resource,
        format: record.format,
        read: read_id,
        original_sha256: record.original_sha256,
        backup: None,
        existing: Some(record.id),
        bytes,
        removals: kept,
        close,
    };
    Ok(match write_ahead(journal, folder, backups, run, planned)? {
        Ok(_) => match close {
            Some(reason) => Outcome::Closed { reason },
            None => Outcome::Written { removed },
        },
        Err(outcome) => outcome,
    })
}

/// Whether the run must stop before the next file: the cancel flag raised, or a volume full.
fn stopped(report: &mut Report, cancel: &AtomicBool) -> bool {
    if cancel.load(Ordering::Relaxed) {
        report.cancelled = true;
    }
    report.cancelled || report.storage_full
}

/// Adds a file's result, and notes a full volume.
fn push(report: &mut Report, file: FileResult) {
    if matches!(
        file.outcome,
        Outcome::Failed {
            why: Failure::StorageFull { .. }
        }
    ) {
        report.storage_full = true;
    }
    report.files.push(file);
}

fn result(record: &Record, outcome: Outcome) -> FileResult {
    FileResult {
        path: record.path.clone(),
        resource: record.resource.clone(),
        outcome,
        restored: Vec::new(),
    }
}

fn edit_result(edit: &Edit, outcome: Outcome) -> FileResult {
    FileResult {
        path: edit.path.clone(),
        resource: edit.resource.clone(),
        outcome,
        restored: Vec::new(),
    }
}

fn restore(recorded: &Recorded, why: Option<String>) -> Restored {
    Restored {
        index: recorded.removal.index,
        hash: recorded.removal.hash,
        run: recorded.run,
        why,
    }
}

fn positions(removals: &[Recorded]) -> Vec<usize> {
    removals.iter().map(|recorded| recorded.removal.index).collect()
}

fn not_tried() -> Outcome {
    Outcome::Skipped { why: Skip::NotTried }
}

fn refused() -> Outcome {
    Outcome::Skipped { why: Skip::Refused }
}
