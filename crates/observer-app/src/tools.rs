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
//! are audited; reports are not. The originals of changed files are kept under
//! `data/tools/backups` while a change is open and for 30 days after it closed.

mod journal;
mod lines;
#[cfg(test)]
mod tests;
mod work;

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use jiff::{SignedDuration, Timestamp};
use observer_domain::audit::{ActorKind, AuditAction};
use observer_domain::error::{ApiError, ErrorCode};
use observer_domain::server::{ServerState, ServerStatus};
use observer_domain::server_data::Game;
use observer_domain::tools::{
    ToolKind, ToolMode, ToolRun, ToolRunMode, ToolRunOutcome, ToolSettings, ToolSettingsChange, ToolTrigger,
    ToolUndoRequest, ToolUndoScope, ToolView, ToolsView,
};
use observer_fxserver::layout::{Flavour, Layout};
use observer_fxserver::pre_launch::{PreLaunch, PreLaunchFuture, PreLaunchRequest, PreLaunchResult};
use observer_fxserver::supervisor::Cause;
use observer_store::artifacts as stored_artifacts;
use observer_store::tools::{self as stored, FinishedRun, NewRun};
use observer_store::{Database, StoreError, launch};
use observer_tools::fix::{Backups, FileResult, FolderId, Selection};
use serde_json::{Value, json};
use tokio::sync::{Mutex, watch};
use tracing::warn;

use crate::audit::Record;
use crate::files::CommitGate;
use crate::{Caller, Clock, Error, OwnPaths, Servers};
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
        Ok(ToolsView {
            tools: vec![ToolView {
                tool: TOOL,
                settings,
                version,
                last_run,
                last_report,
                open: changes.open.iter().map(stored::ToolChangeRow::view).collect(),
                closed: changes.closed.iter().map(stored::ToolChangeRow::view).collect(),
                backup_bytes,
            }],
        })
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

    /// An undo from the command line, in the folder the launch settings give. The caller holds
    /// Observer's lock.
    pub async fn undo_from_cli(&self, scope: ToolUndoScope) -> Result<RunResult, Error> {
        self.write(Asker::Cli, None, None, Writing::Undo(scope)).await
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
            // An undo reads no cfg: it puts entries back whatever starts.
            (None, Writing::Undo(_)) => Target::AllResources {
                folder: self.launch_target().await?.folder().to_path_buf(),
            },
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
    async fn before_start(&self, request: PreLaunchRequest) -> PreLaunchResult {
        // Awaited whatever the cancel flag says: FXServer must not start while another run writes.
        let _writer = Arc::clone(&self.inner.writer).lock_owned().await;
        if request.cancel.load(Ordering::Relaxed) {
            return Ok(Vec::new());
        }
        let (settings, _) = self.settings().await?;
        let mode = if request.cause == Cause::Recovery {
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

    /// At Observer's start, before FXServer's: ends the runs an earlier Observer left unfinished,
    /// and settles the writes it was interrupted in, in the folder the launch settings give.
    pub async fn recover(&self, observer_started: Timestamp) -> Result<(), Error> {
        let now = self.inner.clock.now();
        let ended = self
            .inner
            .db
            .write(move |tx| stored::end_unfinished_runs(tx, observer_started, now))
            .await?;
        if ended > 0 {
            warn!(
                ended,
                "tool runs an earlier Observer left unfinished were ended as failed"
            );
        }
        let target = match self.launch_target().await {
            Ok(target) => target,
            Err(Error::Refused(_)) => return Ok(()),
            Err(error) => return Err(error),
        };
        let folder = target.folder().to_path_buf();
        let identity = blocking(move || {
            observer_tools::fix::ServerFolder::open(&folder).map(|folder| folder.id().clone())
        })
        .await?;
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
            let folder = observer_tools::fix::ServerFolder::open(&folder)?.with_refused(refused);
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

    /// Tells a run under way to end between two files, and waits until it has: as Observer stops,
    /// before its database closes.
    pub async fn close(&self) {
        self.inner.stopping.store(true, Ordering::Relaxed);
        drop(self.inner.writer.lock().await);
    }

    /// How many changes are open in `folder`, for the command line's report.
    pub async fn open_changes(&self, folder: PathBuf) -> Result<u32, Error> {
        let identity = blocking(move || {
            observer_tools::fix::ServerFolder::open(&folder).map(|folder| folder.id().clone())
        })
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
pub struct ToolsHook(Arc<OnceLock<Tools>>);

impl ToolsHook {
    /// Binds the tools; a second call changes nothing.
    pub fn bind(&self, tools: Tools) {
        let _ = self.0.set(tools);
    }
}

impl fmt::Debug for ToolsHook {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ToolsHook")
            .field("bound", &self.0.get().is_some())
            .finish()
    }
}

impl PreLaunch for ToolsHook {
    fn run(&self, request: PreLaunchRequest) -> PreLaunchFuture<'_> {
        Box::pin(async move {
            match self.0.get() {
                Some(tools) => tools.before_start(request).await,
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
