//! The supervisor's pre-launch phase, around the fake: work given in the settings runs before each
//! spawn, whatever its cause, and FXServer never runs while it does.
// The helpers below are test code too, though not inside a `#[test]` function.
#![allow(clippy::unwrap_used)]

mod common;

use std::collections::VecDeque;
use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{Port, Ports};
use observer_domain::launch::{ExtraArgument, LaunchSettings, OneSync};
use observer_fxserver::console::{ConsoleHub, plain};
use observer_fxserver::health::HealthLimits;
use observer_fxserver::launch::ServerLaunch;
use observer_fxserver::pre_launch::{PreLaunch, PreLaunchFuture, PreLaunchRequest, TOOLS};
use observer_fxserver::recovery::RecoveryPolicy;
use observer_fxserver::stdin::UserCommand;
use observer_fxserver::supervisor::{
    Cause, ControlError, Event, EventKind, Settings, State, StopTimings, Supervisor,
};
use tempfile::TempDir;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tokio::time::timeout;

const WAIT: Duration = Duration::from_secs(20);
const HEARTBEAT_MS: &str = "200";
/// How often the fake work looks at its flags.
const POLL: Duration = Duration::from_millis(10);

/// The labs' ports, lent to nine at a time.
static PORTS: Ports = Ports::new(9);

/// What one run of the fake work does.
#[derive(Debug, Clone)]
enum Act {
    /// Hands these lines back at once; `{generation}` stands for the request's.
    Lines(Vec<&'static str>),
    /// Waits until it is told to end, then takes `linger` more, as work finishing a file does.
    UntilTold {
        linger: Duration,
    },
    /// Waits until the test releases it.
    UntilReleased,
    Fail,
    Panic,
}

/// What one run was asked, and how it ended.
#[derive(Debug, Clone)]
struct Seen {
    launch: ServerLaunch,
    endpoint: Option<SocketAddr>,
    cause: Cause,
    generation: u32,
    told_to_end: bool,
    started: Instant,
    ended: Instant,
}

#[derive(Debug, Default)]
struct Work {
    /// What each run does, in order; a run past the list hands back one line.
    acts: Mutex<VecDeque<Act>>,
    seen: Mutex<Vec<Seen>>,
    released: AtomicBool,
}

impl Work {
    fn new(acts: impl IntoIterator<Item = Act>) -> Arc<Self> {
        Arc::new(Self {
            acts: Mutex::new(acts.into_iter().collect()),
            ..Self::default()
        })
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    fn release(&self) {
        self.released.store(true, Ordering::Relaxed);
    }
}

impl PreLaunch for Work {
    fn run(&self, request: PreLaunchRequest) -> PreLaunchFuture<'_> {
        Box::pin(async move {
            let started = Instant::now();
            let act = self
                .acts
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| Act::Lines(vec!["checked for generation {generation}"]));
            let told = || request.cancel.load(Ordering::Relaxed);
            let result = match &act {
                Act::Lines(lines) => Ok(lines
                    .iter()
                    .map(|line| line.replace("{generation}", &request.generation.to_string()))
                    .collect()),
                Act::UntilTold { linger } => {
                    while !told() {
                        tokio::time::sleep(POLL).await;
                    }
                    tokio::time::sleep(*linger).await;
                    Ok(vec!["ended as told".to_owned()])
                }
                Act::UntilReleased => {
                    while !self.released.load(Ordering::Relaxed) {
                        tokio::time::sleep(POLL).await;
                    }
                    Ok(vec!["released".to_owned()])
                }
                Act::Fail | Act::Panic => Err("the resource index is unreadable".into()),
            };
            self.seen.lock().unwrap().push(Seen {
                launch: request.launch.clone(),
                endpoint: request.scan.endpoint,
                cause: request.cause.clone(),
                generation: request.generation,
                told_to_end: told(),
                started,
                ended: Instant::now(),
            });
            assert!(!matches!(act, Act::Panic), "the fake work panics as asked");
            result
        })
    }
}

