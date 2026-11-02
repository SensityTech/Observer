//! The server tools as Observer runs them: from the Tools page, from the command line, and before
//! every start of FXServer. The first tool takes out the archetypes the clients register more than
//! once; what it reads, decides and writes lives in `observer-tools`, and this service decides
//! when it runs and keeps its records.
//!
//! - **One writer at a time.** A fix, a heal and an undo hold the writer lock, and the work before
//!   each start takes it too, so FXServer never starts while a tool writes. The Tools page's Fix
//!   and Undo also run only while FXServer is stopped. While a tool writes, SFTP commits under the
//!   server folder wait at the gate they share.
//! - **A scan writes nothing**: neither a heal nor the settling of an interrupted write. It says
//!   how many changes are open; the next start or the next fix heals them.
//! - **A fix applies what the owner saw.** The panel confirms the digest of the plan it showed; the
//!   fix heals first, analyses again, and applies nothing if the fresh plan is another one, which
//!   it hands back to review instead.
//! - **Before every start**, whatever the mode or the cause, open changes whose copy kept is gone
//!   are healed. A crash recovery and the mode `off` stop there; `report` analyses and says what
//!   changed since the last report, `fix` applies the fresh plan.
//!
//! Every run is recorded with its summary, and its report when it made one. Runs that change files
//! are audited; reports are not. A run that Observer's stop cut short is audited at the next
//! start, from what the journal says it wrote. The originals of changed files are kept under
//! `data/tools/backups` while a change is open and for 30 days after it closed.
//!
//! An undo by file or by run works in the folder the changes were made in, opened only while it is
//! still that folder; an undo of everything works in the launch settings' folder.

mod journal;
mod lines;
#[cfg(test)]
mod tests;
mod work;

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use jiff::{SignedDuration, Timestamp};
use observer_domain::audit::{ActorKind, AuditAction};
use observer_domain::error::{ApiError, ErrorCode};
use observer_domain::features::Feature;
use observer_domain::server::{ServerState, ServerStatus};
use observer_domain::server_data::Game;
use observer_domain::tools::{
    ToolChangeState, ToolCloseReason, ToolKind, ToolMode, ToolRun, ToolRunMode, ToolRunOutcome, ToolSettings,
    ToolSettingsChange, ToolTrigger, ToolUndoRequest, ToolUndoScope, ToolView, ToolsView,
};
use observer_fxserver::layout::{Flavour, Layout};
use observer_fxserver::pre_launch::{PreLaunch, PreLaunchFuture, PreLaunchRequest, PreLaunchResult};
use observer_fxserver::supervisor::Cause;
use observer_store::artifacts as stored_artifacts;
use observer_store::tools::{self as stored, ChangeList, FinishedRun, NewRun, ToolChangeRow};
use observer_store::{Database, StoreError, launch};
use observer_tools::fix::{Backups, FileResult, FolderId, Selection, ServerFolder};
use serde_json::{Value, json};
use tokio::sync::{Mutex, watch};
use tracing::warn;

use crate::audit::Record;
use crate::files::CommitGate;
use crate::{Caller, Clock, Error, Features, OwnPaths, Servers};
use journal::StoreJournal;
use work::{Finished, Job, Prepared, Steps};

pub use lines::outcome_text;
pub use work::{ReportError, Target, report};

/// The tool the routes, the command line and the starts run.
const TOOL: ToolKind = ToolKind::DuplicateArchetypes;
/// How long a closed change stays listed, with its backup kept.
const CLOSED_KEPT: SignedDuration = SignedDuration::from_hours(30 * 24);
/// The most closed changes the Tools page lists.
const CLOSED_LISTED: u32 = 200;
/// The most files an audit entry names; its count says how many there were.
const AUDITED_FILES: usize = 50;
/// The most resources the owner excludes, and the longest name.
const MAX_EXCLUDED: usize = 1000;
const MAX_RESOURCE_NAME: usize = 255;
/// The latest runs looked at for those an earlier Observer left unfinished: only the runs under way
/// when it stopped can be, a writer and the scans beside it.
const UNFINISHED_LOOKED: u32 = 100;

/// What [`Tools::new`] works with.
pub struct ToolsSetup {
    pub db: Database,
    pub clock: Clock,
    /// Observer's data folder: the backups go to its `tools/backups`.
    pub data_dir: PathBuf,
    /// Where the builds are installed, one folder per build number: the selected build's system
    /// resources count as started.
    pub artifacts: PathBuf,
    /// The server's status, which says whether FXServer is stopped; `None` for the command line,
    /// which holds Observer's lock, so that no FXServer of this Observer runs.
    pub status: Option<watch::Receiver<ServerStatus>>,
    /// What SFTP commits take shared and the tools' writes exclusively.
    pub gate: CommitGate,
    /// Observer's own folders and files, never written, should the server folder hold them.
    pub own_paths: OwnPaths,
}

