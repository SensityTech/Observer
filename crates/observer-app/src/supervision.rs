//! FXServer as Observer runs it: the supervisor, started from the stored launch settings and the
//! selected build, the status the panel shows of it, the console it fills, the commands operators
//! send it, the console log files, and the server setup's checks of a folder and its cfg.

mod setup;

use std::collections::{HashMap, VecDeque};
use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use jiff::{SignedDuration, Timestamp};
use observer_domain::audit::{ActorKind, AuditAction, Outcome};
use observer_domain::error::{ApiError, ErrorCode};
use observer_domain::launch::{Platform, StoredLaunch};
use observer_domain::permissions::Permissions;
use observer_domain::schedule::RestartSource;
use observer_domain::server::{
    CommandAck, CommandRefusal, ConsoleLine, ConsoleSnapshot, Delivery, HangCause, HealthProblem,
    LINE_MASKED, LINE_NO_EOL, LINE_TRUNCATED, LaunchReadiness, LineKind, LogFile, PlayerCount, ServerState,
    ServerStatus, SlowResource, StartCommand, StartProgress, StartedResource, StateCause,
};
use observer_fxserver::bridge::BridgeEvent;
use observer_fxserver::console::{self as fx_console, ConsoleHub, ConsoleLog, LineFlags};
use observer_fxserver::health;
use observer_fxserver::launch::ServerLaunch;
use observer_fxserver::stdin::{self, StdinError, UserCommand};
use observer_fxserver::supervisor::{
    self as fx, Cause, ControlError, EventKind, Settings, State, Supervisor,
};
use observer_store::Database;
use observer_store::artifacts::{self as stored_artifacts, ArtifactState};
use observer_store::launch;
use serde_json::json;
use tokio::sync::broadcast::error::{RecvError, TryRecvError};
use tokio::sync::{Mutex as AsyncMutex, Semaphore, broadcast, oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::artifacts::{Promotion, promote_pending};
use crate::audit::Record;
use crate::{Accounts, Caller, Clock, Error, Servers};

/// Who the console and the bridge's events name for what Observer does by itself.
const OBSERVER: &str = "observer";
/// Who they name for someone at the host's own window, who signs in to nothing.
const LOCAL: &str = "local";
/// What a start is given when someone says it needs longer.
#[expect(clippy::cast_possible_truncation, reason = "a quarter of an hour")]
const EXTENSION_MINUTES: u32 = (fx::START_EXTENSION.as_secs() / 60) as u32;
/// Who the console names for the schedule's notes.
const SCHEDULE: &str = "schedule";
/// Commands one user may send within a window.
const COMMANDS_PER_WINDOW: usize = 10;
const COMMAND_WINDOW: SignedDuration = SignedDuration::from_secs(10);
/// The setup's folder reads that may run at once; the next ones wait their turn.
const FOLDER_READS: usize = 3;
/// The most lines a console snapshot holds, which is what the panel keeps.
const SNAPSHOT_LINES: usize = 5000;
/// New lines go out together, at most this often, rather than one message each.
const BATCH_EVERY: Duration = Duration::from_millis(50);
const BATCH_LINES: usize = 1000;

/// What [`Supervision::start`] works with.
pub struct Setup<'a> {
    pub db: Database,
    pub clock: Clock,
    pub accounts: Accounts,
    pub servers: Servers,
    /// Observer's data folder: FXServer's crash dumps and pid file go there, and no server folder
    /// may lie inside it.
    pub data_dir: PathBuf,
    /// Where the panel listens, whose port FXServer must leave alone.
    pub panel: SocketAddr,
    /// Where the builds are installed, one folder per build number.
    pub artifacts: PathBuf,
    /// The folder `log` writes to, which the log routes list.
    pub console_logs: PathBuf,
    pub log: Option<&'a ConsoleLog>,
}

/// The default server under Observer's supervision. Clones share it.
#[derive(Clone)]
pub struct Supervision {
    inner: Arc<Inner>,
}

struct Inner {
    db: Database,
    clock: Clock,
    data_dir: PathBuf,
    panel: SocketAddr,
    artifacts: PathBuf,
    accounts: Accounts,
    servers: Servers,
    supervisor: Supervisor,
    launches: Launches,
    console: Arc<ConsoleHub>,
    console_logs: PathBuf,
    status: watch::Receiver<ServerStatus>,
    /// When each user sent their latest commands, within the window.
    commands: Mutex<HashMap<i64, VecDeque<Timestamp>>>,
    /// The setup's reads of the host's folders under way: each holds a blocking thread, which a
    /// drive that stops answering keeps.
    folder_reads: Semaphore,
    /// An install of a release that waits for the next scheduled restart: told when it comes, in
    /// the place of the restart.
    takeover: Mutex<Option<oneshot::Sender<()>>>,
}

/// The supervisor's task and the one that follows it for the panel. Once the supervisor has
/// stopped FXServer and ended, [`Running::close`] ends the other.
pub struct Running {
    supervisor: Option<JoinHandle<()>>,
    follower: JoinHandle<()>,
    stop: CancellationToken,
}

impl Running {
    /// Resolves once the supervisor has ended. It can be waited on again after a timeout or a
    /// `select!` dropped it.
    pub async fn stopped(&mut self) {
        if let Some(task) = &mut self.supervisor {
            if let Err(error) = task.await {
                warn!(%error, "the supervisor task failed");
            }
            self.supervisor = None;
        }
    }

