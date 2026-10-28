//! The `observer` resource Observer installs in every build: it turns Observer's commands into
//! server events, and reports the heartbeat, readiness, started resources and the JS heap back
//! through the structured trace channel. The resource lives in `resource/observer`.

use std::fs;
use std::io;
use std::path::Path;

use observer_domain::schedule::RestartSource;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::layout::Layout;
use crate::trace::TraceKind;

pub const NAME: &str = "observer";

/// The version of the server events the bridge raises; it must match `CONTRACT_VERSION` in
/// `server.lua`.
pub const CONTRACT_VERSION: u32 = 1;

const FILES: [(&str, &str); 3] = [
    (
        "fxmanifest.lua",
        include_str!("../../../resource/observer/fxmanifest.lua"),
    ),
    (
        "server.lua",
        include_str!("../../../resource/observer/server.lua"),
    ),
    ("heap.js", include_str!("../../../resource/observer/heap.js")),
];

/// Puts the bridge into the build's `system_resources`, where `+ensure observer` finds it. Files
/// whose content already matches are left alone, and anything else in the folder is removed, so
/// the folder holds exactly this version. Returns whether anything changed.
pub fn install(layout: &Layout) -> io::Result<bool> {
    let dir = layout.system_resources().join(NAME);
    fs::create_dir_all(&dir)?;
    let mut changed = false;
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        if !FILES.iter().any(|(name, _)| entry.file_name() == *name) {
            if entry.file_type()?.is_dir() {
                fs::remove_dir_all(entry.path())?;
            } else {
                fs::remove_file(entry.path())?;
            }
            changed = true;
        }
    }
    for (name, content) in FILES {
        let path = dir.join(name);
        if read_if_exists(&path)?.as_deref() != Some(content.as_bytes()) {
            fs::write(&path, content)?;
            changed = true;
        }
    }
    Ok(changed)
}

fn read_if_exists(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Why the server is stopping, from a closed list: it is the only thing Observer's `quit` line
/// carries, and it reaches resources in `observer:shuttingDown`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    Stop,
    Restart,
    Scheduled,
    Shutdown,
    CrashLoop,
}

impl StopReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stop => "stop",
            Self::Restart => "restart",
            Self::Scheduled => "scheduled",
            Self::Shutdown => "shutdown",
            Self::CrashLoop => "crash_loop",
        }
    }
}

/// The server events of the integration contract, raised as `observer:<event>` with this payload.
/// Times are Unix milliseconds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "event",
    content = "payload",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum BridgeEvent {
    /// A restart is coming: at each warning tier, or at once for the nearest one when a restart
    /// is planned or moved inside the tiers.
    RestartScheduled {
        at: i64,
        seconds_remaining: u64,
        reason: RestartSource,
    },
    /// A restart announced in this run will not happen.
    RestartSkipped {
        at: i64,
        by: String,
        reason: RestartSource,
    },
    /// A restart announced in this run moved to `at`. `reason` is the new restart's source.
    RestartRescheduled {
        at: i64,
        previous_at: i64,
        by: String,
        reason: RestartSource,
    },
    /// Raised again, with the time left, when a stop overtakes a restart before `quit`: the
    /// latest one says what follows.
    ShuttingDown {
        delay_ms: u64,
        reason: StopReason,
        by: String,
    },
}

impl BridgeEvent {
    pub fn name(&self) -> &'static str {
        match self {
            Self::RestartScheduled { .. } => "restartScheduled",
            Self::RestartSkipped { .. } => "restartSkipped",
            Self::RestartRescheduled { .. } => "restartRescheduled",
            Self::ShuttingDown { .. } => "shuttingDown",
        }
    }

    /// The argument of `observer_cmd`: the event's JSON in lowercase hex, which leaves no quote,
    /// semicolon or line break for the console parser to act on.
    pub fn encode(&self) -> String {
        let json = serde_json::to_vec(self).expect("strings and numbers always serialise");
        let mut hex = String::with_capacity(json.len() * 2);
        for byte in json {
            hex.push(char::from(HEX[usize::from(byte >> 4)]));
            hex.push(char::from(HEX[usize::from(byte & 0xf)]));
        }
        hex
    }
}

const HEX: &[u8; 16] = b"0123456789abcdef";

/// What the bridge reports, through `PrintStructuredTrace` from the `observer` resource.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum Report {
    /// Once per start, when the cfg has run.
    Ready {
        #[serde(default)]
        contract_version: Option<u32>,
    },
    /// Every three seconds while the main loop runs.
    #[serde(rename = "hb")]
    Heartbeat,
    /// The resources started now, after any change.
    Resources { started: Vec<String> },
    /// The memory of the Node runtime shared by JS resources, in bytes.
    Heap {
        heap_used: u64,
        heap_total: u64,
        #[serde(default)]
        rss: u64,
        #[serde(default)]
        external: u64,
    },
}