struct Lab {
    _dir: TempDir,
    launch: ServerLaunch,
    console: Arc<ConsoleHub>,
    supervisor: Supervisor,
    task: Option<JoinHandle<()>>,
    events: broadcast::Receiver<Event>,
    seen: Vec<EventKind>,
    port: Port,
}

impl Lab {
    async fn new(work: &Arc<Work>) -> Self {
        Self::tuned(work, |_| {}).await
    }

    /// A lab whose supervisor runs `work` before each spawn, with settings `tune` adjusts.
    async fn tuned(work: &Arc<Work>, tune: impl FnOnce(&mut Settings)) -> Self {
        let port = PORTS.lease().await;
        let dir = tempfile::tempdir().unwrap();
        let program = fake_program(&dir.path().join("build"));
        fs::create_dir_all(program.parent().unwrap()).unwrap();
        fs::copy(env!("CARGO_BIN_EXE_fake-fxserver"), &program).unwrap();
        let data = dir.path().join("server-data");
        fs::create_dir_all(&data).unwrap();
        fs::write(
            data.join("server.cfg"),
            format!(
                "endpoint_add_tcp \"127.0.0.1:{0}\"\nendpoint_add_udp \"127.0.0.1:{0}\"\nsv_hostname \"Observer test\"\nsv_maxclients 8\n",
                port.number
            ),
        )
        .unwrap();

        let settings = LaunchSettings {
            data_path: data.to_str().unwrap().to_owned(),
            cfg_file: "server.cfg".to_owned(),
            onesync: OneSync::Off,
            auto_start: false,
            txadmin_events: false,
            extra_arguments: vec![ExtraArgument {
                name: "fake_heartbeat_ms".to_owned(),
                value: HEARTBEAT_MS.to_owned(),
            }],
            hang_detection: true,
            hang_timeout_seconds: 360,
        };
        let launch = ServerLaunch {
            build: dir.path().join("build"),
            settings,
        };
        let mut settings = Settings::new(Some(launch.clone()), dir.path().join("observer-data"));
        settings.health = HealthLimits {
            boot_grace: Duration::from_secs(2),
            heartbeat_late: Duration::from_secs(1),
            heartbeat_fatal: Duration::from_secs(2),
            http_late: Duration::from_secs(2),
            http_fatal: Duration::from_secs(5),
        };
        settings.stop = StopTimings {
            notice: Duration::from_millis(300),
            quit: Duration::from_secs(3),
            shortened: Duration::from_secs(1),
            fatal_exit: Duration::from_secs(1),
        };
        settings.recovery = RecoveryPolicy {
            backoff: [100, 200, 300, 400]
                .into_iter()
                .map(Duration::from_millis)
                .collect(),
            backoff_cap: Duration::from_millis(500),
            ..RecoveryPolicy::default()
        };
        settings.hang_snapshot = None;
        settings.pre_launch = Some(Arc::clone(work) as Arc<dyn PreLaunch>);
        tune(&mut settings);

        let console = Arc::new(ConsoleHub::new(None));
        let (supervisor, task) = Supervisor::spawn(settings, Arc::clone(&console));
        let events = supervisor.events();
        Self {
            _dir: dir,
            launch,
            console,
            supervisor,
            task: Some(task),
            events,
            seen: Vec::new(),
            port,
        }
    }

    /// The next event that `wanted` accepts, skipping the others.
    async fn event(&mut self, wanted: impl Fn(&EventKind) -> bool) -> EventKind {
        self.next(wanted).await.kind
    }

    /// As [`Lab::event`], with the generation the event carries.
    async fn next(&mut self, wanted: impl Fn(&EventKind) -> bool) -> Event {
        let found = timeout(WAIT, async {
            loop {
                let event = self.events.recv().await.unwrap();
                self.seen.push(event.kind.clone());
                if wanted(&event.kind) {
                    return event;
                }
            }
        })
        .await;
        found.unwrap_or_else(|_| panic!("the event never came; saw {:#?}", self.seen))
    }

    /// Waits for the transition into `state` and returns its cause.
    async fn state(&mut self, state: State) -> Cause {
        match self
            .event(|kind| matches!(kind, EventKind::State { to, .. } if *to == state))
            .await
        {
            EventKind::State { cause, .. } => cause,
            _ => unreachable!(),
        }
    }