    pub async fn close(mut self) {
        self.stopped().await;
        self.stop.cancel();
        let _ = self.follower.await;
    }
}

/// Hands the supervisor the stored launch settings and the selected build each time either
/// changes, so that every spawn, a restart after a crash included, launches the newest. Whoever
/// read them before a newer save cannot hand them over after it: both versions only go up.
#[derive(Clone)]
struct Launches {
    supervisor: Supervisor,
    artifacts: PathBuf,
    /// The versions of the launch settings and of the artifact state last given.
    given: Arc<AsyncMutex<Option<(i64, i64)>>>,
}

impl Launches {
    async fn give(&self, stored: &StoredLaunch, state: &ArtifactState) {
        let Some(build) = state.selected else {
            return;
        };
        let versions = (stored.version, state.version);
        let mut given = self.given.lock().await;
        if let Some(last) = *given {
            let outdated = versions.0 < last.0 || versions.1 < last.1;
            if outdated || versions == last {
                return;
            }
        }
        let launch = ServerLaunch {
            build: self.artifacts.join(build.to_string()),
            settings: stored.settings.clone(),
        };
        self.supervisor.configure(launch).await;
        *given = Some(versions);
    }
}

/// The stored launch settings and the artifact state, read together.
async fn read_launch(db: &Database) -> Result<(Option<StoredLaunch>, ArtifactState), Error> {
    db.read(|conn| {
        Ok::<_, Error>((
            launch::get(conn, Servers::DEFAULT)?,
            stored_artifacts::state(conn, Servers::DEFAULT)?,
        ))
    })
    .await
}

/// A restart the schedule makes.
pub(crate) struct ScheduledRestart {
    pub at: Timestamp,
    pub source: RestartSource,
    /// `observer` for the schedule's own times, or whoever planned the one-off.
    pub by: String,
    pub planned_by: Option<String>,
    /// The time as the schedule's zone reads it, for the console: `04:00 Europe/Paris`.
    pub local: String,
}

/// Why the schedule's restart did not happen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScheduledRefusal {
    /// This host refuses the stored launch settings: nothing was stopped.
    LaunchInvalid,
    /// FXServer was stopping, or no longer running.
    Busy,
    ShuttingDown,
}

/// Console lines lost before they reached the subscriber: it starts over from the last it has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinesMissed;

/// The console as one subscriber follows it after its snapshot.
pub struct ConsoleLines {
    live: broadcast::Receiver<Arc<fx_console::ConsoleLine>>,
    last_batch: Option<Instant>,
}

impl ConsoleLines {
    /// The lines that came since the last batch, as soon as there is one, or `None` once the
    /// console is gone.
    pub async fn next(&mut self) -> Option<Result<Vec<ConsoleLine>, LinesMissed>> {
        if let Some(last) = self.last_batch {
            tokio::time::sleep_until(last + BATCH_EVERY).await;
        }
        let first = match self.live.recv().await {
            Ok(line) => line,
            Err(RecvError::Lagged(_)) => return Some(Err(LinesMissed)),
            Err(RecvError::Closed) => return None,
        };
        let mut lines = vec![wire(&first)];
        while lines.len() < BATCH_LINES {
            match self.live.try_recv() {
                Ok(line) => lines.push(wire(&line)),
                Err(TryRecvError::Lagged(_)) => return Some(Err(LinesMissed)),
                Err(TryRecvError::Empty | TryRecvError::Closed) => break,
            }
        }
        self.last_batch = Some(Instant::now());
        Some(Ok(lines))
    }
}

impl Supervision {
    /// Starts the supervisor with FXServer stopped; [`Supervision::boot`] then starts it if the
    /// owner asked for that. `tune` adjusts the supervisor's timings, for tests.
    pub async fn start(setup: Setup<'_>, tune: impl FnOnce(&mut Settings)) -> Result<(Self, Running), Error> {
        let console = Arc::new(ConsoleHub::new(setup.log));
        let mut settings = Settings::new(None, &setup.data_dir);
        tune(&mut settings);
        let (supervisor, supervisor_task) = Supervisor::spawn(settings, Arc::clone(&console));
        let launches = Launches {
            supervisor: supervisor.clone(),
            artifacts: setup.artifacts.clone(),
            given: Arc::default(),
        };

        let (stored, state) = read_launch(&setup.db).await?;
        if let Some(stored) = &stored {
            launches.give(stored, &state).await;
        }
        let readiness = readiness(stored.as_ref(), &state);
        let first = server_status(&supervisor.status().borrow(), None, None, readiness);
        let (status_tx, status) = watch::channel(first);
        let stop = CancellationToken::new();
        let follower = tokio::spawn(follow(
            launches.clone(),
            setup.servers.clone(),
            setup.db.clone(),
            setup.clock.clone(),
            Arc::clone(&console),
            status_tx,
            readiness,
            stop.clone(),
        ));

        let supervision = Self {
            inner: Arc::new(Inner {
                db: setup.db,
                clock: setup.clock,
                data_dir: setup.data_dir,
                panel: setup.panel,
                artifacts: setup.artifacts,
                accounts: setup.accounts,
                servers: setup.servers,
                supervisor,
                launches,
                console,
                console_logs: setup.console_logs,
                status,
                commands: Mutex::default(),
                folder_reads: Semaphore::new(FOLDER_READS),
                takeover: Mutex::default(),
            }),
        };
        let running = Running {
            supervisor: Some(supervisor_task),
            follower,
            stop,
        };
        Ok((supervision, running))
    }