/// The server tools. Clones share the writer lock.
#[derive(Clone)]
pub struct Tools {
    inner: Arc<Inner>,
}

struct Inner {
    db: Database,
    clock: Clock,
    backups: Backups,
    artifacts: PathBuf,
    status: Option<watch::Receiver<ServerStatus>>,
    gate: CommitGate,
    own_paths: OwnPaths,
    writer: Arc<Mutex<()>>,
    /// Set as Observer stops: a run under way ends between two files.
    stopping: Arc<AtomicBool>,
}

/// A run that wrote, or could have: its record, and what became of each file it looked at.
#[derive(Debug, Clone)]
pub struct RunResult {
    pub run: ToolRun,
    /// The writes an earlier run was interrupted in, settled first.
    pub reconciled: Vec<FileResult>,
    /// The files a heal gave entries back to, or tried to.
    pub healed: Vec<FileResult>,
    /// The files of the plan, written or not.
    pub applied: Vec<FileResult>,
    /// The files an undo gave entries back to, or tried to.
    pub undone: Vec<FileResult>,
}

impl RunResult {
    /// Every file the run looked at, in the order it did.
    pub fn files(&self) -> impl Iterator<Item = &FileResult> {
        self.reconciled
            .iter()
            .chain(&self.healed)
            .chain(&self.applied)
            .chain(&self.undone)
    }
}

/// What a run that may write is asked to do.
#[derive(Debug, Clone)]
enum Writing {
    /// Apply the fresh plan; with a digest, only if the fresh plan is that one.
    Fix(Option<[u8; 32]>),
    Undo(ToolUndoScope),
}

/// Who asked for a run, which its record and its audit entries name.
#[derive(Debug, Clone)]
enum Asker {
    Owner(Caller),
    Cli,
    Start,
}

impl Asker {
    fn trigger(&self) -> ToolTrigger {
        match self {
            Self::Owner(_) => ToolTrigger::Panel,
            Self::Cli => ToolTrigger::Cli,
            Self::Start => ToolTrigger::Start,
        }
    }

    fn actor(&self) -> Option<String> {
        match self {
            Self::Owner(caller) => Some(caller.username.clone()),
            Self::Cli | Self::Start => None,
        }
    }

    fn record(&self, action: AuditAction) -> Record {
        let record = match self {
            Self::Owner(caller) => Record::by(caller, action),
            Self::Cli => Record::operator(ActorKind::Cli, action),
            Self::Start => Record::operator(ActorKind::System, action),
        };
        record.target(TOOL.as_str())
    }
}

impl Tools {
    pub fn new(setup: ToolsSetup) -> Self {
        Self {
            inner: Arc::new(Inner {
                db: setup.db,
                clock: setup.clock,
                backups: Backups::new(setup.data_dir.join("tools").join("backups")),
                artifacts: setup.artifacts,
                status: setup.status,
                gate: setup.gate,
                own_paths: setup.own_paths,
                writer: Arc::default(),
                stopping: Arc::default(),
            }),
        }
    }

    /// The Tools page: the settings, the latest runs, the open and recently closed changes, and
    /// what their backups take.
    pub async fn view(&self) -> Result<ToolsView, Error> {
        let since = self.inner.clock.now() - CLOSED_KEPT;
        let (settings, version, last_run, last_report, changes) = self
            .inner
            .db
            .read(move |conn| -> Result<_, StoreError> {
                let (settings, version) = stored::settings(conn, Servers::DEFAULT, TOOL)?;
                let last_run = match stored::runs(conn, Servers::DEFAULT, TOOL, 1)?.first() {
                    Some(latest) => stored::run(conn, latest.id)?,
                    None => None,
                };
                let last_report = stored::last_report(conn, Servers::DEFAULT, TOOL)?
                    .filter(|report| last_run.as_ref().is_none_or(|last| last.id != report.id));
                let changes = stored::changes(conn, Servers::DEFAULT, TOOL, since, CLOSED_LISTED)?;
                Ok((settings, version, last_run, last_report, changes))
            })
            .await?;
        let originals: BTreeSet<[u8; 32]> = changes
            .open
            .iter()
            .chain(&changes.closed)
            .map(|row| row.original_sha256)
            .collect();
        let backups = self.inner.backups.clone();
        let backup_bytes = blocking(move || {
            originals
                .iter()
                .filter_map(|sha256| std::fs::metadata(backups.file(sha256)).ok())
                .map(|metadata| metadata.len())
                .sum()
        })
        .await?;
        let (elsewhere, unreachable) = self.reach(&changes.open).await?;
        Ok(ToolsView {
            tools: vec![ToolView {
                tool: TOOL,
                settings,
                version,
                last_run,
                last_report,
                open: changes.open.iter().map(ToolChangeRow::view).collect(),
                elsewhere,
                unreachable,
                closed: changes.closed.iter().map(ToolChangeRow::view).collect(),
                backup_bytes,
            }],
        })
    }

