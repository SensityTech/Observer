//! A stand-in for FXServer in Observer's tests. It takes FXServer's arguments (`+set`, `+exec`,
//! `+ensure`), prints lines shaped like FXServer 35245's, runs the commands it reads on stdin from
//! its main loop, writes structured traces on fd 3 and serves `/dynamic.json`, `/perf/`,
//! `/players.json` and `/info.json` on the cfg's `endpoint_add_tcp`. Started, a folder of the
//! build's `citizen/system_resources` that holds Observer's bridge plays it, under the folder's name
//! as FXServer knows the bridge: `ready`, the started resources and the list of every resource once,
//! then a heartbeat and the heap, the started resources again half a second after one starts or
//! stops, and the list again after a `refresh`. The build's `citizen/` is the `citizen_dir` convar, as on Linux, or else the one next to the
//! executable.
//!
//! Failures on demand, through convars (`+set fake_x y`, in the cfg, or in a `fake-build.cfg` next
//! to the executable, read before the command line, so that one build can crash while another does
//! not) or console commands:
//! - `fake_quit_delay_ms` (500): time between `quit` and the end, which is what FXServer 35245 does:
//!   exit code 1 on Windows, SIGSEGV on Linux;
//! - `fake_ignore_quit 1`: `quit` does nothing;
//! - `fake_start_ms <ms>` (0): how long each resource takes to start, and `fake_start_ms:<name>`
//!   for one of them. FXServer 38435 prints `Started resource` once a resource is up and nothing
//!   while it reads its stream files, which took a clothing pack minutes on a first start;
//! - `fake_fail_start <name>`: that resource does not start, as one whose asset is disabled;
//! - `start`, `stop`, `restart`, `ensure` and `refresh` do what `ServerResources.cpp`,
//!   `ServerResourceList.cpp` and `ResourceDependencyLoader.cpp` say they do, read at the commit of
//!   build 38435 and measured on no server: a resource starts after those its manifest depends
//!   on; one that stops takes with it, first, every started one that depends on it, and nothing
//!   starts those again; `restart` leaves a stopped resource stopped, with `Can't restart a
//!   stopped resource.`; `refresh` scans the folders again and says what it found. The line
//!   FXServer prints of a dependency it cannot find is not printed;
//! - `fake_resource_page <n>` (100): how many resources one page of the bridge's list holds;
//! - `fake_heavy <names>`, `fake_missing <names>`, each a list separated by commas: as it starts
//!   one of them, FXServer warns of two heavy streamed files, one of them oversized, or of a script
//!   its manifest names and it does not hold, on the channel `resources:<name>:stream` cut at
//!   twenty characters, and counts the warnings on the line that says the resource started;
//! - `fake_heartbeat_ms` (3000): the bridge heartbeat period;
//! - `fake_heap_ms` (10000): how often the bridge reports the JS heap;
//! - a build whose scheduler loads Observer's hook, which is its first line naming the bridge and
//!   `hook.lua` in the bridge's folder: every started resource but the bridge prints the hook's
//!   first report, as each server Lua runtime would, save those of `fake_debug_silent <names>`,
//!   which run no Lua on the server; with `fake_debug_demo 1`, the resources named `pma-voice`,
//!   `chat`, `baseevents`, `garage` and `logs` then count each second what a small busy server
//!   would;
//! - `fake_players <n>`: `/dynamic.json` counts `n` clients and `/players.json` lists ids 1 to `n`
//!   with a ping of `20 + 5 × id`, or the anonymised public list without the right token;
//! - `fake_tick_ms <ms>` (0): how long each svMain tick is observed to take in `/perf/`;
//! - `fake_build <n>` (35245): the build `/info.json` reports;
//! - `fake_busy_percent <p>` (0): the thread named like FXServer's network loop,
//!   `UV loop: svNetwork`, which wakes every 10 ms, spins `p` ms of every 100;
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
//! - on stdin: `fake_join [name] [identifier,...] [token,...]` and
//!   `fake_drop <id> <category> <resource or -> <reason...>`: a player joins or drops, which
//!   `/players.json` follows and the bridge reports (a join shows a full set of identifiers and
//!   tokens, and the address `192.0.2.77`, IPv4-mapped for every other player, unless given);
//!   `fake_drop_before_join <id>`, a resource's kick inside its own `playerJoining` handler;
//!   `fake_label <id> [label id] [name...]`, a resource giving a player a label, or clearing it
//!   without one; the players already there are reported once the bridge starts, and `quit` drops
//!   every player with category 7 through the trace thread, as FXServer does;
//! - on stdin: `fake_connecting [name] [identifier,...] [token,...]`, a player the bridge holds at
//!   `playerConnecting`: it reports them and waits for Observer's verdict, `fake_verdict_ms`
//!   (5000) at most, then lets them in as `fake_join` does, or refuses them; a verdict to drop a
//!   connected player drops them with category 1, the bridge as the resource;
//! - on stdin: `fake_move <id> <x> <y> <z>`, which puts a player at a position the bridge reports
//!   while Observer asks for positions; the others go round a circle. With `onesync off` the bridge
//!   reports no position, as it reads them from OneSync;
//! - on stdin or in the cfg: `fake_entity <vehicle|bike|boat|heli|train|ped|object> <x> <y> <z>
//!   [resource or -] [owner or -] [bucket]`, an entity of the game under the next net id, from 1,
//!   and `fake_entities <vehicles> <peds> <objects>`, as many spread over the map as a small busy
//!   server holds; the bridge lists them while Observer asks, in sweeps of 500 rows a part, with
//!   every player's own ped counted and left out, and deletes those Observer names. Nothing here
//!   was measured on a real FXServer: the entities are the fake's own, and the bridge's natives
//!   are taken to do what their names say. `fake_entity_gone <net id>` removes one as the game
//!   does by itself, `fake_entity_stays <net id>` has the game keep one it is asked to delete,
//!   `fake_enter <player> [net id]` seats a player in a vehicle or takes them out, and
//!   `fake_possess <player> <net id>` makes a ped a player's own;
//! - `fake_entities_every_ms`: how often the bridge sweeps, whatever Observer asks, for tests that
//!   cannot wait five seconds; `fake_entities_row_us` (2) and `fake_entities_list_us`: what a
//!   sweep costs for each entity and for listing them, in microseconds, which cuts a sweep short
//!   past its budget and spaces the next ones out; `set gamename rdr3` and `onesync off` each
//!   leave the server without entities to list;
//! - on stdin: `fake_debug <resource> <list> <fields...>`, one row of a hook's report (`ev`, `np`,
//!   `un`, `sb`, `sbl` or `http`) printed as that resource;
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
//! that tests can see it arrive; a refusal prints the message as JSON after `refused`, which the
//! real bridge shows the player instead. A convar typed alone gets the first line FXServer prints
//! for it, ` "name" is "value"`; unknown commands get FXServer's `No such command <name>.`

