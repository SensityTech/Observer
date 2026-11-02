//! The bridge resource Observer installs in every build: it turns Observer's commands into
//! server events, and reports the heartbeat, readiness, started resources, the JS heap and the
//! players back through the structured trace channel. The resource lives in `resource/observer`;
//! in a build it takes a name drawn for each start of FXServer.

use std::fs;
use std::io;

use observer_domain::schedule::RestartSource;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::layout::Layout;
use crate::stdin::MAX_BRIDGE_JSON;
use crate::trace::TraceKind;

/// The server convar that holds the bridge's name, for resources to find it. Set with `+set`,
/// it stays out of `/info.json` and never reaches clients.
pub const NAME_CONVAR: &str = "observer_bridge";

/// What every drawn name starts with: nothing that says Observer.
const NAME_PREFIX: &str = "sys_";
/// 60 random bits after the prefix.
const NAME_RANDOM_CHARS: usize = 12;
const NAME_ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
/// The bridge's name before it was drawn at each start, which builds may still hold.
pub const FIXED_NAME: &str = "observer";

/// What the bridge's reports carry as `type`.
pub(crate) const REPORT_TYPE: &str = "observer";

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

/// Draws the bridge's resource name for one start of FXServer, so that no script can target the
/// bridge by a name known before the start.
pub fn draw_name() -> io::Result<String> {
    let mut random = [0; NAME_RANDOM_CHARS];
    getrandom::fill(&mut random).map_err(io::Error::other)?;
    let mut name = String::from(NAME_PREFIX);
    // 256 is a multiple of 32: every character is as likely as the others.
    name.extend(
        random
            .iter()
            .map(|byte| char::from(NAME_ALPHABET[usize::from(byte % 32)])),
    );
    Ok(name)
}

/// Whether a folder of `system_resources` named `name` is a bridge Observer wrote, under a drawn
/// name or the fixed one it had before.
pub fn is_folder(name: &str) -> bool {
    name == FIXED_NAME
        || name.strip_prefix(NAME_PREFIX).is_some_and(|random| {
            random.len() == NAME_RANDOM_CHARS && random.bytes().all(|byte| NAME_ALPHABET.contains(&byte))
        })
}

/// Puts the bridge into the build's `system_resources` as `name`, where `+ensure <name>` finds it.
/// The bridges of earlier starts are removed first, so that this one is the only one a command
/// can start, and the folder holds exactly this version.
pub fn install(layout: &Layout, name: &str) -> io::Result<()> {
    let system = layout.system_resources();
    fs::create_dir_all(&system)?;
    for entry in fs::read_dir(&system)? {
        let entry = entry?;
        if entry.file_name().to_str().is_some_and(is_folder) && entry.file_type()?.is_dir() {
            fs::remove_dir_all(entry.path())?;
        }
    }
    let dir = system.join(name);
    fs::create_dir(&dir)?;
    for (file, content) in FILES {
        fs::write(dir.join(file), content)?;
    }
    Ok(())
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
    /// Once per run, when the files of its start are hashed: what changed since the start before.
    StartChanges {
        at: i64,
        #[serde(skip_serializing_if = "Option::is_none")]
        previous_at: Option<i64>,
        /// The first snapshot of the server, which compares with nothing.
        baseline: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        build: Option<NumberMoved>,
        #[serde(skip_serializing_if = "Option::is_none")]
        game_build: Option<NumberMoved>,
        counts: StartCounts,
        resources: Vec<StartResource>,
        /// Resources left out because their name could break a console command.
        skipped: u32,
        /// Resources left out to keep the event within what the console takes.
        truncated: bool,
    },
}

/// A number before and after.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct NumberMoved {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<u32>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartCounts {
    pub added: u32,
    pub removed: u32,
    pub updated: u32,
    pub same_version: u32,
    pub renamed: u32,
    pub moved: u32,
    pub did_not_start: u32,
    /// Resources whose files were not all hashed: left out of the list.
    pub pending: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StartChange {
    Added,
    Removed,
    Updated,
    SameVersion,
    Renamed,
    Moved,
    DidNotStart,
}

