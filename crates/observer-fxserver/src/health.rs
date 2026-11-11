//! How Observer tells a running FXServer from a stuck one: its HTTP endpoints, answered off the
//! main thread, the main thread's tick count in `/perf/`, and the bridge's heartbeat, which only
//! comes while the main loop runs. Until the cfg has run, the resources FXServer starts stand in for
//! the heartbeat. The same probes feed the statistics: `/perf/` and `/players.json` are read in the
//! middle of each five-second slot, once for both.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use http_body_util::{BodyExt, Empty, Limited};
use hyper::body::Bytes;
use hyper::header::{AUTHORIZATION, HOST, USER_AGENT};
use hyper::{Request, StatusCode};
use hyper_util::rt::TokioIo;
use observer_domain::launch::LaunchSettings;
use observer_domain::metrics;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::perf::TickReport;
use crate::stdin::{Credentials, PERF_USER};

const DYNAMIC_EVERY: Duration = Duration::from_secs(1);
const DYNAMIC_TIMEOUT: Duration = Duration::from_millis(1500);
/// Fired at the middle of a slot, they have answered before its end.
const SLOT_PROBE_TIMEOUT: Duration = Duration::from_secs(2);
/// `/info.json` is asked until it answers once: the build does not change while the server runs.
const INFO_EVERY: Duration = Duration::from_secs(5);
const INFO_TIMEOUT: Duration = Duration::from_secs(3);
/// `/perf/` is a few kilobytes; anything much bigger is not FXServer.
const MAX_BODY: usize = 256 * 1024;
/// A full server's `/players.json` lists every player's identifiers: about 1 KiB each.
const MAX_PLAYERS_BODY: usize = 4 * 1024 * 1024;
const PLAYERS_TOKEN_HEADER: &str = "x-players-token";

/// When a silent signal makes the server degraded, then hung. The boot grace and the thresholds
/// that only degrade it are txAdmin's; when it is hung is the owner's choice, in the launch
/// settings, which [`HealthLimits::under`] applies.
#[derive(Debug, Clone, Copy)]
pub struct HealthLimits {
    /// No hang is declared this soon after the start: big servers take a while to load.
    pub boot_grace: Duration,
    pub heartbeat_late: Duration,
    /// The longest the heartbeat may stay silent before a hang, whatever the launch settings
    /// allow: [`NEVER`] by default, so their delay decides. Tests shorten it, as the settings take
    /// a minute at least.
    pub heartbeat_fatal: Duration,
    pub http_late: Duration,
    /// The same for HTTP.
    pub http_fatal: Duration,
}

/// A silence that never comes: no signal stays quiet this long.
pub const NEVER: Duration = Duration::MAX;

impl Default for HealthLimits {
    fn default() -> Self {
        Self {
            boot_grace: Duration::from_secs(45),
            heartbeat_late: Duration::from_secs(10),
            heartbeat_fatal: NEVER,
            http_late: Duration::from_secs(10),
            http_fatal: NEVER,
        }
    }
}

