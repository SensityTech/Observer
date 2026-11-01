//! The journal of the files the tools change, kept in the state database. The fixes call it from a
//! blocking thread, between file operations, so each call waits there for its transaction: a
//! write is recorded before its temporary file exists, and again once its rename landed.
//!
//! The store keeps names as the panel shows them: formats and reasons by the domain's names,
//! SHA-256 digests in hex, and the folder as the launch settings give it, which is also the root
//! the start's sweep opens to remove a temporary file left behind. The folder's identity is what
//! finds the records.

use std::collections::HashMap;

use observer_domain::tools::{
    ArchetypeReport, ToolCloseReason, ToolFileFormat, ToolKeeper, ToolRemoval, sha256_hex,
};
use observer_store::tools::{self as stored, BeginWrite, ToolChangeRow};
use observer_store::{Database, StoreError, Transaction};
use observer_tools::edit::{Format, Keeper, Removal};
use observer_tools::fix::{
    CloseReason, FolderId, Journal, JournalError, Pending, Record, RecordId, Recorded, RunId,
};
use tokio::runtime::Handle;

use super::TOOL;
use crate::{Clock, Servers};

/// The journal of one run, over the store.
pub(super) struct StoreJournal {
    db: Database,
    runtime: Handle,
    clock: Clock,
    /// The server folder as the launch settings give it.
    root: String,
    /// Archetype names by hash, from the run's report and the records read, so that the panel
    /// names what each file holds without.
    names: HashMap<u32, String>,
}

/// The journal's own refusals, besides the store's failures.
#[derive(Debug, thiserror::Error)]
enum Refusal {
    #[error("the journal changed since it was read")]
    Changed,
    #[error("the record {0} is not open as the run expects")]
    NotOpen(RecordId),
    #[error("a stored removal is malformed: {0}")]
    Malformed(&'static str),
}

impl StoreJournal {
    /// Must be made on the runtime the database serves, and used from a blocking thread only.
    pub(super) fn new(db: Database, clock: Clock, root: String, names: HashMap<u32, String>) -> Self {
        Self {
            db,
            runtime: Handle::current(),
            clock,
            root,
            names,
        }
    }

    /// Learns the names a report spells, for the removals recorded from now on.
    pub(super) fn learn(&mut self, report: &ArchetypeReport) {
        for group in &report.groups {
            if let Some(name) = &group.name {
                self.names.entry(group.hash).or_insert_with(|| name.clone());
            }
        }
    }

    fn write<R: Send + 'static>(
        &self,
        work: impl FnOnce(&Transaction<'_>) -> Result<R, StoreError> + Send + 'static,
    ) -> Result<R, JournalError> {
        self.runtime
            .block_on(self.db.write(work))
            .map_err(JournalError::new)
    }

    fn removals(&self, removals: &[Recorded]) -> Result<Vec<ToolRemoval>, JournalError> {
        removals
            .iter()
            .map(|recorded| stored_removal(recorded, self.names.get(&recorded.removal.hash).cloned()))
            .collect()
    }
}

impl Journal for StoreJournal {
    fn open(&mut self, folder: &FolderId) -> Result<Vec<Record>, JournalError> {
        let id = folder.id.clone();
        let rows = self
            .runtime
            .block_on(
                self.db
                    .read(move |conn| stored::open_changes(conn, Servers::DEFAULT, TOOL, &id)),
            )
            .map_err(JournalError::new)?;
        for row in &rows {
            let pending = row.pending.iter().flat_map(|pending| &pending.removals);
            for removal in row.removals.iter().chain(pending) {
                if let Some(name) = &removal.name {
                    self.names.entry(removal.hash).or_insert_with(|| name.clone());
                }
            }
        }
        rows.into_iter().map(record).collect()
    }

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
    ) -> Result<RecordId, JournalError> {
        let removals = self.removals(&pending.removals)?;
        let root = self.root.clone();
        let folder_id = folder.id.clone();
        let path = path.to_owned();
        let resource = resource.to_owned();
        let temporary = pending.temporary.clone();
        let sha256 = pending.expected_sha256;
        let at = self.clock.now();
        let id = self.write(move |tx| {
            stored::begin(
                tx,
                &BeginWrite {
                    server_id: Servers::DEFAULT,
                    tool: TOOL,
                    run,
                    folder_path: &root,
                    folder_id: &folder_id,
                    path: &path,
                    resource: &resource,
                    format: file_format(format),
                    original_sha256,
                    temporary: &temporary,
                    sha256,
                    removals: &removals,
                    existing,
                    at,
                },
            )
        })?;
        id.ok_or_else(|| JournalError::new(Refusal::Changed))
    }

    fn applied(&mut self, id: RecordId) -> Result<(), JournalError> {
        let at = self.clock.now();
        settled(id, self.write(move |tx| stored::applied(tx, id, at))?)
    }

