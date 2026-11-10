//! The supervisor driving the fake through Observer's real launch: the build layout, the bridge,
//! the console, the probes and the process tree.
// The helpers below are test code too, though not inside a `#[test]` function.
#![allow(clippy::unwrap_used)]

use std::fs;
use std::net::{SocketAddr, TcpListener, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};
use std::time::{Duration, Instant};

use observer_domain::diagnostics::ProcessKind;
#[cfg(unix)]
use observer_domain::diagnostics::ThreadRun;
use observer_domain::launch::{ExtraArgument, LaunchSettings, OneSync};
use observer_domain::schedule::RestartSource;
use observer_domain::server::SpawnFailure;
use observer_fxserver::bridge::{self, BridgeEvent};
use observer_fxserver::console::{ConsoleHub, LineKind, plain};
use observer_fxserver::crashes::analysis;
use observer_fxserver::crashes::{HangSnapshotLimits, SnapshotFailure};
use observer_fxserver::health::{HangCause, HealthLimits, Problem};
use observer_fxserver::launch::ServerLaunch;
use observer_fxserver::perf::Thread;
use observer_fxserver::recovery::RecoveryPolicy;
use observer_fxserver::stdin::{StdinError, UserCommand};
use observer_fxserver::supervisor::{
    Cause, ControlError, Event, EventKind, ProcessChange, START_EXTENSION, Settings, StartStatus, State,
    StopTimings, Supervisor, TreeEvent,
};
use observer_platform::{ProcessInfo, Protocol, Socket, SocketState};
use tempfile::TempDir;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tokio::time::timeout;

const WAIT: Duration = Duration::from_secs(20);
const HEARTBEAT_MS: &str = "200";

#[cfg(windows)]
const KILLED: Option<u32> = Some(1);
#[cfg(unix)]
const KILLED: Option<u32> = None;

/// The ports tests may use, lent to one test at a time. 41929 and up belong to other Observers
/// of the development host.
static PORTS: LazyLock<Mutex<Vec<u16>>> = LazyLock::new(|| Mutex::new((41920..=41928).collect()));

struct Port(u16);

