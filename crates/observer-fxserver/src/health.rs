//! How Observer tells a running FXServer from a stuck one: its HTTP endpoints, answered off the
//! main thread, the main thread's tick count in `/perf/`, and the bridge's heartbeat, which only
//! comes while the main loop runs.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use http_body_util::{BodyExt, Empty, Limited};
use hyper::body::Bytes;
use hyper::header::{AUTHORIZATION, HOST, USER_AGENT};
use hyper::{Request, StatusCode};
use hyper_util::rt::TokioIo;
use serde::Serialize;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::stdin::{Credentials, PERF_USER};

const DYNAMIC_EVERY: Duration = Duration::from_secs(1);
const DYNAMIC_TIMEOUT: Duration = Duration::from_millis(1500);
const PERF_EVERY: Duration = Duration::from_secs(5);
const PERF_TIMEOUT: Duration = Duration::from_secs(3);
/// `/perf/` is a few kilobytes; anything much bigger is not FXServer.
const MAX_BODY: usize = 256 * 1024;

/// When a silent signal makes the server degraded, then hung. The defaults are txAdmin's.
#[derive(Debug, Clone, Copy)]
pub struct HealthLimits {
    /// No hang is declared this soon after the start: big servers take a while to load.
    pub boot_grace: Duration,
    pub heartbeat_late: Duration,
    pub heartbeat_fatal: Duration,
    pub http_late: Duration,
    pub http_fatal: Duration,
}