    fn abandon(&mut self, id: RecordId) -> Result<(), JournalError> {
        let at = self.clock.now();
        settled(id, self.write(move |tx| stored::abandon(tx, id, at))?)
    }

    fn close(&mut self, id: RecordId, run: RunId, reason: CloseReason) -> Result<(), JournalError> {
        let at = self.clock.now();
        let reason = close_reason(reason);
        settled(id, self.write(move |tx| stored::close(tx, id, run, reason, at))?)
    }
}

fn settled(id: RecordId, done: bool) -> Result<(), JournalError> {
    if done {
        Ok(())
    } else {
        Err(JournalError::new(Refusal::NotOpen(id)))
    }
}

/// A stored record as the fixes read it.
fn record(row: ToolChangeRow) -> Result<Record, JournalError> {
    let pending = match row.pending {
        Some(pending) => Some(Pending {
            temporary: pending.temporary,
            expected_sha256: pending.sha256,
            removals: recorded(&pending.removals)?,
        }),
        None => None,
    };
    Ok(Record {
        id: row.id,
        folder: FolderId {
            path: row.folder_path,
            id: row.folder_id,
        },
        path: row.path,
        resource: row.resource,
        format: format(row.format),
        original_sha256: row.original_sha256,
        patched_sha256: row.patched_sha256,
        removals: recorded(&row.removals)?,
        pending,
    })
}

fn recorded(removals: &[ToolRemoval]) -> Result<Vec<Recorded>, JournalError> {
    removals
        .iter()
        .map(|removal| {
            let sha256 = observer_domain::tools::sha256_from_hex(&removal.keeper.sha256)
                .ok_or_else(|| JournalError::new(Refusal::Malformed("a keeper's SHA-256")))?;
            Ok(Recorded {
                removal: Removal {
                    index: position(removal.index)?,
                    hash: removal.hash,
                    keeper: Keeper {
                        path: removal.keeper.path.clone(),
                        format: format(removal.keeper.format),
                        index: position(removal.keeper.index)?,
                        sha256,
                    },
                },
                run: removal.run,
            })
        })
        .collect()
}

/// A removal as the store keeps it, with the archetype's name when the run knows it.
pub(super) fn stored_removal(recorded: &Recorded, name: Option<String>) -> Result<ToolRemoval, JournalError> {
    let removal = &recorded.removal;
    Ok(ToolRemoval {
        index: stored_position(removal.index)?,
        hash: removal.hash,
        name,
        keeper: ToolKeeper {
            path: removal.keeper.path.clone(),
            format: file_format(removal.keeper.format),
            index: stored_position(removal.keeper.index)?,
            sha256: sha256_hex(&removal.keeper.sha256),
        },
        run: recorded.run,
    })
}

fn position(index: u32) -> Result<usize, JournalError> {
    usize::try_from(index).map_err(|_| JournalError::new(Refusal::Malformed("a position")))
}

fn stored_position(index: usize) -> Result<u32, JournalError> {
    u32::try_from(index).map_err(|_| JournalError::new(Refusal::Malformed("a position past 2^32")))
}

pub(super) fn file_format(format: Format) -> ToolFileFormat {
    match format {
        Format::Ytyp => ToolFileFormat::Ytyp,
        Format::ModelInfo => ToolFileFormat::ModelInfo,
    }
}

fn format(format: ToolFileFormat) -> Format {
    match format {
        ToolFileFormat::Ytyp => Format::Ytyp,
        ToolFileFormat::ModelInfo => Format::ModelInfo,
    }
}

fn close_reason(reason: CloseReason) -> ToolCloseReason {
    match reason {
        CloseReason::Undone => ToolCloseReason::Undone,
        CloseReason::Healed => ToolCloseReason::Healed,
        CloseReason::Stale => ToolCloseReason::Stale,
        CloseReason::Restored => ToolCloseReason::Restored,
        CloseReason::Abandoned => ToolCloseReason::Abandoned,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_removal_reads_back_as_it_was_recorded_with_its_name_kept_aside() {
        let recorded = Recorded {
            removal: Removal {
                index: 3,
                hash: 0x1234_abcd,
                keeper: Keeper {
                    path: "resources/props/stream/props.ytyp".to_owned(),
                    format: Format::ModelInfo,
                    index: 9,
                    sha256: [0xab; 32],
                },
            },
            run: 4,
        };

        let stored = stored_removal(&recorded, Some("bz_prop_gift".to_owned())).unwrap();

        assert_eq!(stored.keeper.format, ToolFileFormat::ModelInfo);
        assert_eq!(stored.keeper.sha256, "ab".repeat(32));
        assert_eq!(stored.name.as_deref(), Some("bz_prop_gift"));
        assert_eq!(super::recorded(&[stored]).unwrap(), [recorded]);
    }

    #[test]
    fn every_close_reason_is_stored_by_the_same_name() {
        for reason in CloseReason::ALL {
            assert_eq!(close_reason(reason).as_str(), reason.as_str());
        }
    }
}
