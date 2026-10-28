//! FXServer's structured traces on file descriptor 3: compact JSON objects written back to back
//! with no separator, the first one always `{"test":"test"}`.

use std::time::{Duration, Instant};

use observer_platform::Output;
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::{mpsc, watch};
use tracing::warn;

/// The largest object kept. FXServer's own traces are a few hundred bytes; a resource that prints
/// more than this through `PrintStructuredTrace` is not reporting health.
const MAX_OBJECT: usize = 1024 * 1024;
/// Traces of a type Observer does not know are kept for diagnostics, but `script_log` alone
/// repeats every `print` of every resource, so they are capped per window.
const UNKNOWN_PER_WINDOW: u32 = 30;
const UNKNOWN_WINDOW: Duration = Duration::from_mins(1);

#[derive(Debug, Clone, PartialEq)]
pub struct Trace {
    /// The spawn the trace came from.
    pub generation: u32,
    pub kind: TraceKind,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TraceKind {
    /// `{"test":"test"}`: FXServer checked that the descriptor is open, before its first trace.
    #[serde(skip)]
    Probe,
    /// A thread overran its tick budget (150 ms for `svMain` and `svNetwork`, 100 ms for `svSync`);
    /// reported once the tick ends.
    Hitch {
        thread: String,
        #[serde(rename = "time")]
        ms: u64,
    },
    /// The watchdog found a thread stuck for `seconds`; `stack` is its call stack.
    WatchdogBark {
        thread: String,
        #[serde(rename = "time")]
        seconds: u64,
        stack: String,
    },
    /// The watchdog gave up on a stuck thread and is aborting the process.
    WatchdogBite { thread: String },
    /// The game port is taken; FXServer exits right after.
    BindError { address: String },
    /// The Cfx.re reverse proxy URL of the server.
    NucleusConnected { url: String },
    /// A resource called `PrintStructuredTrace`. FXServer fills in `resource` itself, so a resource
    /// cannot speak for another.
    #[serde(rename = "script_structured_trace")]
    Script { resource: String, payload: Value },
    #[serde(skip)]
    Unknown(Value),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Dropped {
    pub oversized: u64,
    pub garbage_bytes: u64,
    pub unknown: u64,
}

/// Turns the bytes read from one spawn's fd 3 into traces. Traces stop coming out once a newer
/// spawn is current, so a process left over from an earlier generation cannot speak for the new
/// one.
pub struct TraceDecoder {
    generation: u32,
    current: watch::Receiver<u32>,
    pending: Vec<u8>,
    skipping: Option<Skip>,
    window: (Instant, u32),
    dropped: Dropped,
}

impl TraceDecoder {
    pub fn new(generation: u32, current: watch::Receiver<u32>) -> Self {
        Self {
            generation,
            current,
            pending: Vec::new(),
            skipping: None,
            window: (Instant::now(), 0),
            dropped: Dropped::default(),
        }
    }

    pub fn dropped(&self) -> Dropped {
        self.dropped
    }

    /// Feeds the next bytes and returns the traces they complete.
    pub fn push(&mut self, bytes: &[u8], now: Instant) -> Vec<Trace> {
        let mut traces = Vec::new();
        let bytes = match &mut self.skipping {
            None => bytes,
            Some(skip) => match skip.end_in(bytes) {
                Some(end) => {
                    self.skipping = None;
                    &bytes[end..]
                }
                None => return traces,
            },
        };
        self.pending.extend_from_slice(bytes);

        let mut start = 0;
        while let Some(offset) = self.pending[start..].iter().position(|&byte| byte == b'{') {
            self.dropped.garbage_bytes += offset as u64;
            start += offset;
            let mut values =
                serde_json::Deserializer::from_slice(&self.pending[start..]).into_iter::<Value>();
            match values.next() {
                Some(Ok(value)) => {
                    let length = values.byte_offset();
                    start += length;
                    if length > MAX_OBJECT {
                        self.drop_oversized();
                    } else if let Some(kind) = self.classify(value, now) {
                        traces.push(Trace {
                            generation: self.generation,
                            kind,
                        });
                    }
                }
                Some(Err(error)) if error.is_eof() => {
                    if self.pending.len() - start <= MAX_OBJECT {
                        break;
                    }
                    self.drop_oversized();
                    let mut skip = Skip::default();
                    if let Some(end) = skip.end_in(&self.pending[start..]) {
                        start += end;
                    } else {
                        self.skipping = Some(skip);
                        start = self.pending.len();
                    }
                }
                // Not an object after all: resynchronise on the next `{`.
                _ => {
                    self.dropped.garbage_bytes += 1;
                    start += 1;
                }
            }
        }
        if !self.pending[start..].contains(&b'{') {
            self.dropped.garbage_bytes += (self.pending.len() - start) as u64;
            start = self.pending.len();
        }
        self.pending.drain(..start);

        if *self.current.borrow() != self.generation {
            traces.clear();
        }
        traces
    }

    fn classify(&mut self, value: Value, now: Instant) -> Option<TraceKind> {
        if value.get("test").and_then(Value::as_str) == Some("test") {
            return Some(TraceKind::Probe);
        }
        if let Some(kind) = value
            .get("data")
            .and_then(|data| TraceKind::deserialize(data).ok())
        {
            return Some(kind);
        }

        let (started, count) = &mut self.window;
        if now.duration_since(*started) >= UNKNOWN_WINDOW {
            (*started, *count) = (now, 0);
        }
        if *count < UNKNOWN_PER_WINDOW {
            *count += 1;
            Some(TraceKind::Unknown(value))
        } else {
            self.dropped.unknown += 1;
            None
        }
    }

    fn drop_oversized(&mut self) {
        self.dropped.oversized += 1;
        warn!(
            generation = self.generation,
            limit = MAX_OBJECT,
            "dropped an oversized trace from fd 3"
        );
    }
}

/// Follows an oversized object to its end without keeping it, tracking strings so that braces
/// inside them do not count.
#[derive(Default)]
struct Skip {
    depth: u32,
    in_string: bool,
    escaped: bool,
}

impl Skip {
    /// Where in `bytes` the object ends, if it does.
    fn end_in(&mut self, bytes: &[u8]) -> Option<usize> {
        for (index, &byte) in bytes.iter().enumerate() {
            if self.in_string {
                match byte {
                    _ if self.escaped => self.escaped = false,
                    b'\\' => self.escaped = true,
                    b'"' => self.in_string = false,
                    _ => {}
                }
                continue;
            }
            match byte {
                b'"' => self.in_string = true,
                b'{' | b'[' => self.depth += 1,
                b'}' | b']' => {
                    self.depth = self.depth.saturating_sub(1);
                    if self.depth == 0 {
                        return Some(index + 1);
                    }
                }
                _ => {}
            }
        }
        None
    }
}

/// Decodes one spawn's fd 3 until every process holding it has closed it, and forwards its traces
/// while that spawn is current.
pub async fn read(mut output: Output, mut decoder: TraceDecoder, sink: mpsc::Sender<Trace>) {
    while let Some(bytes) = output.next().await {
        for trace in decoder.push(&bytes, Instant::now()) {
            // Keep draining even when nobody listens: a full pipe blocks FXServer's trace thread.
            let _ = sink.send(trace).await;
        }
    }
    let dropped = decoder.dropped();
    if dropped != Dropped::default() {
        warn!(
            generation = decoder.generation,
            oversized = dropped.oversized,
            garbage_bytes = dropped.garbage_bytes,
            unknown = dropped.unknown,
            "fd 3 closed; some traces were dropped"
        );
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    // Recorded from FXServer 35245 on Windows, with the lab probe resource running.
    const PROBE: &str = r#"{"test":"test"}"#;
    const HEARTBEAT: &str = r#"{"channel":"citizen-server-impl","data":{"payload":{"kind":"hb","type":"observer"},"resource":"observer","type":"script_structured_trace"},"file":"C:\\gl\\builds\\cfx-fivem-3\\code\\components\\citizen-server-impl\\src\\ServerResources.cpp","func":"operator ()","line":944}"#;
    const HITCH: &str = r#"{"channel":"citizen-server-impl","data":{"thread":"svMain","time":20002,"type":"hitch"},"file":"C:\\gl\\builds\\cfx-fivem-3\\code\\components\\citizen-server-impl\\src\\GameServer.cpp","func":"operator ()","line":226}"#;
    const NUCLEUS: &str = r#"{"channel":"citizen-server-impl","data":{"type":"nucleus_connected","url":"https://owner-abc123.users.cfx.re/"},"file":"C:\\gl\\builds\\cfx-fivem-3\\code\\components\\citizen-server-impl\\src\\ServerNucleusMock.cpp","func":"operator ()","line":145}"#;
    const SCRIPT_LOG: &str = r#"{"channel":"citizen-scripting-core","data":{"resource":"observer","text":"observer lab probe: script loaded","type":"script_log"},"file":"C:\\gl\\builds\\cfx-fivem-3\\code\\components\\citizen-scripting-core\\src\\ScriptHost.cpp","func":"ScriptTrace","line":370}"#;

    fn decoder(generation: u32) -> (watch::Sender<u32>, TraceDecoder) {
        let (current, receiver) = watch::channel(generation);
        (current, TraceDecoder::new(generation, receiver))
    }

    fn kinds(traces: Vec<Trace>) -> Vec<TraceKind> {
        traces.into_iter().map(|trace| trace.kind).collect()
    }

    fn heartbeat() -> TraceKind {
        TraceKind::Script {
            resource: "observer".to_owned(),
            payload: json!({ "kind": "hb", "type": "observer" }),
        }
    }

    fn native(data: &Value) -> String {
        json!({ "channel": "citizen-server-impl", "data": data, "file": "x.cpp", "func": "f", "line": 1 })
            .to_string()
    }

    #[test]
    fn concatenated_objects_come_out_in_order_and_tagged_with_their_generation() {
        let (_current, mut decoder) = decoder(7);
        let stream = [PROBE, HEARTBEAT, HITCH, NUCLEUS].concat();

        let traces = decoder.push(stream.as_bytes(), Instant::now());

        assert!(traces.iter().all(|trace| trace.generation == 7));
        assert_eq!(
            kinds(traces),
            [
                TraceKind::Probe,
                heartbeat(),
                TraceKind::Hitch {
                    thread: "svMain".to_owned(),
                    ms: 20002
                },
                TraceKind::NucleusConnected {
                    url: "https://owner-abc123.users.cfx.re/".to_owned()
                },
            ]
        );
    }

    #[test]
    fn an_object_split_across_reads_comes_out_once_complete() {
        let (_current, mut decoder) = decoder(1);
        let stream = [PROBE, HEARTBEAT].concat();
        let now = Instant::now();

        let mut traces = Vec::new();
        for chunk in stream.as_bytes().chunks(7) {
            traces.extend(decoder.push(chunk, now));
        }

        assert_eq!(kinds(traces), [TraceKind::Probe, heartbeat()]);
        assert_eq!(decoder.dropped(), Dropped::default());
    }

    #[test]
    fn every_native_type_is_surfaced() {
        let (_current, mut decoder) = decoder(1);
        let stream = [
            native(
                &json!({ "type": "watchdog_bark", "thread": "svMain", "time": 60, "stack": "a <- b <- root" }),
            ),
            native(&json!({ "type": "watchdog_bite", "thread": "svMain" })),
            native(&json!({ "type": "bind_error", "address": "127.0.0.1:30140" })),
        ]
        .concat();

        assert_eq!(
            kinds(decoder.push(stream.as_bytes(), Instant::now())),
            [
                TraceKind::WatchdogBark {
                    thread: "svMain".to_owned(),
                    seconds: 60,
                    stack: "a <- b <- root".to_owned()
                },
                TraceKind::WatchdogBite {
                    thread: "svMain".to_owned()
                },
                TraceKind::BindError {
                    address: "127.0.0.1:30140".to_owned()
                },
            ]
        );
    }

    #[test]
    fn other_types_are_kept_as_unknown_up_to_a_rate() {
        let (_current, mut decoder) = decoder(1);
        let now = Instant::now();
        let stream = SCRIPT_LOG.repeat(40);

        let traces = decoder.push(stream.as_bytes(), now);

        assert_eq!(traces.len(), 30);
        assert!(
            matches!(&traces[0].kind, TraceKind::Unknown(value) if value["data"]["type"] == "script_log")
        );
        assert_eq!(decoder.dropped().unknown, 10);
        assert_eq!(decoder.push(SCRIPT_LOG.as_bytes(), now + UNKNOWN_WINDOW).len(), 1);
    }

    #[test]
    fn a_known_type_with_unexpected_fields_is_kept_as_unknown() {
        let (_current, mut decoder) = decoder(1);
        let stream = native(&json!({ "type": "hitch", "thread": "svMain", "time": "slow" }));

        let traces = kinds(decoder.push(stream.as_bytes(), Instant::now()));

        assert!(matches!(&traces[..], [TraceKind::Unknown(_)]));
    }

    #[test]
    fn garbage_between_objects_is_skipped() {
        let (_current, mut decoder) = decoder(1);
        let stream = format!("\r\nnoise{PROBE}]]}}{{\"broken\" 1}}{HITCH}tail");

        let traces = kinds(decoder.push(stream.as_bytes(), Instant::now()));

        assert_eq!(traces.len(), 2);
        assert_eq!(traces[0], TraceKind::Probe);
        assert!(matches!(traces[1], TraceKind::Hitch { .. }));
        assert!(decoder.dropped().garbage_bytes > 0);
    }

    #[test]
    fn an_oversized_object_is_dropped_and_the_stream_resynchronises_after_it() {
        let (_current, mut decoder) = decoder(1);
        let now = Instant::now();
        // Braces inside the string must not end the object early.
        let filler = "x{}\\\"".repeat(MAX_OBJECT / 4);
        let oversized =
            format!(r#"{{"data":{{"type":"script_log","text":"{filler}","nested":{{"a":[1,{{"b":2}}]}}}}}}"#);
        let stream = [PROBE, &oversized, HEARTBEAT].concat();

        let mut traces = Vec::new();
        for chunk in stream.as_bytes().chunks(64 * 1024) {
            traces.extend(decoder.push(chunk, now));
        }

        assert_eq!(kinds(traces), [TraceKind::Probe, heartbeat()]);
        assert_eq!(decoder.dropped().oversized, 1);
    }

    #[test]
    fn an_oversized_object_arriving_whole_is_dropped_too() {
        let (_current, mut decoder) = decoder(1);
        let oversized = format!(r#"{{"text":"{}"}}"#, "x".repeat(MAX_OBJECT));
        let stream = [oversized.as_str(), HITCH].concat();

        let traces = kinds(decoder.push(stream.as_bytes(), Instant::now()));

        assert!(matches!(&traces[..], [TraceKind::Hitch { .. }]));
        assert_eq!(decoder.dropped().oversized, 1);
    }

    #[test]
    fn traces_from_an_older_generation_are_ignored() {
        let (current, mut decoder) = decoder(3);
        assert_eq!(decoder.push(PROBE.as_bytes(), Instant::now()).len(), 1);

        current.send(4).unwrap();

        assert_eq!(decoder.push(HEARTBEAT.as_bytes(), Instant::now()), []);
    }
}