    /// Starts FXServer as Observer starts, when the stored settings ask for it. Settings this host
    /// refuses, or no build selected, leave it stopped, with a note in the console; the status
    /// says why.
    pub async fn boot(&self) -> Result<(), Error> {
        self.boot_with(false).await
    }

    /// Starts FXServer as the new release of Observer starts, when the previous one stopped it to
    /// make way: whether or not the settings ask for a start with Observer.
    pub async fn boot_after_update(&self) -> Result<(), Error> {
        self.boot_with(true).await
    }

    async fn boot_with(&self, after_update: bool) -> Result<(), Error> {
        let (stored, state) = read_launch(&self.inner.db).await?;
        let Some(stored) = stored else {
            info!("FXServer has no launch settings yet: the owner saves them in the server setup");
            return Ok(());
        };
        if !stored.settings.auto_start && !after_update {
            return Ok(());
        }
        if !stored.settings.issues(Platform::host()).is_empty() {
            warn!("FXServer was not started: its launch settings are refused on this host");
            self.note(
                OBSERVER,
                "FXServer was not started: this host refuses its launch settings",
            );
            return Ok(());
        }
        if state.selected.is_none() {
            warn!("FXServer was not started: no build is selected");
            self.note(
                OBSERVER,
                "FXServer was not started: no build is selected; install or import one on the Artifacts page",
            );
            return Ok(());
        }
        self.inner.launches.give(&stored, &state).await;
        let started = self.inner.supervisor.start(OBSERVER).await;
        let details = if after_update {
            json!({ "automatic": true, "afterUpdate": true })
        } else {
            json!({ "automatic": true })
        };
        let record = Record::operator(ActorKind::System, AuditAction::ServerStarted).details(details);
        self.record_control(record, &started).await?;
        if let Err(error) = started {
            warn!(%error, "FXServer did not start with Observer");
        }
        Ok(())
    }

    /// Stops FXServer for good as Observer stops. `shortened` for a closing console, which leaves
    /// a few seconds only; it may follow a first call that was not.
    pub async fn shutdown(&self, shortened: bool) {
        self.inner.supervisor.shutdown(shortened).await;
    }

    /// Stops FXServer for good as Observer restarts into a new release in the place of a
    /// scheduled restart: resources hear `scheduled`, as they would have for the restart.
    pub async fn shutdown_scheduled(&self) {
        self.inner.supervisor.shutdown_scheduled().await;
    }

    /// Lets the next scheduled restart go to an install of a release instead: `takeover` hears of
    /// it, and FXServer keeps running until Observer stops it. A second call replaces the first.
    pub(crate) fn hand_next_restart(&self, takeover: oneshot::Sender<()>) {
        *self.inner.takeover.lock().unwrap_or_else(PoisonError::into_inner) = Some(takeover);
    }

    pub(crate) fn keep_next_restart(&self) {
        self.inner
            .takeover
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
    }

    /// Starts FXServer with the stored launch settings, as they are now.
    pub async fn start_server(&self, caller: &Caller) -> Result<(), Error> {
        self.start_by(&caller.username, Record::by(caller, AuditAction::ServerStarted))
            .await
    }

    pub async fn stop_server(&self, caller: &Caller) -> Result<(), Error> {
        self.stop_by(&caller.username, Record::by(caller, AuditAction::ServerStopped))
            .await
    }

    /// Gives the start under way more time before it is taken for stuck: a first start, or one
    /// with a resource that takes longer than the hang delay, without changing the delay itself.
    pub async fn extend_start(&self, caller: &Caller) -> Result<(), Error> {
        let extended = self.inner.supervisor.extend_start(&caller.username).await;
        let record = Record::by(caller, AuditAction::ServerStartExtended)
            .details(json!({ "minutes": EXTENSION_MINUTES }));
        self.record_control(record, &extended).await?;
        extended.map_err(control_error)
    }

    /// Stops FXServer gracefully and starts it again with the stored settings, which must be fit
    /// to start before anything stops.
    pub async fn restart_server(&self, caller: &Caller) -> Result<(), Error> {
        self.restart_by(&caller.username, Record::by(caller, AuditAction::ServerRestarted))
            .await
    }

    /// [`Supervision::start_server`] for someone at the host's own window.
    pub async fn start_local(&self) -> Result<(), Error> {
        self.start_by(LOCAL, local(AuditAction::ServerStarted)).await
    }

    pub async fn stop_local(&self) -> Result<(), Error> {
        self.stop_by(LOCAL, local(AuditAction::ServerStopped)).await
    }

    pub async fn restart_local(&self) -> Result<(), Error> {
        self.restart_by(LOCAL, local(AuditAction::ServerRestarted)).await
    }

    async fn start_by(&self, by: &str, record: Record) -> Result<(), Error> {
        self.hand_over_launch().await?;
        let started = self.inner.supervisor.start(by).await;
        self.record_control(record, &started).await?;
        started.map_err(control_error)
    }

    async fn stop_by(&self, by: &str, record: Record) -> Result<(), Error> {
        let stopped = self.inner.supervisor.stop(by).await;
        self.record_control(record, &stopped).await?;
        stopped.map_err(control_error)
    }

    async fn restart_by(&self, by: &str, record: Record) -> Result<(), Error> {
        self.hand_over_launch().await?;
        let restarted = self.inner.supervisor.restart(by).await;
        self.record_control(record, &restarted).await?;
        restarted.map_err(control_error)
    }

