//! `observer run` as it ships: the built binary against the fake FXServer, driven over HTTP the
//! way the panel drives it, then asked to stop the way a service manager or a terminal does.
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

use observer_fxserver::hook;
use observer_fxserver::layout::{Flavour, Layout};
use observer_store::Database;
use observer_store::artifacts::{self, ArtifactState};
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
    /// What Observer prints, line by line: the setup link on stdout, its log on stderr.
    lines: mpsc::Receiver<String>,
}

/// Copies what Observer prints on `stream` to `file`, and passes each line on.
fn tee(stream: impl Read + Send + 'static, mut file: fs::File, lines: mpsc::Sender<String>) {
    std::thread::spawn(move || {
        for line in BufReader::new(stream).lines().map_while(Result::ok) {
            let _ = writeln!(file, "{line}");
            let _ = lines.send(line);
        }
    });
}

impl Observer {
    /// Observer on a data folder of its own, as it first starts there, its `observer.toml` ending
    /// with `extra`.
    fn spawn_with(extra: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let port = free_port();
        // Updates stay off: no test reaches GitHub.
        let extra = if extra.contains("[features]\n") {
            extra.replace("[features]\n", "[features]\nupdates = \"off\"\n")
        } else {
            format!("{extra}\n[features]\nupdates = \"off\"\n")
        };
        fs::write(
            dir.path().join("observer.toml"),
            format!("data_dir = \"data\"\n\n[http]\nlisten = \"127.0.0.1:{port}\"\n{extra}"),
        )
        .unwrap();
        let (process, lines) = Self::run_in(dir.path());
        Self {
            dir,
            port,
            process,
            lines,
        }
    }

    /// Kills this Observer and starts another on the same folder and configuration.
    fn respawn(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
        (self.process, self.lines) = Self::run_in(self.dir.path());
    }

    /// `observer run` on the configuration in `dir`, what it prints going to files there too.
    fn run_in(dir: &Path) -> (Child, mpsc::Receiver<String>) {
        let config = dir.join("observer.toml");
        let mut process = Command::new(env!("CARGO_BIN_EXE_observer"))
            .args(["--config", config.to_str().unwrap(), "run"])
            // A test has no one at the host to show a window to.
            .args(cfg!(windows).then_some("--no-window"))
            .env_remove("INVOCATION_ID")
            .env_remove("NOTIFY_SOCKET")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let (lines, printed) = mpsc::channel();
        let file = |name| fs::File::create(dir.join(name)).unwrap();
        tee(
            process.stdout.take().unwrap(),
            file("observer.out"),
            lines.clone(),
        );
        tee(process.stderr.take().unwrap(), file("observer.err"), lines);
        (process, printed)
    }