impl Port {
    async fn lease() -> Self {
        loop {
            if let Some(port) = PORTS.lock().unwrap_or_else(PoisonError::into_inner).pop() {
                return Self(port);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

impl Drop for Port {
    fn drop(&mut self) {
        PORTS.lock().unwrap_or_else(PoisonError::into_inner).push(self.0);
    }
}

struct Lab {
    dir: TempDir,
    launch: ServerLaunch,
    console: Arc<ConsoleHub>,
    supervisor: Supervisor,
    task: Option<JoinHandle<()>>,
    events: broadcast::Receiver<Event>,
    seen: Vec<EventKind>,
    port: Port,
}

impl Lab {
    async fn new(extra: &[(&str, &str)]) -> Self {
        Self::tuned(extra, |_| {}).await
    }

    /// A lab whose supervisor settings `tune` adjusts. A hung server is killed without a snapshot
    /// unless `tune` asks for one.
    async fn tuned(extra: &[(&str, &str)], tune: impl FnOnce(&mut Settings)) -> Self {
        let port = Port::lease().await;
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
                port.0
            ),
        )
        .unwrap();

        let extra_arguments = extra
            .iter()
            .chain(&[("fake_heartbeat_ms", HEARTBEAT_MS)])
            .map(|(name, value)| ExtraArgument {
                name: (*name).to_owned(),
                value: (*value).to_owned(),
            })
            .collect();
        let settings = LaunchSettings {
            data_path: data.to_str().unwrap().to_owned(),
            cfg_file: "server.cfg".to_owned(),
            onesync: OneSync::Off,
            auto_start: false,
            txadmin_events: false,
            extra_arguments,
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
            bind_step: Duration::from_millis(300),
            bind_cap: Duration::from_millis(900),
            ..RecoveryPolicy::default()
        };
        settings.hang_snapshot = None;
        tune(&mut settings);

        let console = Arc::new(ConsoleHub::new(None));
        let (supervisor, task) = Supervisor::spawn(settings, Arc::clone(&console));
        let events = supervisor.events();
        Self {
            dir,
            launch,
            console,
            supervisor,
            task: Some(task),
            events,
            seen: Vec::new(),
            port,
        }
    }

    async fn online(extra: &[(&str, &str)]) -> Self {
        let mut lab = Self::new(extra).await;
        lab.supervisor.start("test").await.unwrap();
        lab.state(State::Online).await;
        lab
    }

    /// The next event that `wanted` accepts, skipping the others.
    async fn event(&mut self, wanted: impl Fn(&EventKind) -> bool) -> EventKind {
        let found = timeout(WAIT, async {
            loop {
                let event = self.events.recv().await.unwrap();
                self.seen.push(event.kind.clone());
                if wanted(&event.kind) {
                    return event.kind;
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

    fn console(&self) -> Vec<(LineKind, Option<String>, String)> {
        self.console
            .subscribe_from(None)
            .backlog
            .iter()
            .map(|line| (line.kind, line.actor.clone(), plain(&line.text)))
            .collect()
    }

    fn console_index(&self, needle: &str) -> Option<usize> {
        self.console()
            .iter()
            .position(|(_, _, text)| text.contains(needle))
    }

    async fn console_until(&self, needle: &str) -> usize {
        let deadline = Instant::now() + WAIT;
        loop {
            if let Some(index) = self.console_index(needle) {
                return index;
            }
            assert!(
                Instant::now() < deadline,
                "no {needle:?} in the console: {:#?}",
                self.console()
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    fn generation(&self) -> u32 {
        self.supervisor.status().borrow().generation
    }

    /// The start under way once it is as `wanted`, as the status publishes it.
    async fn start_until(&self, wanted: impl Fn(&StartStatus) -> bool) -> StartStatus {
        let mut status = self.supervisor.status();
        let found = timeout(WAIT, async {
            loop {
                let start = status.borrow_and_update().start.clone();
                if let Some(start) = start.filter(&wanted) {
                    return start;
                }
                status.changed().await.unwrap();
            }
        })
        .await;
        found.unwrap_or_else(|_| {
            panic!(
                "the start never came to that: {:#?}",
                self.supervisor.status().borrow().start
            )
        })
    }

    /// The bridge's name in the server that runs now.
    fn bridge(&self) -> String {
        self.supervisor.status().borrow().bridge.clone().unwrap()
    }

    /// The build's `system_resources`, next to the fake as FXServer's are next to it.
    fn system_resources(&self) -> PathBuf {
        fake_program(&self.dir.path().join("build"))
            .parent()
            .unwrap()
            .join("citizen/system_resources")
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

fn exited(kind: &EventKind) -> bool {
    matches!(kind, EventKind::Exited { .. })
}

fn starting(kind: &EventKind) -> bool {
    matches!(
        kind,
        EventKind::State {
            to: State::Starting,
            ..
        }
    )
}

#[tokio::test]
async fn the_status_tells_the_players_the_build_and_when_the_server_started() {
    let mut lab = Lab::online(&[]).await;
    let mut status = lab.supervisor.status();

    let running = timeout(
        WAIT,
        status.wait_for(|status| status.players.is_some() && status.build.is_some()),
    )
    .await
    .expect("the probes never filled the status")
    .unwrap()
    .clone();

    let players = running.players.unwrap();
    assert_eq!((players.clients, players.max_clients), (0, Some(8)));
    assert_eq!(running.build, Some(35245));
    assert!(
        running
            .started_ms
            .is_some_and(|started| started <= running.since_ms)
    );
    lab.supervisor.stop("test").await.unwrap();
    lab.state(State::Stopped).await;
    let stopped = lab.supervisor.status().borrow().clone();
    assert_eq!(
        (stopped.players, stopped.build, stopped.started_ms),
        (None, None, None)
    );
    lab.shut_down(false).await;
}

#[tokio::test]
async fn the_readings_hold_what_the_probes_read_of_the_running_server_only() {
    let mut lab = Lab::online(&[("fake_players", "3"), ("fake_tick_ms", "12")]).await;
    let mut readings = lab.supervisor.readings();

    let run = timeout(
        WAIT,
        readings.wait_for(|readings| {
            readings
                .run
                .as_ref()
                .is_some_and(|run| run.dynamic.is_some() && run.perf.is_some() && run.players.is_some())
        }),
    )
    .await
    .expect("the probes never filled the readings")
    .unwrap()
    .run
    .clone()
    .unwrap();

    assert_eq!(run.generation, lab.generation());
    assert!(
        run.tree.upgrade().is_some(),
        "the tree is there while the run lasts"
    );
    assert_eq!(run.dynamic.unwrap().value.clients, 3);
    assert_eq!(
        run.players.unwrap().value.players,
        [(1, Some(25)), (2, Some(30)), (3, Some(35))],
        "the token Observer set is honoured"
    );
    let main = run.perf.unwrap().value.thread(Thread::Main).cloned().unwrap();
    assert!(main.count > 0);
    let ticks = f64::from(u32::try_from(main.count).unwrap());
    assert!((main.sum * 1000.0 - 12.0 * ticks).abs() < 1e-6, "{main:?}");
    lab.supervisor.stop("test").await.unwrap();
    lab.state(State::Stopped).await;
    assert!(lab.supervisor.readings().borrow().run.is_none());
    assert!(
        run.tree.upgrade().is_none(),
        "nothing holds the tree once the run is over"
    );
    lab.shut_down(false).await;
}

#[tokio::test]
async fn a_new_launch_applies_from_the_next_start() {
    let mut lab = Lab::new(&[]).await;
    // A cfg that leaves the player limit to the command line.
    let cfg = lab.dir.path().join("server-data/server.cfg");
    let text = fs::read_to_string(&cfg).unwrap().replace("sv_maxclients 8\n", "");
    fs::write(&cfg, text).unwrap();
    lab.supervisor.start("test").await.unwrap();
    let mut launch = lab.launch.clone();
    launch.settings.extra_arguments.push(ExtraArgument {
        name: "sv_maxclients".to_owned(),
        value: "16".to_owned(),
    });
    let mut status = lab.supervisor.status();
    let max_clients = |status: &observer_fxserver::supervisor::Status| {
        status.players.and_then(|players| players.max_clients)
    };
    // FXServer's own default.
    timeout(WAIT, status.wait_for(|status| max_clients(status) == Some(48)))
        .await
        .unwrap()
        .unwrap();

    lab.supervisor.configure(launch).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        max_clients(&status.borrow()),
        Some(48),
        "the running server keeps its launch"
    );
    lab.supervisor.restart("test").await.unwrap();

    timeout(WAIT, status.wait_for(|status| max_clients(status) == Some(16)))
        .await
        .expect("the restart did not take the new launch")
        .unwrap();
    lab.shut_down(false).await;
}

#[tokio::test]
async fn a_started_server_goes_online_then_stops_gracefully() {
    let mut lab = Lab::new(&[]).await;

    lab.supervisor.start("test").await.unwrap();

    assert_eq!(
        lab.state(State::Starting).await,
        Cause::Requested {
            by: "test".to_owned()
        }
    );
    let spawned = lab.event(|kind| matches!(kind, EventKind::Spawned { .. })).await;
    assert!(matches!(spawned, EventKind::Spawned { probe, .. } if probe.port() == lab.port.0));
    assert_eq!(lab.state(State::Online).await, Cause::Ready);
    assert!(lab.supervisor.status().borrow().pid.is_some());
    let bridge = lab.bridge();
    lab.console_until(&format!("Started resource {bridge}")).await;
    assert_eq!(
        fs::read_dir(lab.system_resources().join(&bridge))
            .unwrap()
            .count(),
        3
    );

    lab.supervisor.stop("test").await.unwrap();

    assert_eq!(
        lab.state(State::Stopping).await,
        Cause::Requested {
            by: "test".to_owned()
        }
    );
    let notice = lab.console_until("observer:shuttingDown").await;
    let quitting = lab.console_until("-> Quitting: stop").await;
    assert!(notice < quitting, "the bridge event must come before quit");
    let (_, _, event) = &lab.console()[notice];
    assert!(
        event.contains(r#""reason":"stop""#)
            && event.contains(r#""by":"test""#)
            && event.contains(r#""delayMs":300"#),
        "{event}"
    );
    assert!(matches!(
        lab.event(exited).await,
        EventKind::Exited { expected: true, .. }
    ));
    assert!(matches!(lab.state(State::Stopped).await, Cause::Exited { .. }));
    assert_eq!(lab.supervisor.status().borrow().pid, None);
    lab.none_within(Duration::from_secs(1), starting).await;
    lab.shut_down(false).await;
}

#[tokio::test]
async fn a_quit_typed_in_the_console_is_a_stop_not_a_crash() {
    let mut lab = Lab::online(&[]).await;

    lab.type_command("quit", "alice").await;

    assert_eq!(
        lab.state(State::Stopping).await,
        Cause::ConsoleQuit {
            by: "alice".to_owned()
        }
    );
    assert!(matches!(
        lab.event(exited).await,
        EventKind::Exited { expected: true, .. }
    ));
    assert!(matches!(lab.state(State::Stopped).await, Cause::Exited { .. }));
    lab.none_within(Duration::from_secs(1), starting).await;
    assert!(
        lab.console()
            .contains(&(LineKind::Command, Some("alice".to_owned()), "quit".to_owned()))
    );
    lab.shut_down(false).await;
}

#[tokio::test]
async fn each_start_gives_the_bridge_a_new_name_and_removes_the_old_folders() {
    let mut lab = Lab::new(&[]).await;
    let old = lab.system_resources().join("observer");
    fs::create_dir_all(&old).unwrap();
    fs::write(
        old.join("server.lua"),
        "RegisterCommand('observer_cmd', function() end, true)",
    )
    .unwrap();
    lab.supervisor.start("test").await.unwrap();
    lab.state(State::Online).await;
    let first = lab.bridge();

    lab.supervisor.restart("test").await.unwrap();
    lab.state(State::Stopping).await;
    lab.state(State::Online).await;

    let second = lab.bridge();
    assert_ne!(first, second);
    for name in [&first, &second] {
        assert!(bridge::is_folder(name) && !name.contains("observer"), "{name}");
    }
    let folders: Vec<String> = fs::read_dir(lab.system_resources())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(folders, [second.as_str()]);
    lab.console_until(&format!("Started resource {second}")).await;
    lab.shut_down(false).await;
}

#[tokio::test]
async fn a_restart_stops_the_server_then_starts_a_new_generation() {
    let mut lab = Lab::online(&[]).await;

    lab.supervisor.restart("test").await.unwrap();

    assert_eq!(
        lab.state(State::Stopping).await,
        Cause::Restart {
            by: "test".to_owned()
        }
    );
    lab.console_until("-> Quitting: restart").await;
    assert_eq!(
        lab.state(State::Starting).await,
        Cause::Restart {
            by: "test".to_owned()
        }
    );
    lab.state(State::Online).await;
    assert_eq!(lab.generation(), 2);
    lab.shut_down(false).await;
}

#[tokio::test]
async fn a_stop_during_a_restart_ends_it_as_a_stop_by_whoever_asked() {
    let mut lab = Lab::online(&[]).await;
    lab.supervisor.restart("grace").await.unwrap();
    lab.state(State::Stopping).await;

    lab.supervisor.stop("ada").await.unwrap();

    assert_eq!(
        lab.state(State::Stopping).await,
        Cause::Requested { by: "ada".to_owned() }
    );
    lab.state(State::Stopped).await;
    lab.none_within(Duration::from_secs(1), starting).await;
    assert_eq!(lab.generation(), 1);
    // Resources heard of grace's restart first: they must also hear that the server stays down.
    let console = lab.console();
    let quitting = lab.console_index("-> Quitting: stop").unwrap();
    let told: Vec<&str> = console[..quitting]
        .iter()
        .filter_map(|(_, _, text)| Some(text.split_once("observer:shuttingDown ")?.1))
        .collect();
    assert_eq!(told.len(), 2, "{console:#?}");
    assert!(
        told[0].contains(r#""reason":"restart""#) && told[0].contains(r#""by":"grace""#),
        "{}",
        told[0]
    );
    assert!(
        told[1].contains(r#""reason":"stop""#) && told[1].contains(r#""by":"ada""#),
        "{}",
        told[1]
    );
    lab.shut_down(false).await;
}

#[tokio::test]
async fn a_scheduled_restart_quits_with_the_scheduled_reason_then_starts_a_new_generation() {
    let mut lab = Lab::online(&[]).await;
    let scheduled = Cause::Scheduled {
        by: "observer".to_owned(),
    };

    lab.supervisor.restart_scheduled("observer").await.unwrap();

    assert_eq!(lab.state(State::Stopping).await, scheduled);
    let notice = lab.console_until("observer:shuttingDown").await;
    let quit = lab.console_until("-> Quitting: scheduled").await;
    assert_eq!(lab.state(State::Starting).await, scheduled);
    lab.state(State::Online).await;
    assert_eq!(lab.generation(), 2);
    let console = lab.console();
    assert!(notice < quit);
    assert!(
        console[notice].2.contains(r#""reason":"scheduled""#)
            && console[notice].2.contains(r#""by":"observer""#),
        "{}",
        console[notice].2
    );
    lab.shut_down(false).await;
}

#[tokio::test]
async fn a_scheduled_restart_never_starts_a_stopped_or_backing_off_server() {
    let mut stopped = Lab::new(&[]).await;
    let mut backing_off = Lab::new(&[("fake_bind_error", "1")]).await;
    let mut stopping = Lab::online(&[]).await;

    let of_a_stopped_server = stopped.supervisor.restart_scheduled("observer").await;
    backing_off.supervisor.start("test").await.unwrap();
    backing_off.state(State::Crashed).await;
    let while_backing_off = backing_off.supervisor.restart_scheduled("observer").await;
    stopping.supervisor.stop("ada").await.unwrap();
    stopping.state(State::Stopping).await;
    let while_stopping = stopping.supervisor.restart_scheduled("observer").await;

    assert_eq!(of_a_stopped_server, Err(ControlError::NotRunning));
    assert_eq!(while_backing_off, Err(ControlError::NotRunning));
    assert_eq!(while_stopping, Err(ControlError::Stopping));
    stopped.none_within(Duration::from_millis(500), starting).await;
    assert_eq!(stopped.generation(), 0);
    stopping.state(State::Stopped).await;
    stopping.none_within(Duration::from_millis(500), starting).await;
    backing_off.shut_down(false).await;
    stopping.shut_down(false).await;
    stopped.shut_down(false).await;
}

#[tokio::test]
async fn an_announcement_reaches_only_a_running_server_whose_bridge_reported() {
    let mut lab = Lab::new(&[]).await;
    let event = BridgeEvent::RestartScheduled {
        at: 1_793_160_000_000,
        seconds_remaining: 60,
        reason: RestartSource::OneOff,
    };

    let while_stopped = lab.supervisor.announce(event.clone()).await;
    lab.supervisor.start("test").await.unwrap();
    lab.state(State::Online).await;
    let while_online = lab.supervisor.announce(event.clone()).await;
    let raised = lab.console_until("observer:restartScheduled").await;
    lab.supervisor.stop("ada").await.unwrap();
    lab.state(State::Stopping).await;
    let while_stopping = lab.supervisor.announce(event).await;

    assert_eq!(while_stopped, Err(ControlError::NotRunning));
    assert_eq!(while_online, Ok(()));
    assert!(
        lab.console()[raised]
            .2
            .contains(r#"{"at":1793160000000,"reason":"oneOff","secondsRemaining":60}"#),
        "{:?}",
        lab.console()[raised]
    );
    assert_eq!(while_stopping, Err(ControlError::NotRunning));
    lab.state(State::Stopped).await;
    assert_eq!(
        lab.console()
            .iter()
            .filter(|(_, _, text)| text.contains("observer:restartScheduled"))
            .count(),
        1
    );
    lab.shut_down(false).await;
}

#[tokio::test]
async fn a_shutdown_during_a_restart_notice_tells_resources_the_server_stays_down() {
    let mut lab = Lab::online(&[]).await;
    lab.supervisor.restart_scheduled("observer").await.unwrap();
    lab.state(State::Stopping).await;

    lab.shut_down(false).await;

    let console = lab.console();
    let quitting = lab.console_index("-> Quitting: shutdown").unwrap();
    let told: Vec<&str> = console[..quitting]
        .iter()
        .filter_map(|(_, _, text)| Some(text.split_once("observer:shuttingDown ")?.1))
        .collect();
    assert_eq!(told.len(), 2, "{console:#?}");
    assert!(told[0].contains(r#""reason":"scheduled""#), "{}", told[0]);
    assert!(
        told[1].contains(r#""reason":"shutdown""#) && told[1].contains(r#""by":"observer""#),
        "{}",
        told[1]
    );
    while let Ok(event) = lab.events.try_recv() {
        assert!(!starting(&event.kind), "{:#?}", event.kind);
    }
    assert_eq!(lab.generation(), 1);
}

#[tokio::test]
async fn a_stop_while_one_is_under_way_changes_nothing_and_says_so() {
    let mut lab = Lab::online(&[]).await;
    lab.supervisor.stop("ada").await.unwrap();
    lab.state(State::Stopping).await;

    let again = lab.supervisor.stop("grace").await;
    lab.state(State::Stopped).await;
    let mut quitting = Lab::online(&[]).await;
    quitting.type_command("quit", "kenji").await;
    let after_a_quit = quitting.supervisor.stop("grace").await;
    quitting.shut_down(false).await;
    let mut stopping = Lab::online(&[]).await;
    stopping.supervisor.shutdown(false).await;
    let while_observer_stops = stopping.supervisor.stop("ada").await;

    assert_eq!(again, Err(ControlError::Stopping));
    assert_eq!(after_a_quit, Err(ControlError::Stopping));
    assert_eq!(while_observer_stops, Err(ControlError::ShuttingDown));
    timeout(WAIT, stopping.task.take().unwrap())
        .await
        .expect("the supervisor did not stop")
        .unwrap();
    lab.shut_down(false).await;
}

#[tokio::test]
async fn a_crash_is_followed_by_a_restart_after_the_backoff() {
    let mut lab = Lab::online(&[]).await;

    lab.type_command("fake_crash", "test").await;

    assert!(matches!(
        lab.event(exited).await,
        EventKind::Exited { expected: false, .. }
    ));
    let cause = lab.state(State::Crashed).await;
    let crash = if cfg!(windows) {
        "access violation"
    } else {
        "SIGSEGV"
    };
    assert!(
        matches!(&cause, Cause::Exited { description } if description.contains(crash)),
        "{cause:?}"
    );
    assert_eq!(
        lab.event(|kind| matches!(kind, EventKind::RestartScheduled { .. }))
            .await,
        EventKind::RestartScheduled { delay_ms: 100 }
    );
    assert_eq!(lab.state(State::Starting).await, Cause::Recovery);
    lab.state(State::Online).await;
    assert_eq!(lab.generation(), 2);
    lab.shut_down(false).await;
}

#[tokio::test]
async fn a_respawn_that_fails_backs_off_and_tries_again() {
    let mut lab = Lab::online(&[]).await;
    let cfg = lab.dir.path().join("server-data/server.cfg");
    let aside = lab.dir.path().join("server-data/server.cfg.saving");
    fs::rename(&cfg, &aside).unwrap();

    lab.type_command("fake_crash", "test").await;

    lab.state(State::Crashed).await;
    let cause = lab.state(State::Crashed).await;
    assert_eq!(
        cause,
        Cause::SpawnFailed {
            reason: SpawnFailure::CfgUnreadable
        }
    );
    lab.console_until("cannot start FXServer: cannot read the server cfg")
        .await;
    fs::rename(&aside, &cfg).unwrap();
    assert_eq!(lab.state(State::Starting).await, Cause::Recovery);
    lab.state(State::Online).await;
    lab.shut_down(false).await;
}

#[tokio::test]
async fn five_failures_in_a_row_trip_the_breaker_until_a_start_by_hand() {
    let mut lab = Lab::new(&[("fake_crash_after_ms", "300")]).await;
    lab.supervisor.start("test").await.unwrap();

    let mut delays = Vec::new();
    for _ in 0..4 {
        lab.state(State::Crashed).await;
        match lab
            .event(|kind| matches!(kind, EventKind::RestartScheduled { .. }))
            .await
        {
            EventKind::RestartScheduled { delay_ms } => delays.push(delay_ms),
            _ => unreachable!(),
        }
    }
    assert_eq!(delays, [100, 200, 300, 400]);
    lab.state(State::Crashed).await;

    assert_eq!(
        lab.state(State::CrashLoop).await,
        Cause::CrashLoop { failures: 5 }
    );
    lab.none_within(Duration::from_secs(1), starting).await;
    lab.supervisor.start("test").await.unwrap();
    assert_eq!(
        lab.state(State::Starting).await,
        Cause::Requested {
            by: "test".to_owned()
        }
    );
    lab.supervisor.stop("test").await.ok();
    lab.shut_down(false).await;
}

#[tokio::test]
async fn a_hung_main_loop_is_detected_and_the_server_restarted() {
    let mut lab = Lab::online(&[]).await;

    lab.type_command("fake_hang", "test").await;

    let degraded = lab.state(State::Degraded).await;
    assert!(
        matches!(&degraded, Cause::Unhealthy { problems } if problems.contains(&Problem::HeartbeatLate)),
        "{degraded:?}"
    );
    assert_eq!(
        lab.state(State::Hung).await,
        Cause::Hang {
            cause: HangCause::NoHeartbeat
        }
    );
    let exit = lab.event(exited).await;
    assert!(
        matches!(&exit, EventKind::Exited { expected: false, code, description, .. }
            if *code == KILLED && description.starts_with("killed by Observer")),
        "{exit:?}"
    );
    assert_eq!(
        lab.state(State::Crashed).await,
        Cause::Hang {
            cause: HangCause::NoHeartbeat
        }
    );
    assert_eq!(lab.state(State::Starting).await, Cause::Recovery);
    lab.state(State::Online).await;
    lab.shut_down(false).await;
}

/// Adds `lines` to the lab's cfg, with a folder for each resource they start.
fn extend_cfg(lab: &Lab, resources: &[&str], lines: &[&str]) {
    let data = lab.dir.path().join("server-data");
    for resource in resources {
        let folder = data.join("resources").join(resource);
        fs::create_dir_all(&folder).unwrap();
        fs::write(
            folder.join("fxmanifest.lua"),
            "fx_version 'cerulean'\ngame 'gta5'\n",
        )
        .unwrap();
    }
    let cfg = data.join("server.cfg");
    let added: String = resources
        .iter()
        .map(|resource| format!("ensure {resource}\n"))
        .chain(lines.iter().map(|line| format!("{line}\n")))
        .collect();
    fs::write(&cfg, fs::read_to_string(&cfg).unwrap() + &added).unwrap();
}

#[tokio::test]
async fn a_start_longer_than_the_hang_delay_goes_online_while_it_keeps_starting_resources() {
    // Eight resources of 600 ms each: a start of five seconds, under limits that take a server
    // silent for two for hung.
    let mut lab = Lab::new(&[("fake_start_ms", "600")]).await;
    extend_cfg(
        &lab,
        &[
            "maps", "clothes", "vehicles", "weapons", "props", "peds", "sounds", "core",
        ],
        &[],
    );

    lab.supervisor.start("test").await.unwrap();

    assert_eq!(lab.state(State::Online).await, Cause::Ready);
    assert!(
        !lab.seen.iter().any(hung_or_exited),
        "the start was taken for a hang: {:#?}",
        lab.seen
    );
    assert_eq!(lab.generation(), 1);
    lab.console_until("Started resource core").await;
    lab.shut_down(false).await;
}

#[tokio::test]
async fn a_start_that_stops_starting_resources_is_hung_and_says_where_it_stopped() {
    let mut lab = Lab::new(&[("fake_start_ms", "300")]).await;
    // The cfg sticks after its second resource, before any thread of the bridge runs.
    extend_cfg(&lab, &["maps", "clothes"], &["fake_hang"]);

    lab.supervisor.start("test").await.unwrap();

    assert_eq!(
        lab.state(State::Hung).await,
        Cause::Hang {
            cause: HangCause::NoHeartbeat
        }
    );
    let stopped = lab.console_until("Started resource clothes").await;
    let note = lab
        .console_until("its start is stuck: no resource started since clothes, ")
        .await;
    assert!(stopped < note);
    assert_eq!(
        lab.state(State::Crashed).await.clone(),
        Cause::Hang {
            cause: HangCause::NoHeartbeat
        }
    );
    lab.supervisor.stop("test").await.ok();
    lab.shut_down(false).await;
}

#[tokio::test]
async fn a_start_is_followed_through_the_commands_of_its_cfg() {
    // Each resource outlasts a tick, so that every command shows in a status.
    let mut lab = Lab::tuned(&[("fake_start_ms", "1800")], |settings| {
        settings.health.heartbeat_fatal = Duration::from_secs(30);
    })
    .await;
    extend_cfg(&lab, &["maps", "clothes", "core"], &[]);
    assert_eq!(lab.supervisor.status().borrow().start, None);

    lab.supervisor.start("test").await.unwrap();

    let start = lab
        .start_until(|start| {
            start
                .progress
                .at
                .as_ref()
                .is_some_and(|at| at.target == "clothes")
        })
        .await;
    let progress = start.progress;
    let at = progress.at.unwrap();
    assert_eq!(
        (at.file.as_str(), at.line, at.done, at.of),
        ("server.cfg", 6, 0, 1)
    );
    let last = progress.last.unwrap();
    assert_eq!(last.name, "maps");
    assert_eq!(at.since_ms, last.at_ms, "FXServer came to clothes as maps was up");
    assert_eq!(
        (
            progress.started,
            progress.failed,
            progress.planned,
            progress.found
        ),
        (2, 0, Some(3), Some(4)),
        "the bridge and maps, of the three the cfg starts"
    );
    assert!(
        progress.slowest.iter().all(|slow| slow.ms >= 1800),
        "{:?}",
        progress.slowest
    );
    assert!(start.stuck_at_ms.is_some_and(|stuck| stuck > last.at_ms));

    let start = lab
        .start_until(|start| start.progress.at.as_ref().is_some_and(|at| at.target == "core"))
        .await;
    assert_eq!(start.progress.started, 3);
    assert_eq!(lab.state(State::Online).await, Cause::Ready);
    assert_eq!(
        lab.supervisor.status().borrow().start,
        None,
        "a start is over once online"
    );
    lab.shut_down(false).await;
}

#[tokio::test]
async fn a_category_of_the_cfg_is_followed_as_the_resources_under_it() {
    let mut lab = Lab::tuned(&[("fake_start_ms", "1800")], |settings| {
        settings.health.heartbeat_fatal = Duration::from_secs(30);
    })
    .await;
    // Created as `resources/[maps]/docks` and `resources/[maps]/pier`, started by one command.
    extend_cfg(&lab, &[], &["set fake_fail_start pier"]);
    extend_cfg(&lab, &["[maps]/docks", "[maps]/pier"], &[]);
    let cfg = lab.dir.path().join("server-data/server.cfg");
    let text = fs::read_to_string(&cfg)
        .unwrap()
        .replace("ensure [maps]/docks\nensure [maps]/pier\n", "ensure [maps]\n");
    fs::write(&cfg, text).unwrap();

    lab.supervisor.start("test").await.unwrap();

    let start = lab
        .start_until(|start| start.progress.at.as_ref().is_some_and(|at| at.target == "[maps]"))
        .await;
    let at = start.progress.at.unwrap();
    assert_eq!((at.line, at.of), (6, 2));
    assert_eq!(start.progress.planned, Some(2));
    assert_eq!(lab.state(State::Online).await, Cause::Ready);
    lab.console_until("Couldn't start resource pier.").await;
    lab.shut_down(false).await;
}

#[tokio::test]
async fn a_start_given_more_time_is_not_taken_for_stuck() {
    // The bridge alone takes longer to start than the two seconds a start may say nothing.
    let mut lab = Lab::new(&[("fake_start_ms", "3500")]).await;
    assert_eq!(
        lab.supervisor.extend_start("test").await,
        Err(ControlError::NotStarting)
    );
    lab.supervisor.start("test").await.unwrap();
    let before = lab.start_until(|start| start.stuck_at_ms.is_some()).await;

    lab.supervisor.extend_start("ada").await.unwrap();

    let extended = lab
        .start_until(|start| start.stuck_at_ms != before.stuck_at_ms)
        .await;
    let given = extended.stuck_at_ms.unwrap() - before.stuck_at_ms.unwrap();
    let extension = i64::try_from(START_EXTENSION.as_millis()).unwrap();
    assert!((extension - 1000..=extension + 1000).contains(&given), "{given}");
    assert_eq!(lab.state(State::Online).await, Cause::Ready);
    assert!(
        !lab.seen.iter().any(hung_or_exited),
        "the start was taken for a hang: {:#?}",
        lab.seen
    );
    lab.console_until("ada gave this start 15 more minutes before it is taken for stuck")
        .await;
    assert_eq!(
        lab.supervisor.extend_start("test").await,
        Err(ControlError::NotStarting),
        "its cfg has run"
    );
    lab.shut_down(false).await;
}

/// Launch settings with hang detection off; the lab's own launch keeps it on.
fn without_hang_detection(settings: &mut Settings) {
    if let Some(launch) = &mut settings.launch {
        launch.settings.hang_detection = false;
    }
}

fn hung_or_exited(kind: &EventKind) -> bool {
    matches!(
        kind,
        EventKind::State { to: State::Hung, .. } | EventKind::Exited { .. }
    )
}

#[tokio::test]
async fn with_hang_detection_off_a_hung_server_is_left_running_and_a_crash_still_restarts_it() {
    let mut lab = Lab::tuned(&[], without_hang_detection).await;
    lab.supervisor.start("test").await.unwrap();
    lab.state(State::Online).await;

    lab.type_command("fake_crash", "test").await;
    assert!(matches!(lab.state(State::Crashed).await, Cause::Exited { .. }));
    assert_eq!(lab.state(State::Starting).await, Cause::Recovery);
    lab.state(State::Online).await;
    lab.type_command("fake_hang", "test").await;

    let degraded = lab.state(State::Degraded).await;
    assert!(
        matches!(&degraded, Cause::Unhealthy { problems } if problems.contains(&Problem::HeartbeatLate)),
        "{degraded:?}"
    );
    // The lab's limits would call it hung two seconds into the silence.
    lab.none_within(Duration::from_secs(5), hung_or_exited).await;
    assert_eq!(lab.supervisor.status().borrow().state, State::Degraded);
    assert_eq!(lab.generation(), 2);
    lab.shut_down(false).await;
}

#[tokio::test]
async fn hang_detection_turned_back_on_reaches_the_running_server_and_counts_from_the_change() {
    let mut lab = Lab::tuned(&[], without_hang_detection).await;
    lab.supervisor.start("test").await.unwrap();
    lab.state(State::Online).await;
    lab.type_command("fake_hang", "test").await;
    lab.state(State::Degraded).await;
    lab.none_within(Duration::from_secs(3), hung_or_exited).await;

    lab.supervisor.configure(lab.launch.clone()).await;

    // Silent longer than the lab's two seconds already, it gets them again from the change.
    lab.none_within(Duration::from_secs(1), hung_or_exited).await;
    assert_eq!(
        lab.state(State::Hung).await,
        Cause::Hang {
            cause: HangCause::NoHeartbeat
        }
    );
    assert_eq!(
        lab.state(State::Crashed).await,
        Cause::Hang {
            cause: HangCause::NoHeartbeat
        }
    );
    assert_eq!(lab.state(State::Starting).await, Cause::Recovery);
    lab.state(State::Online).await;
    assert_eq!(lab.generation(), 2);
    lab.shut_down(false).await;
}

#[tokio::test]
async fn a_taken_port_backs_off_more_each_time_without_tripping_the_breaker() {
    let lab_port = Port::lease().await;
    let taken = TcpListener::bind(("127.0.0.1", lab_port.0)).unwrap();
    let mut lab = Lab::new(&[]).await;
    // Point the cfg at the taken port.
    let cfg = lab.dir.path().join("server-data/server.cfg");
    fs::write(
        &cfg,
        fs::read_to_string(&cfg)
            .unwrap()
            .replace(&lab.port.0.to_string(), &lab_port.0.to_string()),
    )
    .unwrap();
    lab.supervisor.start("test").await.unwrap();

    let mut delays = Vec::new();
    for _ in 0..6 {
        let cause = lab.state(State::Crashed).await;
        assert_eq!(
            cause,
            Cause::BindError {
                address: format!("127.0.0.1:{}", lab_port.0)
            }
        );
        match lab
            .event(|kind| matches!(kind, EventKind::RestartScheduled { .. }))
            .await
        {
            EventKind::RestartScheduled { delay_ms } => delays.push(delay_ms),
            _ => unreachable!(),
        }
    }
    drop(taken);

    assert_eq!(delays, [300, 600, 900, 900, 900, 900]);
    lab.state(State::Online).await;
    assert!(!lab.seen.iter().any(|kind| matches!(
        kind,
        EventKind::State {
            to: State::CrashLoop,
            ..
        }
    )));
    lab.shut_down(false).await;
}

#[tokio::test]
async fn a_fatal_error_that_leaves_the_process_up_ends_with_the_tree_killed() {
    let mut lab = Lab::new(&[("fake_bind_error", "1"), ("fake_fatal_dialog", "1")]).await;
    lab.supervisor.start("test").await.unwrap();
    lab.state(State::Starting).await;
    let started = Instant::now();

    let exit = lab.event(exited).await;

    assert!(
        matches!(exit, EventKind::Exited { expected: false, code, .. } if code == KILLED),
        "{exit:?}"
    );
    assert!(
        started.elapsed() >= Duration::from_millis(900),
        "killed before the fatal exit grace"
    );
    assert!(matches!(lab.state(State::Crashed).await, Cause::BindError { .. }));
    lab.event(|kind| matches!(kind, EventKind::RestartScheduled { .. }))
        .await;
    lab.supervisor.stop("test").await.unwrap();
    lab.state(State::Stopped).await;
    lab.shut_down(false).await;
}

#[tokio::test]
async fn a_restart_asked_while_observer_shuts_down_is_refused() {
    let mut lab = Lab::new(&[("fake_bind_error", "1"), ("fake_fatal_dialog", "1")]).await;
    lab.supervisor.start("test").await.unwrap();
    lab.console_until("Fatal Error: Could not bind").await;
    // Into the fatal error's grace, where the run is already ending.
    tokio::time::sleep(Duration::from_millis(200)).await;

    lab.supervisor.shutdown(false).await;

    assert_eq!(
        lab.supervisor.restart("panel").await,
        Err(ControlError::ShuttingDown)
    );
    timeout(WAIT, lab.task.take().unwrap())
        .await
        .expect("the supervisor did not stop")
        .unwrap();
    assert_eq!(lab.generation(), 1);
}

#[tokio::test]
async fn a_refused_licence_key_stops_the_server_for_good() {
    let mut lab = Lab::new(&[("fake_license", "invalid")]).await;
    lab.supervisor.start("test").await.unwrap();

    let cause = lab.state(State::Stopped).await;

    assert!(
        matches!(&cause, Cause::LicenceRejected { message } if message.contains("Could not authenticate server license key")),
        "{cause:?}"
    );
    assert!(
        lab.seen
            .iter()
            .any(|kind| matches!(kind, EventKind::Exited { code: Some(1), .. }))
    );
    lab.none_within(Duration::from_secs(1), starting).await;
    lab.shut_down(false).await;
}

#[tokio::test]
async fn a_resource_printing_the_licence_error_leaves_the_server_running() {
    let mut lab = Lab::online(&[]).await;

    // What the stock chat resource prints for a player's message.
    lab.type_command(
        "fake_print Bob^7: A valid license check is required to run this server",
        "test",
    )
    .await;
    lab.type_command(
        "fake_print [          svadhesive] Could not authenticate server license key.",
        "test",
    )
    .await;

    lab.console_until("[          svadhesive] Could not authenticate")
        .await;
    lab.none_within(Duration::from_secs(2), |kind| {
        exited(kind) || matches!(kind, EventKind::State { .. })
    })
    .await;
    assert_eq!(lab.supervisor.status().borrow().state, State::Online);
    lab.shut_down(false).await;
}

#[tokio::test]
async fn a_crash_leaves_no_child_holding_the_pipes_behind() {
    let mut lab = Lab::online(&[("fake_spawn_child", "1")]).await;

    lab.type_command("fake_crash 3", "test").await;

    assert!(matches!(
        lab.event(exited).await,
        EventKind::Exited { code: Some(3), .. }
    ));
    let crashed = Instant::now();
    lab.state(State::Crashed).await;
    // The child keeps the pipes open: only the tree kill after the settle time ends them.
    assert!(crashed.elapsed() < Duration::from_secs(10));
    lab.state(State::Starting).await;
    lab.state(State::Online).await;
    lab.shut_down(false).await;
}

#[tokio::test]
async fn a_crash_dump_is_moved_out_of_the_build() {
    let mut lab = Lab::online(&[("fake_crash_dump", "1")]).await;

    lab.type_command("fake_crash", "test").await;

    let EventKind::CrashDumps { paths: dumps, .. } = lab
        .event(|kind| matches!(kind, EventKind::CrashDumps { .. }))
        .await
    else {
        unreachable!()
    };
    assert_eq!(dumps.len(), 1);
    assert!(
        dumps[0].starts_with(lab.dir.path().join("observer-data/crashes")),
        "{dumps:?}"
    );
    assert!(fs::read(&dumps[0]).unwrap().starts_with(b"MDMP"));
    let build_crashes = fake_program(&lab.dir.path().join("build"))
        .parent()
        .unwrap()
        .join("crashes");
    assert_eq!(fs::read_dir(build_crashes).unwrap().count(), 0);
    lab.console_until("crash dump saved to").await;
    lab.shut_down(false).await;
}

#[tokio::test]
async fn an_ignored_quit_is_followed_by_a_kill() {
    let mut lab = Lab::online(&[("fake_ignore_quit", "1")]).await;
    let stopping = Instant::now();

    lab.supervisor.stop("test").await.unwrap();

    assert!(
        matches!(lab.event(exited).await, EventKind::Exited { expected: true, code, .. } if code == KILLED)
    );
    assert!(
        stopping.elapsed() >= Duration::from_secs(3),
        "killed before the quit timeout"
    );
    lab.state(State::Stopped).await;
    lab.shut_down(false).await;
}

#[tokio::test]
async fn the_shortened_stop_sends_event_and_quit_at_once_and_kills_within_its_window() {
    let mut lab = Lab::online(&[("fake_ignore_quit", "1")]).await;
    let closing = Instant::now();

    lab.shut_down(true).await;

    assert!(
        closing.elapsed() < Duration::from_secs(4),
        "took {:?}",
        closing.elapsed()
    );
    let notice = lab.console_until("observer:shuttingDown").await;
    assert!(lab.console()[notice].2.contains(r#""delayMs":0"#));
    assert!(
        lab.console()
            .iter()
            .any(|(kind, _, text)| *kind == LineKind::Observer && text == "quit \"shutdown\"")
    );
}

#[tokio::test]
async fn a_line_fxserver_has_not_read_yet_is_pending_and_nothing_queues_behind_it() {
    let lab = Lab::online(&[]).await;
    lab.type_command("fake_stall_input 4000", "test").await;
    // Into the stall, which the fake's main loop starts.
    tokio::time::sleep(Duration::from_millis(200)).await;

    let filler = "x".repeat(1000);
    let mut pending = None;
    for index in 0..200 {
        let line = format!("set fake_fill_{index} {filler}");
        match lab
            .supervisor
            .command(UserCommand::parse(&line).unwrap(), "test")
            .await
        {
            Ok(()) => {}
            Err(ControlError::Stdin(StdinError::Pending)) => {
                pending = Some(line);
                break;
            }
            Err(error) => panic!("{error}"),
        }
    }
    let pending = pending.expect("the fake read every line");
    let behind = lab
        .supervisor
        .command(UserCommand::parse("fake_print queued behind").unwrap(), "test")
        .await;

    assert_eq!(behind, Err(ControlError::Stdin(StdinError::Busy)));
    lab.console_until(&pending).await;
    lab.type_command("fake_print after the stall", "test").await;
    lab.console_until("after the stall").await;
    let console = lab.console();
    assert!(console.contains(&(LineKind::Command, Some("test".to_owned()), pending)));
    assert!(!console.iter().any(|(_, _, text)| text.contains("queued behind")));
}

#[tokio::test]
async fn credentials_reset_behind_observer_show_as_an_unauthorized_probe() {
    let mut lab = Lab::online(&[]).await;

    lab.type_command("set sv_prometheusBasicAuthPassword changed", "test")
        .await;

    let cause = lab.state(State::Degraded).await;
    // A slow host may add a late heartbeat; the refused probe is what this is about.
    assert!(
        matches!(&cause, Cause::Unhealthy { problems } if problems.contains(&Problem::ProbeUnauthorized)),
        "{cause:?}"
    );
    assert!(lab.console().contains(&(
        LineKind::Command,
        Some("test".to_owned()),
        "set sv_prometheusBasicAuthPassword [REDACTED]".to_owned()
    )));
    lab.shut_down(false).await;
}

/// The next start that `wanted` accepts on the process bus, skipping the others.
async fn started(
    processes: &mut broadcast::Receiver<TreeEvent>,
    wanted: impl Fn(&ProcessInfo, ProcessKind) -> bool,
) -> (ProcessInfo, ProcessKind) {
    let found = timeout(WAIT, async {
        loop {
            if let ProcessChange::Started { process, kind } = processes.recv().await.unwrap().change
                && wanted(&process, kind)
            {
                return (process, kind);
            }
        }
    })
    .await;
    found.expect("the process never reached the bus")
}

/// Whether the program is `name`, by its image or by its argv[0]: `/bin/sh` is `dash` on Debian.
fn named(process: &ProcessInfo, name: &str) -> bool {
    let image = process.image.as_deref();
    let first = process.args.first().map(Path::new);
    [image, first]
        .into_iter()
        .flatten()
        .filter_map(Path::file_name)
        .any(|file| file.to_string_lossy().eq_ignore_ascii_case(name))
}

/// A shell that lives a few seconds, as a resource would start one.
fn shell_command(seconds: u32) -> String {
    if cfg!(windows) {
        format!("fake_spawn cmd /d /c \"ping -n {} 127.0.0.1 >nul\"", seconds + 1)
    } else {
        format!("fake_spawn /bin/sh -c \"sleep {seconds}; true\"")
    }
}

const SHELL: &str = if cfg!(windows) { "cmd.exe" } else { "sh" };

#[tokio::test]
async fn a_shell_the_server_starts_reaches_the_process_bus_as_a_shell() {
    let mut lab = Lab::online(&[]).await;
    let mut processes = lab.supervisor.processes();
    let pid = lab.supervisor.status().borrow().pid.unwrap();

    lab.type_command(&shell_command(2), "test").await;

    let (shell, kind) = started(&mut processes, |process, _| named(process, SHELL)).await;
    assert_eq!(kind, ProcessKind::Shell);
    assert_eq!(shell.parent, Some(pid));
    assert!(shell.command_line.is_some_and(|line| line.contains('2')));
    let ended = timeout(WAIT, async {
        loop {
            if processes.recv().await.unwrap().change == (ProcessChange::Ended { pid: shell.pid }) {
                return;
            }
        }
    })
    .await;
    assert!(ended.is_ok(), "the shell's end never came");
    lab.shut_down(false).await;
}

#[tokio::test]
async fn the_dump_server_the_server_starts_is_known() {
    let mut lab = Lab::online(&[]).await;
    let mut processes = lab.supervisor.processes();

    lab.type_command("fake_dump_server", "test").await;

    let (process, kind) = started(&mut processes, |process, _| {
        process.args.iter().any(|arg| arg.starts_with("-dumpserver:"))
    })
    .await;
    assert_eq!(kind, ProcessKind::DumpServer, "{process:?}");
    lab.shut_down(false).await;
}

/// FXServer starts one crash dump server as it begins; the same command line later is a
/// resource's, whatever it runs before FXServer's code.
#[cfg(unix)]
#[tokio::test]
async fn a_second_dump_server_of_the_server_is_not_known() {
    let mut lab = Lab::online(&[]).await;
    let mut processes = lab.supervisor.processes();
    let dump_server = |process: &ProcessInfo, _: ProcessKind| {
        process.args.iter().any(|arg| arg.starts_with("-dumpserver:"))
    };

    lab.type_command("fake_dump_server", "test").await;
    let (first, first_kind) = started(&mut processes, dump_server).await;
    lab.type_command("fake_dump_server", "test").await;
    let (second, second_kind) = started(&mut processes, |process, kind| {
        process.pid != first.pid && dump_server(process, kind)
    })
    .await;

    lab.shut_down(false).await;
    assert_eq!(first_kind, ProcessKind::DumpServer, "{first:?}");
    assert_eq!(second_kind, ProcessKind::Other, "{second:?}");
}

/// A resource giving bash the dump server's run.sh line, with a startup file of its own added to
/// the environment as Node's `spawn(.., { env })` adds it: bash runs that file before run.sh.
#[cfg(unix)]
#[tokio::test]
async fn bash_given_run_sh_and_a_startup_file_of_a_resource_is_a_shell() {
    let mut lab = Lab::online(&[]).await;
    let mut processes = lab.supervisor.processes();
    let ran = lab.dir.path().join("startup-ran");
    let startup = lab.dir.path().join("server-data/start.sh");
    fs::write(&startup, format!("echo $$ > {}\nsleep 3\n", ran.display())).unwrap();
    let run_sh = lab.dir.path().join("build/run.sh");

    lab.type_command(
        &format!(
            "fake_spawn BASH_ENV={} /bin/bash {} -dumpserver:0 -parentppe:1",
            startup.display(),
            run_sh.display()
        ),
        "test",
    )
    .await;

    let (process, kind) = started(&mut processes, |process, _| named(process, "bash")).await;
    let startup_ran = timeout(WAIT, async {
        while !ran.exists() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    lab.shut_down(false).await;
    assert!(startup_ran.is_ok(), "bash never ran the startup file");
    assert_eq!(kind, ProcessKind::Shell, "{process:?}");
}

#[tokio::test]
async fn a_dropped_notification_is_made_up_for_by_the_reconciliation() {
    let mut lab = Lab::tuned(&[], |settings| settings.drop_first_tree_change = true).await;
    let mut processes = lab.supervisor.processes();
    lab.supervisor.start("test").await.unwrap();
    lab.state(State::Online).await;

    // On Windows the first process of the tree is FXServer's console host; on Linux, this shell.
    let typed = Instant::now();
    lab.type_command(&shell_command(20), "test").await;

    let first = if cfg!(windows) { "conhost.exe" } else { SHELL };
    let (_, kind) = started(&mut processes, |process, _| named(process, first)).await;
    let expected = if cfg!(windows) {
        ProcessKind::ConsoleHost
    } else {
        ProcessKind::Shell
    };
    assert_eq!(kind, expected);
    // A notification reports the shell within milliseconds of its start; the reconciliation only
    // takes processes at least two seconds old, whenever its next tick falls after the spawn.
    assert!(
        cfg!(windows) || typed.elapsed() >= Duration::from_millis(1_900),
        "the shell came before the reconciliation"
    );
    lab.shut_down(false).await;
}

fn hang_snapshot(limits: HangSnapshotLimits) -> impl FnOnce(&mut Settings) {
    move |settings| settings.hang_snapshot = Some(limits)
}

/// Hangs the server with `command` and returns its snapshot event, which comes before the exit.
async fn hang_and_snapshot(lab: &mut Lab, command: &str) -> EventKind {
    lab.supervisor.start("test").await.unwrap();
    lab.state(State::Online).await;
    lab.type_command(command, "test").await;
    lab.state(State::Hung).await;
    let snapshot = lab
        .event(|kind| matches!(kind, EventKind::HangSnapshot { .. } | EventKind::Exited { .. }))
        .await;
    assert!(matches!(snapshot, EventKind::HangSnapshot { .. }), "{snapshot:?}");
    lab.event(exited).await;
    snapshot
}

#[tokio::test]
async fn a_hung_server_is_dumped_before_its_tree_is_killed() {
    let mut lab = Lab::tuned(&[], hang_snapshot(HangSnapshotLimits::default())).await;

    let EventKind::HangSnapshot {
        path, bytes, failure, ..
    } = hang_and_snapshot(&mut lab, "fake_hang").await
    else {
        unreachable!()
    };

    assert_eq!(failure, None);
    let path = path.unwrap();
    assert!(
        path.starts_with(lab.dir.path().join("observer-data/crashes")),
        "{path:?}"
    );
    assert_eq!(fs::metadata(&path).unwrap().len(), bytes);
    #[cfg(windows)]
    {
        let read = analysis::analyse(&path).unwrap();
        assert_eq!(read.analysis.exception, None);
        assert_eq!(
            read.analysis.thread.and_then(|thread| thread.name).as_deref(),
            Some("UV loop: svMain")
        );
        let in_known_module = read.analysis.frames.iter().any(|frame| {
            frame.module.as_deref().is_some_and(|module| {
                module.eq_ignore_ascii_case("ntdll.dll") || module.eq_ignore_ascii_case("FXServer.exe")
            })
        });
        assert!(in_known_module, "{:?}", read.analysis.frames);
        assert!(
            read.signature.starts_with("hang UV loop: svMain"),
            "{}",
            read.signature
        );
    }
    #[cfg(unix)]
    {
        let read = analysis::analyse_report(&path).unwrap();
        let main = &read.analysis.threads[0];
        assert_eq!(main.name, "UV loop: svMain");
        assert_eq!(main.state, ThreadRun::Sleeping);
        if read.analysis.syscalls_readable == Some(true) {
            assert!(
                matches!(main.syscall.as_deref(), Some("clock_nanosleep" | "nanosleep")),
                "{main:?}"
            );
        }
    }
    lab.shut_down(false).await;
}

#[tokio::test]
async fn once_hang_snapshots_are_skipped_a_hung_server_is_killed_without_one() {
    let mut lab = Lab::tuned(&[], hang_snapshot(HangSnapshotLimits::default())).await;
    lab.supervisor.skip_hang_snapshots().await;
    lab.supervisor.start("test").await.unwrap();
    lab.state(State::Online).await;

    lab.type_command("fake_hang", "test").await;

    lab.state(State::Hung).await;
    let next = lab
        .event(|kind| matches!(kind, EventKind::HangSnapshot { .. } | EventKind::Exited { .. }))
        .await;
    assert!(matches!(next, EventKind::Exited { .. }), "{next:?}");
    lab.shut_down(false).await;
}

#[tokio::test]
async fn a_busy_server_is_pictured_with_svmain_on_the_processor() {
    let mut lab = Lab::tuned(&[], hang_snapshot(HangSnapshotLimits::default())).await;

    let EventKind::HangSnapshot { path, .. } = hang_and_snapshot(&mut lab, "fake_spin").await else {
        unreachable!()
    };

    let path = path.unwrap();
    #[cfg(windows)]
    assert!(analysis::analyse(&path).unwrap().analysis.thread.is_some());
    #[cfg(unix)]
    {
        let read = analysis::analyse_report(&path).unwrap();
        let main = &read.analysis.threads[0];
        assert_eq!(main.state, ThreadRun::Running, "{main:?}");
        assert!(main.cpu.is_some_and(|cpu| cpu > 50.0), "{main:?}");
    }
    lab.shut_down(false).await;
}

#[tokio::test]
async fn a_hang_snapshot_is_skipped_when_the_disk_is_nearly_full() {
    let limits = HangSnapshotLimits {
        min_free: u64::MAX,
        ..HangSnapshotLimits::default()
    };
    let mut lab = Lab::tuned(&[], hang_snapshot(limits)).await;

    let snapshot = hang_and_snapshot(&mut lab, "fake_hang").await;

    assert!(
        matches!(
            snapshot,
            EventKind::HangSnapshot {
                path: None,
                failure: Some(SnapshotFailure::LowDisk),
                ..
            }
        ),
        "{snapshot:?}"
    );
    lab.console_until("no snapshot of the hung server").await;
    lab.shut_down(false).await;
}

#[tokio::test]
async fn a_slow_snapshot_does_not_hold_the_kill_past_its_limit() {
    let limits = HangSnapshotLimits {
        timeout: Duration::from_millis(1),
        ..HangSnapshotLimits::default()
    };
    let mut lab = Lab::tuned(&[], hang_snapshot(limits)).await;

    let snapshot = hang_and_snapshot(&mut lab, "fake_hang").await;

    assert!(
        matches!(
            snapshot,
            EventKind::HangSnapshot {
                path: None,
                failure: Some(SnapshotFailure::Timeout),
                ..
            }
        ),
        "{snapshot:?}"
    );
    // The snapshot finished after the kill and was thrown away.
    tokio::time::sleep(Duration::from_secs(1)).await;
    let crashes = lab.dir.path().join("observer-data/crashes");
    let left: Vec<String> = fs::read_dir(&crashes)
        .map(|entries| {
            entries
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    assert!(left.iter().all(|name| !name.starts_with("hang-")), "{left:?}");
    lab.shut_down(false).await;
}

#[tokio::test]
async fn the_connections_the_server_opens_are_listed_with_its_listener() {
    let mut lab = Lab::online(&[]).await;
    let pid = lab.supervisor.status().borrow().pid.unwrap();
    let peer = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = peer.local_addr().unwrap();
    let udp_peer = UdpSocket::bind("127.0.0.1:0").unwrap();
    let udp_address = udp_peer.local_addr().unwrap();

    lab.type_command(&format!("fake_connect {address}"), "test").await;
    lab.type_command(&format!("fake_udp {udp_address}"), "test").await;
    let _accepted = peer.accept().unwrap();
    let held = |sockets: &[Socket]| {
        sockets
            .iter()
            .any(|socket| socket.state == SocketState::Established && socket.remote == Some(address))
    };
    let deadline = Instant::now() + WAIT;
    let mut sockets = observer_platform::sockets(&[pid]).unwrap();
    while !(held(&sockets)
        && sockets
            .iter()
            .any(|socket| socket.local.port() != lab.port.0 && socket.protocol == Protocol::Udp))
        && Instant::now() < deadline
    {
        tokio::time::sleep(Duration::from_millis(50)).await;
        sockets = observer_platform::sockets(&[pid]).unwrap();
    }

    assert!(held(&sockets), "{sockets:#?}");
    let listener: SocketAddr = format!("127.0.0.1:{}", lab.port.0).parse().unwrap();
    assert!(
        sockets
            .iter()
            .any(|socket| socket.state == SocketState::Listen && socket.local == listener),
        "{sockets:#?}"
    );
    let udp = sockets
        .iter()
        .find(|socket| socket.protocol == Protocol::Udp && socket.local.port() != lab.port.0)
        .unwrap_or_else(|| panic!("no UDP socket in {sockets:#?}"));
    // Windows' table holds no remote end for UDP.
    let remote = if cfg!(windows) { None } else { Some(udp_address) };
    assert_eq!(udp.remote, remote);
    assert!(sockets.iter().all(|socket| socket.pid == pid));

    lab.type_command("fake_disconnect", "test").await;
    let deadline = Instant::now() + WAIT;
    while held(&observer_platform::sockets(&[pid]).unwrap()) && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(!held(&observer_platform::sockets(&[pid]).unwrap()));
    lab.shut_down(false).await;
}

/// An older dump in Observer's crash folder that another program holds open, as a debugger
/// reading it does: it may be read, not deleted.
#[cfg(windows)]
fn held_old_dump(lab: &Lab) -> fs::File {
    use std::os::windows::fs::OpenOptionsExt as _;
    const FILE_SHARE_READ: u32 = 1;

    let crashes = lab.dir.path().join("observer-data/crashes");
    fs::create_dir_all(&crashes).unwrap();
    let old = crashes.join("00000000-0000-4000-8000-000000000000.dmp");
    fs::write(&old, b"MDMP").unwrap();
    fs::File::options()
        .write(true)
        .open(&old)
        .unwrap()
        .set_modified(std::time::SystemTime::now() - Duration::from_hours(1))
        .unwrap();
    fs::File::options()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(&old)
        .unwrap()
}

#[cfg(windows)]
#[tokio::test]
async fn a_hang_dump_is_reported_though_an_older_dump_could_not_be_removed() {
    let mut lab = Lab::tuned(&[], |settings| {
        settings.hang_snapshot = Some(HangSnapshotLimits::default());
        settings.dumps = observer_fxserver::crashes::DumpLimits {
            files: 1,
            ..observer_fxserver::crashes::DumpLimits::default()
        };
    })
    .await;
    let _held = held_old_dump(&lab);

    let snapshot = hang_and_snapshot(&mut lab, "fake_hang").await;

    let EventKind::HangSnapshot {
        path: Some(path),
        failure: None,
        removed,
        ..
    } = &snapshot
    else {
        panic!("{snapshot:?}")
    };
    assert!(path.exists(), "{path:?}");
    assert_eq!(removed, &Vec::<PathBuf>::new());
    lab.shut_down(false).await;
}

#[tokio::test]
async fn dumps_the_size_cap_removes_are_reported_even_when_nothing_moved_stays() {
    let mut lab = Lab::tuned(&[("fake_crash_dump", "1")], |settings| {
        settings.dumps = observer_fxserver::crashes::DumpLimits {
            files: 10,
            bytes: 64 * 1024,
        };
    })
    .await;
    lab.supervisor.start("test").await.unwrap();
    lab.state(State::Online).await;
    lab.type_command("fake_crash", "test").await;
    let EventKind::CrashDumps { paths: first, .. } = lab
        .event(|kind| matches!(kind, EventKind::CrashDumps { .. }))
        .await
    else {
        unreachable!()
    };
    lab.state(State::Online).await;
    // FXServer's full dump of the next crash, larger than the cap on its own, and finished after
    // its minidump.
    let build_crashes = fake_program(&lab.dir.path().join("build"))
        .parent()
        .unwrap()
        .join("crashes");
    fs::create_dir_all(&build_crashes).unwrap();
    let full = build_crashes.join("0badc0de-full.dmp");
    fs::write(&full, vec![0; 128 * 1024]).unwrap();
    fs::File::options()
        .write(true)
        .open(&full)
        .unwrap()
        .set_modified(std::time::SystemTime::now() + Duration::from_hours(1))
        .unwrap();

    lab.type_command("fake_crash", "test").await;

    let EventKind::CrashDumps { paths, removed } = lab
        .event(|kind| matches!(kind, EventKind::CrashDumps { .. }))
        .await
    else {
        unreachable!()
    };
    assert!(!first[0].exists());
    assert!(removed.contains(&first[0]), "{removed:?}");
    assert_eq!(paths, Vec::<PathBuf>::new());
    lab.shut_down(false).await;
}

/// Node's `spawn(cmd, { detached: true, env: {...} })` does the same: a session and an environment
/// of its own, so neither FXServer's group nor its marker finds the child.
#[cfg(unix)]
#[tokio::test]
async fn a_detached_child_with_an_environment_of_its_own_is_not_ended_while_it_runs() {
    let mut lab = Lab::online(&[]).await;
    let mut processes = lab.supervisor.processes();

    lab.type_command("fake_spawn /usr/bin/env -i /usr/bin/setsid /bin/sleep 25", "test")
        .await;

    let (process, _) = started(&mut processes, |process, _| {
        process.args.iter().any(|arg| arg == "25")
    })
    .await;
    // Two reconciliations at least.
    let ended = timeout(Duration::from_secs(12), async {
        loop {
            if processes.recv().await.unwrap().change == (ProcessChange::Ended { pid: process.pid }) {
                return;
            }
        }
    })
    .await;
    let alive = Path::new(&format!("/proc/{}", process.pid)).exists();
    // It left the group: the tree's kill does not reach it.
    let _ = std::process::Command::new("kill")
        .arg(process.pid.to_string())
        .status();
    lab.shut_down(false).await;
    assert!(
        ended.is_err() || !alive,
        "{process:?} was reported ended while it ran"
    );
}