    /// The schedule's restart of a running server. The stored settings must be fit to start
    /// before anything stops: otherwise the server keeps running, with a note in the console. A
    /// build waiting for this restart becomes the selected one first.
    pub(crate) async fn restart_on_schedule(
        &self,
        restart: &ScheduledRestart,
    ) -> Result<(), ScheduledRefusal> {
        let fit = self
            .inner
            .servers
            .launch_settings(Servers::DEFAULT)
            .await
            .ok()
            .flatten()
            .is_some_and(|stored| stored.settings.issues(Platform::host()).is_empty());
        if fit {
            self.promote_pending().await;
        }
        // A build waiting for this restart is selected all the same: the new release starts it.
        let takeover = self
            .inner
            .takeover
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(takeover) = takeover
            && takeover.send(()).is_ok()
        {
            self.note(SCHEDULE, "Observer restarts into its new release at this restart");
            return Ok(());
        }
        if let Err(error) = self.hand_over_launch().await {
            warn!(%error, "the scheduled restart did not happen: the launch settings cannot start FXServer");
            self.note(
                SCHEDULE,
                "the scheduled restart did not happen: this host refuses the launch settings",
            );
            return Err(ScheduledRefusal::LaunchInvalid);
        }
        match self.inner.supervisor.restart_scheduled(&restart.by).await {
            Ok(()) => {}
            Err(ControlError::ShuttingDown) => return Err(ScheduledRefusal::ShuttingDown),
            Err(_) => return Err(ScheduledRefusal::Busy),
        }
        self.note(
            SCHEDULE,
            &match &restart.planned_by {
                Some(planner) => format!("restarting as {planner} planned"),
                None => format!("restarting on schedule ({})", restart.local),
            },
        );
        let mut scheduled = json!({ "at": restart.at, "source": restart.source });
        if let Some(planner) = &restart.planned_by {
            scheduled["plannedBy"] = json!(planner);
        }
        let record = Record::operator(ActorKind::System, AuditAction::ServerRestarted)
            .target(Servers::DEFAULT)
            .details(json!({ "scheduled": scheduled }));
        if let Err(error) = self.audit(record).await {
            warn!(%error, "cannot audit the scheduled restart");
        }
        Ok(())
    }

    /// Makes a build waiting for the scheduled restart the selected one, or drops it when it is
    /// gone or broken, then lets everyone who follows the selection know.
    async fn promote_pending(&self) {
        let root = self.inner.artifacts.clone();
        let now = self.inner.clock.now();
        let promoted = self
            .inner
            .db
            .write(move |tx| promote_pending(tx, &root, now))
            .await;
        match promoted {
            Ok(None) => {}
            Ok(Some(promotion)) => {
                let text = match promotion {
                    Promotion::Applied { build, .. } => format!("build {build} is applied at this restart"),
                    Promotion::Dropped { build } => {
                        format!(
                            "build {build} was not applied at this restart: it is no longer installed or valid"
                        )
                    }
                };
                self.note(SCHEDULE, &text);
                self.inner.servers.launch_changed();
            }
            Err(error) => warn!(%error, "cannot apply the pending build at the scheduled restart"),
        }
    }

    /// Starts the last healthy build after a new one failed its probation: at once when FXServer
    /// is down or waiting to come back, or by a restart when it came back on the failed build first.
    pub(crate) async fn start_after_rollback(&self) -> Result<(), Error> {
        self.hand_over_launch().await?;
        let mut result = self.inner.supervisor.start(OBSERVER).await;
        if result == Err(ControlError::AlreadyRunning) {
            result = self.inner.supervisor.restart(OBSERVER).await;
        }
        let record = Record::operator(ActorKind::System, AuditAction::ServerStarted)
            .details(json!({ "rollback": true }));
        self.record_control(record, &result).await?;
        result.map_err(control_error)
    }

    /// Raises an event of the integration contract on a running server whose bridge reported.
    pub(crate) async fn announce(&self, event: BridgeEvent) -> Result<(), ControlError> {
        self.inner.supervisor.announce(event).await
    }

    /// Sends an operator's line to FXServer's console. The line is read as FXServer will read it
    /// first: a command that stops the server or its bridge, or runs what Observer cannot read,
    /// also takes the right to control the server, checked as `method path` for the audit log.
    /// The command is audited before it is written. The answer says whether a secret was masked
    /// in it, which the panel then keeps out of its history.
    pub async fn command(
        &self,
        caller: &Caller,
        line: &str,
        method: &str,
        path: &str,
    ) -> Result<CommandAck, Error> {
        let inner = &self.inner;
        let command = match UserCommand::parse(line) {
            Ok(command) => command,
            Err(refusal) => {
                let refusal = command_refusal(refusal);
                if refusal == CommandRefusal::Reserved {
                    let record = Record::by(caller, AuditAction::ConsoleCommandSent)
                        .target(Servers::DEFAULT)
                        .outcome(Outcome::Denied)
                        .details(json!({ "command": masked(line), "refusal": refusal }));
                    inner
                        .accounts
                        .record_denial(caller, "console_command_sent".to_owned(), record)
                        .await?;
                }
                return Err(ApiError::new(ErrorCode::CommandRefused)
                    .with("reason", serde_json::to_value(refusal).unwrap_or_default())
                    .into());
            }
        };
        let bridge = inner.supervisor.status().borrow().bridge.clone();
        if command.needs_server_control(bridge.as_deref()) {
            inner
                .accounts
                .authorize(caller, Permissions::SERVER_CONTROL, method, path)
                .await?;
        }
        self.throttle(caller)?;
        // Nothing to audit for a server that cannot take it.
        if inner.supervisor.status().borrow().pid.is_none() {
            return Err(ErrorCode::ServerNotRunning.into());
        }

        let shown = fx_console::mask_command(command.as_str());
        let masked = shown.is_some();
        let shown = shown.unwrap_or_else(|| command.as_str().to_owned());
        let record = Record::by(caller, AuditAction::ConsoleCommandSent)
            .target(Servers::DEFAULT)
            .details(json!({ "command": shown }));
        self.audit(record).await?;
        let delivery = match inner.supervisor.command(command, &caller.username).await {
            Ok(()) => Delivery::Written,
            Err(ControlError::Stdin(StdinError::Pending)) => Delivery::Pending,
            Err(ControlError::Stdin(StdinError::Busy)) => Delivery::Busy,
            Err(error) => return Err(control_error(error)),
        };
        Ok(CommandAck { delivery, masked })
    }

