//! A stand-in for FXServer in Observer's tests. It takes FXServer's arguments (`+set`, `+exec`,
//! `+ensure`), prints lines shaped like FXServer 35245's, runs the commands it reads on stdin from
//! its main loop, writes structured traces on fd 3 and serves `/dynamic.json`, `/perf/`,
//! `/players.json` and `/info.json` on the cfg's `endpoint_add_tcp`. With `+ensure observer` it
//! also plays the bridge: `ready` and the started resources once, then a heartbeat and the heap.
//!
//! Failures on demand, through convars (`+set fake_x y`, or in the cfg) or console commands:
//! - `fake_quit_delay_ms` (500): time between `quit` and the end, which is what FXServer 35245 does:
//!   exit code 1 on Windows, SIGSEGV on Linux;
//! - `fake_ignore_quit 1`: `quit` does nothing;
//! - `fake_heartbeat_ms` (3000): the bridge heartbeat period;
//! - `fake_heap_ms` (10000): how often the bridge reports the JS heap;
//! - `fake_players <n>`: `/dynamic.json` counts `n` clients and `/players.json` lists ids 1 to `n`
//!   with a ping of `20 + 5 × id`, or the anonymised public list without the right token;
//! - `fake_tick_ms <ms>` (0): how long each svMain tick is observed to take in `/perf/`;
//! - `fake_crash_after_ms`, `fake_hang_after_ms`: crash or hang the main loop on a timer;
//! - `fake_bind_error 1`: report the port as taken, as when it really is;
//! - `fake_license missing|invalid`: refuse to run for want of a valid licence key, with FXServer's
//!   own messages;
//! - `fake_fatal_dialog 1`: stay up after a fatal error instead of exiting with code 1, as FXServer
//!   on Windows does when it shows its `Fatal Error` dialog;
//! - `fake_crash_dump 1`: on a crash, write a dump into `crashes/` next to the executable and end
//!   the way FXServer's dump server ends it on Windows (`0xFFFFFFFE`);
//! - `fake_spawn_child 1`: start a child that inherits the pipes and outlives a crash, like the
//!   crash dump server;
//! - on stdin: `fake_crash [code]`, `fake_hang`, `fake_spin` (a busy loop), `fake_print <text>`,
//!   `fake_trace <type>`, `fake_stall_input <ms>`, which stops reading stdin for a while, as
//!   FXServer does while its main loop is stuck; `fake_spawn [NAME=value...] <program> <args...>`,
//!   which starts a program as a resource would, with those variables added to its environment as
//!   Node's `spawn(.., { env })` adds them; `fake_dump_server`, which starts the fake again the way FXServer
//!   starts its crash dump server; `fake_connect <address>` and `fake_udp <address>`, which open and
//!   hold a TCP connection or a connected UDP socket, and `fake_disconnect`, which closes them.
//!
//! The main loop runs in a thread named `UV loop: svMain`, as FXServer's does.
//!
//! The bridge's `observer_cmd <hex>` prints the event it decodes, which the real bridge does not, so
//! that tests can see it arrive. Unknown commands get FXServer's `No such command <name>.`

mod console;
mod dump;
mod http;
#[allow(unsafe_code)]
mod traces;