    /// Which open changes lie outside the launch settings' folder, and which of those no undo
    /// reaches: their folder cannot be opened at its path, or is another folder now.
    async fn reach(&self, open: &[ToolChangeRow]) -> Result<(Vec<i64>, Vec<i64>), Error> {
        let launch = match self.launch_target().await {
            Ok(target) => identity(target.folder()).await?,
            Err(Error::Refused(_)) => None,
            Err(error) => return Err(error),
        };
        let mut reached: HashMap<(&str, &str), bool> = HashMap::new();
        let mut elsewhere = Vec::new();
        let mut unreachable = Vec::new();
        for row in open {
            if launch.as_deref() == Some(row.folder_id.as_str()) {
                continue;
            }
            elsewhere.push(row.id);
            let key = (row.folder_id.as_str(), row.folder_path.as_str());
            let reachable = if let Some(&known) = reached.get(&key) {
                known
            } else {
                let found = identity(Path::new(&row.folder_path)).await?;
                let opened = found.as_deref() == Some(row.folder_id.as_str());
                reached.insert(key, opened);
                opened
            };
            if !reachable {
                unreachable.push(row.id);
            }
        }
        Ok((elsewhere, unreachable))
    }

    /// The tool's settings and their version, `None` until first saved.
    pub async fn settings(&self) -> Result<(ToolSettings, Option<i64>), Error> {
        Ok(self
            .inner
            .db
            .read(|conn| stored::settings(conn, Servers::DEFAULT, TOOL))
            .await?)
    }

    /// Saves the owner's settings over the version they were read at, audited with each change
    /// before and after; the same settings again change nothing.
    pub async fn change_settings(
        &self,
        caller: &Caller,
        change: ToolSettingsChange,
    ) -> Result<ToolsView, Error> {
        self.save_settings(&Asker::Owner(caller.clone()), change.settings, change.version)
            .await?;
        self.view().await
    }

    /// [`Tools::change_settings`] from the command line.
    pub async fn configure_from_cli(
        &self,
        settings: ToolSettings,
        version: Option<i64>,
    ) -> Result<(ToolSettings, Option<i64>), Error> {
        self.save_settings(&Asker::Cli, settings, version).await
    }

    async fn save_settings(
        &self,
        asker: &Asker,
        settings: ToolSettings,
        version: Option<i64>,
    ) -> Result<(ToolSettings, Option<i64>), Error> {
        check_settings(&settings)?;
        let record = asker.record(AuditAction::ToolSettingsChanged);
        let now = self.inner.clock.now();
        self.inner
            .db
            .write(move |tx| -> Result<_, Error> {
                let (before, stored_version) = stored::settings(tx, Servers::DEFAULT, TOOL)?;
                if stored_version != version {
                    return Err(ErrorCode::Conflict.into());
                }
                if before == settings {
                    return Ok((before, stored_version));
                }
                let saved = stored::save_settings(tx, Servers::DEFAULT, TOOL, &settings, version, now)?
                    .ok_or(ErrorCode::Conflict)?;
                record
                    .details(setting_changes(&before, &settings))
                    .write(tx, now)?;
                Ok((settings, Some(saved)))
            })
            .await
    }

    /// Scans the server folder as the launch settings give it, and records the report. Writes
    /// nothing; FXServer may run.
    pub async fn scan(&self, caller: &Caller) -> Result<ToolRun, Error> {
        let tools = self.clone();
        let asker = Asker::Owner(caller.clone());
        detached(async move {
            let target = tools.launch_target().await?;
            let (settings, _) = tools.settings().await?;
            tools.scan_target(&asker, target, settings).await
        })
        .await
    }

    async fn scan_target(
        &self,
        asker: &Asker,
        target: Target,
        settings: ToolSettings,
    ) -> Result<ToolRun, Error> {
        let prepared = self.prepare(target.clone()).await?;
        if !prepared.supported() {
            return Err(not_available());
        }
        let open = self.open_count(&prepared.folder).await?;
        let run = self.insert_run(asker, ToolRunMode::Report).await?;
        let job = self.job(
            &target,
            prepared,
            settings,
            run,
            Arc::clone(&self.inner.stopping),
            open,
        );
        let finished = blocking(move || work::run(job, &Steps::Report)).await?;
        Ok(self.finish(run, asker, finished).await?.run)
    }

    /// Applies the plan the owner confirmed, with FXServer stopped: heals first, analyses again,
    /// and applies only if the fresh plan is the one confirmed. Otherwise `plan_changed`, with the
    /// fresh run to review.
    pub async fn fix(&self, caller: &Caller, plan: &str) -> Result<ToolRun, Error> {
        let confirmed = observer_domain::tools::sha256_from_hex(plan).ok_or(ErrorCode::InvalidRequest)?;
        let tools = self.clone();
        let asker = Asker::Owner(caller.clone());
        detached(async move {
            let ran = tools
                .write(asker, None, None, Writing::Fix(Some(confirmed)))
                .await?;
            Ok(ran.run)
        })
        .await
    }

