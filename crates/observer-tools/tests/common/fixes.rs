//! What the tests of the fixes need besides a server: a journal held in memory that keeps the
//! store's rules (one open record per file, uploads recorded with the pending write and forgotten
//! once it is settled), a backups folder outside the server, and the operations run on both. The
//! journal can also fail a call, or run a hook once a write is recorded, to stand for a crash or
//! for another writer.

use std::fs;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use observer_domain::tools::ToolSettings;
use observer_gamefiles::hash::name_hash;
use observer_tools::edit::{Edit, Format};
use observer_tools::fix::{
    self, Backups, CloseReason, FileResult, FolderId, Journal, JournalError, Outcome, Pending, Record,
    RecordId, Report, RunId, Selection, ServerFolder,
};
use tempfile::TempDir;

use super::Server;

/// Where a record stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Open,
    Closed { reason: CloseReason, run: RunId },
}

/// A record and its state, closed ones kept.
#[derive(Debug, Clone)]
pub struct Stored {
    pub record: Record,
    pub state: State,
    pub opened_run: RunId,
    pub changed_run: RunId,
}

/// Runs once a write is recorded, before its temporary file exists, with the file's path.
pub type Hook = Box<dyn FnMut(&str, &Pending)>;

#[derive(Default)]
pub struct MemoryJournal {
    pub stored: Vec<Stored>,
    /// The temporary files recorded as uploads and not forgotten: their folder and name.
    pub uploads: Vec<(String, String)>,
    pub on_begin: Option<Hook>,
    /// Refuses `applied`, as a database that went away after the rename would.
    pub fail_applied: bool,
    next: RecordId,
}

impl MemoryJournal {
    /// The open record of `path`.
    pub fn record(&self, path: &str) -> Option<&Record> {
        self.stored
            .iter()
            .find(|stored| stored.state == State::Open && stored.record.path == path)
            .map(|stored| &stored.record)
    }

    /// The paths of the open records.
    pub fn open_paths(&self) -> Vec<String> {
        self.open_records()
            .into_iter()
            .map(|record| record.path.clone())
            .collect()
    }

    pub fn open_records(&self) -> Vec<&Record> {
        self.stored
            .iter()
            .filter(|stored| stored.state == State::Open)
            .map(|stored| &stored.record)
            .collect()
    }

    /// Why the records of `path` were closed, oldest first.
    pub fn closed(&self, path: &str) -> Vec<CloseReason> {
        self.stored
            .iter()
            .filter(|stored| stored.record.path == path)
            .filter_map(|stored| match stored.state {
                State::Closed { reason, .. } => Some(reason),
                State::Open => None,
            })
            .collect()
    }

    fn open_mut(&mut self, id: RecordId) -> Result<&mut Stored, JournalError> {
        self.stored
            .iter_mut()
            .find(|stored| stored.record.id == id && stored.state == State::Open)
            .ok_or_else(|| JournalError::new(format!("no open record {id}")))
    }

    fn forget(&mut self, path: &str, pending: Option<Pending>) {
        if let Some(pending) = pending {
            let folder = parent(path);
            self.uploads
                .retain(|(at, name)| !(*at == folder && *name == pending.temporary));
        }
    }
}

fn parent(path: &str) -> String {
    path.rsplit_once('/')
        .map_or_else(String::new, |(folder, _)| folder.to_owned())
}

impl Journal for MemoryJournal {
    fn open(&mut self, folder: &FolderId) -> Result<Vec<Record>, JournalError> {
        Ok(self
            .stored
            .iter()
            .filter(|stored| stored.state == State::Open && stored.record.folder.id == folder.id)
            .map(|stored| stored.record.clone())
            .collect())
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
        let id = if let Some(id) = existing {
            let stored = self.open_mut(id)?;
            if stored.record.pending.is_some() || stored.record.path != path {
                return Err(JournalError::new(format!("record {id} cannot begin a write")));
            }
            stored.record.pending = Some(pending.clone());
            stored.changed_run = run;
            id
        } else {
            let taken = self.stored.iter().any(|stored| {
                stored.state == State::Open
                    && stored.record.folder.id == folder.id
                    && stored.record.path == path
            });
            if taken {
                return Err(JournalError::new(format!("{path} already has an open record")));
            }
            self.next += 1;
            self.stored.push(Stored {
                record: Record {
                    id: self.next,
                    folder: folder.clone(),
                    path: path.to_owned(),
                    resource: resource.to_owned(),
                    format,
                    original_sha256,
                    patched_sha256: None,
                    removals: Vec::new(),
                    pending: Some(pending.clone()),
                },
                state: State::Open,
                opened_run: run,
                changed_run: run,
            });
            self.next
        };
        self.uploads.push((parent(path), pending.temporary.clone()));
        if let Some(mut hook) = self.on_begin.take() {
            hook(path, pending);
            self.on_begin = Some(hook);
        }
        Ok(id)
    }