    /// The status as it changes.
    pub fn status(&self) -> watch::Receiver<ServerStatus> {
        self.inner.status.clone()
    }

    /// From now on, a hung server is killed without a snapshot of it first: the diagnostics that
    /// read it were turned off.
    pub async fn skip_hang_snapshots(&self) {
        self.inner.supervisor.skip_hang_snapshots().await;
    }

    /// The supervisor itself, whose status, events and readings the statistics follow.
    pub(crate) fn supervisor(&self) -> &Supervisor {
        &self.inner.supervisor
    }

    /// The lines after `since` with every line from then on, nothing missed or repeated in
    /// between. Without `since`, the latest lines. A snapshot holds 5,000 lines at most: a
    /// subscriber further behind is told of the gap.
    pub fn console(&self, since: Option<u64>) -> (ConsoleSnapshot, ConsoleLines) {
        let subscription = self.inner.console.subscribe_from(since);
        let mut backlog = subscription.backlog;
        let mut gap = subscription.gap;
        if backlog.len() > SNAPSHOT_LINES {
            gap |= since.is_some();
            backlog.drain(..backlog.len() - SNAPSHOT_LINES);
        }
        let snapshot = ConsoleSnapshot {
            lines: backlog.iter().map(|line| wire(line)).collect(),
            gap,
            restarted: subscription.restarted,
            last: subscription.last,
        };
        let lines = ConsoleLines {
            live: subscription.live,
            last_batch: None,
        };
        (snapshot, lines)
    }

    /// The console log files, newest first.
    pub async fn log_files(&self) -> Result<Vec<LogFile>, Error> {
        Ok(self.files().await?.into_iter().map(|(file, _)| file).collect())
    }

    /// Opens one of the files [`Supervision::log_files`] lists, by its name there, and audits the
    /// download. A name it does not list is `not_found`, whatever it points at.
    pub async fn open_log(&self, caller: &Caller, name: &str) -> Result<(LogFile, tokio::fs::File), Error> {
        let (file, path) = self
            .files()
            .await?
            .into_iter()
            .find(|(file, _)| file.name == name)
            .ok_or(ErrorCode::NotFound)?;
        let opened = tokio::fs::File::open(&path).await?;
        self.audit(Record::by(caller, AuditAction::ConsoleLogDownloaded).target(&file.name))
            .await?;
        Ok((file, opened))
    }

    async fn files(&self) -> Result<Vec<(LogFile, PathBuf)>, Error> {
        let dir = self.inner.console_logs.clone();
        let files = tokio::task::spawn_blocking(move || match ConsoleLog::files(&dir) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
            files => files,
        })
        .await
        .map_err(io::Error::other)??;
        Ok(files
            .into_iter()
            .rev()
            .filter_map(|file| {
                let name = file.path.file_name()?.to_str()?.to_owned();
                let modified_at = Timestamp::try_from(file.modified).unwrap_or_default();
                let entry = LogFile {
                    name,
                    bytes: file.size,
                    modified_at,
                };
                Some((entry, file.path))
            })
            .collect())
    }

    /// Refuses a start the stored settings or the selection cannot make, then makes sure the
    /// supervisor has them: a save a moment before may not have reached it yet.
    async fn hand_over_launch(&self) -> Result<(), Error> {
        let (stored, state) = read_launch(&self.inner.db).await?;
        let stored = stored.ok_or(ErrorCode::LaunchNotConfigured)?;
        let issues = stored.settings.issues(Platform::host());
        if !issues.is_empty() {
            return Err(ApiError::new(ErrorCode::InvalidLaunchSettings)
                .with("issues", serde_json::to_value(issues).unwrap_or_default())
                .into());
        }
        if state.selected.is_none() {
            return Err(ErrorCode::NoBuildSelected.into());
        }
        self.inner.launches.give(&stored, &state).await;
        Ok(())
    }

    /// Ten commands per user in any ten seconds.
    fn throttle(&self, caller: &Caller) -> Result<(), Error> {
        let now = self.inner.clock.now();
        let mut commands = self.inner.commands.lock().unwrap_or_else(PoisonError::into_inner);
        let sent = commands.entry(caller.user_id).or_default();
        while sent.front().is_some_and(|at| *at + COMMAND_WINDOW <= now) {
            sent.pop_front();
        }
        if let Some(oldest) = sent.front().filter(|_| sent.len() >= COMMANDS_PER_WINDOW) {
            let wait = now.duration_until(*oldest + COMMAND_WINDOW);
            // Whole seconds, rounded up, so that a client retrying on time is never early.
            let seconds = wait.as_secs() + i64::from(wait.subsec_nanos() > 0);
            return Err(ApiError::new(ErrorCode::TooManyCommands)
                .with("retryAfter", seconds.max(1))
                .into());
        }
        sent.push_back(now);
        Ok(())
    }

    /// Audits a start, stop or restart the supervisor took on, failed launches included. A request
    /// it refused for the state the server is in changed nothing.
    async fn record_control(&self, record: Record, result: &Result<(), ControlError>) -> Result<(), Error> {
        let record = match result {
            Ok(()) => record,
            // The full reason names folders, which only the owner reads: the console has it.
            Err(ControlError::Launch(error)) => record
                .outcome(Outcome::Failure)
                .details(json!({ "reason": error.failure })),
            Err(_) => return Ok(()),
        };
        self.audit(record.target(Servers::DEFAULT)).await
    }

    async fn audit(&self, record: Record) -> Result<(), Error> {
        let now = self.inner.clock.now();
        self.inner
            .db
            .write(move |tx| record.write(tx, now).map_err(Error::from))
            .await
    }

    pub(crate) fn note(&self, actor: &str, text: &str) {
        let generation = self.inner.supervisor.status().borrow().generation;
        self.inner.console.push(
            generation,
            fx_console::LineKind::Observer,
            Some(actor.to_owned()),
            text.to_owned(),
            LineFlags::default(),
        );
    }
}