    /// Puts back what the owner names, with FXServer stopped.
    pub async fn undo(&self, caller: &Caller, request: ToolUndoRequest) -> Result<ToolRun, Error> {
        let tools = self.clone();
        let asker = Asker::Owner(caller.clone());
        detached(async move {
            let ran = tools
                .write(asker, None, None, Writing::Undo(request.scope))
                .await?;
            Ok(ran.run)
        })
        .await
    }

    /// A fix from the command line, of `target` with `settings`: the fresh plan is applied. The
    /// caller holds Observer's lock.
    pub async fn fix_from_cli(&self, target: Target, settings: ToolSettings) -> Result<RunResult, Error> {
        self.write(Asker::Cli, Some(target), Some(settings), Writing::Fix(None))
            .await
    }

    /// An undo from the command line: in the folder of `target` when the command names one, else
    /// where the panel's would. The caller holds Observer's lock.
    pub async fn undo_from_cli(
        &self,
        target: Option<Target>,
        scope: ToolUndoScope,
    ) -> Result<RunResult, Error> {
        self.write(Asker::Cli, target, None, Writing::Undo(scope)).await
    }

    /// A run that may write: under the writer lock, with FXServer stopped, the SFTP gate held
    /// while files are written.
    async fn write(
        &self,
        asker: Asker,
        target: Option<Target>,
        settings: Option<ToolSettings>,
        writing: Writing,
    ) -> Result<RunResult, Error> {
        let _writer = Arc::clone(&self.inner.writer).lock_owned().await;
        self.require_stopped()?;
        let target = match (target, &writing) {
            (Some(target), _) => target,
            (None, Writing::Undo(scope)) => self.undo_target(scope).await?,
            (None, Writing::Fix(_)) => self.launch_target().await?,
        };
        let settings = match settings {
            Some(settings) => settings,
            None => self.settings().await?.0,
        };
        let prepared = self.prepare(target.clone()).await?;
        let (steps, mode) = match writing {
            Writing::Fix(_) if !prepared.supported() => return Err(not_available()),
            Writing::Fix(confirmed) => (Steps::Fix(confirmed), ToolRunMode::Fix),
            Writing::Undo(scope) => (
                Steps::Undo(self.selection(&prepared.folder, scope).await?),
                ToolRunMode::Undo,
            ),
        };
        let open = self.open_count(&prepared.folder).await?;
        let run = self.insert_run(&asker, mode).await?;
        let gate = self.inner.gate.exclusive().await;
        let job = self.job(
            &target,
            prepared,
            settings,
            run,
            Arc::clone(&self.inner.stopping),
            open,
        );
        let finished = blocking(move || work::run(job, &steps)).await?;
        drop(gate);
        let plan_changed = finished.plan_changed;
        let ran = self.finish(run, &asker, finished).await?;
        if plan_changed {
            return Err(ApiError::new(ErrorCode::PlanChanged)
                .with("run", serde_json::to_value(&ran.run).unwrap_or_default())
                .into());
        }
        Ok(ran)
    }

    /// The work before a start of FXServer: under the writer lock, heals the open changes of the
    /// launch's folder, then, unless the start is a crash recovery, reports or fixes as the
    /// settings say. At most one summary line and ten detail lines for the console.
    async fn before_start(&self, request: PreLaunchRequest, on: bool) -> PreLaunchResult {
        // Awaited whatever the cancel flag says: FXServer must not start while another run writes.
        let _writer = Arc::clone(&self.inner.writer).lock_owned().await;
        if request.cancel.load(Ordering::Relaxed) {
            return Ok(Vec::new());
        }
        // Turned off, the tools read nothing of the server folder unless a fix of theirs is still
        // applied there: what it took out must come back once the copy it kept is gone.
        if !on
            && !self
                .inner
                .db
                .read(|conn| stored::any_open(conn, Servers::DEFAULT, TOOL))
                .await?
        {
            return Ok(Vec::new());
        }
        let (settings, _) = self.settings().await?;
        let mode = if request.cause == Cause::Recovery || !on {
            ToolMode::Off
        } else {
            settings.at_start
        };
        let target = Target::Scanned {
            folder: PathBuf::from(&request.launch.settings.data_path),
            system_resources: Some(request.layout.system_resources()),
            scan: Box::new(request.scan),
        };
        let prepared = self.prepare(target.clone()).await?;
        let open = self.open_count(&prepared.folder).await?;
        let (steps, run_mode) = match mode {
            _ if open == 0 && (mode == ToolMode::Off || !prepared.supported()) => return Ok(Vec::new()),
            ToolMode::Off => (Steps::Heal, ToolRunMode::Heal),
            ToolMode::Report => (Steps::HealAndReport, ToolRunMode::Report),
            ToolMode::Fix => (Steps::Fix(None), ToolRunMode::Fix),
        };
        let previous = if mode == ToolMode::Off {
            None
        } else {
            self.inner
                .db
                .read(|conn| stored::last_report(conn, Servers::DEFAULT, TOOL))
                .await?
        };
        let run = self.insert_run(&Asker::Start, run_mode).await?;
        // A report alone writes nothing: SFTP goes on meanwhile.
        let writes = open > 0 || mode == ToolMode::Fix;
        let gate = if writes {
            Some(self.inner.gate.exclusive().await)
        } else {
            None
        };
        let job = self.job(&target, prepared, settings, run, request.cancel, open);
        let finished = blocking(move || work::run(job, &steps)).await?;
        drop(gate);
        let lines = lines::start(&finished, mode, previous.as_ref());
        self.finish(run, &Asker::Start, finished).await?;
        Ok(lines)
    }

