//! The supervisor: one task that starts FXServer, watches its health, stops it gracefully, and
//! brings it back after a crash or a hang. Every change of state goes out on an event bus.
//!
//! When the settings give pre-launch work, each spawn waits for it: the server shows as starting at
//! once, the work runs as a task while requests are still answered, and FXServer is spawned only
//! once it has ended. A stop or a shutdown meanwhile tells it to end, waits, and spawns nothing.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::{Duration, Instant};

use observer_domain::diagnostics::ProcessKind;
use observer_domain::launch::LaunchSettings;
use observer_domain::server::SpawnFailure;
use observer_domain::server_data::CfgFinding;
use observer_platform::{Exit, ExitWatch, Launch, Output, ProcessInfo, ProcessTree, TreeChange, TreeChanges};
use serde::Serialize;
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio::task::{JoinError, JoinHandle};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::bridge::{self, BridgeAction, BridgeEvent, Report, StopReason};
use crate::cfg;
use crate::console::{ConsoleHub, LineAssembler, LineFlags, LineKind, on_channel, plain, sanitize};
use crate::crashes::{self, Collected, DumpLimits, HangSnapshotLimits, SnapshotFailure};
use crate::exit;
use crate::health::{self, HangCause, Health, HealthLimits, PlayerList, Players, Probe, Problem, Verdict};
use crate::hook;
use crate::launch::{self, ServerLaunch};
use crate::layout::{Flavour, Layout};
use crate::perf::TickReport;
use crate::players::{PlayerReport, PlayerReportKind};
use crate::pre_launch::{self, PreLaunch, PreLaunchRequest, PreLaunchResult, TOOLS};
use crate::recovery::{Next, Recovery, RecoveryPolicy};
use crate::resources::Book;
use crate::start::{self, Progress};
use crate::stdin::{
    CommandRefusal, Credentials, ResourceName, ResourceVerb, StdinError, StdinWriter, SystemLine,
    UserCommand, Write,
};
use crate::trace::{self, Trace, TraceDecoder, TraceKind};
use crate::tree::{self, TreeContext};

/// How often health is weighed while a server runs.
const TICK: Duration = Duration::from_secs(1);
/// Once the main process is gone, the rest of the tree gets this long to end by itself: the crash
/// dump server may still be finishing a dump.
const TREE_SETTLE: Duration = Duration::from_secs(3);
const TREE_KILL: Duration = Duration::from_secs(5);
const TASK_JOIN: Duration = Duration::from_secs(2);
/// A dump file touched this recently means the dump server is writing.
const DUMP_ACTIVITY: Duration = Duration::from_secs(10);
/// What someone gives a start that needs longer than the hang delay between two resources.
pub const START_EXTENSION: Duration = Duration::from_mins(15);
/// FXServer's own default, for a cfg that names no TCP endpoint.
const DEFAULT_PORT: u16 = 30120;
const EVENTS: usize = 256;
/// A burst of starts must not push the other events out, so the tree's go on a bus of their own.
const PROCESS_EVENTS: usize = 1024;
/// A restart of a full server reports a join or a drop for each of its players at once.
const PLAYER_REPORTS: usize = 8192;
/// Every resource of a server that counts reports each second it had something to count.
const HOOK_REPORTS: usize = 8192;
/// How often the tree's members are compared with what its notifications said.
const RECONCILE_EVERY: Duration = Duration::from_secs(5);
/// A member younger than this is the notifications' to report: a fork may not have run its program
/// yet.
const RECONCILE_AFTER_MS: i64 = 2_000;
const CHANNEL: usize = 256;
const REQUESTS: usize = 16;
const SUPERVISOR: &str = "supervisor";
/// What a hang snapshot is, for the console.
const SNAPSHOT: &str = if cfg!(windows) {
    "a dump"
} else {
    "a report of the threads"
};

const LICENCE_REJECTIONS: [&str; 3] = [
    "Could not authenticate server license key",
    "This server does not have a license key specified",
    "A valid license check is required to run this server",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Stopped,
    Starting,
    Online,
    Degraded,
    Hung,
    Stopping,
    Crashed,
    /// Automatic restarts are off until someone starts the server.
    CrashLoop,
}

/// Whether the next start counts what its resources send and receive, asked at each spawn.
#[derive(Clone)]
pub struct Counting(Arc<dyn Fn() -> bool + Send + Sync>);

impl Counting {
    pub fn new(asked: impl Fn() -> bool + Send + Sync + 'static) -> Self {
        Self(Arc::new(asked))
    }

    fn asked(&self) -> bool {
        (self.0)()
    }
}

impl fmt::Debug for Counting {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Counting")
    }
}

/// What became of the hook at a start that was to count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hooked {
    /// The start it was for.
    pub generation: u32,
    /// Why the build's scheduler was left as it was, and that start counts nothing.
    pub refused: Option<String>,
}

type LatestHook = Arc<Mutex<Option<Hooked>>>;

/// What a start did with the hook.
enum Hook {
    /// The start does not count: the scheduler is the build's own.
    Out,
    In,
    /// The start was to count, and the scheduler was left as it was.
    Refused(String),
}

/// What one resource counted, with the start it is of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookReport {
    pub generation: u32,
    pub report: hook::Report,
}

#[derive(Debug, Clone)]
pub struct Settings {
    /// What the next spawn launches, none until [`Supervisor::configure`] gives it.
    pub launch: Option<ServerLaunch>,
    pub flavour: Flavour,
    /// Where `/dynamic.json` and `/perf/` answer; by default the cfg's first TCP endpoint.
    pub probe: Option<SocketAddr>,
    /// Observer's data folder: crash dumps go to its `crashes/`.
    pub data_dir: PathBuf,
    /// The boot grace and the thresholds that degrade a server. Whether a stuck one is restarted,
    /// and after how long, comes from the launch settings, which apply at once to a running server.
    pub health: HealthLimits,
    pub recovery: RecoveryPolicy,
    pub stop: StopTimings,
    pub dumps: DumpLimits,
    /// A picture of a hung server before it is killed; `None` kills it at once.
    pub hang_snapshot: Option<HangSnapshotLimits>,
    /// Drops the first notification of each run's tree, as the system may: for tests of the
    /// reconciliation.
    pub drop_first_tree_change: bool,
    /// Work run before each spawn, whatever its cause, while FXServer is down; `None` spawns at
    /// once.
    pub pre_launch: Option<Arc<dyn PreLaunch>>,
    /// How long that work gets before it is told to end. The spawn still waits until it has.
    pub pre_launch_limit: Duration,
    /// Whether a start counts what its resources send and receive; `None` never does.
    pub counting: Option<Counting>,
}