use std::collections::HashMap;
use std::io::{self, BufRead};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::process::{self, Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::json;

use crate::console::{CMD, LICENSE, MAIN, NET, RESOURCES, SCRIPTING, SERVER};
use crate::traces::Traces;

const TICK: Duration = Duration::from_millis(50);
const LICENSE_HELP: &str = "Please set the sv_licenseKey console variable to a key from https://portal.cfx.re/servers/registration-keys.";
/// The argument that turns the process into the child that holds the pipes.
const HOLD_PIPES: &str = "--fake-hold-pipes";
/// What FXServer passes the copy of itself that serves as its crash dump server.
const DUMP_SERVER: &str = "-dumpserver:";
/// FXServer's main thread, as FXServer 35245 names it.
const SV_MAIN: &str = "UV loop: svMain";

/// What the HTTP threads share with the main loop.
pub struct Server {
    convars: Mutex<HashMap<String, String>>,
    main_ticks: AtomicU64,
    /// What svMain's ticks were observed to take, as `/perf/` reports it.
    main_observed: Mutex<Observed>,
    started: Instant,
    /// Uptime in milliseconds until which stdin is left unread.
    input_stalled_until: AtomicU64,
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

    pub fn main_observed(&self) -> Observed {
        self.main_observed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn tick(&self) {
        let ms = self.number("fake_tick_ms").unwrap_or(0);
        let mut observed = self.main_observed.lock().unwrap_or_else(PoisonError::into_inner);
        observed.observe(ms);
        self.main_ticks.fetch_add(1, Ordering::Relaxed);
    }

    pub fn uptime_ms(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }
}

/// One thread's tick histogram in FXServer 35245's bounds.
#[derive(Clone, Default)]
pub struct Observed {
    pub count: u64,
    /// Ticks at or under each bound of [`http::BUCKETS`].
    pub within: [u64; http::BUCKETS.len()],
    pub sum_ms: u64,
}

impl Observed {
    fn observe(&mut self, ms: u64) {
        self.count += 1;
        self.sum_ms += ms;
        for (within, (_, bound_ms)) in self.within.iter_mut().zip(http::BUCKETS) {
            if ms <= bound_ms {
                *within += 1;
            }
        }
    }
}

struct Bridge {
    next_heartbeat: Instant,
    next_heap: Instant,
    ready: bool,
}

struct Fake {
    server: Arc<Server>,
    traces: Traces,
    endpoint: Option<String>,
    bridge: Option<Bridge>,
    /// Every resource started, in order.
    started: Vec<String>,
    quit_at: Option<Instant>,
    /// What `fake_spawn` started, reaped as they end.
    children: Vec<Child>,
    connections: Vec<TcpStream>,
    datagrams: Vec<UdpSocket>,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some(HOLD_PIPES)
        || args.iter().any(|arg| arg.starts_with(DUMP_SERVER))
    {
        hang();
    }
    // Before anything else opens a file, so that fd 3 is still the parent's.
    let traces = Traces::open();
    let main_loop = thread::Builder::new().name(SV_MAIN.to_owned()).spawn(move || {
        let mut fake = Fake {
            server: Arc::new(Server {
                convars: Mutex::new(HashMap::new()),
                main_ticks: AtomicU64::new(0),
                main_observed: Mutex::default(),
                started: Instant::now(),
                input_stalled_until: AtomicU64::new(0),
            }),
            traces,
            endpoint: None,
            bridge: None,
            started: Vec::new(),
            quit_at: None,
            children: Vec::new(),
            connections: Vec::new(),
            datagrams: Vec::new(),
        };
        fake.boot(&command_line(args.into_iter()));
        fake.run()
    });
    // The main loop ends the process itself: it only comes back here if it panicked.
    let _ = main_loop.map(thread::JoinHandle::join);
    process::exit(101);
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
        let refusal = match self.server.convar("fake_license").as_deref() {
            Some("missing") => Some(format!(
                "This server does not have a license key specified. {LICENSE_HELP}"
            )),
            Some("invalid") => {
                Some("Could not authenticate server license key. Invalid key format specified.".to_owned())
            }
            _ => None,
        };
        if let Some(message) = refusal {
            console::line(LICENSE, &format!("Error: {message}"));
            self.fatal(&message);
        }
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
        let commands = read_stdin(Arc::clone(&self.server));
        let crash_at = self.deadline("fake_crash_after_ms");
        let hang_at = self.deadline("fake_hang_after_ms");
        loop {
            let now = Instant::now();
            if self.quit_at.is_some_and(|at| now >= at) {
                self.traces.flush();
                end_after_quit();
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
            self.children
                .retain_mut(|child| matches!(child.try_wait(), Ok(None)));
            self.beat(now);
            self.server.tick();
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
                self.started.retain(|started| started != resource);
                if resource == "observer" {
                    self.bridge = None;
                }
            }
            ("quit", reason) => self.quit(reason),
            ("fake_crash", code) => self.crash(code.and_then(|code| code.parse().ok())),
            ("fake_hang", _) => hang(),
            ("fake_spin", _) => spin(),
            ("fake_spawn", Some(_)) => self.spawn_with_variables(args),
            ("fake_dump_server", _) => self.spawn_dump_server(),
            ("fake_connect", Some(address)) => {
                if let Ok(stream) = TcpStream::connect(address) {
                    self.connections.push(stream);
                }
            }
            ("fake_udp", Some(address)) => {
                let socket = UdpSocket::bind("127.0.0.1:0")
                    .and_then(|socket| socket.connect(address).map(|()| socket));
                if let Ok(socket) = socket {
                    self.datagrams.push(socket);
                }
            }
            ("fake_disconnect", _) => {
                self.connections.clear();
                self.datagrams.clear();
            }
            ("fake_stall_input", Some(ms)) => {
                let until = self.server.uptime_ms() + ms.parse::<u64>().unwrap_or(0);
                self.server.input_stalled_until.store(until, Ordering::Relaxed);
            }
            ("fake_print", Some(_)) => {
                let text = args.join(" ");
                console::script("fake", &text);
                self.traces.native(
                    "ScriptTrace",
                    &json!({ "type": "script_log", "resource": "fake", "text": text }),
                );
            }
            ("fake_trace", Some(kind)) => self.trace(kind),
            ("observer_cmd", Some(hex)) if self.bridge.is_some() => bridge_command(hex),
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
        if !self.started.iter().any(|started| started == resource) {
            self.started.push(resource.to_owned());
        }
        if resource == "observer" {
            let heap_period = Duration::from_millis(self.server.number("fake_heap_ms").unwrap_or(10_000));
            self.bridge = Some(Bridge {
                next_heartbeat: Instant::now(),
                next_heap: Instant::now() + heap_period,
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
            self.traces.script(
                "observer",
                &json!({ "type": "observer", "kind": "ready", "contractVersion": 1 }),
            );
            self.traces.script(
                "observer",
                &json!({ "type": "observer", "kind": "resources", "started": self.started }),
            );
        }
        if now >= bridge.next_heartbeat {
            bridge.next_heartbeat = now + period;
            self.traces
                .script("observer", &json!({ "type": "observer", "kind": "hb" }));
        }
        if now >= bridge.next_heap {
            bridge.next_heap =
                now + Duration::from_millis(self.server.number("fake_heap_ms").unwrap_or(10_000));
            self.traces.script(
                "observer",
                &json!({ "type": "observer", "kind": "heap", "heapUsed": 18_874_368, "heapTotal": 25_165_824, "rss": 62_914_560, "external": 2_097_152 }),
            );
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
        self.fatal(&message)
    }

    /// FXServer's end after a fatal error: the message on stderr, then exit code 1, or on Windows
    /// at times an error dialog that keeps the process up.
    fn fatal(&self, message: &str) -> ! {
        console::fatal(message);
        self.traces.flush();
        if self.server.flag("fake_fatal_dialog") {
            hang();
        }
        process::exit(1)
    }

    /// Without a code, crashes the way the platform does: an access violation on Windows,
    /// SIGSEGV elsewhere.
    fn crash(&self, code: Option<u32>) -> ! {
        self.traces.flush();
        match code {
            Some(code) => process::exit(code.cast_signed()),
            None if self.server.flag("fake_crash_dump") => dump_and_end(),
            None => crash_natively(),
        }
    }

    fn deadline(&self, convar: &str) -> Option<Instant> {
        self.server
            .number(convar)
            .map(|ms| self.server.started + Duration::from_millis(ms))
    }

    /// `fake_spawn`'s words: the variables to add, then the program and its arguments.
    fn spawn_with_variables(&mut self, words: &[String]) {
        let given = words.iter().take_while(|word| word.contains('=')).count();
        let Some((program, args)) = words[given..].split_first() else {
            return;
        };
        let variables: Vec<(&str, &str)> = words[..given]
            .iter()
            .filter_map(|word| word.split_once('='))
            .collect();
        self.spawn(program, args, &variables);
    }

    /// A program a resource starts, with nothing of the fake's own: no pipe to hold open.
    fn spawn(&mut self, program: &str, args: &[String], variables: &[(&str, &str)]) {
        let started = Command::new(program)
            .args(args)
            .envs(variables.iter().copied())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        match started {
            Ok(child) => self.children.push(child),
            Err(error) => console::line(CMD, &format!("cannot start {program}: {error}")),
        }
    }

    /// FXServer's crash dump server: FXServer.exe again on Windows; on Linux the build's loader
    /// running FXServer, which the fake stands for here.
    fn spawn_dump_server(&mut self) {
        let Ok(program) = std::env::current_exe() else {
            return;
        };
        let pid = process::id();
        let args: Vec<String> = if cfg!(windows) {
            vec![format!("{DUMP_SERVER}0"), format!("-parentpid:{pid}")]
        } else {
            let fxserver = program.with_file_name("FXServer");
            // run.sh gives the loader the build's own library folders.
            let alpine = program.ancestors().nth(3).unwrap_or(&program);
            vec![
                "--library-path".to_owned(),
                format!("{}/lib/", alpine.display()),
                "--".to_owned(),
                fxserver.to_string_lossy().into_owned(),
                format!("{DUMP_SERVER}0"),
                format!("-parentppe:{pid}"),
            ]
        };
        self.spawn(&program.to_string_lossy(), &args, &[]);
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

/// What the real bridge raises as `observer:<event>`, printed instead.
fn bridge_command(hex: &str) {
    let bytes: Option<Vec<u8>> = (0..hex.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(hex.get(index..index + 2)?, 16).ok())
        .collect();
    let command = bytes.and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
    if let Some(command) = command {
        console::script(
            "observer",
            &format!(
                "observer:{} {}",
                command["event"].as_str().unwrap_or("?"),
                command["payload"]
            ),
        );
    }
}

/// The dump FXServer's dump server writes, and the report it prints before it ends the process.
fn dump_and_end() -> ! {
    let crashes = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join("crashes")));
    if let Some(crashes) = crashes {
        let dump = crashes.join(format!("{}.dmp", dump_id()));
        let _ = std::fs::create_dir_all(&crashes).and_then(|()| std::fs::write(&dump, dump::crash()));
        console::line(MAIN, "FXServer crashed.");
        console::line(MAIN, &format!("A dump can be found at {}.", dump.display()));
    }
    process::exit(0xFFFF_FFFE_u32.cast_signed())
}

/// A name shaped like the GUIDs FXServer names its dumps by, unique enough for tests.
fn dump_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let pid = process::id();
    format!(
        "{:08x}-{:04x}-4{:03x}-8{:03x}-{:012x}",
        nanos & 0xffff_ffff,
        (nanos >> 32) & 0xffff,
        pid & 0xfff,
        (nanos >> 48) & 0xfff,
        (nanos >> 16) & 0xffff_ffff_ffff,
    )
}

fn stop(resource: &str) {
    console::line(RESOURCES, &format!("Stopping resource {resource}"));
}

/// FXServer 35245 exits with code 1 after `quit` on Windows, and dies of SIGSEGV on Linux.
#[cfg(windows)]
fn end_after_quit() -> ! {
    process::exit(1)
}

#[cfg(unix)]
fn end_after_quit() -> ! {
    crash_natively()
}

#[cfg(windows)]
fn crash_natively() -> ! {
    process::exit(0xC000_0005_u32.cast_signed())
}

#[cfg(unix)]
fn crash_natively() -> ! {
    // Rust's runtime catches the first SIGSEGV to look for a stack overflow, then puts the default
    // action back; the second one ends the process.
    for _ in 0..2 {
        let _ = rustix::process::kill_process(rustix::process::getpid(), rustix::process::Signal::SEGV);
        thread::sleep(Duration::from_millis(20));
    }
    process::abort()
}

fn hang() -> ! {
    loop {
        thread::sleep(Duration::from_secs(3600));
    }
}

fn spin() -> ! {
    loop {
        std::hint::black_box(Instant::now());
    }
}

/// Lines are read one at a time, so that a stall leaves the rest in the pipe.
fn read_stdin(server: Arc<Server>) -> mpsc::Receiver<String> {
    let (lines, commands) = mpsc::channel();
    thread::spawn(move || {
        let mut stdin = io::stdin().lock();
        loop {
            while server.uptime_ms() < server.input_stalled_until.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(10));
            }
            let mut line = String::new();
            match stdin.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            let line = line.trim_end_matches(['\r', '\n']).to_owned();
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