    /// At Observer's start, before FXServer's: settles the writes an earlier Observer was
    /// interrupted in, in the folder the launch settings give, then ends the runs it left
    /// unfinished, audited with the files the journal says they changed or put back.
    pub async fn recover(&self, observer_started: Timestamp) -> Result<(), Error> {
        // First, so that a write an interrupted run had under way counts as that run's once it
        // is known to have landed.
        let settled = self.settle().await;
        let ended = self.end_unfinished(observer_started).await;
        settled.and(ended)
    }

    /// Ends the runs started before `observer_started` that never ended, and audits in the same
    /// transaction what each one wrote: an interrupted run never reached the end where its own
    /// entries are written. The journal names the run behind every write that landed.
    async fn end_unfinished(&self, observer_started: Timestamp) -> Result<(), Error> {
        let now = self.inner.clock.now();
        let ended = self
            .inner
            .db
            .write(move |tx| -> Result<usize, StoreError> {
                let unfinished: Vec<ToolRun> = stored::runs(tx, Servers::DEFAULT, TOOL, UNFINISHED_LOOKED)?
                    .into_iter()
                    .filter(|run| run.finished_at.is_none() && run.started_at < observer_started)
                    .collect();
                let ended = stored::end_unfinished_runs(tx, observer_started, now)?;
                if let Some(since) = unfinished.iter().map(|run| run.started_at).min() {
                    let changes = stored::changes(tx, Servers::DEFAULT, TOOL, since, u32::MAX)?;
                    for record in interrupted_records(&unfinished, &changes) {
                        record.write(tx, now)?;
                    }
                }
                Ok(ended)
            })
            .await?;
        if ended > 0 {
            warn!(
                ended,
                "tool runs an earlier Observer left unfinished were ended as failed"
            );
        }
        Ok(())
    }

    /// Settles the pending writes of the launch settings' folder, in a heal run of its own.
    async fn settle(&self) -> Result<(), Error> {
        let target = match self.launch_target().await {
            Ok(target) => target,
            Err(Error::Refused(_)) => return Ok(()),
            Err(error) => return Err(error),
        };
        let folder = target.folder().to_path_buf();
        let identity =
            blocking(move || ServerFolder::open(&folder).map(|folder| folder.id().clone())).await?;
        let Ok(identity) = identity else {
            // A folder that cannot be opened has nothing to settle now; the next run says why.
            return Ok(());
        };
        let pending = self
            .inner
            .db
            .read(move |conn| stored::open_changes(conn, Servers::DEFAULT, TOOL, &identity.id))
            .await?
            .iter()
            .any(|row| row.pending.is_some());
        if !pending {
            return Ok(());
        }
        let _writer = Arc::clone(&self.inner.writer).lock_owned().await;
        let run = self.insert_run(&Asker::Start, ToolRunMode::Heal).await?;
        let root = target.folder().to_string_lossy().into_owned();
        let mut journal = StoreJournal::new(
            self.inner.db.clone(),
            self.inner.clock.clone(),
            root,
            HashMap::new(),
        );
        let refused = self.inner.own_paths.inside(target.folder());
        let folder = target.folder().to_path_buf();
        let reconciled = blocking(move || {
            let folder = ServerFolder::open(&folder)?.with_refused(refused);
            observer_tools::fix::reconcile(&mut journal, &folder, run)
        })
        .await?;
        let mut finished = Finished::default();
        match reconciled {
            Ok(files) => finished.reconciled = files,
            Err(error) => {
                finished.outcome = Some(ToolRunOutcome::Failed);
                finished.summary.problem = Some(error.to_string());
            }
        }
        self.finish(run, &Asker::Start, finished).await?;
        Ok(())
    }

    /// Tells a run under way to end between two files, without waiting: as soon as Observer begins
    /// to stop. A start queued behind a fix or an undo from the panel waits for it, and FXServer's
    /// stop waits for that start, so the run must hear it before.
    pub fn stopping(&self) {
        self.inner.stopping.store(true, Ordering::Relaxed);
    }

    /// Tells a run under way to end between two files, and waits until it has: as Observer stops,
    /// before its database closes.
    pub async fn close(&self) {
        self.stopping();
        drop(self.inner.writer.lock().await);
    }