/// Keeps the panel's status in step with the supervisor's, the cause of its last change of state
/// and the stored launch settings, which it hands to the supervisor as they change. A tripped
/// crash loop breaker goes to the audit log: the server stays down until someone acts. What the
/// cfg of a run that starts does against Observer goes to the console.
#[expect(
    clippy::too_many_arguments,
    reason = "what the follower works with, each handed over once as it starts"
)]
async fn follow(
    launches: Launches,
    servers: Servers,
    db: Database,
    clock: Clock,
    console: Arc<ConsoleHub>,
    status: watch::Sender<ServerStatus>,
    mut launch: LaunchReadiness,
    stop: CancellationToken,
) {
    let mut supervised = launches.supervisor.status();
    let mut events = launches.supervisor.events();
    let mut launch_changes = servers.launch_changes();
    let mut cause: Option<Cause> = None;
    let mut restart_at: Option<Timestamp> = None;
    loop {
        tokio::select! {
            () = stop.cancelled() => return,
            changed = supervised.changed() => {
                if changed.is_err() {
                    return;
                }
            }
            event = events.recv() => match event {
                Ok(event) => match event.kind {
                    EventKind::State { from, to, cause: new } => {
                        if let Cause::CrashLoop { failures } = new {
                            let record = Record::operator(ActorKind::System, AuditAction::ServerCrashLoop)
                                .target(Servers::DEFAULT)
                                .outcome(Outcome::Failure)
                                .details(json!({ "failures": failures }));
                            let now = clock.now();
                            if let Err(error) = db.write(move |tx| record.write(tx, now)).await {
                                warn!(%error, "cannot audit the crash loop");
                            }
                        }
                        // A server stopped on request ends with its exit; who asked says more.
                        let asked = matches!(cause, Some(Cause::Requested { .. } | Cause::ConsoleQuit { .. }));
                        let exit_of_a_stop = from == State::Stopping && matches!(new, Cause::Exited { .. });
                        if !(exit_of_a_stop && to == State::Stopped && asked) {
                            cause = Some(new);
                        }
                        if to != State::Crashed {
                            restart_at = None;
                        }
                    }
                    EventKind::CfgWarning { finding } => {
                        console.push(
                            event.generation,
                            fx_console::LineKind::Observer,
                            Some(OBSERVER.to_owned()),
                            setup::console_note(&finding),
                            LineFlags::default(),
                        );
                        continue;
                    }
                    EventKind::RestartScheduled { delay_ms } => {
                        let delay = SignedDuration::from_millis(i64::try_from(delay_ms).unwrap_or(i64::MAX));
                        restart_at = Timestamp::from_millisecond(event.at_ms)
                            .ok()
                            .and_then(|at| at.checked_add(delay).ok());
                    }
                    _ => continue,
                },
                // The status itself is still exact; only a cause may be stale until the next change.
                Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => return,
            },
            Ok(()) = launch_changes.changed() => match read_launch(&db).await {
                Ok((stored, state)) => {
                    if let Some(stored) = &stored {
                        launches.give(stored, &state).await;
                    }
                    launch = readiness(stored.as_ref(), &state);
                }
                Err(error) => warn!(%error, "cannot read the launch settings"),
            },
        }
        let next = server_status(
            &supervised.borrow_and_update(),
            cause.as_ref(),
            restart_at,
            launch,
        );
        status.send_if_modified(|current| {
            let changed = *current != next;
            *current = next;
            changed
        });
    }
}

fn local(action: AuditAction) -> Record {
    Record::operator(ActorKind::Local, action)
}

fn readiness(stored: Option<&StoredLaunch>, state: &ArtifactState) -> LaunchReadiness {
    match stored {
        None => LaunchReadiness::NotConfigured,
        Some(stored) if !stored.settings.issues(Platform::host()).is_empty() => LaunchReadiness::Invalid,
        Some(_) if state.selected.is_none() => LaunchReadiness::NoBuild,
        Some(_) => LaunchReadiness::Ready,
    }
}

