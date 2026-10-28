//! `observer run` as it ships: the built binary against the fake FXServer, driven over HTTP the
//! way the panel drives it, then asked to stop the way a service manager does.
// The helpers below are test code too, though not inside a `#[test]` function.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fmt::Write as _;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tempfile::TempDir;

const WAIT: Duration = Duration::from_secs(30);
const PASSWORD: &str = "correct horse battery staple";

/// A port nothing listens on, which the caller then listens on.
fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// `cargo test --workspace` builds the fake for its own tests, next to Observer.
fn fake() -> PathBuf {
    let observer = Path::new(env!("CARGO_BIN_EXE_observer"));
    let fake = observer.with_file_name(format!("fake-fxserver{}", std::env::consts::EXE_SUFFIX));
    assert!(
        fake.is_file(),
        "{} is missing: run the workspace's tests, or `cargo build -p fake-fxserver` first",
        fake.display()
    );
    fake
}

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !done() {
        assert!(Instant::now() < deadline, "{what} never happened");
        std::thread::sleep(Duration::from_millis(100));
    }
}

struct Reply {
    status: u16,
    cookie: Option<String>,
    body: Value,
}

/// One HTTP/1.1 exchange, as the panel would send it.
fn call(
    port: u16,
    method: &str,
    path: &str,
    session: Option<&(String, String)>,
    body: Option<Value>,
) -> Reply {
    let body = body.map(|body| body.to_string()).unwrap_or_default();
    let mut request = format!(
        "{method} /api{path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: http://127.0.0.1:{port}\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    if let Some((cookie, key)) = session {
        let _ = write!(request, "Cookie: {cookie}\r\nX-Observer-Session-Key: {key}\r\n");
    }
    request.push_str("\r\n");
    request.push_str(&body);
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream.write_all(request.as_bytes()).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let (head, body) = response.split_once("\r\n\r\n").unwrap();
    let status = head.split(' ').nth(1).unwrap().parse().unwrap();
    let cookie = head
        .lines()
        .filter_map(|line| {
            line.strip_prefix("set-cookie: ")
                .or_else(|| line.strip_prefix("Set-Cookie: "))
        })
        .map(|cookie| cookie.split(';').next().unwrap().to_owned())
        .next();
    let body = serde_json::from_str(body).unwrap_or(Value::Null);
    Reply { status, cookie, body }
}

struct Observer {
    dir: TempDir,
    port: u16,
    process: Child,
}