    /// How many changes are open in `folder`, for the command line's report.
    pub async fn open_changes(&self, folder: PathBuf) -> Result<u32, Error> {
        let identity = blocking(move || ServerFolder::open(&folder).map(|folder| folder.id().clone()))
            .await?
            .map_err(io::Error::other)?;
        self.open_count(&identity).await
    }

    fn require_stopped(&self) -> Result<(), Error> {
        let stopped = self.inner.status.as_ref().is_none_or(|status| {
            matches!(
                status.borrow().state,
                ServerState::Stopped | ServerState::CrashLoop
            )
        });
        if stopped {
            Ok(())
        } else {
            Err(ErrorCode::ServerNotStopped.into())
        }
    }

    /// The folder and cfg of the stored launch settings, with the selected build's system
    /// resources.
    async fn launch_target(&self) -> Result<Target, Error> {
        let (stored, state) = self
            .inner
            .db
            .read(|conn| -> Result<_, StoreError> {
                Ok((
                    launch::get(conn, Servers::DEFAULT)?,
                    stored_artifacts::state(conn, Servers::DEFAULT)?,
                ))
            })
            .await?;
        let stored = stored.ok_or(ErrorCode::LaunchNotConfigured)?;
        let system_resources = state.selected.map(|build| {
            Layout::new(self.inner.artifacts.join(build.to_string()), Flavour::HOST).system_resources()
        });
        Ok(Target::Cfg {
            folder: PathBuf::from(&stored.settings.data_path),
            cfg_file: stored.settings.cfg_file,
            system_resources,
        })
    }

    /// Where an undo the command line names no folder for works. An undo reads no cfg: it puts
    /// entries back whatever starts. Everything, and a run with nothing open, are undone in the
    /// launch settings' folder; a run's entries and named changes in the folder they were made in,
    /// which the launch settings may no longer give, opened only while it still is that folder.
    async fn undo_target(&self, scope: &ToolUndoScope) -> Result<Target, Error> {
        let now = self.inner.clock.now();
        let open = self
            .inner
            .db
            .read(move |conn| stored::changes(conn, Servers::DEFAULT, TOOL, now, 0))
            .await?
            .open;
        let rows: Vec<&ToolChangeRow> = match scope {
            ToolUndoScope::All => Vec::new(),
            ToolUndoScope::Run(run) => open
                .iter()
                .filter(|row| row.removals.iter().any(|removal| removal.run == *run))
                .collect(),
            ToolUndoScope::Changes(ids) => ids
                .iter()
                .map(|id| open.iter().find(|row| row.id == *id).ok_or(ErrorCode::NotFound))
                .collect::<Result<_, _>>()?,
        };
        let Some(first) = rows.first() else {
            return Ok(Target::AllResources {
                folder: self.launch_target().await?.folder().to_path_buf(),
            });
        };
        if rows.iter().any(|row| row.folder_id != first.folder_id) {
            // Each folder is undone on its own, as the Tools page offers it.
            return Err(ErrorCode::InvalidRequest.into());
        }
        let launch = match self.launch_target().await {
            Ok(target) => Some(target.folder().to_path_buf()),
            Err(Error::Refused(_)) => None,
            Err(error) => return Err(error),
        };
        let paths: BTreeSet<PathBuf> = rows.iter().map(|row| PathBuf::from(&row.folder_path)).collect();
        for folder in launch.into_iter().chain(paths) {
            if identity(&folder).await?.as_deref() == Some(first.folder_id.as_str()) {
                return Ok(Target::AllResources { folder });
            }
        }
        Err(ApiError::new(ErrorCode::NotFound)
            .with("folder", first.folder_path.clone())
            .into())
    }

    async fn prepare(&self, target: Target) -> Result<Prepared, Error> {
        let own = self.inner.own_paths.clone();
        Ok(blocking(move || work::prepare(&target, &own)).await??)
    }

    async fn open_count(&self, folder: &FolderId) -> Result<u32, Error> {
        let id = folder.id.clone();
        let rows = self
            .inner
            .db
            .read(move |conn| stored::open_changes(conn, Servers::DEFAULT, TOOL, &id))
            .await?;
        Ok(u32::try_from(rows.len()).unwrap_or(u32::MAX))
    }

    /// What an undo of `scope` puts back in `folder`: changes named by id must be open there.
    async fn selection(&self, folder: &FolderId, scope: ToolUndoScope) -> Result<Selection, Error> {
        let ids = match scope {
            ToolUndoScope::All => return Ok(Selection::All),
            ToolUndoScope::Run(run) => return Ok(Selection::Run(run)),
            ToolUndoScope::Changes(ids) => ids,
        };
        let folder = folder.id.clone();
        let rows = self
            .inner
            .db
            .read(move |conn| stored::open_changes(conn, Servers::DEFAULT, TOOL, &folder))
            .await?;
        let paths = ids
            .iter()
            .map(|id| {
                rows.iter()
                    .find(|row| row.id == *id)
                    .map(|row| row.path.clone())
                    .ok_or(ErrorCode::NotFound)
            })
            .collect::<Result<Vec<String>, ErrorCode>>()?;
        Ok(Selection::Paths(paths))
    }