fn server_status(
    status: &fx::Status,
    cause: Option<&Cause>,
    restart_at: Option<Timestamp>,
    launch: LaunchReadiness,
) -> ServerStatus {
    let at = |ms: i64| Timestamp::from_millisecond(ms).unwrap_or_default();
    ServerStatus {
        state: server_state(status.state),
        cause: cause.map(state_cause),
        since: at(status.since_ms),
        started_at: status.started_ms.map(at),
        generation: status.generation,
        problems: status.problems.iter().copied().map(problem).collect(),
        build: status.build,
        players: status.players.map(|players| PlayerCount {
            online: players.clients,
            max: players.max_clients,
        }),
        restart_at,
        launch,
        name: status.name.clone(),
        port: status.port,
        start: status.start.as_ref().map(|start| {
            let progress = &start.progress;
            StartProgress {
                started: progress.started,
                failed: progress.failed,
                found: progress.found,
                planned: progress.planned,
                last: progress.last.as_ref().map(|last| StartedResource {
                    name: last.name.clone(),
                    at: at(last.at_ms),
                    warnings: last.warnings,
                }),
                at: progress.at.as_ref().map(|command| StartCommand {
                    file: command.file.clone(),
                    line: command.line,
                    target: command.target.clone(),
                    since: at(command.since_ms),
                    done: command.done,
                    of: command.of,
                    resource: command.resource.clone(),
                }),
                slowest: progress
                    .slowest
                    .iter()
                    .map(|slow| SlowResource {
                        name: slow.name.clone(),
                        ms: u32::try_from(slow.ms).unwrap_or(u32::MAX),
                    })
                    .collect(),
                stuck_at: start.stuck_at_ms.map(at),
                extension_minutes: EXTENSION_MINUTES,
            }
        }),
    }
}

pub(crate) fn server_state(state: State) -> ServerState {
    match state {
        State::Stopped => ServerState::Stopped,
        State::Starting => ServerState::Starting,
        State::Online => ServerState::Online,
        State::Degraded => ServerState::Degraded,
        State::Hung => ServerState::Hung,
        State::Stopping => ServerState::Stopping,
        State::Crashed => ServerState::Crashed,
        State::CrashLoop => ServerState::CrashLoop,
    }
}

fn state_cause(cause: &Cause) -> StateCause {
    match cause.clone() {
        Cause::Requested { by } => StateCause::Requested { by },
        Cause::Restart { by } => StateCause::Restart { by },
        Cause::Recovery => StateCause::Recovery,
        Cause::Scheduled { by } => StateCause::Scheduled { by },
        Cause::Ready => StateCause::Ready,
        Cause::Recovered => StateCause::Recovered,
        Cause::Unhealthy { problems } => StateCause::Unhealthy {
            problems: problems.into_iter().map(problem).collect(),
        },
        Cause::Hang { cause } => StateCause::Hang {
            cause: match cause {
                health::HangCause::NoHeartbeat => HangCause::NoHeartbeat,
                health::HangCause::NoHttp => HangCause::NoHttp,
            },
        },
        Cause::Exited { description } => StateCause::Exited { description },
        Cause::FatalError { message } => StateCause::FatalError { message },
        Cause::BindError { address } => StateCause::BindError { address },
        Cause::LicenceRejected { message } => StateCause::LicenceRejected { message },
        Cause::ConsoleQuit { by } => StateCause::ConsoleQuit { by },
        Cause::Shutdown => StateCause::Shutdown,
        Cause::CrashLoop { failures } => StateCause::CrashLoop {
            failures: u32::try_from(failures).unwrap_or(u32::MAX),
        },
        Cause::SpawnFailed { reason } => StateCause::SpawnFailed { reason },
    }
}

fn problem(problem: health::Problem) -> HealthProblem {
    match problem {
        health::Problem::HttpLate => HealthProblem::HttpLate,
        health::Problem::HeartbeatLate => HealthProblem::HeartbeatLate,
        health::Problem::MainThreadStalled => HealthProblem::MainThreadStalled,
        health::Problem::ProbeUnauthorized => HealthProblem::ProbeUnauthorized,
        health::Problem::WatchdogBark => HealthProblem::WatchdogBark,
    }
}

fn wire(line: &fx_console::ConsoleLine) -> ConsoleLine {
    let kind = match line.kind {
        fx_console::LineKind::Stdout => LineKind::Stdout,
        fx_console::LineKind::Stderr => LineKind::Stderr,
        fx_console::LineKind::Command => LineKind::Command,
        fx_console::LineKind::Observer => LineKind::Observer,
    };
    let flags = [
        (line.flags.truncated, LINE_TRUNCATED),
        (line.flags.no_eol, LINE_NO_EOL),
        (line.flags.masked, LINE_MASKED),
    ]
    .into_iter()
    .filter(|(set, _)| *set)
    .fold(0, |flags, (_, bit)| flags | bit);
    ConsoleLine(
        line.seq,
        line.ts_ms,
        line.generation,
        kind,
        line.actor.clone(),
        line.text.clone(),
        flags,
    )
}

/// A command as the console and the audit log may show it.
fn masked(line: &str) -> String {
    fx_console::mask_command(line).unwrap_or_else(|| line.to_owned())
}