    /// Fails if `unwanted` comes within `window`.
    async fn none_within(&mut self, window: Duration, unwanted: impl Fn(&EventKind) -> bool) {
        let deadline = tokio::time::Instant::now() + window;
        while let Ok(event) = tokio::time::timeout_at(deadline, self.events.recv()).await {
            let kind = event.unwrap().kind;
            assert!(!unwanted(&kind), "unexpected {kind:#?}");
            self.seen.push(kind);
        }
    }

    async fn type_command(&self, line: &str, actor: &str) {
        self.supervisor
            .command(UserCommand::parse(line).unwrap(), actor)
            .await
            .unwrap();
    }

    /// Every console line so far: its generation, who wrote it, and its text.
    fn console(&self) -> Vec<(u32, Option<String>, String)> {
        self.console
            .subscribe_from(None)
            .backlog
            .iter()
            .map(|line| (line.generation, line.actor.clone(), plain(&line.text)))
            .collect()
    }

    fn console_index(&self, needle: &str) -> Option<usize> {
        self.console()
            .iter()
            .position(|(_, _, text)| text.contains(needle))
    }

    fn generation(&self) -> u32 {
        self.supervisor.status().borrow().generation
    }

    async fn shut_down(&mut self, shortened: bool) {
        self.supervisor.shutdown(shortened).await;
        timeout(WAIT, self.task.take().unwrap())
            .await
            .expect("the supervisor did not stop")
            .unwrap();
    }
}

impl Drop for Lab {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

fn fake_program(build: &Path) -> PathBuf {
    if cfg!(windows) {
        build.join("FXServer.exe")
    } else {
        // Run as the loader, the fake reads FXServer's arguments after `--` like the real one.
        build.join("alpine/opt/cfx-server/ld-musl-x86_64.so.1")
    }
}

fn spawned(kind: &EventKind) -> bool {
    matches!(kind, EventKind::Spawned { .. })
}

fn starting(kind: &EventKind) -> bool {
    to(State::Starting)(kind)
}

/// Accepts the transition into `state`.
fn to(state: State) -> impl Fn(&EventKind) -> bool {
    move |kind| matches!(kind, EventKind::State { to, .. } if *to == state)
}

fn by(name: &str) -> String {
    name.to_owned()
}

#[tokio::test]
async fn the_work_runs_before_the_spawn_and_its_lines_carry_the_coming_generation() {
    let work = Work::new([Act::Lines(vec![
        "healed 2 files",
        "nothing else to do for generation {generation}",
    ])]);
    let mut lab = Lab::new(&work).await;

    lab.supervisor.start("test").await.unwrap();

    assert_eq!(
        lab.state(State::Starting).await,
        Cause::Requested { by: by("test") }
    );
    lab.event(spawned).await;
    lab.state(State::Online).await;
    assert_eq!(
        lab.seen.iter().filter(|kind| starting(kind)).count(),
        1,
        "the server shows as starting once: {:#?}",
        lab.seen
    );
    let seen = work.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        seen[0].launch, lab.launch,
        "the work gets the launch the spawn uses"
    );
    assert_eq!(
        seen[0].endpoint.map(|endpoint| endpoint.port()),
        Some(lab.port.number),
        "and the scan of its cfg"
    );
    assert_eq!(seen[0].cause, Cause::Requested { by: by("test") });
    assert_eq!((seen[0].generation, seen[0].told_to_end), (1, false));
    let console = lab.console();
    let tools: Vec<(u32, &str)> = console
        .iter()
        .filter(|(_, actor, _)| actor.as_deref() == Some(TOOLS))
        .map(|(generation, _, text)| (*generation, text.as_str()))
        .collect();
    assert_eq!(
        tools,
        [(1, "healed 2 files"), (1, "nothing else to do for generation 1")]
    );
    assert_eq!(lab.generation(), 1);
    assert!(lab.console_index("healed 2 files").unwrap() < lab.console_index("FXServer started").unwrap());
    lab.shut_down(false).await;
}