    async fn insert_run(&self, asker: &Asker, mode: ToolRunMode) -> Result<i64, Error> {
        let trigger = asker.trigger();
        let actor = asker.actor();
        let started_at = self.inner.clock.now();
        Ok(self
            .inner
            .db
            .write(move |tx| {
                stored::insert_run(
                    tx,
                    &NewRun {
                        server_id: Servers::DEFAULT,
                        tool: TOOL,
                        trigger,
                        actor: actor.as_deref(),
                        mode,
                        started_at,
                    },
                )
            })
            .await?)
    }

    fn job(
        &self,
        target: &Target,
        prepared: Prepared,
        settings: ToolSettings,
        run: i64,
        cancel: Arc<AtomicBool>,
        open: u32,
    ) -> Job {
        let root = target.folder().to_string_lossy().into_owned();
        Job {
            folder: target.folder().to_path_buf(),
            journal: StoreJournal::new(
                self.inner.db.clone(),
                self.inner.clock.clone(),
                root,
                HashMap::new(),
            ),
            prepared,
            settings,
            backups: self.inner.backups.clone(),
            run,
            cancel,
            open,
        }
    }

    /// Records how a run ended, with its audit entries in the same transaction, keeps only the
    /// backups still needed once it wrote, and reads the run back.
    async fn finish(&self, run: i64, asker: &Asker, finished: Finished) -> Result<RunResult, Error> {
        let now = self.inner.clock.now();
        let trigger = asker.trigger().as_str();
        let mut records = Vec::new();
        let changed = finished.changed_paths();
        if !changed.is_empty() {
            records.push(asker.record(AuditAction::ToolFilesChanged).details(json!({
                "run": run,
                "trigger": trigger,
                "files": capped(&changed),
                "count": changed.len(),
            })));
        }
        for (reason, restored) in [
            ("heal", finished.healed_paths()),
            ("undo", finished.undone_paths()),
        ] {
            if !restored.is_empty() {
                records.push(asker.record(AuditAction::ToolFilesRestored).details(json!({
                    "run": run,
                    "trigger": trigger,
                    "reason": reason,
                    "files": capped(&restored),
                    "count": restored.len(),
                })));
            }
        }
        let wrote = finished.wrote();
        let outcome = finished.outcome();
        let Finished {
            summary,
            plan,
            report,
            reconciled,
            healed,
            applied,
            undone,
            ..
        } = finished;
        self.inner
            .db
            .write(move |tx| -> Result<(), StoreError> {
                stored::finish_run(
                    tx,
                    run,
                    &FinishedRun {
                        at: now,
                        outcome,
                        summary: &summary,
                        plan,
                        report: report.as_ref(),
                    },
                )?;
                for record in records {
                    record.write(tx, now)?;
                }
                Ok(())
            })
            .await?;
        if wrote && let Err(error) = self.retain().await {
            warn!(%error, "the tools' backups no longer needed could not be deleted");
        }
        let run = self
            .inner
            .db
            .read(move |conn| stored::run(conn, run))
            .await?
            .ok_or(ErrorCode::NotFound)?;
        Ok(RunResult {
            run,
            reconciled,
            healed,
            applied,
            undone,
        })
    }

    /// Deletes the backups no open change needs and no change closed within 30 days, and the
    /// records of changes closed before. Under the writer lock: a backup is kept before the write
    /// that needs it is recorded.
    async fn retain(&self) -> Result<(), Error> {
        let cutoff = self.inner.clock.now() - CLOSED_KEPT;
        let keep: HashSet<[u8; 32]> = self
            .inner
            .db
            .read(move |conn| stored::referenced_backups(conn, cutoff))
            .await?
            .into_iter()
            .collect();
        let backups = self.inner.backups.clone();
        blocking(move || backups.retain(&keep))
            .await?
            .map_err(io::Error::other)?;
        self.inner
            .db
            .write(move |tx| stored::forget_closed_changes(tx, cutoff))
            .await?;
        Ok(())
    }
}

/// Settles the work before each spawn on the [`Tools`] it is bound to once they exist: the
/// supervisor is made first, and the tools need its status.
#[derive(Clone, Default)]
pub struct ToolsHook {
    tools: Arc<OnceLock<Tools>>,
    /// The switches, whose tools' switch is read before each spawn; without them the tools run.
    features: Option<Features>,
}

impl ToolsHook {
    /// A hook that runs the tools only while their feature does.
    pub fn switched(features: Features) -> Self {
        Self {
            tools: Arc::default(),
            features: Some(features),
        }
    }

    /// Binds the tools; a second call changes nothing.
    pub fn bind(&self, tools: Tools) {
        let _ = self.tools.set(tools);
    }
}