mod console;
mod dump;
mod entities;
mod http;
mod players;
mod resources;
#[allow(unsafe_code)]
mod traces;

use std::collections::HashMap;
use std::io::{self, BufRead};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::path::{Path, PathBuf};
use std::process::{self, Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::json;

use crate::console::{CMD, LICENSE, MAIN, NET, RESOURCES, SCRIPTING, SERVER};
use crate::players::{Player, Players};
use crate::resources::Registry;
use crate::traces::Traces;

const TICK: Duration = Duration::from_millis(50);
const LICENSE_HELP: &str = "Please set the sv_licenseKey console variable to a key from https://portal.cfx.re/servers/registration-keys.";
/// The argument that turns the process into the child that holds the pipes.
const HOLD_PIPES: &str = "--fake-hold-pipes";
/// What FXServer passes the copy of itself that serves as its crash dump server.
const DUMP_SERVER: &str = "-dumpserver:";
/// FXServer's main thread, as FXServer 35245 names it.
const SV_MAIN: &str = "UV loop: svMain";
/// Starts and stops the bridge tells in one report, past which it only counts them.
const RESOURCE_EVENTS_MAX: usize = 64;
/// How long the bridge waits after a start or a stop before it reports, unless its script says.
const RESOURCES_SETTLE_MS: u64 = 500;

/// What the HTTP threads share with the main loop.
pub struct Server {
    convars: Mutex<HashMap<String, String>>,
    players: Mutex<Players>,
    main_ticks: AtomicU64,
    /// What svMain's ticks were observed to take, as `/perf/` reports it.
    main_observed: Mutex<Observed>,
    started: Instant,
    /// Uptime in milliseconds until which stdin is left unread.
    input_stalled_until: AtomicU64,
}

impl Server {
    /// FXServer reads convar names without regard to case.
    pub fn convar(&self, name: &str) -> Option<String> {
        self.convars
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&name.to_ascii_lowercase())
            .cloned()
    }

    fn set(&self, name: &str, value: &str) {
        self.convars
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(name.to_ascii_lowercase(), value.to_owned());
    }

    fn number(&self, name: &str) -> Option<u64> {
        self.convar(name)?.parse().ok()
    }

    fn flag(&self, name: &str) -> bool {
        self.number(name).is_some_and(|value| value != 0)
    }

    pub fn players(&self) -> std::sync::MutexGuard<'_, Players> {
        self.players.lock().unwrap_or_else(PoisonError::into_inner)
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
    /// The resource's name, which FXServer gives each of its traces.
    name: String,
    next_heartbeat: Instant,
    next_heap: Instant,
    ready: bool,
    /// The `CONTRACT_VERSION` of the script, which the real bridge reports when it is ready.
    contract: u64,
    /// The script's `RESOURCES_SETTLE_MS`: one report covers the starts and stops of that long.
    settle: Duration,
}

/// The starts and stops the bridge has not reported yet.
#[derive(Default)]
struct Unreported {
    events: Vec<[String; 2]>,
    dropped: u64,
    report_at: Option<Instant>,
}

/// While the build's scheduler loads Observer's hook: every resource counts and reports.
struct Counting {
    next: Instant,
    /// The seconds counted so far, which the demo varies with.
    second: u64,
    /// How many of the started resources sent their first report.
    greeted: usize,
}

/// While Observer asks for positions: how often the bridge reports them, until when.
struct Following {
    every: Duration,
    until: Instant,
    next: Instant,
}

/// A player the bridge holds at `playerConnecting` until Observer's verdict on `request`.
struct Waiting {
    request: u32,
    player: Player,
    until: Instant,
}

struct Fake {
    server: Arc<Server>,
    waiting: Vec<Waiting>,
    following: Option<Following>,
    /// While Observer asks for the entities.
    listing: Option<Following>,
    world: entities::World,
    /// The answers to deletions, each sent once the bridge has checked what is left.
    deleted: Vec<(Instant, serde_json::Value)>,
    placed: HashMap<u32, [f64; 3]>,
    last_request: u32,
    traces: Traces,
    endpoint: Option<String>,
    bridge: Option<Bridge>,
    counting: Option<Counting>,
    registry: Registry,
    /// The cfg has run: from then on `ensure` stops a started resource before it starts it.
    configured: bool,
    unreported: Unreported,
    /// The lists of resources the bridge sent, which number their pages.
    lists: u64,
    /// Every resource started, in order.
    started: Vec<String>,
    quit_at: Option<Instant>,
    /// What `fake_spawn` started, reaped as they end.
    children: Vec<Child>,
    connections: Vec<TcpStream>,
    datagrams: Vec<UdpSocket>,
}