impl HealthLimits {
    /// These limits under the owner's launch settings: a server whose heartbeat or HTTP stays
    /// silent for their hang timeout is hung, unless these limits say so sooner, and with hang
    /// detection off it never is. Whether it is degraded does not change, nor does a crash restart
    /// it any less: that is the process exiting, which no limit weighs.
    #[must_use]
    pub fn under(self, settings: &LaunchSettings) -> Self {
        let delay = Duration::from_secs(u64::from(settings.hang_timeout_seconds));
        let fatal = |limit: Duration| {
            if settings.hang_detection {
                limit.min(delay)
            } else {
                NEVER
            }
        };
        Self {
            heartbeat_fatal: fatal(self.heartbeat_fatal),
            http_fatal: fatal(self.http_fatal),
            ..self
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Problem {
    /// `/dynamic.json` has not answered properly for a while.
    HttpLate,
    /// The bridge's heartbeat is late: the main loop is slow or stuck.
    HeartbeatLate,
    /// The main thread's tick count stopped between two reads of `/perf/`.
    MainThreadStalled,
    /// `/perf/` refuses Observer's credentials: something reset them after the start.
    ProbeUnauthorized,
    /// FXServer's watchdog reported a stuck thread.
    WatchdogBark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HangCause {
    NoHeartbeat,
    NoHttp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Not ready yet: HTTP has not answered or no heartbeat came.
    Starting,
    Healthy,
    Degraded(Vec<Problem>),
    Hung(HangCause),
}

/// What one run's signals said last.
#[derive(Debug)]
pub struct Health {
    spawned: Instant,
    heartbeat: Option<Instant>,
    /// When the bridge first reported in, which it does once the cfg has run.
    cfg_ran: Option<Instant>,
    /// When FXServer last started a resource of its cfg.
    progress: Option<Instant>,
    http: Option<Instant>,
    /// The last `svMain` tick count, and whether it moved since the read before.
    ticks: Option<u64>,
    stalled: bool,
    unauthorized: bool,
    /// `/perf/` probes refused in a row.
    refusals: u32,
    /// A watchdog bark since the last heartbeat.
    bark: bool,
    /// When hang detection was last turned on, or given another delay, during this run.
    armed: Option<Instant>,
    /// Until when a start is borne with however silent: someone said it needs longer.
    extended: Option<Instant>,
}

impl Health {
    pub fn new(spawned: Instant) -> Self {
        Self {
            spawned,
            heartbeat: None,
            cfg_ran: None,
            progress: None,
            http: None,
            ticks: None,
            stalled: false,
            unauthorized: false,
            refusals: 0,
            bark: false,
            armed: None,
            extended: None,
        }
    }

    /// Hang detection was turned on, or given another delay, at `now`. A silence counts toward a
    /// hang from then at the earliest: an owner who turns it back on during a long first start,
    /// or shortens the delay, means from now on, not a kill at once of a server already silent
    /// longer than that. Whether the server is late, and so degraded, still counts from its last
    /// answer.
    pub fn arm(&mut self, now: Instant) {
        self.armed = Some(now);
    }

    pub fn heartbeat(&mut self, now: Instant) {
        self.cfg_ran.get_or_insert(now);
        self.heartbeat = Some(now);
        self.bark = false;
    }

    /// FXServer started a resource at `at`. It runs its cfg before any thread of the bridge, so no
    /// heartbeat comes during a start however long it takes: a resource started is what shows the
    /// start going on.
    pub fn progressed(&mut self, at: Instant) {
        self.progress = self.progress.max(Some(at));
    }

    /// Whether the bridge has reported in since the spawn: the cfg has run.
    pub fn cfg_ran(&self) -> bool {
        self.cfg_ran.is_some()
    }

    /// The start is not taken for stuck before `until`, whatever its silence.
    pub fn extend(&mut self, until: Instant) {
        self.extended = self.extended.max(Some(until));
    }

    /// When the start is taken for stuck, should FXServer start no resource until then. None once
    /// the cfg has run, which leaves the watch to the heartbeat, and with no limit to a silence.
    pub fn stuck_at(&self, limits: &HealthLimits) -> Option<Instant> {
        if self.cfg_ran.is_some() {
            return None;
        }
        let silent_since = self
            .progress
            .max(self.armed)
            .map_or(self.spawned, |last| last.max(self.spawned));
        let stuck = silent_since.checked_add(limits.heartbeat_fatal)?;
        let grace = self.spawned.checked_add(limits.boot_grace)?;
        Some(stuck.max(grace).max(self.extended.unwrap_or(stuck)))
    }

    pub fn bark(&mut self) {
        self.bark = true;
    }

    pub fn probed(&mut self, probe: &Probe, now: Instant) {
        match probe {
            Probe::Dynamic(Ok(_)) => self.http = Some(now),
            Probe::Perf(Ok(report)) => {
                // A report without svMain says nothing of the main loop.
                let Some(ticks) = report.main_ticks() else {
                    return;
                };
                self.stalled = self.ticks == Some(ticks);
                self.ticks = Some(ticks);
                self.unauthorized = false;
                self.refusals = 0;
            }
            // FXServer reads its credentials from stdin once it is up, so the first probe can be
            // refused. A refusal means something once it has accepted them, or when the next probe
            // is refused too: it reads them well within a probe period, so they were changed.
            Probe::Perf(Err(ProbeError::Unauthorized)) => {
                self.refusals += 1;
                self.unauthorized = self.ticks.is_some() || self.refusals > 1;
            }
            // Silence is measured from the last answer, so a failure has nothing to add.
            Probe::Dynamic(Err(_)) | Probe::Perf(Err(_)) | Probe::Players(_) | Probe::Info(_) => {}
        }
    }

    pub fn ready(&self) -> bool {
        self.heartbeat.is_some() && self.http.is_some()
    }

    pub fn verdict(&self, now: Instant, limits: &HealthLimits) -> Verdict {
        let silent = |last: Option<Instant>| now.saturating_duration_since(last.unwrap_or(self.spawned));
        let heartbeat = silent(self.heartbeat);
        let http = silent(self.http);
        let armed = self
            .armed
            .map_or(Duration::MAX, |armed| now.saturating_duration_since(armed));
        if now.saturating_duration_since(self.spawned) >= limits.boot_grace {
            // A start is stuck once it starts no resource for that long, not once it took that long.
            let stuck = match self.cfg_ran {
                Some(_) => heartbeat.min(armed) >= limits.heartbeat_fatal,
                None => self.stuck_at(limits).is_some_and(|at| now >= at),
            };
            if stuck {
                return Verdict::Hung(HangCause::NoHeartbeat);
            }
            // The cfg opens the HTTP endpoint when it comes to that line: HTTP is owed once it ran.
            let unanswered = self
                .cfg_ran
                .map(|ran| silent(Some(self.http.map_or(ran, |http| http.max(ran)))));
            if unanswered.is_some_and(|unanswered| unanswered.min(armed) >= limits.http_fatal) {
                return Verdict::Hung(HangCause::NoHttp);
            }
        }
        if !self.ready() {
            return Verdict::Starting;
        }
        let problems: Vec<Problem> = [
            (http >= limits.http_late, Problem::HttpLate),
            (heartbeat >= limits.heartbeat_late, Problem::HeartbeatLate),
            (self.stalled, Problem::MainThreadStalled),
            (self.unauthorized, Problem::ProbeUnauthorized),
            (self.bark, Problem::WatchdogBark),
        ]
        .into_iter()
        .filter_map(|(present, problem)| present.then_some(problem))
        .collect();
        if problems.is_empty() {
            Verdict::Healthy
        } else {
            Verdict::Degraded(problems)
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Probe {
    Dynamic(Result<Players, ProbeError>),
    /// The tick time histograms.
    Perf(Result<TickReport, ProbeError>),
    Players(Result<PlayerList, ProbeError>),
    /// The build and the enforced game build, from `/info.json`.
    Info(Result<Info, ProbeError>),
}

/// What `/dynamic.json` says about the players.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Players {
    pub clients: u64,
    pub max_clients: Option<u64>,
}

/// The players `/players.json` lists, as `(net id, ping in ms)`, and what each one shows, which
/// the players service checks the bridge's reports against. It stays in memory: endpoints and
/// `ip:` identifiers are never read.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PlayerList {
    /// The ping is `None` for a client FXServer has no network peer for: it lists it with `-1`.
    pub players: Vec<(u32, Option<u32>)>,
    pub shown: Vec<Listed>,
    /// FXServer did not honour the token and answered the public list, whose ids and pings are 0.
    pub anonymous: bool,
}

/// A listed player's identifiers other than `ip:`, as FXServer gives them, and name.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Listed {
    pub id: u32,
    pub identifiers: Vec<String>,
    pub name: Option<String>,
}

impl PlayerList {
    pub fn anonymous() -> Self {
        Self {
            anonymous: true,
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProbeError {
    #[error("cannot reach FXServer: {0}")]
    Unreachable(String),
    #[error("FXServer did not answer in time")]
    TimedOut,
    #[error("FXServer answered {0}")]
    Status(u16),
    #[error("FXServer refused Observer's credentials")]
    Unauthorized,
    #[error("FXServer's answer is not what it should be: {0}")]
    Invalid(String),
}

/// Polls `/dynamic.json` every second, `/perf/` and `/players.json` in the middle of every slot,
/// each with one request in flight, until `cancel`, and `/info.json` until it answers. Results go
/// to `sink` tagged with `generation`.
pub(crate) fn start(
    address: SocketAddr,
    credentials: &Credentials,
    generation: u32,
    sink: mpsc::Sender<(u32, Probe)>,
    cancel: &CancellationToken,
) -> [JoinHandle<()>; 4] {
    let authorization = format!(
        "Basic {}",
        base64(format!("{PERF_USER}:{}", credentials.perf_password).as_bytes())
    );
    let token = credentials.players_token.clone();
    let dynamic = tokio::spawn(poll(cancel.clone(), DYNAMIC_EVERY, {
        let sink = sink.clone();
        move || {
            let sink = sink.clone();
            async move {
                let result = timed(DYNAMIC_TIMEOUT, dynamic(address)).await;
                let _ = sink.send((generation, Probe::Dynamic(result))).await;
            }
        }
    }));
    let perf = tokio::spawn(every_mid_slot(cancel.clone(), {
        let sink = sink.clone();
        move || {
            let sink = sink.clone();
            let authorization = authorization.clone();
            async move {
                let result = timed(SLOT_PROBE_TIMEOUT, perf(address, &authorization)).await;
                let _ = sink.send((generation, Probe::Perf(result))).await;
            }
        }
    }));
    let players = tokio::spawn(every_mid_slot(cancel.clone(), {
        let sink = sink.clone();
        move || {
            let sink = sink.clone();
            let token = token.clone();
            async move {
                let result = timed(SLOT_PROBE_TIMEOUT, players(address, &token)).await;
                let _ = sink.send((generation, Probe::Players(result))).await;
            }
        }
    }));
    let info = tokio::spawn({
        let cancel = cancel.clone();
        async move {
            loop {
                let result = tokio::select! {
                    () = cancel.cancelled() => return,
                    result = timed(INFO_TIMEOUT, info(address)) => result,
                };
                let answered = result.is_ok();
                let _ = sink.send((generation, Probe::Info(result))).await;
                if answered {
                    return;
                }
                tokio::select! {
                    () = cancel.cancelled() => return,
                    () = tokio::time::sleep(INFO_EVERY) => {}
                }
            }
        }
    });
    [dynamic, perf, players, info]
}

/// Runs `probe` at the middle of every slot of the wall clock. The next one is found from the
/// clock each time, so a clock that steps realigns at once, and a slow answer skips a slot rather
/// than fire late.
async fn every_mid_slot<F, R>(cancel: CancellationToken, mut probe: F)
where
    F: FnMut() -> R,
    R: Future<Output = ()>,
{
    loop {
        let now = jiff::Timestamp::now().as_millisecond();
        let wait = u64::try_from(metrics::next_mid_slot(now) - now).unwrap_or(0);
        tokio::select! {
            () = cancel.cancelled() => return,
            () = tokio::time::sleep(Duration::from_millis(wait)) => {}
        }
        tokio::select! {
            () = cancel.cancelled() => return,
            () = probe() => {}
        }
    }
}

async fn poll<F, R>(cancel: CancellationToken, every: Duration, mut probe: F)
where
    F: FnMut() -> R,
    R: Future<Output = ()>,
{
    let mut next = tokio::time::Instant::now();
    loop {
        tokio::select! {
            () = cancel.cancelled() => return,
            () = tokio::time::sleep_until(next) => {}
        }
        next += every;
        tokio::select! {
            () = cancel.cancelled() => return,
            () = probe() => {}
        }
        // After a slow answer, start again from now rather than firing to catch up.
        next = next.max(tokio::time::Instant::now());
    }
}

async fn timed<T>(
    limit: Duration,
    request: impl Future<Output = Result<T, ProbeError>>,
) -> Result<T, ProbeError> {
    tokio::time::timeout(limit, request)
        .await
        .unwrap_or(Err(ProbeError::TimedOut))
}

async fn dynamic(address: SocketAddr) -> Result<Players, ProbeError> {
    let (status, body) = get(address, "/dynamic.json", &[], MAX_BODY).await?;
    if status != StatusCode::OK {
        return Err(ProbeError::Status(status.as_u16()));
    }
    parse_dynamic(&body)
}

/// What `/info.json` says that Observer keeps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Info {
    pub build: u32,
    /// `sv_enforceGameBuild`, when the server sets it.
    pub game_build: Option<u32>,
    /// `sv_projectName` as [`project_name`] cleans it, when the server sets one.
    pub name: Option<String>,
}

/// The longest project name kept: the panel shows it on one line of its rail.
const MAX_PROJECT_NAME: usize = 64;

/// Only the build, the enforced game build and the project name are read: the rest of `/info.json`
/// lists the server's convars, one of them derived from the licence key.
async fn info(address: SocketAddr) -> Result<Info, ProbeError> {
    let (status, body) = get(address, "/info.json", &[], MAX_BODY).await?;
    if status != StatusCode::OK {
        return Err(ProbeError::Status(status.as_u16()));
    }
    parse_info(&body)
}

async fn perf(address: SocketAddr, authorization: &str) -> Result<TickReport, ProbeError> {
    let headers = [(AUTHORIZATION.as_str(), authorization)];
    let (status, body) = get(address, "/perf/", &headers, MAX_BODY).await?;
    match status {
        StatusCode::OK => TickReport::parse(&String::from_utf8_lossy(&body))
            .ok_or_else(|| ProbeError::Invalid("/perf/ has no tick time histogram".to_owned())),
        StatusCode::UNAUTHORIZED => Err(ProbeError::Unauthorized),
        status => Err(ProbeError::Status(status.as_u16())),
    }
}

/// The token goes in a header: in the query string it would end up in logs.
async fn players(address: SocketAddr, token: &str) -> Result<PlayerList, ProbeError> {
    let headers = [(PLAYERS_TOKEN_HEADER, token)];
    let (status, body) = get(address, "/players.json", &headers, MAX_PLAYERS_BODY).await?;
    if status != StatusCode::OK {
        return Err(ProbeError::Status(status.as_u16()));
    }
    parse_players(&body)
}

/// One request on a fresh connection, as a plain client: FXServer blocks requests that carry
/// browser or proxy headers (`Upgrade-Insecure-Requests`, `Via`) under `sv_requestParanoia`.
async fn get(
    address: SocketAddr,
    path: &str,
    headers: &[(&str, &str)],
    limit: usize,
) -> Result<(StatusCode, Bytes), ProbeError> {
    let http = |error: hyper::Error| ProbeError::Unreachable(error.to_string());
    let stream = TcpStream::connect(address)
        .await
        .map_err(|error| ProbeError::Unreachable(error.to_string()))?;
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .map_err(http)?;
    let mut request = Request::get(path)
        .header(HOST, address.to_string())
        .header(USER_AGENT, "observer");
    for &(name, value) in headers {
        request = request.header(name, value);
    }
    let request = request
        .body(Empty::<Bytes>::new())
        .map_err(|error| ProbeError::Invalid(error.to_string()))?;
    let exchange = async move {
        let response = sender.send_request(request).await.map_err(http)?;
        let status = response.status();
        let body = Limited::new(response.into_body(), limit)
            .collect()
            .await
            .map_err(|error| ProbeError::Invalid(error.to_string()))?
            .to_bytes();
        // Dropping the sender lets the connection close once the exchange is over.
        Ok((status, body))
    };
    let (result, _) = tokio::join!(exchange, connection);
    result
}

/// txAdmin's check: a JSON object with a non-negative integer `clients`, never an HTML page.
fn parse_dynamic(body: &[u8]) -> Result<Players, ProbeError> {
    let invalid = |reason: &str| ProbeError::Invalid(reason.to_owned());
    let text = String::from_utf8_lossy(body);
    if text.trim().is_empty() || text.to_ascii_lowercase().contains("<html") {
        return Err(invalid("not the JSON of /dynamic.json"));
    }
    let value: Value = serde_json::from_str(&text).map_err(|_| invalid("/dynamic.json is not JSON"))?;
    let clients = value
        .get("clients")
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid("/dynamic.json has no player count"))?;
    // FXServer prints it as a string, as convars are.
    let max_clients = value.get("sv_maxclients").and_then(|max| match max {
        Value::String(text) => text.parse().ok(),
        other => other.as_u64(),
    });
    Ok(Players { clients, max_clients })
}

/// `"server": "FXServer-master SERVER v1.0.0.35245 win32"`: the last part of the version; and
/// `vars.sv_enforceGameBuild`, which FXServer lists among the convars it shows.
fn parse_info(body: &[u8]) -> Result<Info, ProbeError> {
    let invalid = || ProbeError::Invalid("/info.json names no build".to_owned());
    let value: Value = serde_json::from_slice(body).map_err(|_| invalid())?;
    let server = value.get("server").and_then(Value::as_str).ok_or_else(invalid)?;
    let build = server
        .split_whitespace()
        .find_map(|word| {
            let version = word.strip_prefix('v')?;
            version.contains('.').then(|| version.rsplit('.').next())?
        })
        .and_then(|build| build.parse().ok())
        .ok_or_else(invalid)?;
    let game_build = value
        .get("vars")
        .and_then(|vars| vars.get("sv_enforceGameBuild"))
        .and_then(Value::as_str)
        .and_then(|game_build| game_build.trim().parse().ok());
    let name = value
        .get("vars")
        .and_then(|vars| vars.get("sv_projectName"))
        .and_then(Value::as_str)
        .and_then(project_name);
    Ok(Info {
        build,
        game_build,
        name,
    })
}

/// A project name without FXServer's colour codes (`^1`) or control characters, trimmed, at most
/// [`MAX_PROJECT_NAME`] characters; none when nothing is left.
pub(crate) fn project_name(raw: &str) -> Option<String> {
    let mut name = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(char) = chars.next() {
        if char == '^' && chars.peek().is_some_and(char::is_ascii_digit) {
            chars.next();
        } else if !char.is_control() {
            name.push(char);
        }
    }
    let kept: String = name.trim().chars().take(MAX_PROJECT_NAME).collect();
    let kept = kept.trim_end();
    (!kept.is_empty()).then(|| kept.to_owned())
}

/// A JSON array of players, each with a numeric `id` and `ping`. The public list FXServer answers
/// without the token has every id at 0.
fn parse_players(body: &[u8]) -> Result<PlayerList, ProbeError> {
    #[derive(Deserialize)]
    struct Entry {
        id: u32,
        ping: i64,
        #[serde(default)]
        identifiers: Vec<Value>,
        #[serde(default)]
        name: Option<Value>,
    }
    let entries: Vec<Entry> = serde_json::from_slice(body)
        .map_err(|_| ProbeError::Invalid("/players.json is not a list of players".to_owned()))?;
    if entries.iter().any(|entry| entry.id == 0) {
        return Ok(PlayerList::anonymous());
    }
    Ok(PlayerList {
        players: entries
            .iter()
            .map(|entry| (entry.id, u32::try_from(entry.ping).ok()))
            .collect(),
        shown: entries
            .into_iter()
            .map(|entry| Listed {
                id: entry.id,
                identifiers: entry
                    .identifiers
                    .iter()
                    .filter_map(Value::as_str)
                    .filter(|identifier| !is_address(identifier))
                    .map(str::to_owned)
                    .collect(),
                name: entry.name.as_ref().and_then(Value::as_str).map(str::to_owned),
            })
            .collect(),
        anonymous: false,
    })
}

/// An `ip:` identifier, whatever its case: never kept, not even in memory.
pub fn is_address(identifier: &str) -> bool {
    identifier
        .get(..3)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("ip:"))
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for group in bytes.chunks(3) {
        let bits = group.iter().enumerate().fold(0u32, |bits, (index, &byte)| {
            bits | u32::from(byte) << (16 - 8 * index)
        });
        for index in 0..4 {
            if index <= group.len() {
                out.push(char::from(ALPHABET[(bits >> (18 - 6 * index) & 0x3f) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMITS: HealthLimits = HealthLimits {
        boot_grace: Duration::from_secs(45),
        heartbeat_late: Duration::from_secs(10),
        heartbeat_fatal: Duration::from_secs(60),
        http_late: Duration::from_secs(10),
        http_fatal: Duration::from_secs(180),
    };

    fn secs(seconds: u64) -> Duration {
        Duration::from_secs(seconds)
    }

    fn http_ok() -> Probe {
        Probe::Dynamic(Ok(Players {
            clients: 0,
            max_clients: Some(8),
        }))
    }

    fn ready(start: Instant) -> Health {
        let mut health = Health::new(start);
        health.heartbeat(start + secs(2));
        health.probed(&http_ok(), start + secs(2));
        health
    }

    #[test]
    fn the_server_is_ready_once_http_answers_and_a_heartbeat_came() {
        let start = Instant::now();
        let mut health = Health::new(start);
        assert_eq!(health.verdict(start + secs(1), &LIMITS), Verdict::Starting);

        health.probed(&http_ok(), start + secs(1));
        assert_eq!(health.verdict(start + secs(2), &LIMITS), Verdict::Starting);
        health.heartbeat(start + secs(3));

        assert_eq!(health.verdict(start + secs(3), &LIMITS), Verdict::Healthy);
    }

    #[test]
    fn nothing_is_hung_during_the_boot_grace() {
        let start = Instant::now();
        let health = Health::new(start);

        assert_eq!(health.verdict(start + secs(44), &LIMITS), Verdict::Starting);
        assert_eq!(
            health.verdict(start + secs(60), &LIMITS),
            Verdict::Hung(HangCause::NoHeartbeat)
        );
    }

    #[test]
    fn a_start_is_hung_once_it_starts_no_resource_for_the_delay_however_long_it_takes() {
        let limits = HealthLimits::default().under(&launch(true, 360));
        let start = Instant::now();
        let mut health = Health::new(start);
        // Twenty minutes of resources, the slowest one five minutes on a cold cache.
        for second in [15, 120, 420, 700, 1000, 1200] {
            health.progressed(start + secs(second));
            assert_eq!(
                health.verdict(start + secs(second + 299), &limits),
                Verdict::Starting,
                "{second}"
            );
        }

        // Told late, an earlier one takes nothing back: the latest counts.
        health.progressed(start + secs(900));
        assert_eq!(health.verdict(start + secs(1559), &limits), Verdict::Starting);
        assert_eq!(
            health.verdict(start + secs(1560), &limits),
            Verdict::Hung(HangCause::NoHeartbeat)
        );
    }

    #[test]
    fn a_start_given_more_time_is_not_hung_before_that_runs_out() {
        let limits = HealthLimits::default().under(&launch(true, 360));
        let start = Instant::now();
        let mut health = Health::new(start);
        health.progressed(start + secs(100));
        assert_eq!(health.stuck_at(&limits), Some(start + secs(460)));

        health.extend(start + secs(1360));
        health.extend(start + secs(900));
        assert_eq!(health.stuck_at(&limits), Some(start + secs(1360)));
        assert_eq!(health.verdict(start + secs(1359), &limits), Verdict::Starting);
        assert_eq!(
            health.verdict(start + secs(1360), &limits),
            Verdict::Hung(HangCause::NoHeartbeat)
        );

        // A resource started late in the time given: its own silence counts from there.
        health.progressed(start + secs(1300));
        assert_eq!(health.stuck_at(&limits), Some(start + secs(1660)));
        assert_eq!(health.verdict(start + secs(1659), &limits), Verdict::Starting);
    }

    #[test]
    fn when_a_start_is_stuck_is_said_only_while_its_cfg_runs_under_hang_detection() {
        let start = Instant::now();
        let mut health = Health::new(start);
        let off = HealthLimits::default().under(&launch(false, 360));
        let short = HealthLimits::default().under(&launch(true, 60));
        let limits = HealthLimits::default().under(&launch(true, 360));

        assert_eq!(health.stuck_at(&off), None);
        assert_eq!(health.stuck_at(&limits), Some(start + secs(360)));
        assert_eq!(
            health.stuck_at(&HealthLimits {
                boot_grace: secs(90),
                ..short
            }),
            Some(start + secs(90)),
            "never during the grace of a start"
        );
        health.arm(start + secs(200));
        assert_eq!(health.stuck_at(&limits), Some(start + secs(560)));

        health.heartbeat(start + secs(210));
        assert_eq!(health.stuck_at(&limits), None, "the heartbeat watches from here");
    }

    #[test]
    fn once_the_cfg_has_run_only_the_heartbeat_tells_the_main_loop_runs() {
        let limits = HealthLimits::default().under(&launch(true, 360));
        let start = Instant::now();
        let mut health = Health::new(start);
        health.probed(&http_ok(), start + secs(10));
        health.progressed(start + secs(300));
        health.heartbeat(start + secs(600));
        assert!(health.cfg_ran());
        for second in (600..1200).step_by(3) {
            health.probed(&http_ok(), start + secs(second));
        }
        // A resource restarted from the console, which the main loop runs before it sticks.
        health.progressed(start + secs(900));

        assert_eq!(
            health.verdict(start + secs(959), &limits),
            Verdict::Degraded(vec![Problem::HeartbeatLate])
        );
        assert_eq!(
            health.verdict(start + secs(960), &limits),
            Verdict::Hung(HangCause::NoHeartbeat)
        );
    }

    #[test]
    fn http_is_owed_from_the_end_of_the_cfg_which_opens_its_endpoint() {
        let limits = HealthLimits::default().under(&launch(true, 360));
        let start = Instant::now();
        let mut health = Health::new(start);
        // A long start whose cfg adds the endpoint last: HTTP never answered meanwhile.
        for second in (60..=1200).step_by(60) {
            health.progressed(start + secs(second));
            assert_eq!(health.verdict(start + secs(second), &limits), Verdict::Starting);
        }
        health.heartbeat(start + secs(1200));
        for second in (1200..1600).step_by(3) {
            health.heartbeat(start + secs(second));
        }

        assert_eq!(health.verdict(start + secs(1201), &limits), Verdict::Starting);
        assert_eq!(health.verdict(start + secs(1559), &limits), Verdict::Starting);
        assert_eq!(
            health.verdict(start + secs(1560), &limits),
            Verdict::Hung(HangCause::NoHttp)
        );
    }

    #[test]
    fn a_silent_heartbeat_degrades_then_hangs_the_server() {
        let start = Instant::now();
        let mut health = ready(start);
        health.probed(&http_ok(), start + secs(70));

        assert_eq!(
            health.verdict(start + secs(13), &LIMITS),
            Verdict::Degraded(vec![Problem::HeartbeatLate])
        );
        assert_eq!(
            health.verdict(start + secs(62), &LIMITS),
            Verdict::Hung(HangCause::NoHeartbeat)
        );
    }

    #[test]
    fn http_that_stops_answering_degrades_then_hangs_the_server() {
        let start = Instant::now();
        let mut health = ready(start);
        for second in (10..200).step_by(3) {
            health.heartbeat(start + secs(second));
        }

        assert_eq!(
            health.verdict(start + secs(15), &LIMITS),
            Verdict::Degraded(vec![Problem::HttpLate])
        );
        assert_eq!(
            health.verdict(start + secs(182), &LIMITS),
            Verdict::Hung(HangCause::NoHttp)
        );
    }

    fn launch(hang_detection: bool, hang_timeout_seconds: u32) -> LaunchSettings {
        LaunchSettings {
            data_path: "/srv/fivem/server-data".to_owned(),
            cfg_file: "server.cfg".to_owned(),
            onesync: observer_domain::launch::OneSync::On,
            auto_start: false,
            txadmin_events: false,
            extra_arguments: Vec::new(),
            hang_detection,
            hang_timeout_seconds,
            stop_hold_seconds: 60,
        }
    }

    #[test]
    fn the_launch_settings_choose_how_long_a_silent_server_is_left_before_it_is_hung() {
        let limits = HealthLimits::default().under(&launch(true, 120));
        let start = Instant::now();
        let starting = Health::new(start);
        let mut silent_http = ready(start);
        for second in (10..200).step_by(3) {
            silent_http.heartbeat(start + secs(second));
        }

        assert_eq!(starting.verdict(start + secs(119), &limits), Verdict::Starting);
        assert_eq!(
            starting.verdict(start + secs(120), &limits),
            Verdict::Hung(HangCause::NoHeartbeat)
        );
        // Its last answer came 2 s after the start.
        assert_eq!(
            silent_http.verdict(start + secs(121), &limits),
            Verdict::Degraded(vec![Problem::HttpLate])
        );
        assert_eq!(
            silent_http.verdict(start + secs(122), &limits),
            Verdict::Hung(HangCause::NoHttp)
        );
        // Six minutes by default: what the defaults alone used to hang is still starting.
        let defaults = HealthLimits::default().under(&launch(true, 360));
        assert_eq!(starting.verdict(start + secs(359), &defaults), Verdict::Starting);
        assert_eq!(
            starting.verdict(start + secs(360), &defaults),
            Verdict::Hung(HangCause::NoHeartbeat)
        );
    }

    #[test]
    fn with_hang_detection_off_a_silent_server_is_never_hung() {
        let limits = LIMITS.under(&launch(false, 60));
        let start = Instant::now();
        let starting = Health::new(start);
        let stuck = ready(start);
        let a_day = secs(24 * 60 * 60);

        assert_eq!(starting.verdict(start + a_day, &limits), Verdict::Starting);
        assert_eq!(
            stuck.verdict(start + a_day, &limits),
            Verdict::Degraded(vec![Problem::HttpLate, Problem::HeartbeatLate])
        );
        assert_eq!(limits.boot_grace, LIMITS.boot_grace);
        assert_eq!(limits.heartbeat_late, LIMITS.heartbeat_late);
    }

    #[test]
    fn shorter_limits_than_the_launch_settings_win() {
        let limits = LIMITS.under(&launch(true, 360));
        let start = Instant::now();

        assert_eq!(limits.heartbeat_fatal, secs(60));
        assert_eq!(limits.http_fatal, secs(180));
        assert_eq!(
            Health::new(start).verdict(start + secs(60), &limits),
            Verdict::Hung(HangCause::NoHeartbeat)
        );
        assert_eq!(LIMITS.under(&launch(true, 120)).http_fatal, secs(120));
    }

    #[test]
    fn a_silence_counts_toward_a_hang_from_when_detection_was_armed_at_the_earliest() {
        let start = Instant::now();
        let six = HealthLimits::default().under(&launch(true, 360));
        let five = HealthLimits::default().under(&launch(true, 300));
        // A first start silent for twelve minutes, with detection turned back on at the tenth.
        let mut starting = Health::new(start);
        starting.arm(start + secs(600));

        assert_eq!(starting.verdict(start + secs(720), &six), Verdict::Starting);
        assert_eq!(starting.verdict(start + secs(959), &six), Verdict::Starting);
        assert_eq!(
            starting.verdict(start + secs(960), &six),
            Verdict::Hung(HangCause::NoHeartbeat)
        );
        // A ready server whose main loop froze at the second, its delay lowered to five minutes
        // at the seventh: still degraded, hung five minutes after the change.
        let mut frozen = ready(start);
        for second in (10..1000).step_by(3) {
            frozen.probed(&http_ok(), start + secs(second));
        }
        frozen.arm(start + secs(420));
        assert_eq!(
            frozen.verdict(start + secs(421), &five),
            Verdict::Degraded(vec![Problem::HeartbeatLate])
        );
        assert_eq!(
            frozen.verdict(start + secs(720), &five),
            Verdict::Hung(HangCause::NoHeartbeat)
        );
        // An answer after the change counts as before: the silence runs from the later of the two.
        let mut answered = ready(start);
        answered.arm(start + secs(60));
        for second in (60..=120).step_by(3) {
            answered.probed(&http_ok(), start + secs(second));
        }
        answered.heartbeat(start + secs(120));
        assert_eq!(
            answered.verdict(start + secs(479), &six),
            Verdict::Degraded(vec![Problem::HttpLate, Problem::HeartbeatLate])
        );
        assert_eq!(
            answered.verdict(start + secs(480), &six),
            Verdict::Hung(HangCause::NoHeartbeat)
        );
    }

    /// A `/perf/` answer whose svMain has ticked `count` times.
    fn perf(count: u64) -> Probe {
        let text = format!(
            "tickTime_count{{name=\"svMain\"}} {count}\ntickTime_sum{{name=\"svMain\"}} 0\n\
             tickTime_bucket{{name=\"svMain\",le=\"0.05\"}} {count}\ntickTime_bucket{{name=\"svMain\",le=\"+Inf\"}} {count}\n"
        );
        Probe::Perf(Ok(TickReport::parse(&text).unwrap()))
    }

    #[test]
    fn a_stalled_tick_count_an_unauthorized_probe_and_a_bark_degrade_the_server() {
        let start = Instant::now();
        let mut health = ready(start);
        health.probed(&perf(388), start + secs(3));
        health.probed(&perf(388), start + secs(8));
        health.probed(&Probe::Perf(Err(ProbeError::Unauthorized)), start + secs(8));
        health.bark();

        assert_eq!(
            health.verdict(start + secs(8), &LIMITS),
            Verdict::Degraded(vec![
                Problem::MainThreadStalled,
                Problem::ProbeUnauthorized,
                Problem::WatchdogBark
            ])
        );

        health.probed(&perf(400), start + secs(9));
        health.heartbeat(start + secs(9));
        assert_eq!(health.verdict(start + secs(9), &LIMITS), Verdict::Healthy);
    }

    #[test]
    fn a_refused_probe_before_fxserver_read_its_credentials_is_not_a_problem() {
        let start = Instant::now();
        let mut health = ready(start);
        health.probed(&Probe::Perf(Err(ProbeError::Unauthorized)), start + secs(1));
        assert_eq!(health.verdict(start + secs(1), &LIMITS), Verdict::Healthy);

        health.probed(&perf(12), start + secs(5));
        health.probed(&Probe::Perf(Err(ProbeError::Unauthorized)), start + secs(10));
        assert_eq!(
            health.verdict(start + secs(10), &LIMITS),
            Verdict::Degraded(vec![Problem::ProbeUnauthorized])
        );
    }

    #[test]
    fn credentials_changed_before_any_accepted_probe_show_once_two_probes_in_a_row_are_refused() {
        let start = Instant::now();
        let mut health = ready(start);
        health.probed(&Probe::Perf(Err(ProbeError::Unauthorized)), start + secs(5));
        assert_eq!(health.verdict(start + secs(5), &LIMITS), Verdict::Healthy);

        health.probed(&Probe::Perf(Err(ProbeError::Unauthorized)), start + secs(10));
        assert_eq!(
            health.verdict(start + secs(10), &LIMITS),
            Verdict::Degraded(vec![Problem::ProbeUnauthorized])
        );
    }

    #[test]
    fn dynamic_json_must_carry_a_player_count() {
        assert_eq!(
            parse_dynamic(br#"{"clients":0,"gametype":"","hostname":"Observer lab","iv":"311736622","mapname":"","sv_maxclients":"8"}"#),
            Ok(Players {
                clients: 0,
                max_clients: Some(8)
            })
        );
        for body in [
            &b""[..],
            b"<html><body>proxy</body></html>",
            b"{\"clients\":-1}",
            b"[]",
            b"not json",
        ] {
            assert!(
                parse_dynamic(body).is_err(),
                "{:?}",
                String::from_utf8_lossy(body)
            );
        }
    }

    #[test]
    fn a_perf_answer_without_svmain_says_nothing_of_the_main_loop() {
        let start = Instant::now();
        let mut health = ready(start);
        health.probed(&perf(388), start + secs(3));
        let sync_only = "tickTime_count{name=\"svSync\"} 1\ntickTime_sum{name=\"svSync\"} 0\n\
                         tickTime_bucket{name=\"svSync\",le=\"0.008\"} 1\ntickTime_bucket{name=\"svSync\",le=\"+Inf\"} 1\n";
        let report = TickReport::parse(sync_only).unwrap();

        health.probed(&Probe::Perf(Ok(report.clone())), start + secs(8));
        health.probed(&Probe::Perf(Ok(report)), start + secs(13));
        health.heartbeat(start + secs(13));
        health.probed(&http_ok(), start + secs(13));

        assert_eq!(health.verdict(start + secs(13), &LIMITS), Verdict::Healthy);
    }

    #[test]
    fn the_player_list_keeps_identifiers_and_names_but_never_addresses() {
        let body = br#"[{"endpoint":"127.0.0.1","id":3,"identifiers":["license:abc","IP:127.0.0.1","discord:1",4],"name":"Ada","ping":42},
                        {"endpoint":"10.0.0.2","id":7,"identifiers":["ip:10.0.0.2"],"name":"Grace","ping":120}]"#;

        let list = parse_players(body).unwrap();

        assert_eq!(list.players, [(3, Some(42)), (7, Some(120))]);
        assert_eq!(
            list.shown,
            [
                Listed {
                    id: 3,
                    identifiers: vec!["license:abc".to_owned(), "discord:1".to_owned()],
                    name: Some("Ada".to_owned()),
                },
                Listed {
                    id: 7,
                    identifiers: Vec::new(),
                    name: Some("Grace".to_owned()),
                },
            ]
        );
        assert!(!format!("{list:?}").contains("127.0.0.1"));
        assert!(!format!("{list:?}").contains("10.0.0.2"));
        assert_eq!(parse_players(b"[]"), Ok(PlayerList::default()));
    }

    #[test]
    fn a_player_without_a_network_peer_stays_in_the_list_without_a_ping() {
        let body = br#"[{"id":3,"name":"Ada","ping":42},{"id":4,"name":"Grace","ping":-1}]"#;

        assert_eq!(parse_players(body).unwrap().players, [(3, Some(42)), (4, None)]);
    }

    #[test]
    fn the_public_list_without_ids_is_anonymous() {
        let body = br#"[{"endpoint":"","id":0,"identifiers":[],"name":"Ada","ping":0},{"id":0,"ping":0}]"#;

        assert_eq!(parse_players(body), Ok(PlayerList::anonymous()));
    }

    #[test]
    fn a_player_list_that_is_not_a_list_of_numbered_players_is_invalid() {
        for body in [
            &b"<html><body>Not found</body></html>"[..],
            br#"{"players":[]}"#,
            br#"[{"id":"3","ping":42}]"#,
            br#"[{"id":3}]"#,
            br#"[{"id":-1,"ping":2}]"#,
            b"",
        ] {
            assert!(
                matches!(parse_players(body), Err(ProbeError::Invalid(_))),
                "{}",
                String::from_utf8_lossy(body)
            );
        }
    }

    #[tokio::test]
    async fn an_answer_past_the_size_limit_is_refused() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut head = [0; 1024];
            let _ = stream.read(&mut head).unwrap();
            let body = format!("[{}]", vec![r#"{"id":1,"ping":1}"#; 300_000].join(","));
            let response = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}", body.len());
            let _ = stream.write_all(response.as_bytes());
        });

        let result = players(address, "token").await;

        assert!(matches!(result, Err(ProbeError::Invalid(_))), "{result:?}");
    }

    #[test]
    fn the_build_is_the_last_part_of_the_version_fxserver_reports() {
        let info = br#"{"enhancedHostSupport":true,"resources":["hardcap","observer"],"server":"FXServer-master SERVER v1.0.0.35245 win32","vars":{"sv_maxClients":"8"},"version":311736622}"#;
        let linux = br#"{"server":"FXServer-master SERVER v1.0.0.37872 linux"}"#;

        assert_eq!(parse_info(info).map(|info| info.build), Ok(35245));
        assert_eq!(parse_info(linux).map(|info| info.build), Ok(37872));
        for invalid in [&br#"{"server":"FXServer"}"#[..], br#"{"version":1}"#, b"<html>"] {
            assert!(parse_info(invalid).is_err());
        }
    }

    #[test]
    fn the_game_build_is_the_one_the_server_enforces() {
        let enforced = br#"{"server":"FXServer-master SERVER v1.0.0.35245 win32","vars":{"sv_enforceGameBuild":"3095"}}"#;
        let default = br#"{"server":"FXServer-master SERVER v1.0.0.35245 win32","vars":{}}"#;

        assert_eq!(parse_info(enforced).unwrap().game_build, Some(3095));
        assert_eq!(parse_info(default).unwrap().game_build, None);
    }

    #[test]
    fn the_project_name_loses_its_colour_codes_and_control_characters() {
        let named = br#"{"server":"FXServer-master SERVER v1.0.0.35245 win32","vars":{"sv_projectName":"^1Sensity ^7| RP\u0007 "}}"#;
        let blank =
            br#"{"server":"FXServer-master SERVER v1.0.0.35245 win32","vars":{"sv_projectName":"^3 ^0"}}"#;
        let long = format!(
            r#"{{"server":"FXServer-master SERVER v1.0.0.35245 win32","vars":{{"sv_projectName":"{}"}}}}"#,
            "é".repeat(80)
        );

        assert_eq!(parse_info(named).unwrap().name.as_deref(), Some("Sensity | RP"));
        assert_eq!(parse_info(blank).unwrap().name, None);
        assert_eq!(parse_info(long.as_bytes()).unwrap().name, Some("é".repeat(64)));
    }

    #[test]
    fn base64_matches_the_standard_alphabet_and_padding() {
        assert_eq!(base64(b"observer:secret"), "b2JzZXJ2ZXI6c2VjcmV0");
        assert_eq!(base64(b"ab"), "YWI=");
        assert_eq!(base64(b"a"), "YQ==");
        assert_eq!(base64(b""), "");
    }

    #[tokio::test]
    async fn a_port_nobody_listens_on_is_unreachable() {
        let address = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();

        assert!(matches!(dynamic(address).await, Err(ProbeError::Unreachable(_))));
    }
}