    /// Waits for Observer to print a line holding `text`, and returns that line.
    fn printed(&self, text: &str) -> String {
        let deadline = Instant::now() + WAIT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let line = self
                .lines
                .recv_timeout(left)
                .unwrap_or_else(|error| panic!("Observer never printed `{text}`: {error}"));
            if line.contains(text) {
                return line;
            }
        }
    }

    /// An Observer of its own data folder, whose owner `ada` the setup link created.
    fn start() -> (Self, (String, String)) {
        Self::start_with("")
    }

    /// [`Observer::start`], its `observer.toml` ending with `extra`.
    fn start_with(extra: &str) -> (Self, (String, String)) {
        let observer = Self::spawn_with(extra);
        // Observer prints the setup link on its first start, to the terminal only.
        let link = observer.printed("/setup#");
        let token = link.split_once("/setup#").unwrap().1.trim();
        let port = observer.port;
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

    /// A build holding the fake in Observer's artifacts folder, and a data folder whose cfgs
    /// print their own name.
    fn lay_out_fake(&self) -> (PathBuf, PathBuf) {
        let build = self.dir.path().join("data/artifacts/35245");
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

    /// Makes the fake's build the one FXServer starts, in the database of the running Observer.
    fn select(&self, build: u32) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let data = self.dir.path().join("data");
        runtime.block_on(async move {
            let db = Database::open(data.join("observer.sqlite3"), data.join("backups"))
                .await
                .unwrap();
            let state = ArtifactState {
                selected: Some(build),
                ..ArtifactState::default()
            };
            db.write(move |tx| artifacts::save_state(tx, "default", &state, jiff::Timestamp::now()))
                .await
                .unwrap();
            db.close().await.unwrap();
        });
    }

    fn save(&self, session: &(String, String), data: &Path, cfg: &str) {
        let stored = call(self.port, "GET", "/servers/default/launch", Some(session), None);
        let settings = json!({
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

    /// What Observer printed: the setup link on stdout, its log on stderr.
    fn log(&self) -> String {
        ["observer.out", "observer.err"]
            .map(|name| fs::read_to_string(self.dir.path().join(name)).unwrap_or_default())
            .concat()
    }

    #[cfg(unix)]
    fn signal(&self, signal: &str) {
        let sent = Command::new("kill")
            .args([signal, &self.process.id().to_string()])
            .status()
            .unwrap();
        assert!(sent.success());
    }

    /// Waits for Observer to end, and checks it went through its whole stop.
    #[cfg(unix)]
    fn stopped_gracefully(&mut self) {
        let deadline = Instant::now() + WAIT;
        let status = loop {
            if let Some(status) = self.process.try_wait().unwrap() {
                break status;
            }
            assert!(Instant::now() < deadline, "Observer did not stop: {}", self.log());
            std::thread::sleep(Duration::from_millis(100));
        };
        assert!(status.success(), "{status:?}: {}", self.log());
        let log = self.log();
        assert!(
            log.contains("stopping Observer") && log.contains("Observer stopped"),
            "{log}"
        );
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
    let (_, data) = observer.lay_out_fake();
    observer.select(35245);
    observer.save(&owner, &data, "a.cfg");

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
    observer.save(&owner, &data, "b.cfg");
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
    observer.signal("-TERM");

    observer.stopped_gracefully();
    assert!(observer.console_log().contains("-> Quitting: shutdown"));
}

#[cfg(unix)]
#[test]
fn a_ctrl_c_while_observer_starts_stops_it_gracefully_rather_than_kill_it() {
    let mut observer = Observer::spawn_with("");
    // Printed once stop requests are caught, before the database is even open.
    observer.printed("starting Observer");

    observer.signal("-INT");

    observer.stopped_gracefully();
}

/// The owner turned the duplicate archetypes fix on: before FXServer starts, the work under the
/// supervisor takes the duplicate out of the server folder, says so in the console, and the Tools
/// page lists the change.
#[test]
fn observer_run_fixes_duplicate_archetypes_before_fxserver_starts_once_the_owner_turned_it_on() {
    let (observer, owner) = Observer::start();
    let (_, data) = observer.lay_out_fake();
    let camp = data.join("resources/props/stream/camp.ytyp");
    fs::create_dir_all(camp.parent().unwrap()).unwrap();
    fs::write(
        data.join("resources/props/fxmanifest.lua"),
        "fx_version 'cerulean'\ngame 'gta5'\n",
    )
    .unwrap();
    fs::write(
        &camp,
        observer_gamefiles::testing::ytyp("camp", &["marshmallow", "stick", "marshmallow"]),
    )
    .unwrap();
    fs::write(
        data.join("c.cfg"),
        "exec server.cfg\nensure props\nfake_print running c.cfg\n",
    )
    .unwrap();
    observer.select(35245);
    observer.save(&owner, &data, "c.cfg");
    let settings = json!({
        "settings": { "atStart": "fix", "differences": "identical", "escrowed": "skip", "excluded": [] },
        "version": null,
    });
    let saved = call(
        observer.port,
        "PUT",
        "/tools/duplicate-archetypes/settings",
        Some(&owner),
        Some(settings),
    );
    assert_eq!(saved.status, 200, "{}", saved.body);

    let started = call(
        observer.port,
        "POST",
        "/servers/default/start",
        Some(&owner),
        None,
    );
    assert_eq!(started.status, 204, "{}", started.body);
    wait_for("c.cfg running", || {
        observer.console_log().contains("] running c.cfg")
    });

    let log = observer.console_log();
    let fixed = log
        .find("tools | duplicate archetypes: fixed 1 duplicate archetype")
        .unwrap_or_else(|| panic!("{log}"));
    assert!(fixed < log.find("] running c.cfg").unwrap(), "{log}");
    let archetypes = observer_gamefiles::ytyp::read(&fs::read(&camp).unwrap())
        .unwrap()
        .archetypes
        .len();
    assert_eq!(archetypes, 2);
    let page = call(observer.port, "GET", "/tools", Some(&owner), None);
    assert_eq!(page.body["tools"][0]["open"].as_array().unwrap().len(), 1);
    assert_eq!(page.body["tools"][0]["lastRun"]["trigger"], "start");
}

/// SFTP's port opens with Observer, closes as soon as the owner turns SFTP off in the panel, and
/// stays closed when Observer starts again.
#[test]
fn sftp_turned_off_in_the_panel_closes_its_port_at_once_and_at_the_next_start() {
    let sftp = free_port();
    let (mut observer, owner) = Observer::start_with(&format!("\n[sftp]\nlisten = \"127.0.0.1:{sftp}\"\n"));
    assert!(
        TcpStream::connect(("127.0.0.1", sftp)).is_ok(),
        "{}",
        observer.log()
    );

    let off = json!({ "enabled": false });
    let switched = call(observer.port, "PUT", "/features/sftp", Some(&owner), Some(off));

    assert_eq!(switched.status, 200, "{}", switched.body);
    assert!(
        TcpStream::connect(("127.0.0.1", sftp)).is_err(),
        "the port is closed by the time the panel hears back"
    );
    let stopped = observer.printed("turned off; its tasks stopped");
    assert!(stopped.contains("sftp"), "{stopped}");
    let files = call(observer.port, "GET", "/files/accesses", Some(&owner), None);
    assert_eq!(files.status, 404, "{}", files.body);
    assert_eq!(files.body["code"], "feature_disabled");

    observer.respawn();

    let off = observer.printed("optional features off");
    assert!(off.contains("sftp"), "{off}");
    let port = observer.port;
    wait_for("the panel answering again", || {
        TcpStream::connect(("127.0.0.1", port)).is_ok()
    });
    assert!(!observer.log().contains("SFTP listening"), "{}", observer.log());
    assert!(
        TcpListener::bind(("127.0.0.1", sftp)).is_ok(),
        "nothing holds SFTP's port"
    );
}

/// Without `[sftp]` in `observer.toml`, the owner gives SFTP its address in the panel: the port
/// opens at once and moves with the address. A port another program holds is held against SFTP,
/// never against Observer, and the address outlives Observer until it is taken back.
#[test]
fn sftp_takes_its_address_from_the_panel_and_its_port_follows_without_a_restart() {
    let (mut observer, owner) = Observer::start();
    let panel = observer.port;
    let give = |port: u16| {
        let listen = json!({ "listen": format!("127.0.0.1:{port}") });
        call(panel, "PUT", "/settings/sftp", Some(&owner), Some(listen))
    };
    let (first, second) = (free_port(), free_port());

    let waiting = call(panel, "GET", "/settings/sftp", Some(&owner), None);
    let given = give(first);
    let opened = TcpStream::connect(("127.0.0.1", first)).is_ok();
    let moved = give(second);

    assert_eq!(
        waiting.body,
        json!({ "listen": null, "pinned": false, "open": false, "fingerprint": null, "refused": null })
    );
    assert_eq!(given.status, 200, "{}", given.body);
    assert_eq!(given.body["open"], true, "{}", given.body);
    let fingerprint = given.body["fingerprint"].as_str().unwrap();
    assert!(fingerprint.starts_with("SHA256:"), "{fingerprint}");
    assert!(observer.log().contains(fingerprint), "the log names the same key");
    assert!(opened, "{}", observer.log());
    assert_eq!(moved.body["open"], true, "{}", moved.body);
    assert_eq!(
        moved.body["fingerprint"], fingerprint,
        "the key stays, wherever the port is"
    );
    assert!(TcpStream::connect(("127.0.0.1", second)).is_ok());
    assert!(
        TcpListener::bind(("127.0.0.1", first)).is_ok(),
        "the port it left is free by the time the panel hears back"
    );

    let held = TcpListener::bind("127.0.0.1:0").unwrap();
    let taken = held.local_addr().unwrap().port();
    let refused = give(taken);
    let switches = call(panel, "GET", "/features", Some(&owner), None);
    drop(held);
    let again = give(taken);

    assert_eq!(refused.status, 200, "{}", refused.body);
    assert_eq!(refused.body["open"], false, "{}", refused.body);
    assert!(refused.body["refused"].is_string(), "{}", refused.body);
    let sftp = &switches.body["features"][0];
    assert_eq!(
        (&sftp["running"], &sftp["held"]),
        (&json!(false), &json!("portRefused"))
    );
    assert_eq!(again.body["open"], true, "{}", again.body);
    assert_eq!(again.body["refused"], Value::Null);

    observer.respawn();
    wait_for(
        "SFTP listening where the owner said, after a restart of Observer",
        || TcpStream::connect(("127.0.0.1", taken)).is_ok(),
    );
    let back = json!({ "listen": null });
    let taken_back = call(observer.port, "PUT", "/settings/sftp", Some(&owner), Some(back));

    assert_eq!(taken_back.status, 200, "{}", taken_back.body);
    assert_eq!(taken_back.body["listen"], Value::Null);
    assert!(
        TcpListener::bind(("127.0.0.1", taken)).is_ok(),
        "the port is closed by the time the panel hears back"
    );
}

/// `[features] sftp = "off"` keeps SFTP's port closed though `[sftp]` names it, and even the
/// owner's session cannot open it: the refusal is audited.
#[test]
fn a_feature_observer_toml_turns_off_never_runs_and_the_panel_cannot_turn_it_on() {
    let sftp = free_port();
    let (observer, owner) = Observer::start_with(&format!(
        "\n[sftp]\nlisten = \"127.0.0.1:{sftp}\"\n\n[features]\nsftp = \"off\"\n"
    ));

    let on = json!({ "enabled": true });
    let refused = call(observer.port, "PUT", "/features/sftp", Some(&owner), Some(on));
    let audit = call(
        observer.port,
        "GET",
        "/audit?action=feature_changed",
        Some(&owner),
        None,
    );

    assert_eq!(refused.status, 403, "{}", refused.body);
    assert_eq!(refused.body["code"], "feature_locked");
    let entry = &audit.body["entries"][0];
    assert_eq!(entry["target"], "sftp", "{}", audit.body);
    assert_eq!(entry["outcome"], "denied");
    assert!(
        TcpListener::bind(("127.0.0.1", sftp)).is_ok(),
        "SFTP's port was never bound"
    );
    let log = observer.log();
    assert!(
        log.contains("optional features off") && !log.contains("SFTP listening"),
        "{log}"
    );
}

/// The owner turns Debug on: the next start puts Observer's two lines into the build's scheduler,
/// what a resource counts reads on the Events page, and a started resource that runs no Lua on the
/// server is named as one that does not report. The bridge, Observer's own, is neither.
#[test]
fn debug_turned_on_in_the_panel_counts_from_the_next_start_and_reads_on_its_pages() {
    let (observer, owner) = Observer::start();
    let (build, data) = observer.lay_out_fake();
    let scheduler = hook::scheduler(&Layout::new(&build, Flavour::HOST));
    fs::create_dir_all(scheduler.parent().unwrap()).unwrap();
    fs::write(
        &scheduler,
        "Citizen.SetEventRoutine(function() end)
",
    )
    .unwrap();
    for resource in ["chat", "clothes"] {
        let folder = data.join("resources").join(resource);
        fs::create_dir_all(&folder).unwrap();
        fs::write(
            folder.join("fxmanifest.lua"),
            "fx_version 'cerulean'
game 'gta5'
",
        )
        .unwrap();
    }
    fs::write(
        data.join("d.cfg"),
        "exec server.cfg
set fake_debug_silent clothes
ensure chat
ensure clothes
         fake_print running d.cfg
",
    )
    .unwrap();
    observer.select(35245);
    observer.save(&owner, &data, "d.cfg");
    let get = |path: &str| call(observer.port, "GET", path, Some(&owner), None);
    let post = |path: &str, body: Option<Value>| call(observer.port, "POST", path, Some(&owner), body);

    let off = get("/debug/status");
    let on = json!({ "enabled": true });
    let turned_on = call(observer.port, "PUT", "/features/debug", Some(&owner), Some(on));
    assert_eq!(turned_on.status, 200, "{}", turned_on.body);
    let started = post("/servers/default/start", None);
    assert_eq!(started.status, 204, "{}", started.body);
    wait_for("the resources' first reports", || {
        let status = get("/debug/status").body;
        status["reporting"] == 1 && status["silentCount"] == 1
    });
    let row = json!({ "line": "fake_debug chat ev toA chat:addMessage 4 1240 310 0" });
    let typed = post("/servers/default/console", Some(row));
    assert_eq!(typed.status, 200, "{}", typed.body);
    wait_for("the chat's event on the page", || {
        get("/debug/events?window=1m").body["events"][0]["name"] == "chat:addMessage"
    });

    assert_eq!(off.status, 404, "{}", off.body);
    assert_eq!(off.body["code"], "feature_disabled");
    let counted = fs::read_to_string(&scheduler).unwrap();
    assert!(counted.starts_with("--[[obs:hook:head "), "{counted}");
    let page = get("/debug/events?window=1m").body;
    assert_eq!(page["status"]["state"], "counting", "{page}");
    assert_eq!(
        page["status"]["silent"],
        json!(["clothes"]),
        "the bridge is no resource of the server's"
    );
    let event = &page["events"][0];
    assert_eq!(
        (
            &event["way"],
            &event["resources"],
            &event["count"],
            &event["bytes"]
        ),
        (&json!("toAll"), &json!(["chat"]), &json!(4), &json!(1240))
    );
    assert_eq!(page["totals"]["toClients"], json!({ "count": 4, "bytes": 1240 }));
}