#[tokio::test]
async fn a_start_is_answered_while_the_work_runs_and_nothing_spawns_before_it_ends() {
    let work = Work::new([Act::UntilReleased]);
    let mut lab = Lab::new(&work).await;

    timeout(Duration::from_secs(5), lab.supervisor.start("test"))
        .await
        .expect("the start waited for the work")
        .unwrap();

    lab.state(State::Starting).await;
    let status = lab.supervisor.status().borrow().clone();
    assert_eq!(
        (status.state, status.pid, status.generation),
        (State::Starting, None, 1),
        "the start took its generation as the work began"
    );
    assert_eq!(
        lab.supervisor.start("test").await,
        Err(ControlError::AlreadyRunning),
        "requests are still answered"
    );
    assert_eq!(
        lab.supervisor
            .command(UserCommand::parse("status").unwrap(), "test")
            .await,
        Err(ControlError::NotRunning)
    );
    lab.none_within(Duration::from_millis(700), spawned).await;
    assert!(work.seen().is_empty(), "the work still runs");

    work.release();

    lab.event(spawned).await;
    let spawned_at = Instant::now();
    lab.state(State::Online).await;
    assert!(work.seen()[0].ended <= spawned_at);
    assert!(lab.console_index("released").unwrap() < lab.console_index("FXServer started").unwrap());
    assert_eq!(lab.generation(), 1);
    lab.shut_down(false).await;
}

#[tokio::test]
async fn a_stop_during_the_work_tells_it_to_end_and_never_spawns() {
    let work = Work::new([Act::UntilTold {
        linger: Duration::from_millis(300),
    }]);
    let mut lab = Lab::new(&work).await;
    lab.supervisor.start("test").await.unwrap();
    lab.state(State::Starting).await;

    lab.supervisor.stop("ada").await.unwrap();
    let stopped_again = lab.supervisor.stop("grace").await;
    let started_again = lab.supervisor.start("grace").await;

    assert_eq!(
        lab.state(State::Stopping).await,
        Cause::Requested { by: by("ada") }
    );
    assert_eq!(
        lab.state(State::Stopped).await,
        Cause::Requested { by: by("ada") }
    );
    assert_eq!(stopped_again, Err(ControlError::Stopping));
    assert_eq!(started_again, Err(ControlError::Stopping));
    lab.none_within(Duration::from_secs(1), |kind| spawned(kind) || starting(kind))
        .await;
    assert_eq!(
        lab.generation(),
        1,
        "the start that never spawned keeps its generation"
    );
    let seen = work.seen();
    assert_eq!(seen.len(), 1);
    assert!(seen[0].told_to_end);
    assert!(
        lab.console()
            .contains(&(1, Some(TOOLS.to_owned()), "ended as told".to_owned())),
        "what the work did before it ended is told: {:#?}",
        lab.console()
    );

    // A start after it runs work of its own, with a generation of its own.
    lab.supervisor.start("test").await.unwrap();
    lab.state(State::Online).await;
    assert_eq!(lab.generation(), 2);
    let generations: Vec<u32> = work.seen().iter().map(|seen| seen.generation).collect();
    assert_eq!(generations, [1, 2]);
    lab.shut_down(false).await;
}

#[tokio::test]
async fn a_shutdown_during_the_work_waits_for_it_and_ends_without_spawning() {
    let work = Work::new([Act::UntilTold {
        linger: Duration::from_millis(300),
    }]);
    let mut lab = Lab::new(&work).await;
    lab.supervisor.start("test").await.unwrap();
    lab.state(State::Starting).await;

    lab.shut_down(false).await;

    let ended_at = Instant::now();
    let seen = work.seen();
    assert_eq!(seen.len(), 1, "the supervisor ended after the work");
    assert!(seen[0].told_to_end && seen[0].ended <= ended_at);
    let mut states = Vec::new();
    while let Ok(event) = lab.events.try_recv() {
        assert!(!spawned(&event.kind), "{:#?}", event.kind);
        if let EventKind::State { to, cause, .. } = event.kind {
            states.push((to, cause));
        }
    }
    assert_eq!(
        states,
        [
            (State::Stopping, Cause::Shutdown),
            (State::Stopped, Cause::Shutdown)
        ]
    );
    assert_eq!(lab.generation(), 1);
}

