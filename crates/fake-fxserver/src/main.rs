//! A stand-in for FXServer in Observer's tests. It takes FXServer's arguments (`+set`, `+exec`,
//! `+ensure`), prints lines shaped like FXServer 35245's, runs the commands it reads on stdin from
//! its main loop, writes structured traces on fd 3 and serves `/dynamic.json`, `/perf/` and
//! `/players.json` on the cfg's `endpoint_add_tcp`. With `+ensure observer` it also plays the
//! bridge: `ready` once, then a heartbeat.
//!
//! Failures on demand, through convars (`+set fake_x y`, or in the cfg) or console commands:
//! - `fake_quit_delay_ms` (500): time between `quit` and the exit, with code 1 as FXServer;
//! - `fake_ignore_quit 1`: `quit` does nothing;
//! - `fake_heartbeat_ms` (3000): the bridge heartbeat period;
//! - `fake_crash_after_ms`, `fake_hang_after_ms`: crash or hang the main loop on a timer;
//! - `fake_bind_error 1`: report the port as taken, as when it really is;
//! - `fake_spawn_child 1`: start a child that inherits the pipes and outlives a crash, like the
//!   crash dump server;
//! - on stdin: `fake_crash [code]`, `fake_hang`, `fake_print <text>`, `fake_trace <type>`.
//!
//! Unknown commands get FXServer's `No such command <name>.`

mod console;
mod http;
#[allow(unsafe_code)]
mod traces;