impl Settings {
    pub fn new(launch: Option<ServerLaunch>, data_dir: impl Into<PathBuf>) -> Self {
        Self {
            launch,
            flavour: Flavour::HOST,
            probe: None,
            data_dir: data_dir.into(),
            health: HealthLimits::default(),
            recovery: RecoveryPolicy::default(),
            stop: StopTimings::default(),
            dumps: DumpLimits::default(),
            hang_snapshot: Some(HangSnapshotLimits::default()),
            drop_first_tree_change: false,
            pre_launch: None,
            pre_launch_limit: pre_launch::DEFAULT_LIMIT,
            counting: None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct StopTimings {
    /// Between `observer:shuttingDown` and `quit`, for resources to save and warn players.
    pub notice: Duration,
    /// How long `quit` gets before the tree is killed.
    pub quit: Duration,
    /// The wait after `quit` when the console is closing and Windows gives about five seconds.
    pub shortened: Duration,
    /// After a fatal error, how long FXServer gets to exit before the tree is killed: on Windows
    /// it sometimes stays up behind an error dialog.
    pub fatal_exit: Duration,
}

impl Default for StopTimings {
    fn default() -> Self {
        Self {
            notice: Duration::from_secs(5),
            quit: Duration::from_secs(15),
            shortened: Duration::from_secs(4),
            fatal_exit: Duration::from_secs(5),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Event {
    pub at_ms: i64,
    /// The spawn the event is about, from the start that leads to it; 0 before the first. A start
    /// stopped or failed before its spawn leaves its generation without one.
    pub generation: u32,
    pub kind: EventKind,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventKind {
    State {
        from: State,
        to: State,
        cause: Cause,
    },
    Spawned {
        pid: u32,
        probe: SocketAddr,
        /// The build folder this run was started from.
        build: PathBuf,
        /// The settings it was launched with.
        launch: Box<LaunchSettings>,
    },
    /// A finding of the cfg of the run that just started, of severity warning or blocking.
    CfgWarning {
        finding: CfgFinding,
    },
    Exited {
        code: Option<u32>,
        signal: Option<i32>,
        description: String,
        /// Observer had asked it to stop, or an operator typed `quit`.
        expected: bool,
    },
    /// The problems of a degraded server changed.
    Health {
        problems: Vec<Problem>,
    },
    RestartScheduled {
        delay_ms: u64,
    },
    /// Dumps moved into Observer's crash folder, and the older files its limits removed.
    CrashDumps {
        paths: Vec<PathBuf>,
        removed: Vec<PathBuf>,
    },
    /// The picture of a hung server taken before the kill, or why there is none.
    HangSnapshot {
        path: Option<PathBuf>,
        bytes: u64,
        took_ms: u64,
        failure: Option<SnapshotFailure>,
        removed: Vec<PathBuf>,
    },
    Hitch {
        thread: String,
        ms: u64,
    },
    WatchdogBark {
        thread: String,
        seconds: u64,
        stack: String,
    },
    WatchdogBite {
        thread: String,
    },
    NucleusConnected {
        url: String,
    },
    Resources {
        started: Vec<String>,
        /// What started and stopped since the last report, in order.
        events: Vec<ResourceEvent>,
        /// Starts and stops past the 64 a report tells.
        dropped: u32,
    },
    Heap {
        heap_used: u64,
        heap_total: u64,
        rss: u64,
        external: u64,
    },
    /// The bridge listed every resource FXServer knows, once its cfg has run and after each
    /// `refresh`: [`Supervisor::resources`] has them.
    ResourceList {
        resources: usize,
    },
}

/// A resource the server started or stopped while it ran.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResourceEvent {
    pub name: String,
    /// `false` for a stop.
    pub started: bool,
}

/// A process of the tree started or ended, the main process left out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeEvent {
    pub generation: u32,
    pub at_ms: i64,
    pub change: ProcessChange,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessChange {
    Started { process: ProcessInfo, kind: ProcessKind },
    Ended { pid: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Cause {
    /// A start or a stop someone asked for.
    Requested {
        by: String,
    },
    /// A restart someone asked for: its stop, then the start that follows.
    Restart {
        by: String,
    },
    /// An automatic restart after a failure.
    Recovery,
    /// A restart the schedule made: its stop, then the start that follows. `by` is `observer`, or
    /// whoever planned a single restart.
    Scheduled {
        by: String,
    },
    Ready,
    Recovered,
    Unhealthy {
        problems: Vec<Problem>,
    },
    Hang {
        cause: HangCause,
    },
    Exited {
        description: String,
    },
    FatalError {
        message: String,
    },
    BindError {
        address: String,
    },
    LicenceRejected {
        message: String,
    },
    ConsoleQuit {
        by: String,
    },
    Shutdown,
    CrashLoop {
        failures: usize,
    },
    SpawnFailed {
        reason: SpawnFailure,
    },
}

/// Why a spawn failed: the step, which anyone may see, and the full text, which names folders, for
/// the console and the log.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct SpawnError {
    pub failure: SpawnFailure,
    pub message: String,
}

impl SpawnError {
    fn new(failure: SpawnFailure, message: impl Into<String>) -> Self {
        Self {
            failure,
            message: message.into(),
        }
    }
}

impl From<launch::LaunchError> for SpawnError {
    fn from(error: launch::LaunchError) -> Self {
        Self::new(error.failure(), error.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Status {
    pub state: State,
    pub generation: u32,
    pub pid: Option<u32>,
    pub since_ms: i64,
    pub problems: Vec<Problem>,
    /// When the server that runs now was started.
    pub started_ms: Option<i64>,
    /// What `/dynamic.json` said last, while a server runs.
    pub players: Option<Players>,
    /// The build the running server reports, once it has.
    pub build: Option<u32>,
    /// The game build it enforces, once it has said.
    pub game_build: Option<u32>,
    /// What the server called itself, `sv_projectName`, as of the last run that said.
    pub name: Option<String>,
    /// The game port the probes reached, as of the last run.
    pub port: Option<u16>,
    /// The bridge's resource name in the server that runs now, drawn for its start.
    pub bridge: Option<String>,
    /// How far the start has come, while the server is starting.
    pub start: Option<StartStatus>,
}

/// A start under way: what FXServer said of it, and how long Observer still waits for more.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StartStatus {
    pub progress: start::Snapshot,
    /// When the start is taken for stuck, should FXServer start no resource until then. None with
    /// hang detection off, and once the cfg has run.
    pub stuck_at_ms: Option<i64>,
}

/// What the probes read last of the server that runs now, for the statistics. It changes every
/// second and holds what the panel never sees, so it stays out of [`Status`].
#[derive(Debug, Clone, Default)]
pub struct Readings {
    pub run: Option<RunReadings>,
}

#[derive(Debug, Clone)]
pub struct RunReadings {
    pub generation: u32,
    pub started_ms: i64,
    /// The game endpoint the probes reach.
    pub probe: SocketAddr,
    /// Upgraded only for one measurement at a time, so that the tree never outlives its run for
    /// longer than that.
    pub tree: Weak<ProcessTree>,
    pub dynamic: Option<Stamped<Players>>,
    pub perf: Option<Stamped<TickReport>>,
    pub players: Option<Stamped<PlayerList>>,
}

/// A probe's answer and when it came.
#[derive(Debug, Clone)]
pub struct Stamped<T> {
    pub at: Instant,
    pub value: T,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ControlError {
    #[error("the server is already running")]
    AlreadyRunning,
    #[error("the server is not running")]
    NotRunning,
    #[error("the server is stopping")]
    Stopping,
    #[error("the server is not starting")]
    NotStarting,
    #[error("Observer is shutting down")]
    ShuttingDown,
    #[error("cannot start FXServer: {0}")]
    Launch(SpawnError),
    #[error(transparent)]
    Refused(#[from] CommandRefusal),
    #[error(transparent)]
    Stdin(#[from] StdinError),
    #[error("Observer's bridge is not a resource to start or stop")]
    Bridge,
}

type Reply<T> = oneshot::Sender<Result<T, ControlError>>;
/// How a line handed to the stdin writer went, once it is known.
type Written = oneshot::Receiver<Result<(), StdinError>>;

enum Request {
    Start {
        by: String,
        reply: Reply<()>,
    },
    Stop {
        by: String,
        reply: Reply<()>,
    },
    Restart {
        by: String,
        reply: Reply<()>,
    },
    ExtendStart {
        by: String,
        reply: Reply<()>,
    },
    RestartScheduled {
        by: String,
        reply: Reply<()>,
    },
    Announce {
        event: BridgeEvent,
        reply: Reply<()>,
    },
    Act {
        action: BridgeAction,
        reply: Reply<()>,
    },
    Command {
        command: UserCommand,
        actor: String,
        reply: Reply<Written>,
    },
    Resource {
        verb: ResourceVerb,
        name: ResourceName,
        by: String,
        reply: Reply<Written>,
    },
    RefreshResources {
        by: String,
        reply: Reply<Written>,
    },
    Shutdown {
        shortened: bool,
        /// What resources hear: `shutdown`, or `scheduled` when the stop takes the place of a
        /// scheduled restart.
        reason: StopReason,
    },
    Configure(Box<ServerLaunch>),
    SkipHangSnapshots,
}

/// The bridge's report of the resources started, with the starts and stops since the one before
/// as `[name, "start" | "stop"]`: taken into what the run knows of its resources, and handed on.
fn resources_started(
    held: &RunResources,
    started: Vec<String>,
    events: Vec<(String, String)>,
    dropped: u32,
) -> EventKind {
    let events: Vec<ResourceEvent> = events
        .into_iter()
        .filter_map(|(name, change)| {
            let started = match change.as_str() {
                "start" => true,
                "stop" => false,
                _ => return None,
            };
            Some(ResourceEvent { name, started })
        })
        .collect();
    let turns: Vec<(String, bool)> = events
        .iter()
        .map(|event| (event.name.clone(), event.started))
        .collect();
    if let Some((_, book)) = &mut *held.lock().unwrap_or_else(PoisonError::into_inner) {
        book.started(&started, &turns, now_ms());
    }
    EventKind::Resources {
        started,
        events,
        dropped,
    }
}

/// The latest spawn's progress and when it was spawned: what FXServer said as it started, kept
/// after the start and until the next one.
type LatestStart = Arc<Mutex<Option<(i64, Arc<Progress>)>>>;

/// What the bridge of the run that is up reported of its resources, with the run's generation:
/// none while no server runs.
type RunResources = Arc<Mutex<Option<(u32, Book)>>>;

/// The resources of the run that is up.
#[derive(Debug, Clone)]
pub struct Resources {
    pub generation: u32,
    /// As the bridge reported them: nothing listed until the cfg has run.
    pub book: Book,
    /// What the console said of them.
    pub marks: start::Marks,
}

/// The handle to the supervisor task. Clones talk to the same task.
#[derive(Clone)]
pub struct Supervisor {
    latest_start: LatestStart,
    resources: RunResources,
    latest_hook: LatestHook,
    requests: mpsc::Sender<Request>,
    events: broadcast::Sender<Event>,
    processes: broadcast::Sender<TreeEvent>,
    players: broadcast::Sender<PlayerReport>,
    counts: broadcast::Sender<HookReport>,
    status: watch::Receiver<Status>,
    readings: watch::Receiver<Readings>,
}

impl Supervisor {
    /// Starts the supervisor with the server stopped. The task ends after [`Supervisor::shutdown`]
    /// has stopped the server.
    pub fn spawn(settings: Settings, console: Arc<ConsoleHub>) -> (Self, JoinHandle<()>) {
        let (requests, requests_rx) = mpsc::channel(REQUESTS);
        let events = broadcast::channel(EVENTS).0;
        let processes = broadcast::channel(PROCESS_EVENTS).0;
        let players = broadcast::channel(PLAYER_REPORTS).0;
        let counts = broadcast::channel(HOOK_REPORTS).0;
        let initial = Status {
            state: State::Stopped,
            generation: 0,
            pid: None,
            since_ms: now_ms(),
            problems: Vec::new(),
            started_ms: None,
            players: None,
            build: None,
            game_build: None,
            name: None,
            port: None,
            bridge: None,
            start: None,
        };
        let (status_tx, status) = watch::channel(initial);
        let (readings_tx, readings) = watch::channel(Readings::default());
        let (traces_tx, traces) = mpsc::channel(CHANNEL);
        let (signals_tx, signals) = mpsc::channel(CHANNEL);
        let (probes_tx, probes) = mpsc::channel(CHANNEL);
        let latest_start = LatestStart::default();
        let resources = RunResources::default();
        let latest_hook = LatestHook::default();
        let actor = Actor {
            latest_start: Arc::clone(&latest_start),
            resources: Arc::clone(&resources),
            latest_hook: Arc::clone(&latest_hook),
            counts: counts.clone(),
            recovery: Recovery::new(settings.recovery.clone()),
            settings,
            console,
            events: events.clone(),
            processes: processes.clone(),
            players: players.clone(),
            status: status_tx,
            readings: readings_tx,
            requests: requests_rx,
            traces_tx,
            traces,
            signals_tx,
            signals,
            probes_tx,
            probes,
            current: watch::channel(0).0,
            generation: 0,
            state: State::Stopped,
            since_ms: now_ms(),
            problems: Vec::new(),
            run: None,
            name: None,
            port: None,
            pre_launch: None,
            phase: Phase::Idle,
            last_tick: Instant::now(),
            shutdown: None,
            orphaned: false,
            done: false,
        };
        let task = tokio::spawn(actor.run());
        (
            Self {
                latest_start,
                resources,
                latest_hook,
                requests,
                events,
                processes,
                players,
                counts,
                status,
                readings,
            },
            task,
        )
    }

    pub async fn start(&self, by: &str) -> Result<(), ControlError> {
        self.ask(|reply| Request::Start {
            by: by.to_owned(),
            reply,
        })
        .await
    }

    /// Stops the server gracefully: `observer:shuttingDown`, the notice delay, `quit`, then the
    /// tree is killed if it is still there.
    pub async fn stop(&self, by: &str) -> Result<(), ControlError> {
        self.ask(|reply| Request::Stop {
            by: by.to_owned(),
            reply,
        })
        .await
    }

    pub async fn restart(&self, by: &str) -> Result<(), ControlError> {
        self.ask(|reply| Request::Restart {
            by: by.to_owned(),
            reply,
        })
        .await
    }

    /// What FXServer warned about as it started its resources, in the latest start, with when
    /// that start was spawned. None before the first start.
    pub fn start_warnings(&self) -> Option<(i64, start::Warnings)> {
        let latest = self.latest_start.lock().unwrap_or_else(PoisonError::into_inner);
        latest
            .as_ref()
            .map(|(spawned_ms, progress)| (*spawned_ms, progress.warnings()))
    }

    /// The resources of the run that is up, as its bridge reported them and as its console spoke
    /// of them. None while no server runs.
    pub fn resources(&self) -> Option<Resources> {
        let (generation, book) = self
            .resources
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()?;
        let marks = self
            .latest_start
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(|(_, progress)| progress.marks())
            .unwrap_or_default();
        Some(Resources {
            generation,
            book,
            marks,
        })
    }

    /// Writes FXServer's own `start`, `stop` or `restart` of `name` to its console, echoed there
    /// as `by`'s command: only on a running server whose bridge has reported, and never for the
    /// bridge itself.
    pub async fn resource(
        &self,
        verb: ResourceVerb,
        name: ResourceName,
        by: &str,
    ) -> Result<(), ControlError> {
        let written = self
            .ask(|reply| Request::Resource {
                verb,
                name,
                by: by.to_owned(),
                reply,
            })
            .await?;
        Ok(written.await.map_err(|_| StdinError::Closed)??)
    }

    /// Writes FXServer's `refresh`, which reads its resource folders again, under the conditions
    /// of [`Supervisor::resource`].
    pub async fn refresh_resources(&self, by: &str) -> Result<(), ControlError> {
        let written = self
            .ask(|reply| Request::RefreshResources {
                by: by.to_owned(),
                reply,
            })
            .await?;
        Ok(written.await.map_err(|_| StdinError::Closed)??)
    }

    /// Gives the start under way [`START_EXTENSION`] more before it is taken for stuck, counted
    /// from when it would have been. Only of a server whose cfg is still running.
    pub async fn extend_start(&self, by: &str) -> Result<(), ControlError> {
        self.ask(|reply| Request::ExtendStart {
            by: by.to_owned(),
            reply,
        })
        .await
    }

    /// The schedule's restart: like [`Supervisor::restart`], with `scheduled` as the reason
    /// resources hear, but only of a server that runs. A stopped server, or one waiting to restart
    /// after a failure, is never started by it.
    pub async fn restart_scheduled(&self, by: &str) -> Result<(), ControlError> {
        self.ask(|reply| Request::RestartScheduled {
            by: by.to_owned(),
            reply,
        })
        .await
    }

    /// Raises an event of the integration contract, only on a running server whose bridge has
    /// reported: nothing is queued for a server still starting or already stopping.
    pub async fn announce(&self, event: BridgeEvent) -> Result<(), ControlError> {
        self.ask(|reply| Request::Announce { event, reply }).await
    }

    /// Has the bridge act, under the same conditions as [`Supervisor::announce`].
    pub async fn act(&self, action: BridgeAction) -> Result<(), ControlError> {
        self.ask(|reply| Request::Act { action, reply }).await
    }

    /// Writes an operator's command to FXServer and echoes it into the console once written.
    pub async fn command(&self, command: UserCommand, actor: &str) -> Result<(), ControlError> {
        let written = self
            .ask(|reply| Request::Command {
                command,
                actor: actor.to_owned(),
                reply,
            })
            .await?;
        Ok(written.await.map_err(|_| StdinError::Closed)??)
    }

    /// Sets what the next spawn launches, whether started by hand or after a failure. A running
    /// server keeps the launch it was started with, but for hang detection, which applies to it at
    /// once and counts its silence from the change.
    pub async fn configure(&self, launch: ServerLaunch) {
        let _ = self.requests.send(Request::Configure(Box::new(launch))).await;
    }

    /// From now on, a hung server is killed without a snapshot of it first.
    pub async fn skip_hang_snapshots(&self) {
        let _ = self.requests.send(Request::SkipHangSnapshots).await;
    }

    /// Stops the server, then the supervisor. `shortened` is for a closing console, which leaves
    /// a few seconds only.
    pub async fn shutdown(&self, shortened: bool) {
        let _ = self
            .requests
            .send(Request::Shutdown {
                shortened,
                reason: StopReason::Shutdown,
            })
            .await;
    }

    /// Stops the server, then the supervisor, in the place of a scheduled restart: resources hear
    /// `scheduled`, as they would for the restart, and FXServer comes back with the next Observer.
    pub async fn shutdown_scheduled(&self) {
        let _ = self
            .requests
            .send(Request::Shutdown {
                shortened: false,
                reason: StopReason::Scheduled,
            })
            .await;
    }

    pub fn events(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    /// The processes each run's tree starts and ends, classified.
    pub fn processes(&self) -> broadcast::Receiver<TreeEvent> {
        self.processes.subscribe()
    }

    /// What the bridge reports about players. They stay off the event bus, which is printed and
    /// logged, since a join carries the player's address.
    pub fn player_reports(&self) -> broadcast::Receiver<PlayerReport> {
        self.players.subscribe()
    }

    /// What the resources of a start that counts report, each second. Off the event bus too:
    /// there are hundreds a second.
    pub fn hook_reports(&self) -> broadcast::Receiver<HookReport> {
        self.counts.subscribe()
    }

    /// What became of the hook at the latest start, if that start was to count.
    pub fn hooked(&self) -> Option<Hooked> {
        self.latest_hook
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub fn status(&self) -> watch::Receiver<Status> {
        self.status.clone()
    }

    pub fn readings(&self) -> watch::Receiver<Readings> {
        self.readings.clone()
    }

    async fn ask<T>(&self, request: impl FnOnce(Reply<T>) -> Request) -> Result<T, ControlError> {
        let (reply, answer) = oneshot::channel();
        self.requests
            .send(request(reply))
            .await
            .map_err(|_| ControlError::ShuttingDown)?;
        answer.await.map_err(|_| ControlError::ShuttingDown)?
    }
}

/// What the stopping server should do once it is gone.
#[derive(Debug, Clone)]
enum After {
    Stop,
    /// The cause the start that follows goes by.
    Restart {
        cause: Cause,
    },
    /// Observer itself is stopping.
    Exit,
}

/// Why a run is ending, known before its process is gone.
#[derive(Debug, Clone)]
enum Ending {
    Fatal { message: String },
    Hang { cause: HangCause },
    BindError { address: String },
    LicenceRejected { message: String },
}

impl Ending {
    /// A fatal error often follows a more telling reason: a taken port, a refused licence.
    fn rank(&self) -> u8 {
        match self {
            Self::Fatal { .. } => 0,
            Self::Hang { .. } => 1,
            Self::BindError { .. } => 2,
            Self::LicenceRejected { .. } => 3,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Phase {
    /// No server: stopped, or in a crash loop.
    Idle,
    /// The pre-launch work of a spawn runs, FXServer down. It is told to end at `deadline`, `None`
    /// once it was told.
    PreLaunch {
        deadline: Option<Instant>,
    },
    Running,
    /// `observer:shuttingDown` is out; `quit` follows at `quit_at`.
    Notice {
        quit_at: Instant,
        reason: StopReason,
    },
    /// `quit` is out; the tree is killed at `kill_at`.
    Quitting {
        kill_at: Instant,
    },
    /// The run is over (fatal error, hang); the tree is killed at `kill_at` unless it ends first.
    Ending {
        kill_at: Instant,
    },
    /// Waiting to restart after a failure.
    Backoff {
        respawn_at: Instant,
    },
}

#[derive(Debug)]
enum Signal {
    Fatal(String),
    LicenceRejected(String),
}

impl Signal {
    /// Only FXServer's own words count: its licence check prints on the `svadhesive` channel and
    /// ends with a bare `Fatal Error:` on stderr. Whatever a resource prints, a player's chat
    /// message included, comes under its `script:` channel, so quoting these texts stops nothing.
    fn read(kind: LineKind, text: &str) -> Option<Self> {
        match kind {
            LineKind::Stderr => {
                let message = text.strip_prefix("Fatal Error: ")?.trim();
                if is_licence_rejection(message) {
                    Some(Self::LicenceRejected(text.trim().to_owned()))
                } else {
                    Some(Self::Fatal(message.to_owned()))
                }
            }
            LineKind::Stdout => {
                let message = on_channel(text, "svadhesive")?;
                let message = message.strip_prefix("Error: ").unwrap_or(message);
                is_licence_rejection(message).then(|| Self::LicenceRejected(text.trim().to_owned()))
            }
            LineKind::Command | LineKind::Observer => None,
        }
    }
}

fn is_licence_rejection(message: &str) -> bool {
    LICENCE_REJECTIONS
        .iter()
        .any(|rejection| message.starts_with(rejection))
}

/// One spawn of FXServer, from its start until its tree is empty.
struct Run {
    generation: u32,
    pid: u32,
    started_ms: i64,
    players: Option<Players>,
    build: Option<u32>,
    game_build: Option<u32>,
    layout: Layout,
    /// The bridge's name in this run: only its reports are taken.
    bridge: String,
    exit: ExitWatch,
    tree: Arc<ProcessTree>,
    stdin: StdinWriter,
    stdin_task: JoinHandle<()>,
    cancel: CancellationToken,
    tasks: Vec<JoinHandle<()>>,
    health: Health,
    /// What FXServer says of its start: what shows it going on before the bridge's first report.
    progress: Arc<Progress>,
    /// The start as published last, and the instant its `stuck_at_ms` was worked out from.
    start: Option<(StartStatus, Option<Instant>)>,
    bridge_ready: bool,
    /// The name the bridge gave the server for maintenance, as `/info.json` shows it: not the
    /// server's own.
    maintenance_name: Option<String>,
    ready_at: Option<Instant>,
    after: Option<After>,
    ending: Option<Ending>,
    /// Observer killed the tree: the exit status says nothing about the server then.
    killed: bool,
}

impl Run {
    fn end(&mut self, ending: Ending) {
        if self
            .ending
            .as_ref()
            .is_none_or(|current| current.rank() < ending.rank())
        {
            self.ending = Some(ending);
        }
    }

    fn system(&self, line: SystemLine) {
        // Fire and forget: a line FXServer does not take is covered by the deadline that follows.
        drop(self.stdin.send(Write::System(line)));
    }
}

/// A launch prepared for its spawn, its cfg scan already handed on.
struct Ready {
    launch: Launch,
    layout: Layout,
    bridge: String,
    warnings: Vec<CfgFinding>,
    endpoint: Option<SocketAddr>,
    /// The launch settings the spawn goes by, which the spawn's event reports.
    settings: LaunchSettings,
    plan: start::Plan,
}

impl Ready {
    fn split(prepared: launch::Prepared, settings: LaunchSettings) -> (Self, cfg::Scan) {
        let launch::Prepared {
            launch,
            layout,
            bridge,
            warnings,
            endpoint,
            scan,
            plan,
        } = prepared;
        let ready = Self {
            launch,
            layout,
            bridge,
            warnings,
            endpoint,
            settings,
            plan,
        };
        (ready, scan)
    }
}

/// The pre-launch work of one spawn, from the start of its phase until its task ends.
struct PreLaunching {
    task: JoinHandle<PreLaunchResult>,
    cancel: Arc<AtomicBool>,
    /// The generation the start took as the phase began, which the work's lines are written with
    /// and the spawn carries.
    generation: u32,
    cause: Cause,
    ready: Ready,
    /// What comes once the work ended, when it was told to end for something else than the spawn.
    instead: Option<Instead>,
}

impl PreLaunching {
    fn tell_to_end(&mut self, instead: Instead) {
        self.cancel.store(true, Ordering::Relaxed);
        self.instead = Some(instead);
    }
}

/// What follows pre-launch work told to end before its spawn.
enum Instead {
    /// The server stays stopped.
    Stop { by: String },
    /// A start anew, with fresh work and the launch configured by then.
    Restart { cause: Cause },
    /// Observer itself is stopping.
    Exit,
}

enum Wake {
    Request(Option<Request>),
    Exit(Exit),
    Trace(Trace),
    Signal(u32, Signal),
    Probe(u32, Probe),
    PreLaunched(Result<PreLaunchResult, JoinError>),
    Tick,
}

struct Actor {
    latest_start: LatestStart,
    resources: RunResources,
    latest_hook: LatestHook,
    settings: Settings,
    console: Arc<ConsoleHub>,
    events: broadcast::Sender<Event>,
    processes: broadcast::Sender<TreeEvent>,
    players: broadcast::Sender<PlayerReport>,
    counts: broadcast::Sender<HookReport>,
    status: watch::Sender<Status>,
    readings: watch::Sender<Readings>,
    requests: mpsc::Receiver<Request>,
    traces_tx: mpsc::Sender<Trace>,
    traces: mpsc::Receiver<Trace>,
    signals_tx: mpsc::Sender<(u32, Signal)>,
    signals: mpsc::Receiver<(u32, Signal)>,
    probes_tx: mpsc::Sender<(u32, Probe)>,
    probes: mpsc::Receiver<(u32, Probe)>,
    /// The current generation, which trace decoders compare theirs against.
    current: watch::Sender<u32>,
    generation: u32,
    state: State,
    since_ms: i64,
    problems: Vec<Problem>,
    recovery: Recovery,
    run: Option<Run>,
    /// What the server called itself and the port it took, kept once its run ends.
    name: Option<String>,
    port: Option<u16>,
    /// The pre-launch work under way, in [`Phase::PreLaunch`] only.
    pre_launch: Option<PreLaunching>,
    phase: Phase,
    last_tick: Instant,
    /// Set once Observer asked to stop; `true` for the shortened stop.
    shutdown: Option<bool>,
    /// Every handle is gone, so no request can come any more.
    orphaned: bool,
    done: bool,
}

impl Actor {
    async fn run(mut self) {
        #[cfg(unix)]
        self.sweep_leftovers().await;
        let folder = self.settings.data_dir.join("crashes");
        let _ = tokio::task::spawn_blocking(move || crashes::remove_stale_partials(&folder)).await;
        while !self.done {
            let deadline = self.deadline();
            let wake = tokio::select! {
                request = self.requests.recv(), if !self.orphaned => Wake::Request(request),
                exit = exit_of(&mut self.run) => Wake::Exit(exit),
                Some(trace) = self.traces.recv() => Wake::Trace(trace),
                Some((generation, signal)) = self.signals.recv() => Wake::Signal(generation, signal),
                Some((generation, probe)) = self.probes.recv() => Wake::Probe(generation, probe),
                ended = pre_launch_of(&mut self.pre_launch) => Wake::PreLaunched(ended),
                () = tokio::time::sleep_until(deadline.into()) => Wake::Tick,
            };
            match wake {
                Wake::Request(Some(request)) => self.request(request).await,
                Wake::Request(None) => {
                    self.orphaned = true;
                    self.shutdown(false, StopReason::Shutdown);
                }
                Wake::Exit(exit) => self.exited(exit).await,
                Wake::Trace(trace) => self.trace(trace),
                Wake::Signal(generation, signal) => self.signal(generation, signal),
                Wake::Probe(generation, probe) => self.probed(generation, probe),
                Wake::PreLaunched(ended) => self.pre_launched(ended).await,
                Wake::Tick => self.tick().await,
            }
        }
        info!("the supervisor stopped");
    }

    fn deadline(&self) -> Instant {
        let phase = match self.phase {
            Phase::PreLaunch { deadline } => deadline,
            Phase::Notice { quit_at, .. } => Some(quit_at),
            Phase::Quitting { kill_at } | Phase::Ending { kill_at } => Some(kill_at),
            Phase::Backoff { respawn_at } => Some(respawn_at),
            Phase::Idle | Phase::Running => None,
        };
        let tick = self.run.as_ref().map(|_| self.last_tick + TICK);
        [phase, tick]
            .into_iter()
            .flatten()
            .min()
            .unwrap_or_else(|| Instant::now() + Duration::from_hours(1))
    }

    async fn request(&mut self, request: Request) {
        match request {
            Request::Start { by, reply } => {
                let result = self.start(by).await;
                let _ = reply.send(result);
            }
            Request::Stop { by, reply } => {
                let _ = reply.send(self.stop(by));
            }
            Request::Restart { by, reply } => {
                let result = self.restart(by).await;
                let _ = reply.send(result);
            }
            Request::ExtendStart { by, reply } => {
                let _ = reply.send(self.extend_start(&by));
            }
            Request::RestartScheduled { by, reply } => {
                let _ = reply.send(self.restart_scheduled(by));
            }
            Request::Announce { event, reply } => {
                let _ = reply.send(self.announce(event));
            }
            Request::Act { action, reply } => {
                let renamed = match &action {
                    BridgeAction::ServerName { name } => Some(name.as_deref().and_then(health::project_name)),
                    _ => None,
                };
                let written = self.to_bridge(SystemLine::Action(action));
                if let (Ok(()), Some(name), Some(run)) = (&written, renamed, self.run.as_mut()) {
                    run.maintenance_name = name;
                }
                let _ = reply.send(written);
            }
            Request::Command {
                command,
                actor,
                reply,
            } => {
                let _ = reply.send(self.command(command, actor));
            }
            Request::Resource {
                verb,
                name,
                by,
                reply,
            } => {
                let _ = reply.send(self.resource(verb, name, by));
            }
            Request::RefreshResources { by, reply } => {
                let _ = reply.send(self.asked(SystemLine::Refresh, by));
            }
            Request::Shutdown { shortened, reason } => self.shutdown(shortened, reason),
            Request::Configure(launch) => self.configure(*launch),
            Request::SkipHangSnapshots => self.settings.hang_snapshot = None,
        }
    }

    /// What the next spawn launches. Hang detection reaches the running server too, from now on:
    /// see [`Health::arm`].
    fn configure(&mut self, launch: ServerLaunch) {
        let hang = |settings: &LaunchSettings| (settings.hang_detection, settings.hang_timeout_seconds);
        let before = self.settings.launch.as_ref().map(|old| hang(&old.settings));
        if before != Some(hang(&launch.settings))
            && let Some(run) = &mut self.run
        {
            run.health.arm(Instant::now());
        }
        self.settings.launch = Some(launch);
    }

    fn probed(&mut self, generation: u32, probe: Probe) {
        let Some(run) = self.run.as_mut().filter(|run| run.generation == generation) else {
            return;
        };
        let now = Instant::now();
        run.health.probed(&probe, now);
        let changed = match &probe {
            Probe::Dynamic(Ok(players)) if run.players != Some(*players) => {
                run.players = Some(*players);
                true
            }
            Probe::Info(Ok(info)) => {
                // Read after the bridge renamed the server for maintenance, the name is not its own.
                let own = info.name.is_none() || info.name != run.maintenance_name;
                let changed = run.build != Some(info.build)
                    || run.game_build != info.game_build
                    || (own && self.name != info.name);
                run.build = Some(info.build);
                run.game_build = info.game_build;
                if own {
                    self.name.clone_from(&info.name);
                }
                changed
            }
            _ => false,
        };
        if changed {
            self.publish_status();
        }
        self.readings.send_if_modified(|readings| {
            let Some(run) = readings.run.as_mut().filter(|run| run.generation == generation) else {
                return false;
            };
            match probe {
                Probe::Dynamic(Ok(value)) => run.dynamic = Some(Stamped { at: now, value }),
                Probe::Perf(Ok(value)) => run.perf = Some(Stamped { at: now, value }),
                Probe::Players(Ok(value)) => run.players = Some(Stamped { at: now, value }),
                _ => return false,
            }
            true
        });
    }

    async fn start(&mut self, by: String) -> Result<(), ControlError> {
        if self.shutdown.is_some() {
            return Err(ControlError::ShuttingDown);
        }
        match self.phase {
            Phase::Idle | Phase::Backoff { .. } => {
                self.recovery.reset();
                self.spawn(Cause::Requested { by })
                    .await
                    .map_err(ControlError::Launch)
            }
            Phase::PreLaunch { .. } if self.pre_launch_ending() => Err(ControlError::Stopping),
            Phase::Notice { .. } | Phase::Quitting { .. } => Err(ControlError::Stopping),
            Phase::PreLaunch { .. } | Phase::Running | Phase::Ending { .. } => {
                Err(ControlError::AlreadyRunning)
            }
        }
    }

    /// Whether the pre-launch work under way was told to end for something else than its spawn.
    fn pre_launch_ending(&self) -> bool {
        self.pre_launch
            .as_ref()
            .is_some_and(|work| work.instead.is_some())
    }

    /// Stops the server, unless a stop is already under way: then nothing changes and it says so,
    /// rather than have someone named for a stop they did not make.
    fn stop(&mut self, by: String) -> Result<(), ControlError> {
        if self.shutdown.is_some() {
            return Err(ControlError::ShuttingDown);
        }
        match self.phase {
            Phase::Idle if self.state == State::CrashLoop => {
                self.set_state(State::Stopped, Cause::Requested { by });
                Ok(())
            }
            Phase::Idle => Err(ControlError::NotRunning),
            Phase::Backoff { .. } => {
                self.phase = Phase::Idle;
                self.set_state(State::Stopped, Cause::Requested { by });
                Ok(())
            }
            // The work is told to end; the server stays stopped once it has.
            Phase::PreLaunch { .. } => {
                let Some(work) = &mut self.pre_launch else {
                    return Err(ControlError::NotRunning);
                };
                if matches!(work.instead, Some(Instead::Stop { .. } | Instead::Exit)) {
                    return Err(ControlError::Stopping);
                }
                work.tell_to_end(Instead::Stop { by: by.clone() });
                self.phase = Phase::PreLaunch { deadline: None };
                self.set_state(State::Stopping, Cause::Requested { by });
                Ok(())
            }
            Phase::Notice { .. } | Phase::Quitting { .. } | Phase::Ending { .. } => {
                let Some(run) = &mut self.run else {
                    return Err(ControlError::NotRunning);
                };
                let restarting = match run.after {
                    // A stop, or a console `quit`, already ends the run as a stop.
                    Some(After::Stop | After::Exit) => return Err(ControlError::Stopping),
                    Some(After::Restart { .. }) => true,
                    // A run ending after a fatal error or a hang: it is not restarted now.
                    None => false,
                };
                run.after = Some(After::Stop);
                // The restart's stop goes on as a stop, made by whoever asked for it. Resources
                // were told of a restart: until `quit` is out, they hear that the server stays
                // down, with the time they have left.
                if restarting && self.state == State::Stopping {
                    if let Phase::Notice { quit_at, .. } = self.phase {
                        let left = quit_at.saturating_duration_since(Instant::now());
                        run.system(SystemLine::Bridge(BridgeEvent::ShuttingDown {
                            delay_ms: u64::try_from(left.as_millis()).unwrap_or(u64::MAX),
                            reason: StopReason::Stop,
                            by: by.clone(),
                        }));
                        self.phase = Phase::Notice {
                            quit_at,
                            reason: StopReason::Stop,
                        };
                    }
                    self.set_state(State::Stopping, Cause::Requested { by });
                }
                Ok(())
            }
            Phase::Running => {
                self.begin_stop(StopReason::Stop, Cause::Requested { by }, After::Stop, false);
                Ok(())
            }
        }
    }

    async fn restart(&mut self, by: String) -> Result<(), ControlError> {
        if self.shutdown.is_some() {
            return Err(ControlError::ShuttingDown);
        }
        match self.phase {
            Phase::Idle | Phase::Backoff { .. } => self.start(by).await,
            Phase::PreLaunch { .. } => self.restart_pre_launch(Cause::Restart { by }),
            Phase::Notice { .. } | Phase::Quitting { .. } => Err(ControlError::Stopping),
            Phase::Ending { .. } => {
                if let Some(run) = &mut self.run {
                    run.after = Some(After::Restart {
                        cause: Cause::Restart { by },
                    });
                }
                Ok(())
            }
            Phase::Running => {
                let cause = Cause::Restart { by };
                self.begin_stop(
                    StopReason::Restart,
                    cause.clone(),
                    After::Restart { cause },
                    false,
                );
                Ok(())
            }
        }
    }

    fn restart_scheduled(&mut self, by: String) -> Result<(), ControlError> {
        if self.shutdown.is_some() {
            return Err(ControlError::ShuttingDown);
        }
        match self.phase {
            Phase::Running => {
                let cause = Cause::Scheduled { by };
                self.begin_stop(
                    StopReason::Scheduled,
                    cause.clone(),
                    After::Restart { cause },
                    false,
                );
                Ok(())
            }
            // The spawn would go by the launch prepared when the phase began, before the schedule
            // handed over the build this restart applies.
            Phase::PreLaunch { .. } => self.restart_pre_launch(Cause::Scheduled { by }),
            Phase::Notice { .. } | Phase::Quitting { .. } => Err(ControlError::Stopping),
            // A server stopped, or between a failure and its recovery, starts with the launch of
            // now anyway, and the schedule never starts one.
            Phase::Idle | Phase::Backoff { .. } | Phase::Ending { .. } => Err(ControlError::NotRunning),
        }
    }

    /// A restart during the pre-launch phase, as for a server that runs: the work is told to end,
    /// then the start begins anew, with fresh work and the launch configured by then.
    fn restart_pre_launch(&mut self, cause: Cause) -> Result<(), ControlError> {
        let Some(work) = &mut self.pre_launch else {
            return Err(ControlError::NotRunning);
        };
        if work.instead.is_some() {
            return Err(ControlError::Stopping);
        }
        work.tell_to_end(Instead::Restart { cause: cause.clone() });
        self.phase = Phase::PreLaunch { deadline: None };
        self.set_state(State::Stopping, cause);
        Ok(())
    }

    fn announce(&self, event: BridgeEvent) -> Result<(), ControlError> {
        self.to_bridge(SystemLine::Bridge(event))
    }

    fn to_bridge(&self, line: SystemLine) -> Result<(), ControlError> {
        match &self.run {
            Some(run) if matches!(self.phase, Phase::Running) && run.bridge_ready => {
                run.system(line);
                Ok(())
            }
            _ => Err(ControlError::NotRunning),
        }
    }

    /// The bridge is Observer's own: it ends with the server, and by nothing asked for here.
    fn resource(&self, verb: ResourceVerb, name: ResourceName, by: String) -> Result<Written, ControlError> {
        let bridge = self
            .run
            .as_ref()
            .is_some_and(|run| name.as_str().eq_ignore_ascii_case(&run.bridge));
        if bridge {
            return Err(ControlError::Bridge);
        }
        self.asked(SystemLine::Resource(verb, name), by)
    }

    /// Writes a line of Observer's that `by` asked for, on a running server whose bridge has
    /// reported: nothing is queued for a server still starting or already stopping.
    fn asked(&self, line: SystemLine, by: String) -> Result<Written, ControlError> {
        match (&self.run, self.phase) {
            (Some(run), Phase::Running) if run.bridge_ready => {
                Ok(run.stdin.send(Write::Asked { line, actor: by }))
            }
            (Some(_), Phase::Notice { .. } | Phase::Quitting { .. }) => Err(ControlError::Stopping),
            _ => Err(ControlError::NotRunning),
        }
    }

    fn command(&mut self, command: UserCommand, actor: String) -> Result<Written, ControlError> {
        let Some(run) = &mut self.run else {
            return Err(ControlError::NotRunning);
        };
        let quits = command.quits() && run.after.is_none();
        if quits {
            run.after = Some(After::Stop);
        }
        let written = run.stdin.send(Write::User {
            command,
            actor: actor.clone(),
        });
        if quits {
            // FXServer quits by itself; Observer only makes sure it does.
            self.set_state(State::Stopping, Cause::ConsoleQuit { by: actor });
            self.phase = Phase::Quitting {
                kill_at: Instant::now() + self.settings.stop.quit,
            };
        }
        Ok(written)
    }

    fn shutdown(&mut self, shortened: bool, reason: StopReason) {
        let shortened = shortened || self.shutdown == Some(true);
        self.shutdown = Some(shortened);
        let now = Instant::now();
        match self.phase {
            Phase::Idle => self.done = true,
            Phase::Backoff { .. } => {
                self.phase = Phase::Idle;
                self.set_state(State::Stopped, Cause::Shutdown);
                self.done = true;
            }
            // The supervisor ends once the work has: whatever it writes is never cut short by it.
            Phase::PreLaunch { .. } => {
                let Some(work) = &mut self.pre_launch else {
                    return;
                };
                // A stop under way already says the server stays down.
                let announced = matches!(work.instead, Some(Instead::Stop { .. } | Instead::Exit));
                work.tell_to_end(Instead::Exit);
                self.phase = Phase::PreLaunch { deadline: None };
                if !announced {
                    self.set_state(State::Stopping, Cause::Shutdown);
                }
            }
            Phase::Running => self.begin_stop(reason, Cause::Shutdown, After::Exit, shortened),
            Phase::Notice { quit_at, reason } => {
                let Some(run) = &mut self.run else {
                    return;
                };
                let restarting = matches!(run.after, Some(After::Restart { .. }));
                run.after = Some(After::Exit);
                // Resources were told of a restart: they hear that the server stays down, with
                // the time they have left, as for a stop that overtakes a restart.
                let reason = if restarting {
                    let left = if shortened {
                        Duration::ZERO
                    } else {
                        quit_at.saturating_duration_since(now)
                    };
                    run.system(SystemLine::Bridge(BridgeEvent::ShuttingDown {
                        delay_ms: u64::try_from(left.as_millis()).unwrap_or(u64::MAX),
                        reason: StopReason::Shutdown,
                        by: "observer".to_owned(),
                    }));
                    self.phase = Phase::Notice {
                        quit_at,
                        reason: StopReason::Shutdown,
                    };
                    self.set_state(State::Stopping, Cause::Shutdown);
                    StopReason::Shutdown
                } else {
                    reason
                };
                if shortened {
                    self.quit(reason, self.settings.stop.shortened);
                }
            }
            Phase::Quitting { kill_at } | Phase::Ending { kill_at } => {
                if let Some(run) = &mut self.run {
                    run.after = Some(After::Exit);
                }
                if shortened {
                    let kill_at = kill_at.min(now + self.settings.stop.shortened);
                    self.phase = match self.phase {
                        Phase::Quitting { .. } => Phase::Quitting { kill_at },
                        _ => Phase::Ending { kill_at },
                    };
                }
            }
        }
    }

    /// The stop sequence: the bridge event, the notice delay when the bridge is there to pass it
    /// on and time allows, then `quit`.
    fn begin_stop(&mut self, reason: StopReason, cause: Cause, after: After, shortened: bool) {
        let Some(run) = &mut self.run else {
            return;
        };
        let by = match &cause {
            Cause::Requested { by } | Cause::Restart { by } | Cause::Scheduled { by } => by.clone(),
            _ => "observer".to_owned(),
        };
        run.after = Some(after);
        let notice = if shortened {
            Duration::ZERO
        } else {
            self.settings.stop.notice
        };
        let bridge_ready = run.bridge_ready;
        if bridge_ready {
            run.system(SystemLine::Bridge(BridgeEvent::ShuttingDown {
                delay_ms: u64::try_from(notice.as_millis()).unwrap_or(u64::MAX),
                reason,
                by,
            }));
        }
        self.set_state(State::Stopping, cause);
        if bridge_ready && !notice.is_zero() {
            self.phase = Phase::Notice {
                quit_at: Instant::now() + notice,
                reason,
            };
        } else {
            let wait = if shortened {
                self.settings.stop.shortened
            } else {
                self.settings.stop.quit
            };
            self.quit(reason, wait);
        }
    }

    fn quit(&mut self, reason: StopReason, wait: Duration) {
        if let Some(run) = &self.run {
            run.system(SystemLine::Quit(reason));
        }
        self.phase = Phase::Quitting {
            kill_at: Instant::now() + wait,
        };
    }

    fn kill(&mut self) {
        let Some(run) = &mut self.run else {
            return;
        };
        run.killed = true;
        if let Err(error) = run.tree.kill() {
            error!(%error, generation = run.generation, "cannot kill FXServer's process tree");
        }
        // Try again if the exit does not follow.
        self.phase = Phase::Ending {
            kill_at: Instant::now() + TREE_KILL,
        };
    }

    /// Pictures the hung server for the crash record, before the kill. Requests wait behind it, up
    /// to its time limit; a console close kills the tree meanwhile anyway.
    async fn snapshot(&mut self, limits: HangSnapshotLimits) {
        let Some(run) = &self.run else {
            return;
        };
        let data_dir = self.settings.data_dir.clone();
        let started = Instant::now();
        // The data folder itself may not exist yet; its volume does.
        let free = tokio::task::spawn_blocking(move || {
            data_dir
                .ancestors()
                .find_map(|dir| observer_platform::available_space(dir).ok())
        })
        .await
        .ok()
        .flatten();
        let (path, bytes, failure, removed) = if free.is_some_and(|free| free < limits.min_free) {
            self.note("no snapshot of the hung server: its data volume has less than 1 GiB free");
            (None, 0, Some(SnapshotFailure::LowDisk), Vec::new())
        } else {
            let (tree, pid, generation) = (Arc::clone(&run.tree), run.pid, run.generation);
            let (dir, dumps) = (self.settings.data_dir.join("crashes"), self.settings.dumps);
            let abandoned = Arc::new(AtomicBool::new(false));
            let late = Arc::clone(&abandoned);
            let writing = tokio::task::spawn_blocking(move || {
                let (path, bytes) = crashes::snapshot(&tree, pid, &dir, generation, &late)?;
                let removed = crashes::trim(&dir, dumps);
                Ok::<_, std::io::Error>((path, bytes, removed))
            });
            match tokio::time::timeout(limits.timeout, writing).await {
                Ok(Ok(Ok((path, bytes, removed)))) => {
                    let kept = !removed.contains(&path);
                    self.note(&format!(
                        "snapshot of the hung server saved to {} ({bytes} bytes)",
                        path.display()
                    ));
                    (kept.then_some(path), bytes, None, removed)
                }
                Ok(Ok(Err(error))) => {
                    warn!(%error, "cannot take a snapshot of the hung server");
                    self.note(&format!("cannot take a snapshot of the hung server: {error}"));
                    (None, 0, Some(SnapshotFailure::Failed), Vec::new())
                }
                Ok(Err(error)) => {
                    warn!(%error, "the hang snapshot failed");
                    (None, 0, Some(SnapshotFailure::Failed), Vec::new())
                }
                Err(_) => {
                    abandoned.store(true, Ordering::Relaxed);
                    self.note("the snapshot of the hung server took too long; killing without it");
                    (None, 0, Some(SnapshotFailure::Timeout), Vec::new())
                }
            }
        };
        self.emit(EventKind::HangSnapshot {
            path,
            bytes,
            took_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            failure,
            removed,
        });
    }

    async fn tick(&mut self) {
        let now = Instant::now();
        self.last_tick = now;
        match self.phase {
            Phase::PreLaunch {
                deadline: Some(deadline),
            } if now >= deadline => self.pre_launch_overdue(),
            Phase::Notice { quit_at, reason } if now >= quit_at => {
                self.quit(reason, self.settings.stop.quit);
            }
            Phase::Quitting { kill_at } | Phase::Ending { kill_at } if now >= kill_at => {
                if self.run.is_some() {
                    self.note("FXServer did not end in time; killing its process tree");
                }
                self.kill();
            }
            Phase::Backoff { respawn_at } if now >= respawn_at => {
                // A failed respawn backs off again, which the events already say.
                let _ = self.spawn(Cause::Recovery).await;
            }
            Phase::Running => self.weigh_health(now).await,
            _ => {}
        }
        if let Some(ready_at) = self.run.as_ref().and_then(|run| run.ready_at) {
            self.recovery.up_for(now.saturating_duration_since(ready_at));
        }
    }

    /// The launch settings given last, rather than the spawn's: turning hang detection off, or
    /// giving a big first start more time, must reach the server that is starting now.
    fn health_limits(&self) -> HealthLimits {
        match &self.settings.launch {
            Some(launch) => self.settings.health.under(&launch.settings),
            None => self.settings.health,
        }
    }

    fn extend_start(&mut self, by: &str) -> Result<(), ControlError> {
        let limits = self.health_limits();
        let now = Instant::now();
        match &mut self.run {
            Some(run) if self.state == State::Starting && !run.health.cfg_ran() => {
                let stuck = run.health.stuck_at(&limits).map_or(now, |at| at.max(now));
                run.health.extend(stuck + START_EXTENSION);
            }
            _ => return Err(ControlError::NotStarting),
        }
        self.note(&format!(
            "{by} gave this start {} more minutes before it is taken for stuck",
            START_EXTENSION.as_secs() / 60
        ));
        self.follow_start(now, &limits);
        Ok(())
    }

    /// Publishes how far the start has come, when that changed since it was last published.
    fn follow_start(&mut self, now: Instant, limits: &HealthLimits) {
        let Some(run) = &mut self.run else {
            return;
        };
        if self.state != State::Starting {
            return;
        }
        let stuck = run.health.stuck_at(limits);
        let stuck_at_ms = match &run.start {
            Some((status, from)) if *from == stuck => status.stuck_at_ms,
            _ => stuck.map(|at| {
                let ahead = i64::try_from(at.saturating_duration_since(now).as_millis()).unwrap_or(i64::MAX);
                let behind = i64::try_from(now.saturating_duration_since(at).as_millis()).unwrap_or(i64::MAX);
                now_ms().saturating_add(ahead).saturating_sub(behind)
            }),
        };
        let status = StartStatus {
            progress: run.progress.snapshot(),
            stuck_at_ms,
        };
        if run
            .start
            .as_ref()
            .is_some_and(|(published, _)| *published == status)
        {
            return;
        }
        run.start = Some((status, stuck));
        self.publish_status();
    }

    async fn weigh_health(&mut self, now: Instant) {
        let limits = self.health_limits();
        let Some(run) = &mut self.run else {
            return;
        };
        if !run.health.cfg_ran()
            && let Some(at) = run.progress.moved()
        {
            run.health.progressed(at);
        }
        let verdict = run.health.verdict(now, &limits);
        let newly_ready = !matches!(verdict, Verdict::Starting | Verdict::Hung(_)) && run.ready_at.is_none();
        if newly_ready {
            run.ready_at = Some(now);
        }
        let crashes = run.layout.crashes();
        if newly_ready {
            self.recovery.ready();
        }
        match verdict {
            Verdict::Starting => self.follow_start(now, &limits),
            Verdict::Healthy => match self.state {
                State::Starting => self.set_state(State::Online, Cause::Ready),
                State::Degraded => self.set_state(State::Online, Cause::Recovered),
                _ => {}
            },
            Verdict::Degraded(problems) => {
                if self.state == State::Starting {
                    self.set_state(State::Online, Cause::Ready);
                }
                if self.state != State::Degraded {
                    self.problems.clone_from(&problems);
                    self.set_state(State::Degraded, Cause::Unhealthy { problems });
                } else if self.problems != problems {
                    self.problems.clone_from(&problems);
                    self.publish_status();
                    self.emit(EventKind::Health { problems });
                }
            }
            Verdict::Hung(cause) => {
                let dumping =
                    tokio::task::spawn_blocking(move || crashes::being_written(&crashes, DUMP_ACTIVITY))
                        .await
                        .unwrap_or(false);
                if dumping {
                    return;
                }
                let why = match &self.run {
                    Some(run) if !run.health.cfg_ran() => stalled_text(run.progress.last(), now),
                    _ => hang_text(cause).to_owned(),
                };
                if let Some(run) = &mut self.run {
                    run.end(Ending::Hang { cause });
                }
                self.set_state(State::Hung, Cause::Hang { cause });
                match self.settings.hang_snapshot {
                    Some(limits) => {
                        self.note(&format!(
                            "FXServer is not responding ({why}); writing {SNAPSHOT} of the hung server before killing it"
                        ));
                        self.snapshot(limits).await;
                        self.note("killing FXServer's process tree");
                    }
                    None => self.note(&format!(
                        "FXServer is not responding ({why}); killing its process tree"
                    )),
                }
                self.kill();
            }
        }
    }

    fn trace(&mut self, trace: Trace) {
        let Some(run) = self.run.as_mut().filter(|run| run.generation == trace.generation) else {
            return;
        };
        if let Some(report) = hook::Report::from_trace(&trace.kind) {
            // The bridge is Observer's own: what it counts is not the server's.
            if report.resource != run.bridge {
                let _ = self.counts.send(HookReport {
                    generation: trace.generation,
                    report,
                });
            }
            return;
        }
        if let Some(kind) = PlayerReportKind::from_trace(&trace.kind, &run.bridge) {
            let _ = self.players.send(PlayerReport {
                generation: trace.generation,
                at_ms: now_ms(),
                received: Instant::now(),
                kind,
            });
            return;
        }
        let event = match Report::from_trace(&trace.kind, &run.bridge) {
            Some(Report::Ready { .. } | Report::Heartbeat) => {
                run.health.heartbeat(Instant::now());
                run.bridge_ready = true;
                return;
            }
            Some(Report::Resources {
                started,
                events,
                dropped,
            }) => resources_started(&self.resources, started, events, dropped),
            Some(Report::ResourceList {
                number,
                page,
                last,
                resources,
            }) => {
                let mut held = self.resources.lock().unwrap_or_else(PoisonError::into_inner);
                let Some((_, book)) = &mut *held else {
                    return;
                };
                if !book.page(number, page, last, resources, &run.bridge, now_ms()) {
                    return;
                }
                EventKind::ResourceList {
                    resources: book.resources().len(),
                }
            }
            Some(Report::Heap {
                heap_used,
                heap_total,
                rss,
                external,
            }) => EventKind::Heap {
                heap_used,
                heap_total,
                rss,
                external,
            },
            None => match trace.kind {
                TraceKind::Hitch { thread, ms } => EventKind::Hitch { thread, ms },
                TraceKind::WatchdogBark {
                    thread,
                    seconds,
                    stack,
                } => {
                    run.health.bark();
                    warn!(%thread, seconds, "FXServer's watchdog reports a stuck thread");
                    EventKind::WatchdogBark {
                        thread,
                        seconds,
                        stack,
                    }
                }
                TraceKind::WatchdogBite { thread } => EventKind::WatchdogBite { thread },
                TraceKind::NucleusConnected { url } => EventKind::NucleusConnected { url },
                TraceKind::BindError { address } => {
                    self.end_run(Ending::BindError { address });
                    return;
                }
                TraceKind::Probe | TraceKind::Script { .. } | TraceKind::Unknown(_) => return,
            },
        };
        self.emit(event);
    }

    fn signal(&mut self, generation: u32, signal: Signal) {
        if self.run.as_ref().is_none_or(|run| run.generation != generation) {
            return;
        }
        let ending = match signal {
            Signal::LicenceRejected(message) => Ending::LicenceRejected { message },
            Signal::Fatal(message) => match message
                .strip_prefix("Could not bind on ")
                .and_then(|rest| rest.split_whitespace().next())
            {
                Some(address) => Ending::BindError {
                    address: address.to_owned(),
                },
                None => Ending::Fatal { message },
            },
        };
        self.end_run(ending);
    }

    /// FXServer said it cannot go on. It usually exits within seconds, but on Windows it may stay
    /// up behind an error dialog, so the tree is killed if it does not.
    fn end_run(&mut self, ending: Ending) {
        let Some(run) = &mut self.run else {
            return;
        };
        run.end(ending);
        if matches!(self.phase, Phase::Running) {
            self.phase = Phase::Ending {
                kill_at: Instant::now() + self.settings.stop.fatal_exit,
            };
        }
    }

    /// Starts FXServer for `cause`. With pre-launch work, its phase begins here and the spawn
    /// follows once the work ended: the start is taken on, and a later failure goes to the status.
    /// A launch that cannot be prepared fails at once either way.
    async fn spawn(&mut self, cause: Cause) -> Result<(), SpawnError> {
        let (server, prepared) = match self.prepare_launch().await {
            Ok(prepared) => prepared,
            Err(error) => {
                self.spawn_failed(&cause, &error);
                return Err(error);
            }
        };
        let (ready, scan) = Ready::split(prepared, server.settings.clone());
        match self.settings.pre_launch.clone() {
            Some(work) => {
                self.begin_pre_launch(work, cause, server, ready, scan);
                Ok(())
            }
            None => self.spawn_ready(cause, ready, false).await,
        }
    }

    /// Spawns a prepared launch. `announced` when the pre-launch phase already took the generation
    /// and set the state to starting with the cause.
    async fn spawn_ready(&mut self, cause: Cause, ready: Ready, announced: bool) -> Result<(), SpawnError> {
        match self.launch(ready, announced).await {
            Ok((run, probe, warnings, launch)) => {
                let pid = run.pid;
                let build = run.layout.build().to_path_buf();
                self.readings.send_replace(Readings {
                    run: Some(RunReadings {
                        generation: run.generation,
                        started_ms: run.started_ms,
                        probe,
                        tree: Arc::downgrade(&run.tree),
                        dynamic: None,
                        perf: None,
                        players: None,
                    }),
                });
                self.run = Some(run);
                self.port = Some(probe.port());
                self.phase = Phase::Running;
                self.last_tick = Instant::now();
                if announced {
                    self.publish_status();
                } else {
                    self.set_state(State::Starting, cause);
                }
                self.note(&format!("FXServer started (pid {pid})"));
                self.emit(EventKind::Spawned {
                    pid,
                    probe,
                    build,
                    launch: Box::new(launch),
                });
                for finding in warnings {
                    warn!(
                        kind = ?finding.kind,
                        file = finding.file,
                        line = finding.line,
                        "the server cfg works against Observer"
                    );
                    self.emit(EventKind::CfgWarning { finding });
                }
                Ok(())
            }
            Err(error) => {
                self.spawn_failed(&cause, &error);
                Err(error)
            }
        }
    }

    fn spawn_failed(&mut self, cause: &Cause, error: &SpawnError) {
        error!(%error, "cannot start FXServer");
        self.note(&format!("cannot start FXServer: {error}"));
        let failed = Cause::SpawnFailed {
            reason: error.failure,
        };
        if *cause == Cause::Recovery {
            // What stops a respawn is often brief (a cfg being saved, a scan holding the
            // executable), so it is one more failure for the breaker to count.
            self.failed(failed);
        } else {
            self.phase = Phase::Idle;
            self.set_state(State::Stopped, failed);
        }
    }

    /// Starts the pre-launch work as a task and shows the server as starting, with the cause the
    /// spawn will go by. The start takes its generation here, so that its state, the work's lines
    /// and the run that follows all carry the one the statistics know the run by; a start that
    /// never spawns leaves it unused.
    fn begin_pre_launch(
        &mut self,
        work: Arc<dyn PreLaunch>,
        cause: Cause,
        server: ServerLaunch,
        ready: Ready,
        scan: cfg::Scan,
    ) {
        let generation = self.take_generation();
        let cancel = Arc::new(AtomicBool::new(false));
        let request = PreLaunchRequest {
            launch: server,
            layout: ready.layout.clone(),
            scan,
            cause: cause.clone(),
            generation,
            cancel: Arc::clone(&cancel),
        };
        // Called inside the task, so that a panic in the call itself ends there too.
        let task = tokio::spawn(async move { work.run(request).await });
        self.pre_launch = Some(PreLaunching {
            task,
            cancel,
            generation,
            cause: cause.clone(),
            ready,
            instead: None,
        });
        self.phase = Phase::PreLaunch {
            deadline: Instant::now().checked_add(self.settings.pre_launch_limit),
        };
        self.set_state(State::Starting, cause);
    }

    /// The work ran out of time: it is told to end, and the spawn waits until it has.
    fn pre_launch_overdue(&mut self) {
        let Some(work) = &self.pre_launch else {
            return;
        };
        work.cancel.store(true, Ordering::Relaxed);
        let generation = work.generation;
        self.phase = Phase::PreLaunch { deadline: None };
        let limit = self.settings.pre_launch_limit;
        warn!(?limit, "the pre-launch work ran out of time; it was told to end");
        self.note_as(
            SUPERVISOR,
            generation,
            &format!(
                "the work before the start took more than {:.1} s; FXServer starts once it has ended",
                limit.as_secs_f64()
            ),
        );
    }

    /// The pre-launch work ended: its lines go to the console, then the spawn, or what came
    /// instead of it.
    async fn pre_launched(&mut self, ended: Result<PreLaunchResult, JoinError>) {
        let Some(work) = self.pre_launch.take() else {
            return;
        };
        let PreLaunching {
            generation,
            cause,
            ready,
            instead,
            ..
        } = work;
        match ended {
            Ok(Ok(lines)) => {
                for line in lines {
                    self.note_as(TOOLS, generation, &line);
                }
            }
            Ok(Err(error)) => {
                warn!(%error, "the pre-launch work failed");
                self.note_as(TOOLS, generation, &format!("failed before the start: {error}"));
            }
            Err(error) => {
                error!(%error, "the pre-launch work failed");
                self.note_as(TOOLS, generation, "failed before the start, unexpectedly");
            }
        }
        self.phase = Phase::Idle;
        match instead {
            None => {
                // A failure goes to the status and the console, which is all a start taken on has.
                let _ = self.spawn_ready(cause, ready, true).await;
            }
            Some(Instead::Stop { by }) => self.set_state(State::Stopped, Cause::Requested { by }),
            Some(Instead::Restart { cause }) => {
                let _ = self.spawn(cause).await;
            }
            Some(Instead::Exit) => {
                self.set_state(State::Stopped, Cause::Shutdown);
                self.done = true;
            }
        }
    }

    /// Prepares the configured launch, with the launch itself for the pre-launch work.
    async fn prepare_launch(&self) -> Result<(ServerLaunch, launch::Prepared), SpawnError> {
        let Some(server) = self.settings.launch.clone() else {
            return Err(SpawnError::new(
                SpawnFailure::NotConfigured,
                "no launch settings are configured",
            ));
        };
        let prepared = prepare(server.clone(), self.settings.flavour).await?;
        Ok((server, prepared))
    }

    /// Takes the generation of a coming spawn: the events and lines that follow carry it, and the
    /// trace decoders of an earlier one fall silent.
    fn take_generation(&mut self) -> u32 {
        self.generation += 1;
        self.current.send_replace(self.generation);
        self.generation
    }

    /// Spawns FXServer. `taken` when the pre-launch phase took the generation as it began.
    async fn launch(
        &mut self,
        prepared: Ready,
        taken: bool,
    ) -> Result<(Run, SocketAddr, Vec<CfgFinding>, LaunchSettings), SpawnError> {
        let layout = prepared.layout.clone();
        let hook = self.install(&layout, &prepared.bridge).await?;
        let credentials = Credentials::generate().map_err(|error| {
            let message = format!("cannot generate the launch credentials: {error}");
            SpawnError::new(SpawnFailure::Spawn, message)
        })?;

        let generation = self.generation_of(taken, hook);
        let spawned = observer_platform::spawn(&prepared.launch)
            .map_err(|error| SpawnError::new(SpawnFailure::Spawn, error.to_string()))?;
        let tree = Arc::new(spawned.tree);
        let fxserver_startup = tree
            .describe(spawned.pid)
            .and_then(|main| main.startup_variables)
            .unwrap_or_default();
        #[cfg(unix)]
        if let Err(error) = self.pid_file().record(&tree) {
            warn!(%error, "cannot record FXServer's process group; a crash of Observer could leave it running");
        }

        let probe = self
            .settings
            .probe
            .or(prepared.endpoint)
            .unwrap_or_else(|| SocketAddr::from((Ipv4Addr::LOCALHOST, DEFAULT_PORT)));
        let cancel = CancellationToken::new();
        let (stdin, stdin_task) = StdinWriter::start(spawned.stdin, generation, Arc::clone(&self.console));
        let progress = self.progress_of(prepared.plan);
        *self.resources.lock().unwrap_or_else(PoisonError::into_inner) = Some((generation, Book::default()));
        let mut tasks = vec![
            tokio::spawn(read_console(
                spawned.stdout,
                LineKind::Stdout,
                generation,
                Arc::clone(&self.console),
                self.signals_tx.clone(),
                Arc::clone(&progress),
            )),
            tokio::spawn(read_console(
                spawned.stderr,
                LineKind::Stderr,
                generation,
                Arc::clone(&self.console),
                self.signals_tx.clone(),
                Arc::clone(&progress),
            )),
            tokio::spawn(trace::read(
                spawned.traces,
                TraceDecoder::new(generation, self.current.subscribe()),
                self.traces_tx.clone(),
            )),
            tokio::spawn(follow_tree(TreeFollower {
                changes: spawned.changes,
                tree: Arc::downgrade(&tree),
                context: TreeContext::new(&layout, spawned.pid, fxserver_startup),
                generation,
                bus: self.processes.clone(),
                cancel: cancel.clone(),
                drop_next: self.settings.drop_first_tree_change,
                running: HashMap::new(),
            })),
        ];
        tasks.extend(health::start(
            probe,
            &credentials,
            generation,
            self.probes_tx.clone(),
            &cancel,
        ));
        for line in credentials.lines() {
            drop(stdin.send(Write::System(line)));
        }

        let run = Run {
            generation,
            pid: spawned.pid,
            started_ms: now_ms(),
            players: None,
            build: None,
            game_build: None,
            layout,
            bridge: prepared.bridge,
            exit: spawned.exit,
            tree,
            stdin,
            stdin_task,
            cancel,
            tasks,
            health: Health::new(Instant::now()),
            progress,
            start: None,
            bridge_ready: false,
            maintenance_name: None,
            ready_at: None,
            after: None,
            ending: None,
            killed: false,
        };
        Ok((run, probe, prepared.warnings, prepared.settings))
    }

    /// What a spawn says of its start from now on, kept as the latest start's.
    fn progress_of(&self, plan: start::Plan) -> Arc<Progress> {
        let spawned_ms = now_ms();
        let progress = Arc::new(Progress::new(plan, Instant::now(), spawned_ms));
        *self.latest_start.lock().unwrap_or_else(PoisonError::into_inner) =
            Some((spawned_ms, Arc::clone(&progress)));
        progress
    }

    /// The generation of the spawn about to happen, taken now unless the pre-launch phase took
    /// it as it began. What became of the hook is kept for it, and the console says why a start
    /// that should count counts nothing.
    fn generation_of(&mut self, taken: bool, hook: Hook) -> u32 {
        let generation = if taken {
            self.generation
        } else {
            self.take_generation()
        };
        let hooked = match hook {
            Hook::Out => None,
            Hook::In => Some(None),
            Hook::Refused(reason) => {
                warn!(%reason, "this start counts nothing: the build's scheduler was left as it was");
                let note = format!("Debug counts nothing in this start: {reason}");
                self.note_as(SUPERVISOR, generation, &note);
                Some(Some(reason))
            }
        };
        *self.latest_hook.lock().unwrap_or_else(PoisonError::into_inner) =
            hooked.map(|refused| Hooked { generation, refused });
        generation
    }

    /// Writes the bridge into the build as `bridge`, with the hook and its two lines in the
    /// scheduler when the start counts and without them when it does not, and moves away the
    /// dumps of a run Observer did not see end, such as before Observer itself crashed. Answers,
    /// for a start that counts, why the scheduler was left as it was: the server starts all the
    /// same.
    async fn install(&mut self, layout: &Layout, bridge: &str) -> Result<Hook, SpawnError> {
        let counting = self.settings.counting.as_ref().is_some_and(Counting::asked);
        let installing = layout.clone();
        let bridge = bridge.to_owned();
        let dumps = (
            layout.crashes(),
            self.settings.data_dir.join("crashes"),
            self.settings.dumps,
        );
        let (installed, moved) = tokio::task::spawn_blocking(move || {
            let installed = bridge::install(&installing, &bridge)
                .map(|()| hook::install(&installing, &bridge, counting).err());
            (installed, crashes::collect(&dumps.0, &dumps.1, dumps.2))
        })
        .await
        .map_err(|error| SpawnError::new(SpawnFailure::Spawn, error.to_string()))?;
        let refused = installed.map_err(|error| {
            let message = format!("cannot install the observer bridge: {error}");
            SpawnError::new(SpawnFailure::Bridge, message)
        })?;
        self.report_dumps(moved);
        Ok(match refused {
            _ if !counting => Hook::Out,
            None => Hook::In,
            Some(refusal) => Hook::Refused(refusal.to_string()),
        })
    }

    async fn exited(&mut self, exit: Exit) {
        let Some(run) = &self.run else {
            return;
        };
        let generation = run.generation;
        let expected = run.after.is_some();
        // On Windows a killed tree exits with 1, which is also FXServer's own exit after `quit`.
        let description = if run.killed {
            format!("killed by Observer ({})", exit::raw(exit))
        } else {
            exit::describe(exit)
        };
        let (code, signal) = match exit {
            Exit::Code(code) => (Some(code), None),
            Exit::Signal(signal) => (None, Some(signal)),
            Exit::Unknown => (None, None),
        };
        info!(generation, %description, expected, "FXServer exited");
        self.note(&format!("FXServer exited: {description}"));
        self.emit(EventKind::Exited {
            code,
            signal,
            description: description.clone(),
            expected,
        });
        self.wind_down().await;
        // Its output is all read now, and what it said last may explain the exit: FXServer
        // reports a taken port or a fatal error and is gone a moment later.
        while let Ok(trace) = self.traces.try_recv() {
            self.trace(trace);
        }
        while let Ok((generation, signal)) = self.signals.try_recv() {
            self.signal(generation, signal);
        }
        let Some(run) = self.run.take() else {
            return;
        };
        *self.resources.lock().unwrap_or_else(PoisonError::into_inner) = None;
        self.readings.send_replace(Readings::default());
        self.collect_dumps(&run.layout).await;
        let Run { after, ending, .. } = run;

        if let Some(after) = after {
            match after {
                After::Stop => {
                    self.phase = Phase::Idle;
                    self.set_state(State::Stopped, Cause::Exited { description });
                }
                After::Restart { cause } => {
                    let _ = self.spawn(cause).await;
                }
                After::Exit => {
                    self.phase = Phase::Idle;
                    self.set_state(State::Stopped, Cause::Shutdown);
                    self.done = true;
                }
            }
            return;
        }
        match ending {
            Some(Ending::LicenceRejected { message }) => {
                self.phase = Phase::Idle;
                self.note("the licence key was refused; the server stays stopped until it is started again");
                self.set_state(State::Stopped, Cause::LicenceRejected { message });
            }
            Some(Ending::BindError { address }) => {
                let delay = self.recovery.bind_error();
                self.set_state(State::Crashed, Cause::BindError { address });
                self.back_off(delay);
            }
            Some(Ending::Hang { cause }) => self.failed(Cause::Hang { cause }),
            Some(Ending::Fatal { message }) => self.failed(Cause::FatalError { message }),
            None => self.failed(Cause::Exited { description }),
        }
    }

    fn failed(&mut self, cause: Cause) {
        self.set_state(State::Crashed, cause);
        match self.recovery.failure(Instant::now()) {
            Next::RestartIn(delay) => self.back_off(delay),
            Next::CrashLoop { failures } => {
                self.phase = Phase::Idle;
                self.note(&format!(
                    "{failures} failures in a row; automatic restarts are off until the server is started again"
                ));
                self.set_state(State::CrashLoop, Cause::CrashLoop { failures });
            }
        }
    }

    fn back_off(&mut self, delay: Duration) {
        self.phase = Phase::Backoff {
            respawn_at: Instant::now() + delay,
        };
        self.note(&format!("restarting in {:.1} s", delay.as_secs_f64()));
        self.emit(EventKind::RestartScheduled {
            delay_ms: u64::try_from(delay.as_millis()).unwrap_or(u64::MAX),
        });
    }

    /// Waits for the whole tree to be gone, then for the run's tasks, which end with its pipes.
    async fn wind_down(&mut self) {
        let Some(run) = &mut self.run else {
            return;
        };
        run.cancel.cancel();
        if !run.tree.wait_empty(TREE_SETTLE).await.unwrap_or(false) {
            if let Err(error) = run.tree.kill() {
                error!(%error, "cannot kill what is left of FXServer's process tree");
            }
            if !run.tree.wait_empty(TREE_KILL).await.unwrap_or(false) {
                error!(
                    generation = run.generation,
                    "processes of FXServer's tree survived the kill"
                );
            }
        }
        // Nothing more will be written to a server that is gone.
        run.stdin_task.abort();
        for mut task in run.tasks.drain(..) {
            if tokio::time::timeout(TASK_JOIN, &mut task).await.is_err() {
                task.abort();
            }
        }
        #[cfg(unix)]
        if let Err(error) = self.pid_file().clear() {
            warn!(%error, "cannot remove FXServer's pid file");
        }
    }

    async fn collect_dumps(&mut self, layout: &Layout) {
        let (from, to, limits) = (
            layout.crashes(),
            self.settings.data_dir.join("crashes"),
            self.settings.dumps,
        );
        let moved = tokio::task::spawn_blocking(move || crashes::collect(&from, &to, limits))
            .await
            .unwrap_or_else(|error| Err(std::io::Error::other(error)));
        self.report_dumps(moved);
    }

    fn report_dumps(&mut self, collected: std::io::Result<Collected>) {
        match collected {
            // The trim may have taken older dumps even when every moved one went too.
            Ok(collected) if collected.moved.is_empty() && collected.removed.is_empty() => {}
            Ok(Collected { moved, removed }) => {
                for path in &moved {
                    self.note(&format!("crash dump saved to {}", path.display()));
                }
                self.emit(EventKind::CrashDumps {
                    paths: moved,
                    removed,
                });
            }
            Err(error) => warn!(%error, "cannot move FXServer's crash dumps"),
        }
    }

    #[cfg(unix)]
    fn pid_file(&self) -> observer_platform::PidFile {
        observer_platform::PidFile::new(self.settings.data_dir.join("fxserver.pid"))
    }

    /// Without systemd nothing ends a server Observer left behind when it died, so the next
    /// Observer does before it starts one.
    #[cfg(unix)]
    async fn sweep_leftovers(&self) {
        let pid_file = self.pid_file();
        match tokio::task::spawn_blocking(move || pid_file.sweep()).await {
            Ok(Ok(observer_platform::Sweep::Killed { group, processes })) => {
                warn!(
                    group,
                    processes, "killed an FXServer left running by an earlier Observer"
                );
            }
            Ok(Ok(_)) => {}
            Ok(Err(error)) => warn!(%error, "cannot check for an FXServer left by an earlier Observer"),
            Err(error) => warn!(%error, "the leftover check failed"),
        }
    }

    fn set_state(&mut self, to: State, cause: Cause) {
        let from = std::mem::replace(&mut self.state, to);
        self.since_ms = now_ms();
        if to != State::Degraded {
            self.problems.clear();
        }
        info!(?from, ?to, ?cause, generation = self.generation, "server state");
        self.publish_status();
        self.emit(EventKind::State { from, to, cause });
    }

    fn publish_status(&self) {
        let run = self.run.as_ref();
        self.status.send_replace(Status {
            state: self.state,
            generation: self.generation,
            pid: run.map(|run| run.pid),
            since_ms: self.since_ms,
            problems: self.problems.clone(),
            started_ms: run.map(|run| run.started_ms),
            players: run.and_then(|run| run.players),
            build: run.and_then(|run| run.build),
            game_build: run.and_then(|run| run.game_build),
            name: self.name.clone(),
            port: self.port,
            bridge: run.map(|run| run.bridge.clone()),
            start: run
                .filter(|_| self.state == State::Starting)
                .and_then(|run| run.start.as_ref())
                .map(|(status, _)| status.clone()),
        });
    }

    fn emit(&self, kind: EventKind) {
        let _ = self.events.send(Event {
            at_ms: now_ms(),
            generation: self.generation,
            kind,
        });
    }

    fn note(&self, text: &str) {
        self.note_as(SUPERVISOR, self.generation, text);
    }

    /// A line of Observer's own, by `actor`, for `generation`: the pre-launch work's lines are the
    /// coming spawn's.
    fn note_as(&self, actor: &str, generation: u32, text: &str) {
        self.console.push(
            generation,
            LineKind::Observer,
            Some(actor.to_owned()),
            text.to_owned(),
            LineFlags::default(),
        );
    }
}

async fn exit_of(run: &mut Option<Run>) -> Exit {
    match run {
        Some(run) => run.exit.wait().await,
        None => std::future::pending().await,
    }
}

/// Resolves when the pre-launch work under way ends. Its handler takes the work out at once, so a
/// task that ended is never waited on again.
async fn pre_launch_of(work: &mut Option<PreLaunching>) -> Result<PreLaunchResult, JoinError> {
    match work {
        Some(work) => (&mut work.task).await,
        None => std::future::pending().await,
    }
}

/// Prepares a launch off the runtime: the cfg's scan reads files and walks the resources. The
/// bridge's name is drawn here, once for each start.
async fn prepare(server: ServerLaunch, flavour: Flavour) -> Result<launch::Prepared, SpawnError> {
    let bridge = bridge::draw_name().map_err(|error| {
        SpawnError::new(
            SpawnFailure::Bridge,
            format!("cannot draw the bridge's name: {error}"),
        )
    })?;
    tokio::task::spawn_blocking(move || launch::prepare(&server, flavour, &bridge))
        .await
        .map_err(|error| SpawnError::new(SpawnFailure::Spawn, error.to_string()))?
        .map_err(SpawnError::from)
}

/// Reads one of FXServer's console streams into the console, line by line, and passes on what
/// the supervisor must act on.
async fn read_console(
    mut output: Output,
    kind: LineKind,
    generation: u32,
    console: Arc<ConsoleHub>,
    signals: mpsc::Sender<(u32, Signal)>,
    progress: Arc<Progress>,
) {
    let mut lines = LineAssembler::default();
    let publish = |assembled: crate::console::Assembled| {
        let flags = LineFlags {
            truncated: assembled.truncated,
            no_eol: assembled.no_eol,
            masked: false,
        };
        let line = console.push(generation, kind, None, sanitize(&assembled.text), flags);
        let text = plain(&line.text);
        if kind == LineKind::Stdout {
            progress.read(&text, Instant::now(), now_ms());
        }
        if let Some(signal) = Signal::read(kind, &text) {
            let _ = signals.try_send((generation, signal));
        }
    };
    loop {
        let flush_at = lines
            .deadline()
            .unwrap_or_else(|| Instant::now() + Duration::from_hours(1));
        tokio::select! {
            chunk = output.next() => {
                let Some(bytes) = chunk else {
                    lines.finish().into_iter().for_each(&publish);
                    return;
                };
                lines.push(&bytes, Instant::now()).into_iter().for_each(&publish);
            }
            () = tokio::time::sleep_until(flush_at.into()) => lines.flush(Instant::now()).into_iter().for_each(&publish),
        }
    }
}

/// One run's tree as the supervisor follows it.
struct TreeFollower {
    changes: TreeChanges,
    tree: Weak<ProcessTree>,
    context: TreeContext,
    generation: u32,
    bus: broadcast::Sender<TreeEvent>,
    cancel: CancellationToken,
    drop_next: bool,
    /// The processes reported started and not ended, with their start stamps.
    running: HashMap<u32, Option<u64>>,
}

/// Classifies what the tree reports and hands it on. Every few seconds it compares the tree's
/// members with what it reported, and makes up for the notifications the system never delivered.
async fn follow_tree(mut follower: TreeFollower) {
    let start = tokio::time::Instant::now() + RECONCILE_EVERY;
    let mut reconcile = tokio::time::interval_at(start, RECONCILE_EVERY);
    loop {
        tokio::select! {
            () = follower.cancel.cancelled() => return,
            change = follower.changes.next() => {
                let Some(change) = change else {
                    return;
                };
                if std::mem::take(&mut follower.drop_next) {
                    continue;
                }
                follower.apply(change);
            }
            _ = reconcile.tick() => follower.reconcile().await,
        }
    }
}

impl TreeFollower {
    fn apply(&mut self, change: TreeChange) {
        match change {
            TreeChange::Started(process) => {
                if let Some(stamp) = self.running.get(&process.pid) {
                    if stamp.is_none() || process.stamp.is_none() || *stamp == process.stamp {
                        return;
                    }
                    // The pid was reused by a process started after the one it named.
                    self.context.forget(process.pid);
                    self.publish(ProcessChange::Ended { pid: process.pid });
                }
                self.running.insert(process.pid, process.stamp);
                let kind = tree::classify(&process, &self.context);
                self.context.record(&process, kind);
                self.publish(ProcessChange::Started { process, kind });
            }
            TreeChange::Ended { pid } => {
                if self.running.remove(&pid).is_some() {
                    self.context.forget(pid);
                    self.publish(ProcessChange::Ended { pid });
                }
            }
        }
    }

    async fn reconcile(&mut self) {
        let Some(tree) = self.tree.upgrade() else {
            return;
        };
        let main = self.context.main_pid;
        let reported: Vec<(u32, Option<u64>)> =
            self.running.iter().map(|(&pid, &stamp)| (pid, stamp)).collect();
        let looked = tokio::task::spawn_blocking(move || {
            let members: HashSet<u32> = tree.all_members().ok()?.into_iter().collect();
            // Reported processes the members miss may only have left the tree's session.
            let gone: Vec<u32> = reported
                .into_iter()
                .filter(|(pid, stamp)| !members.contains(pid) && !tree.still_runs(*pid, *stamp))
                .map(|(pid, _)| pid)
                .collect();
            let mut members: Vec<ProcessInfo> = members
                .into_iter()
                .filter(|&pid| pid != main)
                .filter_map(|pid| tree.describe(pid))
                .collect();
            // Parents first: what a process is may depend on what started it.
            members.sort_by_key(|member| (member.stamp, member.pid));
            Some((members, gone))
        })
        .await;
        let Ok(Some((members, gone))) = looked else {
            return;
        };
        for pid in gone {
            self.apply(TreeChange::Ended { pid });
        }
        let young = now_ms() - RECONCILE_AFTER_MS;
        for member in members {
            if member.created_ms.is_some_and(|created| created > young) {
                continue;
            }
            self.apply(TreeChange::Started(member));
        }
    }

    fn publish(&self, change: ProcessChange) {
        let _ = self.bus.send(TreeEvent {
            generation: self.generation,
            at_ms: now_ms(),
            change,
        });
    }
}

fn hang_text(cause: HangCause) -> &'static str {
    match cause {
        HangCause::NoHeartbeat => "no heartbeat from the observer bridge",
        HangCause::NoHttp => "its HTTP endpoint does not answer",
    }
}

/// Why a start whose cfg never finished is taken for stuck: the last resource it started, and how
/// long ago.
fn stalled_text(last: Option<(Instant, String)>, now: Instant) -> String {
    match last {
        Some((at, resource)) => {
            let seconds = now.saturating_duration_since(at).as_secs();
            let ago = if seconds >= 120 {
                format!("{} min", seconds / 60)
            } else {
                format!("{seconds} s")
            };
            format!("its start is stuck: no resource started since {resource}, {ago} ago")
        }
        None => "its start is stuck: it started no resource".to_owned(),
    }
}

fn now_ms() -> i64 {
    jiff::Timestamp::now().as_millisecond()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stuck_start_names_the_last_resource_started_and_how_long_ago() {
        let started = Instant::now();
        let last = || Some((started, "clothes".to_owned()));
        let after = |seconds| started + Duration::from_secs(seconds);

        assert_eq!(
            stalled_text(last(), after(371)),
            "its start is stuck: no resource started since clothes, 6 min ago"
        );
        assert_eq!(
            stalled_text(last(), after(45)),
            "its start is stuck: no resource started since clothes, 45 s ago"
        );
        assert_eq!(
            stalled_text(None, after(45)),
            "its start is stuck: it started no resource"
        );
    }

    #[test]
    fn licence_rejections_are_recognised_on_either_stream() {
        let lines = [
            (
                LineKind::Stderr,
                "Fatal Error: This server does not have a license key specified. Please set the sv_licenseKey console variable to a key from https://portal.cfx.re/servers/registration-keys.",
            ),
            (
                LineKind::Stdout,
                "[          svadhesive] Error: This server does not have a license key specified.",
            ),
            (
                LineKind::Stderr,
                "Fatal Error: Could not authenticate server license key. Invalid key format specified.",
            ),
            (
                LineKind::Stdout,
                "[          svadhesive] Could not authenticate server license key. The specified key (cfxk...AbCd) does not exist.",
            ),
        ];

        for (kind, text) in lines {
            assert!(
                matches!(Signal::read(kind, text), Some(Signal::LicenceRejected(_))),
                "{text}"
            );
        }
    }

    #[test]
    fn licence_texts_quoted_by_a_resource_or_an_operator_are_not_rejections() {
        let lines = [
            (
                LineKind::Stdout,
                "[           script:chat] Bob: lol A valid license check is required to run this server",
            ),
            (
                LineKind::Stdout,
                "[           script:chat] [          svadhesive] Could not authenticate server license key.",
            ),
            (
                LineKind::Stdout,
                "[                 cmd]  \"x\" is \"Could not authenticate server license key\"",
            ),
            (
                LineKind::Stdout,
                "[      script:svadhesive] This server does not have a license key specified.",
            ),
            (
                LineKind::Stderr,
                "SCRIPT ERROR: @chat/sv_chat.lua:12: Fatal Error: Could not authenticate server license key",
            ),
            (LineKind::Command, "say Could not authenticate server license key"),
        ];

        for (kind, text) in lines {
            assert!(Signal::read(kind, text).is_none(), "{text}");
        }
    }

    #[test]
    fn a_fatal_error_comes_from_stderr_only() {
        let message =
            "Fatal Error: Could not bind on 127.0.0.1:30140 - is this address valid and not already in use?";

        assert!(matches!(
            Signal::read(LineKind::Stderr, message),
            Some(Signal::Fatal(text)) if text.starts_with("Could not bind on 127.0.0.1:30140")
        ));
        assert!(Signal::read(LineKind::Stdout, message).is_none());
        assert!(
            Signal::read(
                LineKind::Stdout,
                "[ citizen-server-impl] Error: Could not bind on 127.0.0.1:30140"
            )
            .is_none()
        );
    }
}