#[tokio::test]
async fn work_past_its_limit_is_told_to_end_and_the_spawn_waits_until_it_has() {
    let work = Work::new([Act::UntilTold {
        linger: Duration::from_millis(500),
    }]);
    let mut lab = Lab::tuned(&work, |settings| {
        settings.pre_launch_limit = Duration::from_millis(300);
    })
    .await;

    lab.supervisor.start("test").await.unwrap();

    lab.event(spawned).await;
    let spawned_at = Instant::now();
    lab.state(State::Online).await;
    let seen = work.seen();
    assert!(seen[0].told_to_end);
    assert!(seen[0].ended <= spawned_at, "FXServer never runs beside the work");
    assert!(
        seen[0].ended - seen[0].started >= Duration::from_millis(800),
        "{:?}",
        seen[0].ended - seen[0].started
    );
    let overdue = lab.console_index("took more than 0.3 s").unwrap();
    let told = lab.console_index("ended as told").unwrap();
    let started = lab.console_index("FXServer started").unwrap();
    assert!(overdue < told && told < started, "{:#?}", lab.console());
    assert_eq!(lab.console()[overdue].0, 1);
    lab.shut_down(false).await;
}

#[tokio::test]
async fn restarts_and_recoveries_go_through_the_work_with_their_cause_once_the_old_server_is_gone() {
    let work = Work::new([]);
    let mut lab = Lab::new(&work).await;
    lab.supervisor.start("test").await.unwrap();
    lab.state(State::Online).await;

    lab.supervisor.restart("grace").await.unwrap();
    assert_eq!(
        lab.state(State::Starting).await,
        Cause::Restart { by: by("grace") }
    );
    lab.state(State::Online).await;
    lab.supervisor.restart_scheduled("observer").await.unwrap();
    assert_eq!(
        lab.state(State::Starting).await,
        Cause::Scheduled { by: by("observer") }
    );
    lab.state(State::Online).await;
    lab.type_command("fake_crash", "test").await;
    assert_eq!(lab.state(State::Starting).await, Cause::Recovery);
    lab.state(State::Online).await;

    assert_eq!(lab.generation(), 4);
    let asked: Vec<(Cause, u32)> = work
        .seen()
        .into_iter()
        .map(|seen| (seen.cause, seen.generation))
        .collect();
    assert_eq!(
        asked,
        [
            (Cause::Requested { by: by("test") }, 1),
            (Cause::Restart { by: by("grace") }, 2),
            (Cause::Scheduled { by: by("observer") }, 3),
            (Cause::Recovery, 4),
        ]
    );
    let console = lab.console();
    let find = |generation: u32, actor: &str, text: &str| {
        console
            .iter()
            .position(|(at, by, line)| {
                *at == generation && by.as_deref() == Some(actor) && line.starts_with(text)
            })
            .unwrap_or_else(|| panic!("no {text:?} for generation {generation}: {console:#?}"))
    };
    for generation in 2..=4 {
        let exited = find(generation - 1, "supervisor", "FXServer exited");
        let checked = find(generation, TOOLS, &format!("checked for generation {generation}"));
        let started = find(generation, "supervisor", "FXServer started");
        assert!(exited < checked && checked < started, "{console:#?}");
    }
    lab.shut_down(false).await;
}

#[tokio::test]
async fn failing_or_panicking_work_still_lets_the_server_start() {
    for (act, said) in [
        (
            Act::Fail,
            "failed before the start: the resource index is unreadable",
        ),
        (Act::Panic, "failed before the start, unexpectedly"),
    ] {
        let work = Work::new([act]);
        let mut lab = Lab::new(&work).await;

        lab.supervisor.start("test").await.unwrap();

        lab.state(State::Online).await;
        let console = lab.console();
        assert!(
            console.contains(&(1, Some(TOOLS.to_owned()), said.to_owned())),
            "{console:#?}"
        );
        lab.shut_down(false).await;
    }
}

