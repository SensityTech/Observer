//! The fake driven through Observer's own spawn: the same pipes, fd 3 and process tree the real
//! server gets.
// The helpers below are test code too, though not inside a `#[test]` function.
#![allow(clippy::unwrap_used)]

use std::collections::VecDeque;
use std::ffi::OsString;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

use observer_fxserver::bridge;
use observer_fxserver::layout::{Flavour, Layout};
use observer_fxserver::trace::{Trace, TraceDecoder, TraceKind};
use observer_platform::{Exit, Launch, Output, Spawned, spawn};
use serde_json::json;
use tempfile::TempDir;
use tokio::sync::watch;
use tokio::time::timeout;

const WAIT: Duration = Duration::from_secs(10);

#[cfg(windows)]
const KILLED: Exit = Exit::Code(1);
#[cfg(unix)]
const KILLED: Exit = Exit::Signal(9);
/// How FXServer 35245 ends after `quit`.
#[cfg(windows)]
const QUIT: Exit = Exit::Code(1);
#[cfg(unix)]
const QUIT: Exit = Exit::Signal(11);

struct Fake {
    spawned: Spawned,
    /// The cfg's endpoint, on 127.0.0.1.
    port: Option<u16>,
    /// The name the bridge was installed and ensured under.
    bridge: String,
    stdout: String,
    traces: VecDeque<Trace>,
    decoder: TraceDecoder,
    _generation: watch::Sender<u32>,
    _dir: TempDir,
}

impl Fake {
    /// The fake with a build folder of its own, given as `citizen_dir`, which holds the bridge as
    /// Observer writes it; the bridge is ensured before the cfg.
    fn start(port: Option<u16>, extra: &[&str]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        fs::create_dir_all(&data).unwrap();
        let endpoint = port
            .map(|port| format!("endpoint_add_tcp \"127.0.0.1:{port}\"\n"))
            .unwrap_or_default();
        fs::write(
            data.join("server.cfg"),
            format!("{endpoint}sv_hostname \"Observer test\"\nsv_maxclients 8\n"),
        )
        .unwrap();
        let layout = Layout::new(dir.path().join("build"), Flavour::HOST);
        let bridge = bridge::draw_name().unwrap();
        bridge::install(&layout, &bridge).unwrap();
        let mut citizen = layout.citizen_root().join("citizen").into_os_string();
        citizen.push("/");
        let mut args: Vec<OsString> = vec![
            "+set".into(),
            "citizen_dir".into(),
            citizen,
            "+ensure".into(),
            bridge.clone().into(),
            "+exec".into(),
            "server.cfg".into(),
        ];
        args.extend(extra.iter().map(OsString::from));
        let spawned = spawn(&Launch {
            program: env!("CARGO_BIN_EXE_fake-fxserver").into(),
            args,
            cwd: data,
        })
        .unwrap();
        let (generation, current) = watch::channel(1);
        Self {
            spawned,
            port,
            bridge,
            stdout: String::new(),
            traces: VecDeque::new(),
            decoder: TraceDecoder::new(1, current),
            _generation: generation,
            _dir: dir,
        }
    }

    async fn bridge_started(&mut self) {
        let started = format!("Started resource {}", self.bridge);
        self.stdout_until(&started).await;
    }

    /// Waits for the endpoint to take connections. Like FXServer, the fake binds it once its whole
    /// command line has run, after the bridge started.
    async fn serving(&self) {
        let port = self.port.expect("the fake was started without an endpoint");
        let deadline = Instant::now() + WAIT;
        while TcpStream::connect(("127.0.0.1", port)).is_err() {
            assert!(Instant::now() < deadline, "nothing listens on {port}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn send(&self, line: &str) {
        self.spawned
            .stdin
            .write(format!("{line}\n").into_bytes())
            .await
            .unwrap();
    }

    async fn stdout_until(&mut self, needle: &str) {
        let found = timeout(WAIT, async {
            while !self.stdout.contains(needle) {
                let Some(chunk) = self.spawned.stdout.next().await else {
                    return false;
                };
                self.stdout.push_str(&String::from_utf8_lossy(&chunk));
            }
            true
        })
        .await;
        assert_eq!(found, Ok(true), "no {needle:?} on stdout, got {:?}", self.stdout);
    }

    async fn trace(&mut self, wanted: impl Fn(&TraceKind) -> bool) -> TraceKind {
        let found = timeout(WAIT, async {
            loop {
                while let Some(trace) = self.traces.pop_front() {
                    if wanted(&trace.kind) {
                        return Some(trace.kind);
                    }
                }
                let chunk = self.spawned.traces.next().await?;
                self.traces.extend(self.decoder.push(&chunk, Instant::now()));
            }
        })
        .await;
        found.ok().flatten().expect("the trace never came")
    }

    async fn exit(&mut self) -> Exit {
        timeout(WAIT, self.spawned.exit.wait())
            .await
            .expect("the process did not exit")
    }

    async fn kill(&mut self) {
        self.spawned.tree.kill().unwrap();
        assert!(self.spawned.tree.wait_empty(WAIT).await.unwrap());
    }
}

async fn drain(output: &mut Output) {
    while output.next().await.is_some() {}
}

/// A plain HTTP/1.1 GET; the fake closes the connection after each answer.
fn get(port: u16, path: &str, authorization: Option<&str>) -> (u16, String) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream.set_read_timeout(Some(WAIT)).unwrap();
    let authorization = authorization
        .map(|value| format!("Authorization: {value}\r\n"))
        .unwrap_or_default();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n{authorization}\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let status = response[9..12].parse().unwrap();
    let body = response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.to_owned())
        .unwrap_or_default();
    (status, body)
}