use std::collections::HashMap;
use std::io::{self, BufRead};
use std::net::TcpListener;
use std::process::{self, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::json;

use crate::console::{CMD, LICENSE, MAIN, NET, RESOURCES, SCRIPTING, SERVER};
use crate::traces::Traces;

const TICK: Duration = Duration::from_millis(50);
/// The argument that turns the process into the child that holds the pipes.
const HOLD_PIPES: &str = "--fake-hold-pipes";

/// What the HTTP threads share with the main loop.
pub struct Server {
    convars: Mutex<HashMap<String, String>>,
    main_ticks: AtomicU64,
    started: Instant,
}

impl Server {
    pub fn convar(&self, name: &str) -> Option<String> {
        self.convars
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(name)
            .cloned()
    }

    fn set(&self, name: &str, value: &str) {
        self.convars
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(name.to_owned(), value.to_owned());
    }

    fn number(&self, name: &str) -> Option<u64> {
        self.convar(name)?.parse().ok()
    }

    fn flag(&self, name: &str) -> bool {
        self.number(name).is_some_and(|value| value != 0)
    }

    pub fn main_ticks(&self) -> u64 {
        self.main_ticks.load(Ordering::Relaxed)
    }

    pub fn uptime_ms(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }
}

struct Bridge {
    next_heartbeat: Instant,
    ready: bool,
}

struct Fake {
    server: Arc<Server>,
    traces: Traces,
    endpoint: Option<String>,
    bridge: Option<Bridge>,
    quit_at: Option<Instant>,
}

fn main() {
    if std::env::args().nth(1).as_deref() == Some(HOLD_PIPES) {
        loop {
            thread::sleep(Duration::from_secs(3600));
        }
    }
    // Before anything else opens a file, so that fd 3 is still the parent's.
    let traces = Traces::open();
    let mut fake = Fake {
        server: Arc::new(Server {
            convars: Mutex::new(HashMap::new()),
            main_ticks: AtomicU64::new(0),
            started: Instant::now(),
        }),
        traces,
        endpoint: None,
        bridge: None,
        quit_at: None,
    };
    fake.boot(&command_line(std::env::args().skip(1)));
    fake.run();
}

impl Fake {
    fn boot(&mut self, commands: &[Vec<String>]) {
        // FXServer applies every `+set` of its command line before it runs anything else.
        for command in commands.iter().filter(|command| is_set(command)) {
            self.execute(command);
        }
        console::line(RESOURCES, "\x1b[32mScanning resources.\x1b[0m");
        console::line(RESOURCES, "\x1b[32mFound 4 resources.\x1b[0m");
        console::line(LICENSE, "Authenticating server license key...");
        console::line(LICENSE, "Server license key authentication succeeded. Welcome!");
        for command in commands.iter().filter(|command| !is_set(command)) {
            self.execute(command);
        }

        if let Some(address) = self.endpoint.clone() {
            if self.server.flag("fake_bind_error") {
                self.bind_error(&address);
            }
            match TcpListener::bind(&address) {
                Ok(listener) => http::serve(listener, Arc::clone(&self.server)),
                Err(_) => self.bind_error(&address),
            }
        }
        if self.server.flag("fake_spawn_child") {
            spawn_child();
        }
    }

    fn run(&mut self) -> ! {
        let commands = read_stdin();
        let crash_at = self.deadline("fake_crash_after_ms");
        let hang_at = self.deadline("fake_hang_after_ms");
        loop {
            let now = Instant::now();
            if self.quit_at.is_some_and(|at| now >= at) {
                self.exit(1);
            }
            if crash_at.is_some_and(|at| now >= at) {
                self.crash(None);
            }
            if hang_at.is_some_and(|at| now >= at) {
                hang();
            }
            while let Ok(line) = commands.try_recv() {
                for command in split_commands(&line) {
                    self.execute(&command);
                }
            }
            self.beat(now);
            self.server.main_ticks.fetch_add(1, Ordering::Relaxed);
            thread::sleep(TICK);
        }
    }

    fn execute(&mut self, command: &[String]) {
        let Some((name, args)) = command.split_first() else {
            return;
        };
        let first = args.first().map(String::as_str);
        match (name.to_ascii_lowercase().as_str(), first) {
            ("set" | "sets" | "setr" | "seta", Some(convar)) => {
                self.server.set(convar, args.get(1).map_or("", String::as_str));
            }
            ("exec", Some(file)) => self.exec(file),
            ("endpoint_add_tcp", Some(address)) => self.endpoint = Some(address.to_owned()),
            ("endpoint_add_udp", _) => {}
            ("ensure" | "start", Some(resource)) => {
                if self.bridge.is_some() && resource == "observer" {
                    stop(resource);
                }
                self.start(resource);
            }
            ("restart", Some(resource)) => {
                stop(resource);
                self.start(resource);
            }
            ("stop", Some(resource)) => {
                stop(resource);
                if resource == "observer" {
                    self.bridge = None;
                }
            }
            ("quit", reason) => self.quit(reason),
            ("fake_crash", code) => self.crash(code.and_then(|code| code.parse().ok())),
            ("fake_hang", _) => hang(),
            ("fake_print", Some(_)) => {
                let text = args.join(" ");
                console::script("fake", &text);
                self.traces.native(
                    "ScriptTrace",
                    &json!({ "type": "script_log", "resource": "fake", "text": text }),
                );
            }
            ("fake_trace", Some(kind)) => self.trace(kind),
            // Typing a convar's name with a value sets it.
            (convar, Some(value)) if convar.starts_with("sv_") || convar.starts_with("fake_") => {
                self.server.set(name, value);
            }
            _ => console::line(CMD, &format!("No such command {name}.")),
        }
    }

    fn exec(&mut self, file: &str) {
        let Ok(cfg) = std::fs::read_to_string(file) else {
            console::line(CMD, &format!("No such config file: {file}"));
            return;
        };
        for line in cfg.lines() {
            for command in split_commands(line) {
                self.execute(&command);
            }
        }
    }

    fn start(&mut self, resource: &str) {
        console::line(SCRIPTING, &format!("Creating script environments for {resource}"));
        console::line(RESOURCES, &format!("Started resource {resource}"));
        if resource == "observer" {
            self.bridge = Some(Bridge {
                next_heartbeat: Instant::now(),
                ready: false,
            });
        }
    }

    /// The bridge's traces. They come from the main loop, so they stop while it is hung.
    fn beat(&mut self, now: Instant) {
        let period = Duration::from_millis(self.server.number("fake_heartbeat_ms").unwrap_or(3000));
        let Some(bridge) = &mut self.bridge else {
            return;
        };
        if !bridge.ready {
            bridge.ready = true;
            self.traces
                .script("observer", &json!({ "type": "observer", "kind": "ready" }));
        }
        if now >= bridge.next_heartbeat {
            bridge.next_heartbeat = now + period;
            self.traces
                .script("observer", &json!({ "type": "observer", "kind": "hb" }));
        }
    }

    fn quit(&mut self, reason: Option<&str>) {
        if self.server.flag("fake_ignore_quit") || self.quit_at.is_some() {
            return;
        }
        console::line(
            MAIN,
            &format!("-> Quitting: {}", reason.unwrap_or("Quit command executed.")),
        );
        let delay = self.server.number("fake_quit_delay_ms").unwrap_or(500);
        self.quit_at = Some(Instant::now() + Duration::from_millis(delay));
    }

    fn trace(&self, kind: &str) {
        let data = match kind {
            "hitch" => json!({ "type": "hitch", "thread": "svMain", "time": 200 }),
            "watchdog_bark" => {
                json!({ "type": "watchdog_bark", "thread": "svMain", "time": 60, "stack": "fake <- root" })
            }
            "watchdog_bite" => json!({ "type": "watchdog_bite", "thread": "svMain" }),
            "nucleus_connected" => {
                json!({ "type": "nucleus_connected", "url": "https://fake-abc123.users.cfx.re/" })
            }
            other => json!({ "type": other }),
        };
        self.traces.native("operator ()", &data);
    }

    /// What FXServer does when its port is taken: a `bind_error` trace, then a fatal error.
    fn bind_error(&self, address: &str) -> ! {
        self.traces.native(
            "CreateUdpHost",
            &json!({ "type": "bind_error", "address": address }),
        );
        let message = format!("Could not bind on {address} - is this address valid and not already in use?");
        console::line(NET, "Listening on socket failed - libuv error EADDRINUSE.");
        console::line(SERVER, &format!("Error: {message}"));
        console::fatal(&message);
        self.exit(1)
    }

    /// Without a code, crashes the way the platform does: an access violation on Windows, an
    /// abort (SIGABRT) elsewhere.
    fn crash(&self, code: Option<u32>) -> ! {
        self.traces.flush();
        match code {
            Some(code) => process::exit(code.cast_signed()),
            None if cfg!(windows) => process::exit(0xC000_0005_u32.cast_signed()),
            None => process::abort(),
        }
    }

    fn exit(&self, code: i32) -> ! {
        self.traces.flush();
        process::exit(code)
    }

    fn deadline(&self, convar: &str) -> Option<Instant> {
        self.server
            .number(convar)
            .map(|ms| self.server.started + Duration::from_millis(ms))
    }
}

fn spawn_child() {
    let Ok(program) = std::env::current_exe() else {
        return;
    };
    let _ = Command::new(program)
        .arg(HOLD_PIPES)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn();
}

fn stop(resource: &str) {
    console::line(RESOURCES, &format!("Stopping resource {resource}"));
}

fn hang() -> ! {
    loop {
        thread::sleep(Duration::from_secs(3600));
    }
}

fn read_stdin() -> mpsc::Receiver<String> {
    let (lines, commands) = mpsc::channel();
    thread::spawn(move || {
        for line in io::stdin().lock().lines() {
            let Ok(line) = line else {
                break;
            };
            if lines.send(line).is_err() {
                break;
            }
        }
    });
    commands
}

fn is_set(command: &[String]) -> bool {
    command
        .first()
        .is_some_and(|name| matches!(name.as_str(), "set" | "sets" | "setr" | "seta"))
}

/// FXServer's command line: each `+name` starts a command whose arguments run up to the next
/// `+`. Anything before the first one (the loader's own arguments on Linux) is not a command.
fn command_line(args: impl Iterator<Item = String>) -> Vec<Vec<String>> {
    let mut commands: Vec<Vec<String>> = Vec::new();
    for arg in args {
        match arg.strip_prefix('+') {
            Some(name) => commands.push(vec![name.to_owned()]),
            None => {
                if let Some(command) = commands.last_mut() {
                    command.push(arg);
                }
            }
        }
    }
    commands
}

/// Splits a console line into commands at `;` outside quotes, then into words, as FXServer does.
fn split_commands(line: &str) -> Vec<Vec<String>> {
    let mut commands = Vec::new();
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quoted = false;
    let mut in_word = false;
    for c in line.trim_end_matches(['\r', '\n']).chars() {
        match c {
            '"' => {
                quoted = !quoted;
                in_word = true;
            }
            ';' if !quoted => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
                commands.push(std::mem::take(&mut words));
            }
            c if c.is_whitespace() && !quoted => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            c => {
                word.push(c);
                in_word = true;
            }
        }
    }
    if in_word {
        words.push(word);
    }
    commands.push(words);
    commands.retain(|command| !command.is_empty());
    commands
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(commands: &[&[&str]]) -> Vec<Vec<String>> {
        commands
            .iter()
            .map(|command| command.iter().map(ToString::to_string).collect())
            .collect()
    }

    #[test]
    fn the_command_line_groups_arguments_under_each_plus_command() {
        let args = [
            "--library-path",
            "/x",
            "+set",
            "a",
            "b c",
            "+exec",
            "server.cfg",
            "+ensure",
            "observer",
        ];

        assert_eq!(
            command_line(args.into_iter().map(String::from)),
            words(&[
                &["set", "a", "b c"],
                &["exec", "server.cfg"],
                &["ensure", "observer"]
            ])
        );
    }

    #[test]
    fn console_lines_split_at_semicolons_outside_quotes() {
        assert_eq!(
            split_commands("quit \"a; b\"; fake_hang\r"),
            words(&[&["quit", "a; b"], &["fake_hang"]])
        );
        assert_eq!(
            split_commands("sv_hostname \"Observer lab\""),
            words(&[&["sv_hostname", "Observer lab"]])
        );
        assert_eq!(split_commands("  "), Vec::<Vec<String>>::new());
    }
}