impl Observer {
    /// An Observer of its own data folder, whose owner `ada` the setup link created.
    fn start() -> (Self, (String, String)) {
        let dir = tempfile::tempdir().unwrap();
        let port = free_port();
        let config = dir.path().join("observer.toml");
        fs::write(
            &config,
            format!("data_dir = \"data\"\n\n[http]\nlisten = \"127.0.0.1:{port}\"\n"),
        )
        .unwrap();
        // Observer prints the setup link on its first start, to the terminal only.
        let mut process = Command::new(env!("CARGO_BIN_EXE_observer"))
            .args(["--config", config.to_str().unwrap(), "run"])
            .env_remove("INVOCATION_ID")
            .env_remove("NOTIFY_SOCKET")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(fs::File::create(dir.path().join("observer.err")).unwrap())
            .spawn()
            .unwrap();
        let (token_tx, token) = mpsc::channel();
        let stdout = process.stdout.take().unwrap();
        let mut out = fs::File::create(dir.path().join("observer.out")).unwrap();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some((_, token)) = line.split_once("/setup#") {
                    let _ = token_tx.send(token.trim().to_owned());
                }
                let _ = writeln!(out, "{line}");
            }
        });
        let token = token.recv_timeout(WAIT).expect("Observer printed no setup link");
        let observer = Self { dir, port, process };
        wait_for("the panel answering", || {
            TcpStream::connect(("127.0.0.1", port)).is_ok()
        });

        let setup = json!({ "token": token, "username": "ada", "password": PASSWORD });
        let reply = call(port, "POST", "/setup", None, Some(setup));
        assert_eq!(reply.status, 201, "{}", reply.body);
        let session = (
            reply.cookie.unwrap(),
            reply.body["sessionKey"].as_str().unwrap().to_owned(),
        );
        let reauth = call(
            port,
            "POST",
            "/auth/reauth",
            Some(&session),
            Some(json!({ "password": PASSWORD })),
        );
        assert_eq!(reauth.status, 204, "{}", reauth.body);
        (observer, session)
    }

    /// A build folder holding the fake, and a data folder whose cfgs print their own name.
    fn lay_out_fake(&self) -> (PathBuf, PathBuf) {
        let build = self.dir.path().join("build");
        let program = if cfg!(windows) {
            build.join("FXServer.exe")
        } else {
            build.join("alpine/opt/cfx-server/ld-musl-x86_64.so.1")
        };
        fs::create_dir_all(program.parent().unwrap()).unwrap();
        fs::copy(fake(), &program).unwrap();
        let data = self.dir.path().join("server-data");
        fs::create_dir_all(&data).unwrap();
        let endpoint = free_port();
        fs::write(
            data.join("server.cfg"),
            format!("endpoint_add_tcp \"127.0.0.1:{endpoint}\"\nendpoint_add_udp \"127.0.0.1:{endpoint}\"\n"),
        )
        .unwrap();
        for cfg in ["a.cfg", "b.cfg"] {
            fs::write(
                data.join(cfg),
                format!("exec server.cfg\nfake_print running {cfg}\n"),
            )
            .unwrap();
        }
        (build, data)
    }

    fn save(&self, session: &(String, String), build: &Path, data: &Path, cfg: &str) {
        let stored = call(self.port, "GET", "/servers/default/launch", Some(session), None);
        let settings = json!({
            "buildPath": build.to_str().unwrap(),
            "dataPath": data.to_str().unwrap(),
            "cfgFile": cfg,
            "onesync": "on",
            "autoStart": false,
            "txadminEvents": false,
            "extraArguments": [{ "name": "fake_heartbeat_ms", "value": "200" }],
        });
        let change = json!({ "settings": settings, "version": stored.body["version"] });
        let saved = call(
            self.port,
            "PUT",
            "/servers/default/launch",
            Some(session),
            Some(change),
        );
        assert_eq!(saved.status, 200, "{}", saved.body);
    }

    /// What the console log files hold so far.
    fn console_log(&self) -> String {
        let dir = self.dir.path().join("data/logs/console");
        let Ok(files) = fs::read_dir(dir) else {
            return String::new();
        };
        files
            .filter_map(|file| fs::read_to_string(file.ok()?.path()).ok())
            .collect()
    }

    #[cfg(unix)]
    /// What Observer printed: the setup link on stdout, its log on stderr.
    fn log(&self) -> String {
        ["observer.out", "observer.err"]
            .map(|name| fs::read_to_string(self.dir.path().join(name)).unwrap_or_default())
            .concat()
    }
}

impl Drop for Observer {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

#[test]
fn observer_run_restarts_a_crashed_server_with_the_settings_saved_meanwhile_and_stops_with_it() {
    let (observer, owner) = Observer::start();
    let (build, data) = observer.lay_out_fake();
    observer.save(&owner, &build, &data, "a.cfg");

    let started = call(
        observer.port,
        "POST",
        "/servers/default/start",
        Some(&owner),
        None,
    );
    assert_eq!(started.status, 204, "{}", started.body);
    wait_for("a.cfg running", || {
        observer.console_log().contains("] running a.cfg")
    });
    observer.save(&owner, &build, &data, "b.cfg");
    let crash = json!({ "line": "fake_crash 3" });
    let crashed = call(
        observer.port,
        "POST",
        "/servers/default/console",
        Some(&owner),
        Some(crash),
    );
    assert_eq!(crashed.status, 200, "{}", crashed.body);

    wait_for("the restart after the crash running b.cfg", || {
        observer.console_log().contains("] running b.cfg")
    });
    let log = observer.console_log();
    assert!(
        log.find("FXServer exited").unwrap() < log.find("] running b.cfg").unwrap(),
        "{log}"
    );

    #[cfg(unix)]
    stops_gracefully_on_sigterm(observer);
}

/// What a service manager does to stop Observer, and FXServer with it.
#[cfg(unix)]
fn stops_gracefully_on_sigterm(mut observer: Observer) {
    let terminated = Command::new("kill")
        .args(["-TERM", &observer.process.id().to_string()])
        .status()
        .unwrap();
    assert!(terminated.success());
    let deadline = Instant::now() + WAIT;
    let status = loop {
        if let Some(status) = observer.process.try_wait().unwrap() {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "Observer did not stop: {}",
            observer.log()
        );
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(status.success(), "{status:?}: {}", observer.log());
    let log = observer.log();
    assert!(
        log.contains("stopping Observer") && log.contains("Observer stopped"),
        "{log}"
    );
    assert!(observer.console_log().contains("-> Quitting: shutdown"));
}