    fn applied(&mut self, id: RecordId) -> Result<(), JournalError> {
        if self.fail_applied {
            return Err(JournalError::new("the database went away"));
        }
        let stored = self.open_mut(id)?;
        let pending = stored
            .record
            .pending
            .take()
            .ok_or_else(|| JournalError::new(format!("record {id} has no pending write")))?;
        stored.record.patched_sha256 = Some(pending.expected_sha256);
        stored.record.removals.clone_from(&pending.removals);
        let path = stored.record.path.clone();
        self.forget(&path, Some(pending));
        Ok(())
    }

    fn abandon(&mut self, id: RecordId) -> Result<(), JournalError> {
        let stored = self.open_mut(id)?;
        let pending = stored.record.pending.take();
        if stored.record.patched_sha256.is_none() {
            stored.state = State::Closed {
                reason: CloseReason::Abandoned,
                run: stored.changed_run,
            };
        }
        let path = stored.record.path.clone();
        self.forget(&path, pending);
        Ok(())
    }

    fn close(&mut self, id: RecordId, run: RunId, reason: CloseReason) -> Result<(), JournalError> {
        let stored = self.open_mut(id)?;
        let pending = stored.record.pending.take();
        stored.state = State::Closed { reason, run };
        let path = stored.record.path.clone();
        self.forget(&path, pending);
        Ok(())
    }
}

/// A journal, a backups folder outside the server, and the operations run with them.
pub struct Fixes {
    pub journal: MemoryJournal,
    pub backups: Backups,
    data: TempDir,
}

impl Fixes {
    pub fn new() -> Self {
        let data = tempfile::tempdir().unwrap();
        Self {
            journal: MemoryJournal::default(),
            backups: Backups::new(data.path().join("tools").join("backups")),
            data,
        }
    }

    /// The folder, opened for one operation: like a run, each opens its own.
    pub fn folder(server: &Server) -> ServerFolder {
        ServerFolder::open(server.root()).unwrap()
    }

    pub fn apply(&mut self, server: &Server, edits: &[Edit], run: RunId) -> Report {
        self.try_apply(server, edits, run).unwrap()
    }

    pub fn try_apply(&mut self, server: &Server, edits: &[Edit], run: RunId) -> Result<Report, fix::Error> {
        let folder = Self::folder(server);
        fix::apply(
            edits,
            &server.index(),
            &mut self.journal,
            &folder,
            &self.backups,
            run,
            &AtomicBool::new(false),
        )
    }

    pub fn heal(&mut self, server: &Server, settings: &ToolSettings, run: RunId) -> Report {
        let folder = Self::folder(server);
        fix::heal(
            &server.index(),
            settings,
            &mut self.journal,
            &folder,
            &self.backups,
            run,
            &AtomicBool::new(false),
        )
        .unwrap()
    }

    pub fn undo(&mut self, server: &Server, selection: &Selection, run: RunId) -> Report {
        let folder = Self::folder(server);
        fix::undo(
            &mut self.journal,
            &folder,
            &self.backups,
            selection,
            run,
            &AtomicBool::new(false),
        )
        .unwrap()
    }

    pub fn reconcile(&mut self, server: &Server, run: RunId) -> Vec<FileResult> {
        let folder = Self::folder(server);
        fix::reconcile(&mut self.journal, &folder, run).unwrap()
    }

    /// The names in the backups folder, sorted.
    pub fn backup_names(&self) -> Vec<String> {
        names_in(&self.data.path().join("tools").join("backups"))
    }
}

/// Each file's outcome, by path.
pub fn outcomes(report: &Report) -> Vec<(&str, &Outcome)> {
    report
        .files
        .iter()
        .map(|file| (file.path.as_str(), &file.outcome))
        .collect()
}

/// The names in `dir`, sorted; none when it is not there.
pub fn names_in(dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// The archetype hashes of the `.ytyp` at `path`, in list order.
pub fn archetypes(path: &Path) -> Vec<u32> {
    observer_gamefiles::ytyp::read(&fs::read(path).unwrap())
        .unwrap()
        .archetypes
        .iter()
        .map(|archetype| archetype.name)
        .collect()
}

/// The model names of the model-info file at `path`, in order.
pub fn items(path: &Path) -> Vec<u32> {
    observer_gamefiles::model_info::read(&fs::read(path).unwrap())
        .unwrap()
        .unwrap()
        .items
        .iter()
        .map(|item| item.hash)
        .collect()
}

pub fn hashes(names: &[&str]) -> Vec<u32> {
    names.iter().map(|name| name_hash(name)).collect()
}

pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    use sha2::Digest;
    sha2::Sha256::digest(bytes).into()
}