impl Default for HealthLimits {
    fn default() -> Self {
        Self {
            boot_grace: Duration::from_secs(45),
            heartbeat_late: Duration::from_secs(10),
            heartbeat_fatal: Duration::from_secs(60),
            http_late: Duration::from_secs(10),
            http_fatal: Duration::from_secs(180),
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
    http: Option<Instant>,
    /// The last `svMain` tick count, and whether it moved since the read before.
    ticks: Option<u64>,
    stalled: bool,
    unauthorized: bool,
    /// A watchdog bark since the last heartbeat.
    bark: bool,
}

impl Health {
    pub fn new(spawned: Instant) -> Self {
        Self {
            spawned,
            heartbeat: None,
            http: None,
            ticks: None,
            stalled: false,
            unauthorized: false,
            bark: false,
        }
    }

    pub fn heartbeat(&mut self, now: Instant) {
        self.heartbeat = Some(now);
        self.bark = false;
    }

    pub fn bark(&mut self) {
        self.bark = true;
    }

    pub fn probed(&mut self, probe: &Probe, now: Instant) {
        match probe {
            Probe::Dynamic(Ok(_)) => self.http = Some(now),
            Probe::Perf(Ok(ticks)) => {
                self.stalled = self.ticks == Some(*ticks);
                self.ticks = Some(*ticks);
                self.unauthorized = false;
            }
            Probe::Perf(Err(ProbeError::Unauthorized)) => self.unauthorized = true,
            // Silence is measured from the last answer, so a failure has nothing to add.
            Probe::Dynamic(Err(_)) | Probe::Perf(Err(_)) => {}
        }
    }

    pub fn ready(&self) -> bool {
        self.heartbeat.is_some() && self.http.is_some()
    }

    pub fn verdict(&self, now: Instant, limits: &HealthLimits) -> Verdict {
        let silent = |last: Option<Instant>| now.saturating_duration_since(last.unwrap_or(self.spawned));
        let heartbeat = silent(self.heartbeat);
        let http = silent(self.http);
        if now.saturating_duration_since(self.spawned) >= limits.boot_grace {
            if heartbeat >= limits.heartbeat_fatal {
                return Verdict::Hung(HangCause::NoHeartbeat);
            }
            if http >= limits.http_fatal {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Probe {
    Dynamic(Result<Players, ProbeError>),
    /// The `svMain` tick count.
    Perf(Result<u64, ProbeError>),
}

/// What `/dynamic.json` says about the players.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Players {
    pub clients: u64,
    pub max_clients: Option<u64>,
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

/// Polls `/dynamic.json` every second and `/perf/` every five, each with one request in flight,
/// until `cancel`. Results go to `sink` tagged with `generation`.
pub(crate) fn start(
    address: SocketAddr,
    credentials: &Credentials,
    generation: u32,
    sink: mpsc::Sender<(u32, Probe)>,
    cancel: &CancellationToken,
) -> [JoinHandle<()>; 2] {
    let authorization = format!(
        "Basic {}",
        base64(format!("{PERF_USER}:{}", credentials.perf_password).as_bytes())
    );
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
    let perf = tokio::spawn(poll(cancel.clone(), PERF_EVERY, move || {
        let sink = sink.clone();
        let authorization = authorization.clone();
        async move {
            let result = timed(PERF_TIMEOUT, perf(address, &authorization)).await;
            let _ = sink.send((generation, Probe::Perf(result))).await;
        }
    }));
    [dynamic, perf]
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
    let (status, body) = get(address, "/dynamic.json", None).await?;
    if status != StatusCode::OK {
        return Err(ProbeError::Status(status.as_u16()));
    }
    parse_dynamic(&body)
}

async fn perf(address: SocketAddr, authorization: &str) -> Result<u64, ProbeError> {
    let (status, body) = get(address, "/perf/", Some(authorization)).await?;
    match status {
        StatusCode::OK => main_ticks(&String::from_utf8_lossy(&body)),
        StatusCode::UNAUTHORIZED => Err(ProbeError::Unauthorized),
        status => Err(ProbeError::Status(status.as_u16())),
    }
}

/// One request on a fresh connection, as a plain client: FXServer blocks requests that carry
/// browser or proxy headers (`Upgrade-Insecure-Requests`, `Via`) under `sv_requestParanoia`.
async fn get(
    address: SocketAddr,
    path: &str,
    authorization: Option<&str>,
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
    if let Some(authorization) = authorization {
        request = request.header(AUTHORIZATION, authorization);
    }
    let request = request
        .body(Empty::<Bytes>::new())
        .map_err(|error| ProbeError::Invalid(error.to_string()))?;
    let exchange = async move {
        let response = sender.send_request(request).await.map_err(http)?;
        let status = response.status();
        let body = Limited::new(response.into_body(), MAX_BODY)
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
    let value: serde_json::Value =
        serde_json::from_str(&text).map_err(|_| invalid("/dynamic.json is not JSON"))?;
    let clients = value
        .get("clients")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| invalid("/dynamic.json has no player count"))?;
    // FXServer prints it as a string, as convars are.
    let max_clients = value.get("sv_maxclients").and_then(|max| match max {
        serde_json::Value::String(text) => text.parse().ok(),
        other => other.as_u64(),
    });
    Ok(Players { clients, max_clients })
}

fn main_ticks(text: &str) -> Result<u64, ProbeError> {
    text.lines()
        .find_map(|line| line.strip_prefix("tickTime_count{name=\"svMain\"} "))
        .and_then(|count| count.trim().parse().ok())
        .ok_or_else(|| ProbeError::Invalid("/perf/ has no svMain tick count".to_owned()))
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

    #[test]
    fn a_stalled_tick_count_an_unauthorized_probe_and_a_bark_degrade_the_server() {
        let start = Instant::now();
        let mut health = ready(start);
        health.probed(&Probe::Perf(Ok(388)), start + secs(3));
        health.probed(&Probe::Perf(Ok(388)), start + secs(8));
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

        health.probed(&Probe::Perf(Ok(400)), start + secs(9));
        health.heartbeat(start + secs(9));
        assert_eq!(health.verdict(start + secs(9), &LIMITS), Verdict::Healthy);
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
    fn the_main_tick_count_is_read_from_perf() {
        let perf = "# HELP tickTime Time spent on server ticks\n\
                    # TYPE tickTime histogram\n\
                    tickTime_count{name=\"svNetwork\"} 9000\n\
                    tickTime_count{name=\"svMain\"} 388\n\
                    tickTime_bucket{name=\"svMain\",le=\"0.07000000000000001\"} 388\n";

        assert_eq!(main_ticks(perf), Ok(388));
        assert!(main_ticks("tickTime_count{name=\"svSync\"} 1\n").is_err());
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
        let address = SocketAddr::from(([127, 0, 0, 1], 41929));

        assert!(matches!(dynamic(address).await, Err(ProbeError::Unreachable(_))));
    }
}