/// What a resource of the demo server counts in its `second`, by its name: a voice resource that
/// talks to clients all the time, with one client far above the others, a chat, a garage that
/// writes state bags and refuses an event, and a logger that calls a webhook.
fn demo_counts(resource: &str, second: u64) -> Option<serde_json::Value> {
    // A slow wave, so that the traffic moves on the page.
    let wave = (second % 60).abs_diff(30);
    Some(match resource {
        "pma-voice" => {
            let out = 150 + wave;
            let talkers = [(1, 19), (2, 3), (3, 2)];
            let sent: u64 = talkers.iter().map(|(_, count)| count).sum();
            let mut events = vec![
                json!(["toC", "pma-voice:setTalkingOnRadio", out, out * 41, 44, 0, 0]),
                json!([
                    "inN",
                    "pma-voice:setTalkingOnRadio",
                    sent,
                    sent * 38,
                    40,
                    sent * 20,
                    60
                ]),
            ];
            if second.is_multiple_of(3) {
                events.push(json!(["toC", "pma-voice:syncRadioData", 1, 1200, 3800, 0, 0]));
            }
            if second.is_multiple_of(5) {
                events.push(json!(["inN", "pma-voice:setPlayerRadio", 1, 36, 36, 40, 40]));
            }
            json!({
                "ev": events,
                "np": talkers
                    .iter()
                    .map(|(player, count)| json!(["pma-voice:setTalkingOnRadio", player, count, count * 38]))
                    .collect::<Vec<_>>(),
                "sb": [
                    ["player", "proximity", 28, 252, 9, 28],
                    ["player", "voiceIntent", 9, 72, 8, 9],
                    ["player", "radioChannel", 3, 15, 5, 3],
                ],
                "sbl": [["player:1", "proximity", 9, 1]],
            })
        }
        "chat" if second.is_multiple_of(4) => json!({
            "ev": [
                ["toA", "chat:addMessage", 1, 310, 310, 0, 0],
                ["inN", "_chat:messageEntered", 1, 120, 120, 90, 90],
            ],
            "np": [["_chat:messageEntered", 2, 1, 120]],
        }),
        "baseevents" if second.is_multiple_of(10) => json!({
            "ev": [
                ["inN", "baseevents:enteredVehicle", 1, 84, 84, 30, 30],
                ["inI", "playerJoining", 1, 12, 12, 15, 15],
                ["in", "playerDropped", 1, 48, 48, 20, 20],
            ],
            "np": [["baseevents:enteredVehicle", 3, 1, 84]],
        }),
        "garage" => {
            let mut report = json!({
                "sb": [["entity", "fuel", 4, 1240, 2100, 4], ["global", "weather", 1, 64, 64, 0]],
                "sbl": [["entity:1042", "fuel", 2100, 1]],
            });
            if second.is_multiple_of(20) {
                report["un"] = json!([["garage:giveVehicle", 1, 1]]);
                report["ev"] = json!([["inN", "garage:giveVehicle", 1, 60, 60, 0, 0]]);
                report["np"] = json!([["garage:giveVehicle", 1, 1, 60]]);
            }
            if second == 30 {
                report["http"] = json!([["POST", "paste.example", 1, 2300]]);
            }
            report
        }
        "logs" if second % 5 < 3 => json!({ "http": [["POST", "discord.com", 1, 400]] }),
        _ => return None,
    })
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
                players: Mutex::default(),
                main_ticks: AtomicU64::new(0),
                main_observed: Mutex::default(),
                started: Instant::now(),
                input_stalled_until: AtomicU64::new(0),
            }),
            traces,
            waiting: Vec::new(),
            following: None,
            listing: None,
            world: entities::World::default(),
            deleted: Vec::new(),
            placed: HashMap::new(),
            last_request: 0,
            endpoint: None,
            bridge: None,
            counting: None,
            registry: Registry::default(),
            configured: false,
            unreported: Unreported::default(),
            lists: 0,
            started: Vec::new(),
            quit_at: None,
            children: Vec::new(),
            connections: Vec::new(),
            datagrams: Vec::new(),
        };
        let mut commands = build_cfg();
        commands.extend(command_line(args.into_iter()));
        fake.boot(&commands);
        fake.run()
    });
    // The main loop ends the process itself: it only comes back here if it panicked.
    let _ = main_loop.map(thread::JoinHandle::join);
    process::exit(101);
}

/// The resources under the folders named `category` of `folder`, in name order, as FXServer
/// starts every one with that folder on its path.
fn category_members(folder: &Path, category: &str) -> Vec<String> {
    let mut members = Vec::new();
    let mut folders = vec![(folder.to_path_buf(), false)];
    while let Some((folder, inside)) = folders.pop() {
        let Ok(entries) = std::fs::read_dir(folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if !entry.path().is_dir() {
                continue;
            }
            if name.starts_with('[') && name.ends_with(']') {
                folders.push((entry.path(), inside || name.eq_ignore_ascii_case(category)));
            } else if inside {
                members.push(name);
            }
        }
    }
    members.sort_unstable();
    members
}