impl fmt::Debug for ToolsHook {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ToolsHook")
            .field("bound", &self.tools.get().is_some())
            .field("switched", &self.features.is_some())
            .finish()
    }
}

impl PreLaunch for ToolsHook {
    fn run(&self, request: PreLaunchRequest) -> PreLaunchFuture<'_> {
        Box::pin(async move {
            let on = self
                .features
                .as_ref()
                .is_none_or(|features| features.runs(Feature::Tools));
            match self.tools.get() {
                Some(tools) => tools.before_start(request, on).await,
                None => Ok(Vec::new()),
            }
        })
    }
}

/// Runs a run as a task of its own, so that a request dropped midway still lets it end and be
/// recorded.
async fn detached<T: Send + 'static>(
    work: impl Future<Output = Result<T, Error>> + Send + 'static,
) -> Result<T, Error> {
    tokio::spawn(work).await.map_err(io::Error::other)?
}

async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Result<T, Error> {
    Ok(tokio::task::spawn_blocking(work)
        .await
        .map_err(io::Error::other)?)
}

/// The identity of the server folder at `path`, which the records go by, or `None` when it cannot
/// be opened.
async fn identity(path: &Path) -> Result<Option<String>, Error> {
    let path = path.to_path_buf();
    blocking(move || {
        ServerFolder::open(&path)
            .ok()
            .map(|folder| folder.id().id.clone())
    })
    .await
}

/// The audit entries of runs an earlier Observer left unfinished, from the journal: per run, the
/// files whose latest write it made, as changed when it took entries out of them and as put back
/// otherwise, and the files it wrote the original back to. Written by Observer at its next start,
/// naming who asked for the run.
fn interrupted_records(runs: &[ToolRun], changes: &ChangeList) -> Vec<Record> {
    let mut records = Vec::new();
    for run in runs {
        let mut edited = Vec::new();
        let mut restored = Vec::new();
        for row in changes.open.iter().chain(&changes.closed) {
            let by_run = row.changed_run == Some(run.id);
            let took_out = row.removals.iter().any(|removal| removal.run == run.id);
            let wrote_original = match row.close_reason {
                Some(ToolCloseReason::Healed | ToolCloseReason::Undone) => row.closed_run == Some(run.id),
                // The original the run wrote back, which a later run found and closed.
                Some(ToolCloseReason::Restored) => by_run && row.removals.is_empty(),
                _ => false,
            };
            if row.state == ToolChangeState::Applied && by_run && took_out {
                edited.push(row.path.clone());
            } else if (row.state == ToolChangeState::Applied && by_run) || wrote_original {
                restored.push(row.path.clone());
            }
        }
        let reason = if run.mode == ToolRunMode::Undo {
            "undo"
        } else {
            "heal"
        };
        let entries = [
            (AuditAction::ToolFilesChanged, None, edited),
            (AuditAction::ToolFilesRestored, Some(reason), restored),
        ];
        for (action, reason, files) in entries {
            if files.is_empty() {
                continue;
            }
            let mut details = json!({
                "run": run.id,
                "trigger": run.trigger.as_str(),
                "interrupted": true,
                "files": capped(&files),
                "count": files.len(),
            });
            if let Some(reason) = reason {
                details["reason"] = json!(reason);
            }
            if let Some(actor) = &run.actor {
                details["actor"] = json!(actor);
            }
            records.push(
                Record::operator(ActorKind::System, action)
                    .target(TOOL.as_str())
                    .details(details),
            );
        }
    }
    records
}

fn not_available() -> Error {
    ApiError::new(ErrorCode::ToolNotAvailable)
        .with("game", serde_json::to_value(Game::Rdr3).unwrap_or_default())
        .into()
}

/// Refuses an exclusion list no resource could match: empty or overlong names, too many of them.
fn check_settings(settings: &ToolSettings) -> Result<(), Error> {
    let fits = settings.excluded.len() <= MAX_EXCLUDED
        && settings
            .excluded
            .iter()
            .all(|name| !name.is_empty() && name.len() <= MAX_RESOURCE_NAME && !name.contains(['/', '\\']));
    if fits {
        Ok(())
    } else {
        Err(ErrorCode::InvalidRequest.into())
    }
}

/// Each setting that differs, with its value before and after.
fn setting_changes(before: &ToolSettings, after: &ToolSettings) -> Value {
    let fields = |settings: &ToolSettings| match serde_json::to_value(settings) {
        Ok(Value::Object(fields)) => fields,
        _ => serde_json::Map::new(),
    };
    let old = fields(before);
    let changed = fields(after)
        .into_iter()
        .filter_map(|(name, value)| {
            let was = old.get(&name).cloned().unwrap_or(Value::Null);
            (was != value).then(|| (name, json!({ "before": was, "after": value })))
        })
        .collect();
    Value::Object(changed)
}

fn capped(paths: &[String]) -> &[String] {
    &paths[..paths.len().min(AUDITED_FILES)]
}