fn command_refusal(refusal: stdin::CommandRefusal) -> CommandRefusal {
    match refusal {
        stdin::CommandRefusal::Empty => CommandRefusal::Empty,
        stdin::CommandRefusal::TooLong => CommandRefusal::TooLong,
        stdin::CommandRefusal::ControlCharacter => CommandRefusal::ControlCharacter,
        stdin::CommandRefusal::Reserved => CommandRefusal::Reserved,
    }
}

fn control_error(error: ControlError) -> Error {
    match error {
        ControlError::AlreadyRunning => ErrorCode::ServerRunning.into(),
        ControlError::NotRunning | ControlError::Stdin(StdinError::Closed) => {
            ErrorCode::ServerNotRunning.into()
        }
        ControlError::Stopping => ErrorCode::ServerStopping.into(),
        ControlError::NotStarting => ErrorCode::ServerNotStarting.into(),
        ControlError::ShuttingDown => ErrorCode::ShuttingDown.into(),
        ControlError::Launch(error) => ApiError::new(ErrorCode::LaunchFailed)
            .with("reason", serde_json::to_value(error.failure).unwrap_or_default())
            .into(),
        ControlError::Refused(refusal) => ApiError::new(ErrorCode::CommandRefused)
            .with(
                "reason",
                serde_json::to_value(command_refusal(refusal)).unwrap_or_default(),
            )
            .into(),
        // Pending and busy are answers, not failures, and an operator's line is never a system one.
        ControlError::Stdin(StdinError::NotAllowed | StdinError::Pending | StdinError::Busy) => {
            ErrorCode::Internal.into()
        }
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::PasswordCost;

    async fn supervision() -> (TempDir, Supervision, Running) {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path().join("observer.sqlite3"), dir.path().join("backups"))
            .await
            .unwrap();
        let clock = Clock::manual("2025-01-06T09:00:00Z".parse().unwrap());
        let accounts = Accounts::new(db.clone(), clock.clone(), PasswordCost::TEST)
            .await
            .unwrap();
        let setup = Setup {
            servers: Servers::new(db.clone(), clock.clone()),
            db,
            clock,
            accounts,
            data_dir: dir.path().join("data"),
            panel: SocketAddr::from(([127, 0, 0, 1], 41975)),
            artifacts: dir.path().join("data/artifacts"),
            console_logs: dir.path().join("data/logs/console"),
            log: None,
        };
        let (supervision, running) = Supervision::start(setup, |_| {}).await.unwrap();
        (dir, supervision, running)
    }

    fn print(supervision: &Supervision, text: &str) -> u64 {
        supervision
            .inner
            .console
            .push(
                1,
                fx_console::LineKind::Stdout,
                None,
                text.to_owned(),
                LineFlags::default(),
            )
            .seq
    }

    #[tokio::test]
    async fn a_snapshot_holds_the_latest_lines_and_tells_a_client_too_far_behind() {
        let (_dir, supervision, _running) = supervision().await;
        let seqs: Vec<u64> = (0..SNAPSHOT_LINES + 5)
            .map(|index| print(&supervision, &format!("line {index}")))
            .collect();

        let (latest, _) = supervision.console(None);
        let (behind, _) = supervision.console(Some(seqs[0]));
        let (close, _) = supervision.console(Some(seqs[SNAPSHOT_LINES - 1]));

        assert_eq!(latest.lines.len(), SNAPSHOT_LINES);
        assert_eq!(latest.lines[0].0, seqs[5]);
        assert!(
            !latest.gap,
            "a client that asked for the latest lines missed none it asked for"
        );
        assert_eq!(behind.lines.len(), SNAPSHOT_LINES);
        assert!(behind.gap);
        assert_eq!(
            close.lines.iter().map(|line| line.0).collect::<Vec<_>>(),
            seqs[SNAPSHOT_LINES..]
        );
        assert!(!close.gap);
    }

    #[tokio::test]
    async fn new_lines_go_out_together_at_most_every_fifty_milliseconds() {
        let (_dir, supervision, _running) = supervision().await;
        let (_, mut lines) = supervision.console(None);

        for index in 0..3 {
            print(&supervision, &format!("burst {index}"));
        }
        let burst = lines.next().await.unwrap().unwrap();
        let sent = Instant::now();
        supervision.inner.console.push(
            1,
            fx_console::LineKind::Command,
            Some("ada".to_owned()),
            "set rcon_password hunter2".to_owned(),
            LineFlags::default(),
        );
        let next = lines.next().await.unwrap().unwrap();

        assert_eq!(burst.len(), 3);
        assert_eq!(burst[0].5, "burst 0");
        // The batch before it went out a moment before `sent`, which waited out the rest.
        assert!(sent.elapsed() + Duration::from_millis(5) >= BATCH_EVERY);
        assert_eq!(
            next,
            [ConsoleLine(
                burst[2].0 + 1,
                next[0].1,
                1,
                LineKind::Command,
                Some("ada".to_owned()),
                "set rcon_password [REDACTED]".to_owned(),
                LINE_MASKED,
            )]
        );
    }

    #[tokio::test]
    async fn a_subscriber_that_lost_lines_is_told_so() {
        let (_dir, supervision, _running) = supervision().await;
        let (_, mut lines) = supervision.console(None);

        for index in 0..2000 {
            print(&supervision, &format!("flood {index}"));
        }

        assert_eq!(lines.next().await, Some(Err(LinesMissed)));
    }
}