impl Fake {
    fn boot(&mut self, commands: &[Vec<String>]) {
        // FXServer applies every `+set` of its command line before it runs anything else, the cfg
        // included, wherever it stands: a convar the cfg sets too keeps the cfg's value.
        for command in commands.iter().filter(|command| is_set(command)) {
            self.execute(command);
        }
        self.registry = Registry::scan(&server_folder(), self.system_resources().as_deref());
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
        self.configured = true;

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
        network_loop(Arc::clone(&self.server));
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
            self.admit_unanswered(now);
            self.report_positions(now);
            self.report_resources(now);
            self.report_entities(now);
            self.beat(now);
            self.count(now);
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
            ("ensure" | "start", Some(category)) if category.starts_with('[') && category.ends_with(']') => {
                self.ensure_category(category);
            }
            ("ensure", Some(resource)) => self.ensure(resource),
            ("start", Some(resource)) => self.start_command(resource),
            ("restart", Some(resource)) => self.restart(resource),
            ("stop", Some(resource)) => self.stop(resource),
            ("refresh", None) => self.refresh(),
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
            ("fake_debug", Some(resource)) => self.counted(resource, &args[1..]),
            ("fake_join", _) => self.join(args.first(), args.get(1), args.get(2)),
            ("fake_connecting", _) => self.connecting(args.first(), args.get(1), args.get(2)),
            ("fake_move", Some(id)) => self.place(id, &args[1..]),
            (entity, Some(_)) if entities::COMMANDS.contains(&entity) => self.world.command(entity, args),
            ("fake_drop", Some(id)) => {
                let category = args
                    .get(1)
                    .and_then(|category| category.parse().ok())
                    .unwrap_or(2);
                let resource = args
                    .get(2)
                    .map_or("", |resource| if resource == "-" { "" } else { resource });
                let reason = args.get(3..).map(|words| words.join(" ")).unwrap_or_default();
                if let Ok(id) = id.parse() {
                    self.drop_player(id, category, resource, &reason);
                }
            }
            ("fake_label", Some(id)) => self.label(id, &args[1..]),
            ("fake_drop_before_join", Some(id)) => self.drop_before_join(id),
            ("observer_cmd", Some(hex)) if self.bridge.is_some() => self.bridge_command(hex),
            // A convar's name followed by a value sets it, as FXServer's console does for a convar
            // that exists: `sv_` ones are declared, any other once something set it.
            (convar, Some(value))
                if convar.starts_with("sv_")
                    || convar.starts_with("fake_")
                    || self.server.convar(convar).is_some() =>
            {
                self.server.set(name, value);
            }
            // Alone, it prints the first line FXServer prints for it (`Console.Variables.h`).
            (convar, None) if self.server.convar(convar).is_some() => {
                let value = self.server.convar(convar).unwrap_or_default();
                console::line(CMD, &format!(" \"{name}\" is \"{value}\""));
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

    fn ensure_category(&mut self, category: &str) {
        let members = category_members(Path::new("resources"), category);
        if members.is_empty() {
            console::line(SERVER, &format!("Couldn't find resource category {category}."));
        }
        for member in members {
            self.ensure(&member);
        }
    }

    fn is_started(&self, resource: &str) -> bool {
        self.started.iter().any(|started| started == resource)
    }

    /// FXServer's `ensure`: a start, after a stop of a started resource once the cfg has run.
    fn ensure(&mut self, resource: &str) {
        if self.configured && self.is_started(resource) {
            self.stop(resource);
        }
        self.start_command(resource);
    }

    /// FXServer's `start`: nothing for a resource that is up.
    fn start_command(&mut self, resource: &str) {
        if !self.is_started(resource) && !self.start_after_needs(resource, &mut Vec::new()) {
            console::line(SERVER, &format!("Couldn't start resource {resource}."));
        }
    }

    /// Starts `resource` once every resource its manifest depends on is up, each started in turn.
    /// `starting` holds those on the way, so that two that depend on each other end.
    fn start_after_needs(&mut self, resource: &str, starting: &mut Vec<String>) -> bool {
        if self.server.convar("fake_fail_start").as_deref() == Some(resource) {
            return false;
        }
        starting.push(resource.to_owned());
        for need in self.registry.needs(resource) {
            let on = self.registry.resolve(&need, &self.started);
            let up = self.is_started(&on) || starting.contains(&on);
            if !up && !self.start_after_needs(&on, starting) {
                return false;
            }
        }
        self.start(resource);
        self.resource_event(resource, "start");
        true
    }

    /// FXServer's `stop`: every started resource that depends on this one stops first, and
    /// nothing starts it again.
    fn stop(&mut self, resource: &str) {
        if !self.is_started(resource) {
            return;
        }
        // Out of the started ones first, so that two that depend on each other end.
        self.started.retain(|started| started != resource);
        for dependant in self.registry.dependants(resource, &self.started) {
            self.stop(&dependant);
        }
        console::line(RESOURCES, &format!("Stopping resource {resource}"));
        self.resource_event(resource, "stop");
        if self.bridge.as_ref().is_some_and(|bridge| bridge.name == resource) {
            self.bridge = None;
            self.listing = None;
            self.unreported = Unreported::default();
        }
    }

    /// FXServer's `restart`: a stop then a start, of a started resource only.
    fn restart(&mut self, resource: &str) {
        if !self.is_started(resource) {
            console::line(SERVER, "Can't restart a stopped resource.");
            return;
        }
        self.stop(resource);
        self.start_command(resource);
    }

    /// FXServer's `refresh`: the folders are scanned again, and the bridge hears of it.
    fn refresh(&mut self) {
        console::line(RESOURCES, "\x1b[32mScanning resources.\x1b[0m");
        let refreshed = self
            .registry
            .refresh(&server_folder(), self.system_resources().as_deref());
        let found = if refreshed.read > 0 {
            format!(
                "Found {} new resources, and refreshed {}/{} resources.",
                refreshed.added, refreshed.changed, refreshed.read
            )
        } else {
            format!("Found {} resources.", refreshed.added)
        };
        console::line(RESOURCES, &format!("\x1b[32m{found}\x1b[0m"));
        if self.bridge.as_ref().is_some_and(|bridge| bridge.ready) {
            self.report_resource_list();
        }
    }

    fn start(&mut self, resource: &str) {
        console::line(SCRIPTING, &format!("Creating script environments for {resource}"));
        let own = self.server.number(&format!("fake_start_ms:{resource}"));
        if let Some(ms) = own.or_else(|| self.server.number("fake_start_ms")) {
            thread::sleep(Duration::from_millis(ms));
        }
        let listed = |convar: &str| {
            self.server
                .convar(convar)
                .is_some_and(|names| names.split(',').any(|name| name == resource))
        };
        let stream = format!("resources:{resource}:stream");
        let channel = (stream.get(..20).unwrap_or(&stream), 83);
        let mut warnings = Vec::new();
        if listed("fake_heavy") {
            warnings.push(format!(
                "Asset {resource}/{resource}_hi.ydr uses 49.5 MiB of physical memory. Oversized assets \
                 can and WILL lead to streaming issues (such as models not loading/rendering)."
            ));
            warnings.push(format!(
                "Asset {resource}/{resource}.ytd uses 16.2 MiB of physical memory."
            ));
        }
        if listed("fake_missing") {
            warnings.push(
                "Warning: could not find client_script `client/gone.lua` (defined in fxmanifest.lua:9)"
                    .to_owned(),
            );
        }
        for warning in &warnings {
            console::line(channel, warning);
        }
        let counted = match warnings.len() {
            0 => String::new(),
            1 => " (1 warning)".to_owned(),
            several => format!(" ({several} warnings)"),
        };
        console::line(RESOURCES, &format!("Started resource {resource}{counted}"));
        if !self.started.iter().any(|started| started == resource) {
            self.started.push(resource.to_owned());
        }
        let script = self.bridge_script(resource);
        if let Some(contract) = script.as_deref().and_then(contract_version) {
            let settle = script
                .as_deref()
                .and_then(|script| constant(script, "RESOURCES_SETTLE_MS"))
                .unwrap_or(RESOURCES_SETTLE_MS);
            let heap_period = Duration::from_millis(self.server.number("fake_heap_ms").unwrap_or(10_000));
            self.counting = self.hooked(resource).then(|| Counting {
                next: Instant::now(),
                second: 0,
                greeted: 0,
            });
            self.listing = None;
            self.deleted.clear();
            self.world.bridge_started(self.server.uptime_ms());
            self.bridge = Some(Bridge {
                name: resource.to_owned(),
                next_heartbeat: Instant::now(),
                next_heap: Instant::now() + heap_period,
                ready: false,
                contract,
                settle: Duration::from_millis(settle),
            });
        }
    }

    /// A start or a stop the bridge hears of once the cfg has run: it reports them together a
    /// moment later, 64 of them at most.
    fn resource_event(&mut self, resource: &str, change: &str) {
        let Some(bridge) = self.bridge.as_ref().filter(|bridge| bridge.ready) else {
            return;
        };
        if self.unreported.events.len() < RESOURCE_EVENTS_MAX {
            self.unreported
                .events
                .push([resource.to_owned(), change.to_owned()]);
        } else {
            self.unreported.dropped += 1;
        }
        self.unreported
            .report_at
            .get_or_insert_with(|| Instant::now() + bridge.settle);
    }

    /// The bridge's report of the resources started, with the starts and stops since the last.
    fn report_resources(&mut self, now: Instant) {
        if self.unreported.report_at.is_none_or(|at| now < at) {
            return;
        }
        let unreported = std::mem::take(&mut self.unreported);
        let mut report = json!({ "type": "observer", "kind": "resources", "started": self.started });
        if !unreported.events.is_empty() {
            report["events"] = json!(unreported.events);
        }
        if unreported.dropped > 0 {
            report["dropped"] = json!(unreported.dropped);
        }
        self.bridge_report(&report);
    }

    /// The bridge's list of every resource FXServer knows, in pages.
    fn report_resource_list(&mut self) {
        self.lists += 1;
        let per_page = usize::try_from(self.server.number("fake_resource_page").unwrap_or(100))
            .unwrap_or(100)
            .max(1);
        let listed = self.registry.listed(&self.started);
        let pages = listed.len().div_ceil(per_page).max(1);
        for page in 0..pages {
            let resources: Vec<&serde_json::Value> =
                listed.iter().skip(page * per_page).take(per_page).collect();
            self.bridge_report(&json!({
                "type": "observer", "kind": "resource_list", "list": self.lists,
                "page": page + 1, "last": page + 1 == pages, "resources": resources,
            }));
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
                &bridge.name,
                &json!({ "type": "observer", "kind": "ready", "contractVersion": bridge.contract }),
            );
            self.traces.script(
                &bridge.name,
                &json!({ "type": "observer", "kind": "resources", "started": self.started }),
            );
            let name = bridge.name.clone();
            self.report_resource_list();
            let roster = self.server.players().roster(self.server.uptime_ms());
            for page in roster {
                self.traces.script(&name, &page);
            }
        }
        let Some(bridge) = &mut self.bridge else {
            return;
        };
        if now >= bridge.next_heartbeat {
            bridge.next_heartbeat = now + period;
            self.traces
                .script(&bridge.name, &json!({ "type": "observer", "kind": "hb" }));
        }
        if now >= bridge.next_heap {
            bridge.next_heap =
                now + Duration::from_millis(self.server.number("fake_heap_ms").unwrap_or(10_000));
            self.traces.script(
                &bridge.name,
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
        let reason = format!(
            "Server shutting down: {}",
            reason.unwrap_or("Quit command executed.")
        );
        let ids = self.server.players().ids();
        for id in ids {
            self.drop_player(id, 7, "", &reason);
        }
    }

    fn join(&self, name: Option<&String>, identifiers: Option<&String>, tokens: Option<&String>) {
        let player = self.server.players().coming(
            name.map(String::as_str),
            identifiers.map(String::as_str),
            tokens.map(String::as_str),
        );
        self.admit(&player);
    }

    fn admit(&self, player: &Player) {
        let id = self.server.players().admit(player.clone());
        console::line(MAIN, &format!("Player {} connected (id {id})", player.name));
        let report = players::join_report(id, player, self.server.uptime_ms());
        self.bridge_report(&report);
    }

    /// A player the bridge holds until Observer answers; without the bridge, they get in at once.
    fn connecting(&mut self, name: Option<&String>, identifiers: Option<&String>, tokens: Option<&String>) {
        let player = self.server.players().coming(
            name.map(String::as_str),
            identifiers.map(String::as_str),
            tokens.map(String::as_str),
        );
        if !self.bridge.as_ref().is_some_and(|bridge| bridge.ready) {
            self.admit(&player);
            return;
        }
        self.last_request += 1;
        let mut report = players::entry(0, &player);
        report["type"] = json!("observer");
        report["kind"] = json!("connecting");
        report["request"] = json!(self.last_request);
        if let Some(entry) = report.as_object_mut() {
            entry.remove("id");
        }
        self.bridge_report(&report);
        let wait = Duration::from_millis(self.server.number("fake_verdict_ms").unwrap_or(5000));
        self.waiting.push(Waiting {
            request: self.last_request,
            player,
            until: Instant::now() + wait,
        });
    }

    /// The players' positions, as the bridge reports them while Observer asks.
    fn report_positions(&mut self, now: Instant) {
        let Some(following) = self.following.as_mut() else {
            return;
        };
        if now >= following.until {
            self.following = None;
            return;
        }
        if now < following.next {
            return;
        }
        following.next = now + following.every;
        let t = self.server.uptime_ms();
        let onesync = self.server.convar("onesync").is_some_and(|value| value != "off");
        let players: Vec<serde_json::Value> = if onesync {
            self.server
                .players()
                .ids()
                .into_iter()
                .map(|id| {
                    let [x, y, z] = self.placed.get(&id).copied().unwrap_or_else(|| {
                        #[expect(clippy::cast_precision_loss, reason = "a test player's place on a circle")]
                        let angle = t as f64 / 20_000.0 + f64::from(id);
                        [400.0 * angle.cos() + 300.0, 400.0 * angle.sin() - 1200.0, 30.0]
                    });
                    json!({ "id": id, "x": x, "y": y, "z": z, "h": 90.0 })
                })
                .collect()
        } else {
            Vec::new()
        };
        for (index, page) in players.chunks(100).enumerate() {
            let last = (index + 1) * 100 >= players.len();
            self.bridge_report(&json!({
                "type": "observer", "kind": "positions", "t": t, "onesync": onesync,
                "page": index + 1, "last": last, "players": page,
            }));
        }
        if players.is_empty() {
            self.bridge_report(&json!({
                "type": "observer", "kind": "positions", "t": t, "onesync": onesync,
                "page": 1, "last": true, "players": [],
            }));
        }
    }

    /// The entities, swept while Observer asks, and the answers to its deletions once due.
    fn report_entities(&mut self, now: Instant) {
        let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut self.deleted)
            .into_iter()
            .partition(|(at, _)| now >= *at);
        self.deleted = later;
        for (_, reply) in due {
            self.bridge_report(&reply);
        }
        let Some(listing) = self.listing.as_mut() else {
            return;
        };
        if now >= listing.until {
            self.listing = None;
            self.world.unspaced();
            return;
        }
        if now < listing.next {
            return;
        }
        let every_ms = u64::try_from(listing.every.as_millis()).unwrap_or(u64::MAX);
        let parts = self.world.sweep(&entities::Server {
            players: self.server.players().len(),
            onesync: self.server.convar("onesync").is_some_and(|value| value != "off"),
            gta: self.server.convar("gamename").is_none_or(|game| game == "gta5"),
            every_ms,
            row_us: self.server.number("fake_entities_row_us").unwrap_or(2),
            list_us: self.server.number("fake_entities_list_us"),
        });
        let cadence = self
            .server
            .number("fake_entities_every_ms")
            .unwrap_or_else(|| self.world.cadence_ms(every_ms));
        if let Some(listing) = self.listing.as_mut() {
            listing.next = now + Duration::from_millis(cadence);
        }
        for part in parts {
            self.bridge_report(&part);
        }
    }

    /// Who waited for a verdict longer than the bridge waits gets in.
    fn admit_unanswered(&mut self, now: Instant) {
        let (due, waiting): (Vec<Waiting>, Vec<Waiting>) = std::mem::take(&mut self.waiting)
            .into_iter()
            .partition(|waiting| now >= waiting.until);
        self.waiting = waiting;
        for waiting in due {
            self.admit(&waiting.player);
        }
    }

    /// A resource's kick inside its own `playerJoining` handler: the drop comes before the join.
    fn drop_before_join(&self, id: &str) {
        let Ok(id) = id.parse::<u32>() else {
            return;
        };
        let t = self.server.uptime_ms();
        self.bridge_report(&players::drop_report(id, t, 1, "gate", "Not whitelisted"));
        let player = Player {
            name: "Late".to_owned(),
            identifiers: vec![format!("license:{id:040x}")],
            tokens: Vec::new(),
        };
        self.bridge_report(&players::join_report(id, &player, t));
    }

    /// A resource giving player `id` a label of `[label id, name...]`, or clearing it with nothing.
    fn label(&self, id: &str, label: &[String]) {
        let Ok(id) = id.parse::<u32>() else {
            return;
        };
        let label = label
            .split_first()
            .map(|(label_id, name)| json!({ "id": label_id, "name": name.join(" ") }));
        self.bridge_report(&json!({
            "type": "observer", "kind": "labels",
            "labels": [{ "id": id, "label": label, "resource": "fake" }],
        }));
    }

    /// Player `id` standing at `[x, y, z]`.
    fn place(&mut self, id: &str, coordinates: &[String]) {
        let coordinates: Vec<f64> = coordinates
            .iter()
            .filter_map(|value| value.parse().ok())
            .collect();
        if let (Ok(id), [x, y, z]) = (id.parse(), coordinates.as_slice()) {
            self.placed.insert(id, [*x, *y, *z]);
        }
    }

    fn drop_player(&self, id: u32, category: u64, resource: &str, reason: &str) {
        if self.server.players().leave(id).is_none() {
            return;
        }
        let report = players::drop_report(id, self.server.uptime_ms(), category, resource, reason);
        self.bridge_report(&report);
    }

    /// A report of the bridge, once it started: the real one only hears what happens after.
    fn bridge_report(&self, report: &serde_json::Value) {
        if let Some(bridge) = self.bridge.as_ref().filter(|bridge| bridge.ready) {
            self.traces.script(&bridge.name, report);
        }
    }

    /// The script of `resource` if it is Observer's bridge: a folder of the build's
    /// `system_resources` whose script registers `observer_cmd`. Its name changes with each start.
    fn citizen(&self) -> Option<PathBuf> {
        self.server.convar("citizen_dir").map(PathBuf::from).or_else(|| {
            let program = std::env::current_exe().ok()?;
            Some(program.parent()?.join("citizen"))
        })
    }

    /// Whether the build's scheduler loads the hook of `bridge`, as the real FXServer would find
    /// when it runs the scheduler's first line in each resource.
    fn hooked(&self, bridge: &str) -> bool {
        let Some(citizen) = self.citizen() else {
            return false;
        };
        let scheduler = citizen.join("scripting").join("lua").join("scheduler.lua");
        let loads = std::fs::read_to_string(scheduler).is_ok_and(|scheduler| {
            scheduler
                .trim_start_matches('\u{feff}')
                .starts_with("--[[obs:hook:head ")
                && scheduler.contains(&format!("LoadResourceFile('{bridge}', 'hook.lua')"))
        });
        loads
            && citizen
                .join("system_resources")
                .join(bridge)
                .join("hook.lua")
                .is_file()
    }

    /// What the hook prints from every resource: a first report from each, then, with
    /// `fake_debug_demo`, what a small busy server counts each second.
    fn count(&mut self, now: Instant) {
        let Some(counting) = &mut self.counting else {
            return;
        };
        if now < counting.next {
            return;
        }
        counting.next = now + Duration::from_secs(1);
        counting.second += 1;
        let second = counting.second;
        let greeted = std::mem::replace(&mut counting.greeted, self.started.len());
        let demo = self.server.flag("fake_debug_demo");
        let silent = self.server.convar("fake_debug_silent").unwrap_or_default();
        let bridge = self.bridge.as_ref().map(|bridge| bridge.name.as_str());
        for (index, resource) in self.started.iter().enumerate() {
            if Some(resource.as_str()) == bridge || silent.split(',').any(|name| name == resource) {
                continue;
            }
            let counted = if demo { demo_counts(resource, second) } else { None };
            let mut report = match (counted, index >= greeted) {
                (Some(counted), _) => counted,
                (None, true) => json!({}),
                (None, false) => continue,
            };
            if index >= greeted {
                report["hello"] = json!({ "hook": "1", "timer": "nanotime" });
            }
            report["type"] = json!("observer_debug");
            report["v"] = json!(1);
            report["seq"] = json!(second);
            report["dur"] = json!(1000);
            self.traces.script(resource, &report);
        }
    }

    /// `fake_debug <resource> <list> <fields>`: one row of the hook's report, as `resource`.
    fn counted(&self, resource: &str, row: &[String]) {
        let Some((list, fields)) = row.split_first() else {
            return;
        };
        let fields: Vec<serde_json::Value> = fields
            .iter()
            .map(|field| {
                field
                    .parse::<i64>()
                    .map_or_else(|_| json!(field), |number| json!(number))
            })
            .collect();
        let mut report = json!({ "type": "observer_debug", "v": 1, "seq": 1, "dur": 1000 });
        report[list.as_str()] = json!([fields]);
        self.traces.script(resource, &report);
    }

    /// The build's own resources, where the bridge is written too.
    fn system_resources(&self) -> Option<PathBuf> {
        Some(self.citizen()?.join("system_resources"))
    }

    fn bridge_script(&self, resource: &str) -> Option<String> {
        let script = self.system_resources()?.join(resource).join("server.lua");
        std::fs::read_to_string(script)
            .ok()
            .filter(|script| script.contains("RegisterCommand('observer_cmd'"))
    }

    /// What the real bridge raises as `observer:<event>`, printed instead.
    fn bridge_command(&mut self, hex: &str) {
        let Some(bridge) = &self.bridge else {
            return;
        };
        let bridge = bridge.name.clone();
        let bytes: Option<Vec<u8>> = (0..hex.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(hex.get(index..index + 2)?, 16).ok())
            .collect();
        let command = bytes.and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
        let Some(command) = command else {
            return;
        };
        let payload = &command["payload"];
        match command["action"].as_str() {
            Some("verdict") => self.verdict(&bridge, payload),
            Some("positions") => {
                let every = payload["everyMs"].as_u64().unwrap_or(2000).clamp(1000, 10_000);
                let lasting = payload["forMs"].as_u64().unwrap_or(0).min(60_000);
                let now = Instant::now();
                self.following = (lasting > 0).then(|| Following {
                    every: Duration::from_millis(every),
                    until: now + Duration::from_millis(lasting),
                    next: now,
                });
            }
            Some("entities") => {
                let every = payload["everyMs"].as_u64().unwrap_or(15_000).clamp(5000, 60_000);
                let lasting = payload["forMs"].as_u64().unwrap_or(0).min(60_000);
                let now = Instant::now();
                // An ask that renews one keeps the sweeps where they were.
                let next = self.listing.as_ref().map_or(now, |listing| listing.next);
                self.listing = (lasting > 0).then(|| Following {
                    every: Duration::from_millis(every),
                    until: now + Duration::from_millis(lasting),
                    next,
                });
                if self.listing.is_none() {
                    self.world.unspaced();
                }
            }
            Some("deleteEntities") => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |since| i64::try_from(since.as_secs()).unwrap_or(i64::MAX));
                if let Some(reply) = self.world.delete(payload, now) {
                    // The real bridge looks again a quarter of a second after it asked the game.
                    self.deleted
                        .push((Instant::now() + Duration::from_millis(250), reply));
                }
            }
            Some("drop") => {
                let (Some(id), Some(message)) = (
                    payload["player"].as_u64().and_then(|id| u32::try_from(id).ok()),
                    payload["message"].as_str(),
                ) else {
                    return;
                };
                console::script(&bridge, &format!("observer:playerBanned {}", payload["event"]));
                self.drop_player(id, 1, &bridge, message);
            }
            Some(_) => {}
            None => console::script(
                &bridge,
                &format!(
                    "observer:{} {}",
                    command["event"].as_str().unwrap_or("?"),
                    payload
                ),
            ),
        }
    }