#[tokio::test]
async fn a_restart_during_the_work_starts_it_over_with_the_restart_s_cause() {
    let work = Work::new([Act::UntilTold {
        linger: Duration::from_millis(100),
    }]);
    let mut lab = Lab::new(&work).await;
    lab.supervisor.start("test").await.unwrap();
    lab.state(State::Starting).await;

    lab.supervisor.restart("grace").await.unwrap();

    assert_eq!(
        lab.state(State::Stopping).await,
        Cause::Restart { by: by("grace") }
    );
    assert_eq!(
        lab.state(State::Starting).await,
        Cause::Restart { by: by("grace") }
    );
    lab.state(State::Online).await;
    assert_eq!(lab.generation(), 2, "each start takes a generation of its own");
    let runs: Vec<(Cause, u32, bool)> = work
        .seen()
        .into_iter()
        .map(|seen| (seen.cause, seen.generation, seen.told_to_end))
        .collect();
    assert_eq!(
        runs,
        [
            (Cause::Requested { by: by("test") }, 1, true),
            (Cause::Restart { by: by("grace") }, 2, false),
        ]
    );
    lab.shut_down(false).await;
}

#[tokio::test]
async fn a_start_through_the_work_carries_one_generation_from_its_state_to_its_exit() {
    let work = Work::new([]);
    let mut lab = Lab::new(&work).await;

    lab.supervisor.start("test").await.unwrap();
    let first = [
        lab.next(starting).await.generation,
        lab.next(spawned).await.generation,
        lab.next(to(State::Online)).await.generation,
    ];
    lab.supervisor.restart("grace").await.unwrap();
    let ended = [
        lab.next(to(State::Stopping)).await.generation,
        lab.next(|kind| matches!(kind, EventKind::Exited { .. }))
            .await
            .generation,
    ];
    let second = [
        lab.next(starting).await.generation,
        lab.next(spawned).await.generation,
        lab.next(to(State::Online)).await.generation,
    ];

    assert_eq!(
        (first, ended, second),
        ([1, 1, 1], [1, 1], [2, 2, 2]),
        "the statistics know a run by the generation of its start"
    );
    let tools: Vec<u32> = lab
        .console()
        .iter()
        .filter(|(_, actor, _)| actor.as_deref() == Some(TOOLS))
        .map(|(generation, _, _)| *generation)
        .collect();
    assert_eq!(tools, [1, 2]);
    lab.shut_down(false).await;
}

#[tokio::test]
async fn a_scheduled_restart_during_the_work_starts_it_over_with_the_launch_configured_by_then() {
    let work = Work::new([Act::UntilTold {
        linger: Duration::from_millis(100),
    }]);
    let mut lab = Lab::new(&work).await;
    lab.supervisor.start("test").await.unwrap();
    lab.state(State::Starting).await;
    let mut promoted = lab.launch.clone();
    promoted.settings.extra_arguments.push(ExtraArgument {
        name: "sv_maxclients".to_owned(),
        value: "16".to_owned(),
    });

    // As the schedule does: it hands over the launch of the build it applies, then restarts.
    lab.supervisor.configure(promoted.clone()).await;
    lab.supervisor.restart_scheduled("observer").await.unwrap();
    let again = lab.supervisor.restart_scheduled("observer").await;

    let scheduled = Cause::Scheduled { by: by("observer") };
    assert_eq!(again, Err(ControlError::Stopping));
    assert_eq!(lab.state(State::Stopping).await, scheduled);
    assert_eq!(lab.state(State::Starting).await, scheduled);
    let EventKind::Spawned { launch, .. } = lab.event(spawned).await else {
        unreachable!()
    };
    assert_eq!(*launch, promoted.settings, "the spawn goes by the launch of now");
    lab.state(State::Online).await;
    let runs: Vec<(Cause, bool, bool)> = work
        .seen()
        .into_iter()
        .map(|seen| (seen.cause, seen.told_to_end, seen.launch == promoted))
        .collect();
    assert_eq!(
        runs,
        [
            (Cause::Requested { by: by("test") }, true, false),
            (scheduled, false, true),
        ]
    );
    lab.shut_down(false).await;
}