fn main_ticks(port: u16, authorization: Option<&str>) -> u64 {
    let (status, body) = get(port, "/perf/", authorization);
    assert_eq!(status, 200);
    body.lines()
        .find_map(|line| line.strip_prefix("tickTime_count{name=\"svMain\"} "))
        .and_then(|count| count.parse().ok())
        .unwrap()
}

#[tokio::test]
async fn a_started_server_speaks_on_every_channel_then_quits() {
    let mut fake = Fake::start(Some(41920), &["+set", "fake_heartbeat_ms", "100"]);

    fake.bridge_started().await;
    assert_eq!(fake.trace(|_| true).await, TraceKind::Probe);
    let ready = json!({ "type": "observer", "kind": "ready", "contractVersion": 3 });
    fake.trace(|kind| matches!(kind, TraceKind::Script { payload, .. } if *payload == ready))
        .await;
    let bridge = fake.bridge.clone();
    for _ in 0..3 {
        fake.trace(|kind| matches!(kind, TraceKind::Script { resource, payload } if *resource == bridge && payload["kind"] == "hb"))
            .await;
    }
    let (status, body) = get(41920, "/dynamic.json", None);
    assert_eq!(status, 200);
    let dynamic: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(dynamic["hostname"], "Observer test");
    assert_eq!(dynamic["sv_maxclients"], "8");
    assert_eq!(get(41920, "/players.json", None), (200, "[]".to_owned()));

    fake.send("quit").await;

    fake.stdout_until("-> Quitting: Quit command executed.").await;
    assert_eq!(fake.exit().await, QUIT);
    assert!(fake.spawned.tree.wait_empty(WAIT).await.unwrap());
}

#[tokio::test]
async fn unknown_commands_are_answered_like_fxserver_does() {
    let mut fake = Fake::start(None, &[]);

    fake.send("frobnicate now").await;

    fake.stdout_until("No such command frobnicate.").await;
    fake.kill().await;
}

