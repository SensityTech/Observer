//! The supervisor driving the fake through Observer's real launch: the build layout, the bridge,
//! the console, the probes and the process tree.
// The helpers below are test code too, though not inside a `#[test]` function.
#![allow(clippy::unwrap_used)]

use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};
use std::time::{Duration, Instant};

use observer_fxserver::console::{ConsoleHub, LineKind, plain};
use observer_fxserver::health::{HangCause, HealthLimits, Problem};
use observer_fxserver::launch::LaunchSettings;
use observer_fxserver::recovery::RecoveryPolicy;
use observer_fxserver::stdin::UserCommand;
use observer_fxserver::supervisor::{Cause, Event, EventKind, Settings, State, StopTimings, Supervisor};
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

/// The ports tests may use, lent to one test at a time.
static PORTS: LazyLock<Mutex<Vec<u16>>> = LazyLock::new(|| Mutex::new((41920..=41929).collect()));

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
    console: Arc<ConsoleHub>,
    supervisor: Supervisor,
    task: Option<JoinHandle<()>>,
    events: broadcast::Receiver<Event>,
    seen: Vec<EventKind>,
    port: Port,
}

impl Lab {
    async fn new(extra: &[(&str, &str)]) -> Self {
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

        let mut extra: Vec<(String, String)> = extra
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        extra.push(("fake_heartbeat_ms".to_owned(), HEARTBEAT_MS.to_owned()));
        let mut settings = Settings::new(
            LaunchSettings {
                build: dir.path().join("build"),
                data,
                cfg: PathBuf::from("server.cfg"),
                onesync: None,
                txadmin_events: false,
                extra,
            },
            dir.path().join("observer-data"),
        );
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

        let console = Arc::new(ConsoleHub::new(None));
        let (supervisor, task) = Supervisor::spawn(settings, Arc::clone(&console));
        let events = supervisor.events();
        Self {
            dir,
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
    lab.console_until("Started resource observer").await;
    let citizen = fake_program(&lab.dir.path().join("build"))
        .parent()
        .unwrap()
        .join("citizen");
    assert_eq!(
        fs::read_dir(citizen.join("system_resources/observer"))
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
async fn a_restart_stops_the_server_then_starts_a_new_generation() {
    let mut lab = Lab::online(&[]).await;

    lab.supervisor.restart("test").await.unwrap();

    assert_eq!(
        lab.state(State::Stopping).await,
        Cause::Requested {
            by: "test".to_owned()
        }
    );
    lab.console_until("-> Quitting: restart").await;
    assert_eq!(
        lab.state(State::Starting).await,
        Cause::Requested {
            by: "test".to_owned()
        }
    );
    lab.state(State::Online).await;
    assert_eq!(lab.generation(), 2);
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
    assert!(
        matches!(lab.event(exited).await, EventKind::Exited { expected: false, code, .. } if code == KILLED)
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

    let EventKind::CrashDumps { paths: dumps } = lab
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
    assert_eq!(fs::read(&dumps[0]).unwrap(), b"MDMP");
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
async fn credentials_reset_behind_observer_show_as_an_unauthorized_probe() {
    let mut lab = Lab::online(&[]).await;

    lab.type_command("set sv_prometheusBasicAuthPassword changed", "test")
        .await;

    let cause = lab.state(State::Degraded).await;
    assert_eq!(
        cause,
        Cause::Unhealthy {
            problems: vec![Problem::ProbeUnauthorized]
        }
    );
    assert!(lab.console().contains(&(
        LineKind::Command,
        Some("test".to_owned()),
        "set sv_prometheusBasicAuthPassword [REDACTED]".to_owned()
    )));
    lab.shut_down(false).await;
}