    /// Observer's answer for a player held at `playerConnecting`: in, or refused with a message.
    fn verdict(&mut self, bridge: &str, payload: &serde_json::Value) {
        let Some(index) = self
            .waiting
            .iter()
            .position(|waiting| payload["request"].as_u64() == Some(u64::from(waiting.request)))
        else {
            return;
        };
        let waiting = self.waiting.remove(index);
        match payload["refusal"]["message"].as_str() {
            Some(message) if !message.is_empty() => {
                console::script(bridge, &format!("refused {}", json!(message)));
                console::script(
                    bridge,
                    &format!("observer:playerRefused {}", payload["refusal"]["event"]),
                );
            }
            _ => self.admit(&waiting.player),
        }
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

/// A thread by the name FXServer's network loop has, for the thread measures to find: Windows
/// keeps the name as the thread's description, Linux cuts it to `UV loop: svNetw`.
fn network_loop(server: Arc<Server>) {
    const PERIOD: Duration = Duration::from_millis(100);
    const WAKE: Duration = Duration::from_millis(10);
    let _ = thread::Builder::new()
        .name("UV loop: svNetwork".to_owned())
        .spawn(move || {
            loop {
                let period = Instant::now();
                let busy = Duration::from_millis(server.number("fake_busy_percent").unwrap_or(0).min(100));
                while period.elapsed() < busy {
                    std::hint::black_box(period.elapsed());
                }
                while period.elapsed() < PERIOD {
                    thread::sleep(WAKE.min(PERIOD.saturating_sub(period.elapsed())));
                }
            }
        });
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

/// The server folder, which FXServer runs in.
fn server_folder() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
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

/// The commands of `fake-build.cfg` next to the executable, which stand for what a build does
/// whatever the server's cfg says.
fn build_cfg() -> Vec<Vec<String>> {
    let Some(file) = std::env::current_exe()
        .ok()
        .and_then(|program| Some(program.parent()?.join("fake-build.cfg")))
    else {
        return Vec::new();
    };
    std::fs::read_to_string(file)
        .map(|cfg| cfg.lines().flat_map(split_commands).collect())
        .unwrap_or_default()
}

/// The `local CONTRACT_VERSION = <n>` a bridge script reports when it is ready.
fn contract_version(script: &str) -> Option<u64> {
    constant(script, "CONTRACT_VERSION")
}

/// The number a bridge script gives one of its constants, as `local NAME = <n>`.
fn constant(script: &str, name: &str) -> Option<u64> {
    let declared = format!("local {name} = ");
    script
        .lines()
        .find_map(|line| line.strip_prefix(declared.as_str()))?
        .trim()
        .parse()
        .ok()
}

/// Only `+set` itself, so spelled, goes before the rest (`OptionTokenizer.h`): `+setr`, `+sets` and
/// `+SET` run in their place, as any other command.
fn is_set(command: &[String]) -> bool {
    command.first().is_some_and(|name| name == "set")
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
            "sys_k3jx7qmw2pza",
        ];

        assert_eq!(
            command_line(args.into_iter().map(String::from)),
            words(&[
                &["set", "a", "b c"],
                &["exec", "server.cfg"],
                &["ensure", "sys_k3jx7qmw2pza"]
            ])
        );
    }

    #[test]
    fn only_plus_set_itself_runs_before_the_rest_of_the_command_line() {
        let args = [
            "+set",
            "a",
            "1",
            "+setr",
            "b",
            "2",
            "+SET",
            "c",
            "3",
            "+exec",
            "server.cfg",
        ];

        let early: Vec<bool> = command_line(args.into_iter().map(String::from))
            .iter()
            .map(|command| is_set(command))
            .collect();

        assert_eq!(early, [true, false, false, false]);
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