/// A resource of `observer:startChanges`. Its name and versions are safe to put in a console
/// command: one that is not is never sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartResource {
    pub name: String,
    pub change: StartChange,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    /// A version that could break a console command, left out; the panel still shows it.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub from_hidden: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub to_hidden: bool,
}

/// What `observer:startChanges` says before its resources are listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartHead {
    pub at: i64,
    pub previous_at: Option<i64>,
    pub baseline: bool,
    pub build: Option<NumberMoved>,
    pub game_build: Option<NumberMoved>,
    pub counts: StartCounts,
}

/// `observer:startChanges` with as many of `resources` as fit the console's line, in the order
/// given: added, updated and changed under the same version first is the caller's to sort. A
/// name outside `[A-Za-z0-9_.-]{1,64}` is counted in `skipped`, never listed; a version outside
/// `[A-Za-z0-9_.+~-]{1,32}` is left out and marked hidden. The encoded event is measured, never a
/// count of entries.
pub fn start_changes(
    head: StartHead,
    resources: impl IntoIterator<Item = (String, StartChange, Option<String>, Option<String>)>,
) -> BridgeEvent {
    let mut listed = Vec::new();
    let mut skipped = 0;
    for (name, change, from, to) in resources {
        if !safe(&name, 64, b"_.-") {
            skipped += 1;
            continue;
        }
        let version = |version: Option<String>| match version {
            Some(version) if safe(&version, 32, b"_.+~-") => (Some(version), false),
            Some(_) => (None, true),
            None => (None, false),
        };
        let ((from, from_hidden), (to, to_hidden)) = (version(from), version(to));
        listed.push(StartResource {
            name,
            change,
            from,
            to,
            from_hidden,
            to_hidden,
        });
    }
    let event = |kept: usize| BridgeEvent::StartChanges {
        at: head.at,
        previous_at: head.previous_at,
        baseline: head.baseline,
        build: head.build,
        game_build: head.game_build,
        counts: head.counts,
        resources: listed[..kept].to_vec(),
        skipped,
        truncated: kept < listed.len(),
    };
    let fits = |kept: usize| serde_json::to_vec(&event(kept)).is_ok_and(|json| json.len() <= MAX_BRIDGE_JSON);
    // The event only grows with each resource kept: the longest list that fits is found by halves.
    let (mut low, mut high) = (0, listed.len());
    while low < high {
        let middle = (low + high).div_ceil(2);
        if fits(middle) {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    event(low)
}

/// Whether `text` is 1 to `max` bytes of ASCII letters, digits and `extra`.
fn safe(text: &str, max: usize, extra: &[u8]) -> bool {
    (1..=max).contains(&text.len())
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || extra.contains(&byte))
}

impl BridgeEvent {
    pub fn name(&self) -> &'static str {
        match self {
            Self::RestartScheduled { .. } => "restartScheduled",
            Self::RestartSkipped { .. } => "restartSkipped",
            Self::RestartRescheduled { .. } => "restartRescheduled",
            Self::ShuttingDown { .. } => "shuttingDown",
            Self::StartChanges { .. } => "startChanges",
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

/// What the bridge reports, through `PrintStructuredTrace`.
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
    /// The resources started now, after any change, and the starts and stops since the last
    /// report once the cfg has run, as `[name, "start" | "stop"]`, past 64 only counted.
    Resources {
        started: Vec<String>,
        #[serde(default)]
        events: Vec<(String, String)>,
        #[serde(default)]
        dropped: u32,
    },
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
    /// printed it, and `bridge` is the name drawn for this start: no resource can take it ahead
    /// and pose as the bridge.
    pub fn from_trace(trace: &TraceKind, bridge: &str) -> Option<Self> {
        let TraceKind::Script { resource, payload } = trace else {
            return None;
        };
        if resource != bridge || payload.get("type").and_then(Value::as_str) != Some(REPORT_TYPE) {
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

    /// The contract as it ships, next to the bridge, and the example resource that follows it.
    const CONTRACT: &str = include_str!("../../../resource/CONTRACT.md");
    const EXAMPLE: &str = include_str!("../../../resource/observer_example/server.lua");

    fn every_event() -> [BridgeEvent; 5] {
        [
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
            BridgeEvent::StartChanges {
                at: 0,
                previous_at: Some(0),
                baseline: false,
                build: Some(NumberMoved {
                    from: Some(0),
                    to: Some(0),
                }),
                game_build: Some(NumberMoved {
                    from: Some(0),
                    to: Some(0),
                }),
                counts: StartCounts::default(),
                resources: Vec::new(),
                skipped: 0,
                truncated: false,
            },
        ]
    }

    fn head() -> StartHead {
        StartHead {
            at: 1_793_000_000_000,
            previous_at: Some(1_792_999_000_000),
            baseline: false,
            build: Some(NumberMoved {
                from: Some(25770),
                to: Some(26100),
            }),
            game_build: None,
            counts: StartCounts {
                updated: 1,
                ..StartCounts::default()
            },
        }
    }

    fn resources(event: &BridgeEvent) -> (&[StartResource], u32, bool) {
        let BridgeEvent::StartChanges {
            resources,
            skipped,
            truncated,
            ..
        } = event
        else {
            panic!("not a startChanges event: {event:?}");
        };
        (resources, *skipped, *truncated)
    }

    #[test]
    fn start_changes_list_what_changed_with_versions_safe_for_a_console_command() {
        let event = start_changes(
            head(),
            [
                (
                    "qb-core".to_owned(),
                    StartChange::Updated,
                    Some("1.2.0".to_owned()),
                    Some("1.3.0".to_owned()),
                ),
                ("evil;quit".to_owned(), StartChange::Added, None, None),
                (
                    "shop".to_owned(),
                    StartChange::SameVersion,
                    Some("1.0 \"; quit\r\n".to_owned()),
                    Some("1.0".to_owned()),
                ),
            ],
        );

        assert_eq!(
            decode(&event.encode()),
            json!({
                "event": "startChanges",
                "payload": {
                    "at": 1_793_000_000_000_i64,
                    "previousAt": 1_792_999_000_000_i64,
                    "baseline": false,
                    "build": { "from": 25770, "to": 26100 },
                    "counts": {
                        "added": 0, "removed": 0, "updated": 1, "sameVersion": 0, "renamed": 0,
                        "moved": 0, "didNotStart": 0, "pending": 0
                    },
                    "resources": [
                        { "name": "qb-core", "change": "updated", "from": "1.2.0", "to": "1.3.0" },
                        { "name": "shop", "change": "sameVersion", "to": "1.0", "fromHidden": true }
                    ],
                    "skipped": 1,
                    "truncated": false
                }
            })
        );
    }

    #[test]
    fn start_changes_stay_within_what_the_console_takes() {
        let many = (0..1000).map(|index| {
            (
                format!("{index:0>64}"),
                StartChange::Updated,
                Some("v".repeat(32)),
                Some("w".repeat(32)),
            )
        });

        let event = start_changes(head(), many);

        let (listed, skipped, truncated) = resources(&event);
        assert!(truncated);
        assert_eq!(skipped, 0);
        assert_ne!(listed.len(), 0);
        let json = serde_json::to_vec(&event).unwrap();
        assert!(json.len() <= MAX_BRIDGE_JSON, "{} bytes", json.len());
        assert!(crate::stdin::allowed(&format!("observer_cmd {}", event.encode())));
        let one_more = start_changes(
            head(),
            (0..=listed.len()).map(|index| {
                (
                    format!("{index:0>64}"),
                    StartChange::Updated,
                    Some("v".repeat(32)),
                    Some("w".repeat(32)),
                )
            }),
        );
        assert_eq!(resources(&one_more).0.len(), listed.len());
    }

    #[test]
    fn names_and_versions_past_their_length_are_left_out() {
        let event = start_changes(
            head(),
            [
                ("n".repeat(65), StartChange::Added, None, None),
                (
                    "ok".to_owned(),
                    StartChange::Updated,
                    Some("1".repeat(33)),
                    Some(String::new()),
                ),
            ],
        );

        let (listed, skipped, _) = resources(&event);
        assert_eq!(skipped, 1);
        assert!(listed[0].from_hidden && listed[0].to_hidden);
        assert_eq!((listed[0].from.as_deref(), listed[0].to.as_deref()), (None, None));
    }

    fn decode(hex: &str) -> Value {
        let bytes: Vec<u8> = hex
            .as_bytes()
            .chunks(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[test]
    fn each_start_draws_a_name_that_says_nothing_of_observer() {
        let names: Vec<String> = (0..64).map(|_| draw_name().unwrap()).collect();

        for name in &names {
            assert!(is_folder(name), "{name}");
            assert!(!name.contains("observer"), "{name}");
            assert!(safe(name, 64, b"_"), "{name}");
            assert_eq!(names.iter().filter(|other| *other == name).count(), 1, "{name}");
        }
    }

    #[test]
    fn only_drawn_names_and_the_old_fixed_one_are_bridge_folders() {
        for name in ["observer", "sys_abcdefghijkl", "sys_234567zzzzzz"] {
            assert!(is_folder(name), "{name}");
        }
        for name in [
            "monitor",
            "chat",
            "Observer",
            "sys_",
            "sys_abcdefghijk",
            "sys_abcdefghijklm",
            "sys_abcdefghijk1",
            "sys_ABCDEFGHIJKL",
            "xys_abcdefghijkl",
        ] {
            assert!(!is_folder(name), "{name}");
        }
    }

    #[test]
    fn the_bridge_is_written_under_the_name_drawn_for_the_start() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path(), Flavour::Windows);
        let name = draw_name().unwrap();

        install(&layout, &name).unwrap();

        let installed = layout.system_resources().join(&name);
        for (file, content) in FILES {
            assert_eq!(fs::read_to_string(installed.join(file)).unwrap(), content);
        }
        let manifest = FILES[0].1;
        assert!(manifest.contains("fx_version 'cerulean'"));
        assert!(manifest.contains("game 'common'"));
        assert!(!manifest.contains("name '"), "the manifest names the bridge");
    }

    #[test]
    fn the_bridges_of_earlier_starts_are_removed_and_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::new(dir.path(), Flavour::Windows);
        let system = layout.system_resources();
        let earlier = draw_name().unwrap();
        install(&layout, &earlier).unwrap();
        fs::write(system.join(&earlier).join("extra.lua"), "print('extra')").unwrap();
        for kept in ["monitor", "chat", "sys_not_a_bridge"] {
            fs::create_dir_all(system.join(kept)).unwrap();
            fs::write(system.join(kept).join("fxmanifest.lua"), "fx_version 'cerulean'").unwrap();
        }
        fs::create_dir_all(system.join("observer")).unwrap();
        fs::write(system.join("observer/server.lua"), FILES[1].1).unwrap();

        let name = draw_name().unwrap();
        install(&layout, &name).unwrap();

        let mut folders: Vec<String> = fs::read_dir(&system)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        folders.sort();
        let mut expected = vec![
            "chat".to_owned(),
            "monitor".to_owned(),
            "sys_not_a_bridge".to_owned(),
            name,
        ];
        expected.sort();
        assert_eq!(folders, expected);
    }

    #[test]
    fn the_contract_version_matches_the_resource() {
        let lua = FILES[1].1;

        assert!(lua.contains(&format!("local CONTRACT_VERSION = {CONTRACT_VERSION}\n")));
    }

    #[test]
    fn the_resource_accepts_exactly_the_events_observer_can_send() {
        let lua = FILES[1].1;
        let events = every_event();

        for event in &events {
            assert!(lua.contains(&format!("    {} = true,\n", event.name())));
            assert_eq!(decode(&event.encode())["event"], event.name());
        }
        assert_eq!(lua.matches(" = true,\n").count(), events.len());
    }

    #[test]
    fn the_contract_documents_every_event_with_every_field_of_its_payload() {
        for event in every_event() {
            let wire = serde_json::to_value(&event).unwrap();
            let heading = format!("### `observer:{}`", event.name());
            let start = CONTRACT
                .find(&heading)
                .unwrap_or_else(|| panic!("CONTRACT.md has no section {heading}"));
            let section = &CONTRACT[start + heading.len()..];
            let section = &section[..section.find("\n#").unwrap_or(section.len())];

            assert!(CONTRACT.contains(&format!("| `observer:{}` |", event.name())));
            for field in wire["payload"].as_object().unwrap().keys() {
                assert!(
                    section.contains(&format!("| `{field}` |")),
                    "{heading} does not document `{field}`"
                );
            }
        }
        for reason in [
            StopReason::Stop,
            StopReason::Restart,
            StopReason::Scheduled,
            StopReason::Shutdown,
        ] {
            assert!(CONTRACT.contains(&format!("`{}`", reason.as_str())), "{reason:?}");
        }
        assert!(CONTRACT.contains(&format!("contract version **{CONTRACT_VERSION}**")));
    }

    #[test]
    fn the_example_resource_handles_every_event_the_bridge_raises() {
        let raised: Vec<&str> = FILES[1]
            .1
            .lines()
            .filter_map(|line| line.trim().strip_suffix(" = true,"))
            .collect();

        assert_eq!(raised.len(), every_event().len());
        for name in raised {
            assert!(
                EXAMPLE.contains(&format!("AddEventHandler('observer:{name}'")),
                "the example does not handle observer:{name}"
            );
        }
        assert!(EXAMPLE.contains(&format!("local CONTRACT_VERSION = {CONTRACT_VERSION}\n")));
        assert!(!EXAMPLE.contains("RegisterNetEvent("));
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

    const BRIDGE: &str = "sys_k3jx7qmw2pza";

    #[test]
    fn reports_come_only_from_the_bridge_of_this_start() {
        let trace = |resource: &str, payload: Value| TraceKind::Script {
            resource: resource.to_owned(),
            payload,
        };

        assert_eq!(
            Report::from_trace(
                &trace(BRIDGE, json!({ "type": "observer", "kind": "hb" })),
                BRIDGE
            ),
            Some(Report::Heartbeat)
        );
        assert_eq!(
            Report::from_trace(
                &trace(
                    BRIDGE,
                    json!({ "type": "observer", "kind": "ready", "contractVersion": 1 })
                ),
                BRIDGE
            ),
            Some(Report::Ready {
                contract_version: Some(1)
            })
        );
        assert_eq!(
            Report::from_trace(
                &trace(
                    BRIDGE,
                    json!({ "type": "observer", "kind": "heap", "heapUsed": 10, "heapTotal": 20, "rss": 30, "external": 1 })
                ),
                BRIDGE
            ),
            Some(Report::Heap {
                heap_used: 10,
                heap_total: 20,
                rss: 30,
                external: 1
            })
        );
        for impostor in ["mischief", "observer", "sys_aaaaaaaaaaaa"] {
            assert_eq!(
                Report::from_trace(
                    &trace(impostor, json!({ "type": "observer", "kind": "hb" })),
                    BRIDGE
                ),
                None,
                "{impostor}"
            );
        }
        assert_eq!(
            Report::from_trace(&trace(BRIDGE, json!({ "type": "other", "kind": "hb" })), BRIDGE),
            None
        );
    }

    #[test]
    fn a_resources_report_carries_its_starts_and_stops_when_it_has_some() {
        let report = |payload: Value| {
            Report::from_trace(
                &TraceKind::Script {
                    resource: BRIDGE.to_owned(),
                    payload,
                },
                BRIDGE,
            )
        };

        assert_eq!(
            report(json!({
                "type": "observer",
                "kind": "resources",
                "started": ["chat", "qb-core"],
                "events": [["qb-core", "stop"], ["qb-core", "start"]],
                "dropped": 2
            })),
            Some(Report::Resources {
                started: vec!["chat".to_owned(), "qb-core".to_owned()],
                events: vec![
                    ("qb-core".to_owned(), "stop".to_owned()),
                    ("qb-core".to_owned(), "start".to_owned())
                ],
                dropped: 2
            })
        );
        assert_eq!(
            report(json!({ "type": "observer", "kind": "resources", "started": ["chat"] })),
            Some(Report::Resources {
                started: vec!["chat".to_owned()],
                events: Vec::new(),
                dropped: 0
            })
        );
    }
}