#[tokio::test]
async fn perf_requires_the_credentials_set_on_stdin() {
    let mut fake = Fake::start(Some(41922), &[]);
    fake.serving().await;
    assert_eq!(get(41922, "/perf/", None).0, 200);

    fake.send("set sv_prometheusBasicAuthUser observer").await;
    fake.send("set sv_prometheusBasicAuthPassword secret").await;

    let deadline = Instant::now() + WAIT;
    while get(41922, "/perf/", None).0 != 401 {
        assert!(
            Instant::now() < deadline,
            "/perf/ still answers without credentials"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(get(41922, "/perf/", Some("Basic b2JzZXJ2ZXI6d3Jvbmc=")).0, 401);
    assert!(main_ticks(41922, Some("Basic b2JzZXJ2ZXI6c2VjcmV0")) > 0);
    fake.kill().await;
}

#[tokio::test]
async fn a_hung_main_loop_freezes_the_tick_count_while_http_still_answers() {
    let mut fake = Fake::start(Some(41921), &[]);
    fake.serving().await;

    fake.send("fake_hang").await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let frozen = main_ticks(41921, None);
    tokio::time::sleep(Duration::from_millis(500)).await;

    assert_eq!(main_ticks(41921, None), frozen);
    assert_eq!(get(41921, "/dynamic.json", None).0, 200);
    fake.send("quit").await;
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_eq!(fake.spawned.exit.get(), None, "a hung main loop cannot run quit");
    fake.kill().await;
    assert_eq!(fake.exit().await, KILLED);
}

#[tokio::test]
async fn a_taken_port_is_a_bind_error_then_a_fatal_exit() {
    let _taken = TcpListener::bind("127.0.0.1:41923").unwrap();
    let mut fake = Fake::start(Some(41923), &[]);

    let error = fake
        .trace(|kind| matches!(kind, TraceKind::BindError { .. }))
        .await;

    assert_eq!(
        error,
        TraceKind::BindError {
            address: "127.0.0.1:41923".to_owned()
        }
    );
    assert_eq!(fake.exit().await, Exit::Code(1));
    let mut stderr = Vec::new();
    while let Some(chunk) = fake.spawned.stderr.next().await {
        stderr.extend(chunk);
    }
    assert!(String::from_utf8_lossy(&stderr).starts_with("Fatal Error: Could not bind on 127.0.0.1:41923"));
}

#[tokio::test]
async fn a_fatal_error_can_leave_the_process_up_until_the_tree_is_killed() {
    let mut fake = Fake::start(
        Some(41924),
        &["+set", "fake_bind_error", "1", "+set", "fake_fatal_dialog", "1"],
    );

    fake.trace(|kind| matches!(kind, TraceKind::BindError { .. }))
        .await;
    tokio::time::sleep(Duration::from_millis(500)).await;

    assert_eq!(fake.spawned.exit.get(), None);
    fake.kill().await;
    assert_eq!(fake.exit().await, KILLED);
}

#[tokio::test]
async fn an_ignored_quit_leaves_the_tree_kill() {
    let mut fake = Fake::start(
        None,
        &[
            "+set",
            "fake_ignore_quit",
            "1",
            "+set",
            "fake_quit_delay_ms",
            "50",
        ],
    );
    fake.bridge_started().await;

    fake.send("quit").await;
    tokio::time::sleep(Duration::from_millis(500)).await;

    assert_eq!(fake.spawned.exit.get(), None);
    fake.kill().await;
    assert_eq!(fake.exit().await, KILLED);
}

#[tokio::test]
async fn a_crash_is_reported_with_the_platform_code() {
    let mut fake = Fake::start(None, &[]);
    fake.bridge_started().await;

    fake.send("fake_crash").await;

    #[cfg(windows)]
    assert_eq!(fake.exit().await, Exit::Code(0xC000_0005));
    #[cfg(unix)]
    assert_eq!(fake.exit().await, Exit::Signal(11));
}

#[tokio::test]
async fn native_traces_come_on_demand() {
    let mut fake = Fake::start(None, &[]);

    fake.send("fake_trace watchdog_bark").await;

    assert_eq!(
        fake.trace(|kind| matches!(kind, TraceKind::WatchdogBark { .. }))
            .await,
        TraceKind::WatchdogBark {
            thread: "svMain".to_owned(),
            seconds: 60,
            stack: "fake <- root".to_owned()
        }
    );
    fake.kill().await;
}

#[tokio::test]
async fn killing_the_tree_ends_a_child_that_still_holds_the_pipes() {
    let mut fake = Fake::start(None, &["+set", "fake_spawn_child", "1"]);
    fake.bridge_started().await;

    fake.send("fake_crash 3").await;
    assert_eq!(fake.exit().await, Exit::Code(3));

    // The child inherited stdout: the main process is gone but the pipe stays open.
    assert!(
        timeout(Duration::from_millis(500), drain(&mut fake.spawned.stdout))
            .await
            .is_err()
    );
    assert!(!fake.spawned.tree.is_empty().unwrap());
    fake.kill().await;
    assert!(timeout(WAIT, drain(&mut fake.spawned.stdout)).await.is_ok());
}

/// Measured on FXServer 38220: every `+set` runs before the cfg, which keeps its own value for a
/// convar it sets too, its name alone followed by the value included.
#[tokio::test]
async fn the_cfg_wins_over_the_command_line_set_and_a_convar_typed_alone_prints_its_value() {
    let mut fake = Fake::start(
        None,
        &["+set", "sv_hostname", "from_cli", "+set", "lab_cli_only", "cli"],
    );

    fake.send("sv_hostname").await;
    fake.stdout_until(" \"sv_hostname\" is \"Observer test\"").await;
    fake.send("LAB_CLI_ONLY").await;
    fake.stdout_until(" \"LAB_CLI_ONLY\" is \"cli\"").await;
    fake.send("lab_never_set").await;
    fake.stdout_until("No such command lab_never_set.").await;
    fake.kill().await;
}