impl Report {
    /// The bridge's report in a trace, if the trace is one. FXServer names the resource that
    /// printed it, so another resource cannot pose as the bridge.
    pub fn from_trace(trace: &TraceKind) -> Option<Self> {
        let TraceKind::Script { resource, payload } = trace else {
            return None;
        };
        if resource != NAME || payload.get("type").and_then(Value::as_str) != Some(NAME) {
            return None;
        }
        Self::deserialize(payload).ok()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::layout::Flavour;

    fn decode(hex: &str) -> Value {
        let bytes: Vec<u8> = hex
            .as_bytes()
            .chunks(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[test]
    fn the_bridge_is_written_once_and_kept_as_is() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path(), Flavour::Windows);

        assert!(install(&layout).unwrap());
        let installed = layout.system_resources().join("observer");
        let manifest = fs::read_to_string(installed.join("fxmanifest.lua")).unwrap();
        assert!(manifest.contains("fx_version 'cerulean'"));
        assert!(manifest.contains("game 'common'"));
        assert!(!install(&layout).unwrap());
    }

    #[test]
    fn a_changed_or_extra_file_is_put_back_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path(), Flavour::Windows);
        install(&layout).unwrap();
        let installed = layout.system_resources().join("observer");
        fs::write(installed.join("server.lua"), "print('edited')").unwrap();
        fs::write(installed.join("extra.lua"), "print('extra')").unwrap();

        assert!(install(&layout).unwrap());

        assert_eq!(
            fs::read_to_string(installed.join("server.lua")).unwrap(),
            FILES[1].1
        );
        assert!(!installed.join("extra.lua").exists());
    }

    #[test]
    fn the_contract_version_matches_the_resource() {
        let lua = FILES[1].1;

        assert!(lua.contains(&format!("local CONTRACT_VERSION = {CONTRACT_VERSION}\n")));
    }

    #[test]
    fn the_resource_accepts_exactly_the_events_observer_can_send() {
        let lua = FILES[1].1;
        let events = [
            BridgeEvent::RestartScheduled {
                at: 0,
                seconds_remaining: 0,
                reason: RestartSource::Schedule,
            },
            BridgeEvent::RestartSkipped {
                at: 0,
                by: String::new(),
                reason: RestartSource::Schedule,
            },
            BridgeEvent::RestartRescheduled {
                at: 0,
                previous_at: 0,
                by: String::new(),
                reason: RestartSource::OneOff,
            },
            BridgeEvent::ShuttingDown {
                delay_ms: 0,
                reason: StopReason::Stop,
                by: String::new(),
            },
        ];

        for event in &events {
            assert!(lua.contains(&format!("    {} = true,\n", event.name())));
            assert_eq!(decode(&event.encode())["event"], event.name());
        }
        assert_eq!(lua.matches(" = true,\n").count(), events.len());
    }

    #[test]
    fn an_event_travels_as_lowercase_hex_json() {
        let event = BridgeEvent::ShuttingDown {
            delay_ms: 5000,
            reason: StopReason::Restart,
            by: "alice\";\r\n\\".to_owned(),
        };

        let hex = event.encode();

        assert!(
            hex.bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        );
        assert_eq!(
            decode(&hex),
            json!({
                "event": "shuttingDown",
                "payload": { "delayMs": 5000, "reason": "restart", "by": "alice\";\r\n\\" }
            })
        );
    }

    #[test]
    fn reports_come_only_from_the_bridge_resource() {
        let trace = |resource: &str, payload: Value| TraceKind::Script {
            resource: resource.to_owned(),
            payload,
        };

        assert_eq!(
            Report::from_trace(&trace("observer", json!({ "type": "observer", "kind": "hb" }))),
            Some(Report::Heartbeat)
        );
        assert_eq!(
            Report::from_trace(&trace(
                "observer",
                json!({ "type": "observer", "kind": "ready", "contractVersion": 1 })
            )),
            Some(Report::Ready {
                contract_version: Some(1)
            })
        );
        assert_eq!(
            Report::from_trace(&trace(
                "observer",
                json!({ "type": "observer", "kind": "heap", "heapUsed": 10, "heapTotal": 20, "rss": 30, "external": 1 })
            )),
            Some(Report::Heap {
                heap_used: 10,
                heap_total: 20,
                rss: 30,
                external: 1
            })
        );
        assert_eq!(
            Report::from_trace(&trace("mischief", json!({ "type": "observer", "kind": "hb" }))),
            None
        );
        assert_eq!(
            Report::from_trace(&trace("observer", json!({ "type": "other", "kind": "hb" }))),
            None
        );
    }
}
