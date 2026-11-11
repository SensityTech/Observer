//! The bridge resource Observer installs in every build: it turns Observer's commands into
//! server events, and reports the heartbeat, readiness, started resources, the JS heap and the
//! players back through the structured trace channel. The resource lives in `resource/observer`;
//! in a build it takes a name drawn for each start of FXServer.

use std::fs;
use std::io;

use observer_domain::schedule::RestartSource;
use serde::{Deserialize, Serialize, Serializer};
use serde_json::Value;

use crate::layout::Layout;
use crate::resources::Reported;
use crate::stdin::{MAX_BRIDGE_JSON, safe};
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
const FIXED_NAME: &str = "observer";

/// What the bridge's reports carry as `type`.
pub(crate) const REPORT_TYPE: &str = "observer";

/// The version of the server events the bridge raises; it must match `CONTRACT_VERSION` in
/// `server.lua`.
pub const CONTRACT_VERSION: u32 = 5;

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

/// The bridge's script, which the contract's documentation reads its events and exports from.
pub(crate) fn script() -> &'static str {
    FILES[1].1
}

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
    /// The staff's announcement to the whole server, which the server's resources show.
    Announcement { message: String, by: String },
    /// The staff's message to one connected player, which the server's resources show.
    DirectMessage {
        player: u32,
        message: String,
        by: String,
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

impl BridgeEvent {
    pub fn name(&self) -> &'static str {
        match self {
            Self::RestartScheduled { .. } => "restartScheduled",
            Self::RestartSkipped { .. } => "restartSkipped",
            Self::RestartRescheduled { .. } => "restartRescheduled",
            Self::ShuttingDown { .. } => "shuttingDown",
            Self::StartChanges { .. } => "startChanges",
            Self::Announcement { .. } => "announcement",
            Self::DirectMessage { .. } => "directMessage",
        }
    }

    /// The argument of `observer_cmd`: the event's JSON in lowercase hex, which leaves no quote,
    /// semicolon or line break for the console parser to act on.
    pub fn encode(&self) -> String {
        hex_json(self)
    }
}

fn hex_json(value: &impl Serialize) -> String {
    let json = serde_json::to_vec(value).expect("strings and numbers always serialise");
    let mut hex = String::with_capacity(json.len() * 2);
    for byte in json {
        hex.push(char::from(HEX[usize::from(byte >> 4)]));
        hex.push(char::from(HEX[usize::from(byte & 0xf)]));
    }
    hex
}

const HEX: &[u8; 16] = b"0123456789abcdef";

/// What Observer has the bridge do itself, through `observer_cmd` as the events go, rather than
/// raise for the server's resources.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "action",
    content = "payload",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum BridgeAction {
    /// The answer to a `connecting` report: in, or refused with the owner's message.
    Verdict {
        request: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        refusal: Option<Refusal>,
    },
    /// Drops a connected player with the owner's message, after `observer:playerBanned`.
    Drop {
        player: u32,
        message: String,
        event: BanEvent,
    },
    /// Has the bridge report where the players stand every `every_ms` for `for_ms`, or stop with 0.
    Positions { every_ms: u32, for_ms: u32 },
    /// Drops `players`, or everyone connected with `all`, with the message they read, after
    /// `observer:playerKicked` for each.
    Kick {
        #[serde(skip_serializing_if = "Vec::is_empty")]
        players: Vec<u32>,
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        all: bool,
        message: String,
        event: KickEvent,
    },
    /// Has the bridge raise txAdmin's `playerBanned` for a ban just made, when the owner asked for
    /// txAdmin's events. Observer has no event of its own for a ban that drops nobody.
    Banned { event: TxAdminBan },
    /// Has the bridge raise txAdmin's `actionRevoked` for a ban just lifted, when the owner asked
    /// for txAdmin's events. Observer has no event of its own for a lifted ban.
    Lifted { event: TxAdminRevocation },
}

/// What a banned player reads, and the `observer:playerRefused` the bridge raises for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Refusal {
    pub message: String,
    pub event: BanEvent,
}

/// The payload of `observer:playerRefused` and `observer:playerBanned`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BanEvent {
    /// The ban's code.
    pub ban: String,
    pub reason: String,
    /// Unix milliseconds; absent for a permanent ban.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ends_at: Option<i64>,
    pub by: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    /// The net id of the player dropped; absent for a refused connection, which has none yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub player: Option<u32>,
}

/// Why players are kicked, from a closed list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum KickCause {
    /// The staff kicked this player.
    Kick,
    /// The staff kicked everyone.
    KickAll,
}

/// The payload of `txAdmin:events:playerBanned`, under txAdmin's own names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TxAdminBan {
    pub author: String,
    pub reason: String,
    /// The ban's code.
    pub action_id: String,
    /// Unix seconds; `false` for a permanent ban.
    #[serde(serialize_with = "or_false")]
    pub expiration: Option<i64>,
    /// As [`TxAdminBan::duration`] words it.
    pub duration_input: String,
    /// The same words, since Observer translates nothing; absent for a permanent ban.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_translated: Option<String>,
    /// The connected player the ban drops; absent when it drops nobody.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_net_id: Option<u32>,
    /// `kind:value`, the licence first.
    pub target_ids: Vec<String>,
    /// Hardware tokens, as FXServer gives them.
    pub target_hwids: Vec<String>,
    /// txAdmin writes `identifiers` for a ban that names no player.
    pub target_name: String,
    /// What a player the ban drops reads.
    pub kick_message: String,
}

impl TxAdminBan {
    /// A ban's length in the words txAdmin's panel sends: `permanent`, or a count of the largest
    /// of its units that holds the length whole, a month being 30 days. A length txAdmin cannot
    /// make, with minutes left over, is counted in minutes.
    pub fn duration(minutes: Option<u32>) -> String {
        const UNITS: [(u32, &str); 4] = [
            (30 * 24 * 60, "month"),
            (7 * 24 * 60, "week"),
            (24 * 60, "day"),
            (60, "hour"),
        ];
        let Some(minutes) = minutes else {
            return "permanent".to_owned();
        };
        let (count, unit) = UNITS
            .iter()
            .find(|(length, _)| minutes % length == 0)
            .map_or((minutes, "minute"), |(length, unit)| (minutes / length, *unit));
        format!("{count} {unit}{}", if count == 1 { "" } else { "s" })
    }
}

/// The payload of `txAdmin:events:actionRevoked` for a lifted ban, under txAdmin's own names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TxAdminRevocation {
    /// The ban's code.
    pub action_id: String,
    pub action_type: TxAdminAction,
    pub action_reason: String,
    /// Who made the ban.
    pub action_author: String,
    /// `false` for a ban that names no player.
    #[serde(serialize_with = "or_false")]
    pub player_name: Option<String>,
    /// `kind:value`, the licence first.
    pub player_ids: Vec<String>,
    /// Hardware tokens, as FXServer gives them.
    pub player_hwids: Vec<String>,
    /// Who lifted the ban.
    pub revoked_by: String,
}

/// What txAdmin revoked. It also revokes warnings, which Observer has not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TxAdminAction {
    Ban,
}

/// A value txAdmin may not have, which it then writes as `false`.
#[expect(clippy::ref_option, reason = "serde hands the field over by reference")]
fn or_false<T: Serialize, S: Serializer>(value: &Option<T>, serializer: S) -> Result<S::Ok, S::Error> {
    match value {
        Some(value) => value.serialize(serializer),
        None => serializer.serialize_bool(false),
    }
}

/// The payload of `observer:playerKicked`, to which the bridge adds the `player` it drops.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KickEvent {
    pub reason: String,
    pub by: String,
    pub cause: KickCause,
}

impl BridgeAction {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Verdict { .. } => "verdict",
            Self::Drop { .. } => "drop",
            Self::Positions { .. } => "positions",
            Self::Kick { .. } => "kick",
            Self::Banned { .. } => "banned",
            Self::Lifted { .. } => "lifted",
        }
    }

    /// The kick of everyone connected, as the server counts them when it runs the action.
    pub fn kick_all(message: String, event: KickEvent) -> Self {
        Self::Kick {
            players: Vec::new(),
            all: true,
            message,
            event,
        }
        .fitted()
    }

    /// The kicks that drop `players`, each within the console's line: one action while the list
    /// fits beside the message, several past that. None for an empty list.
    pub fn kicks(players: &[u32], message: String, event: KickEvent) -> Vec<Self> {
        // Fitted around the longest id, so that every list of one fits.
        let fitted = Self::Kick {
            players: vec![u32::MAX],
            all: false,
            message,
            event,
        }
        .fitted();
        let Self::Kick { message, event, .. } = &fitted else {
            return Vec::new();
        };
        // What the ids and the commas between them may take of the line.
        let room = (MAX_BRIDGE_JSON + u32::MAX.to_string().len()).saturating_sub(fitted.encode().len() / 2);
        let mut lists = Vec::new();
        let mut listed = Vec::new();
        let mut used = 0;
        for &player in players {
            let digits = player.to_string().len();
            if !listed.is_empty() && used + 1 + digits > room {
                lists.push(std::mem::take(&mut listed));
                used = 0;
            }
            used += digits + usize::from(!listed.is_empty());
            listed.push(player);
        }
        if !listed.is_empty() {
            lists.push(listed);
        }
        lists
            .into_iter()
            .map(|players| Self::Kick {
                players,
                all: false,
                message: message.clone(),
                event: event.clone(),
            })
            .collect()
    }

    pub fn encode(&self) -> String {
        hex_json(self)
    }

    /// The action shortened until it fits the console's line: the longest of its message and its
    /// event's name and reason first, each cut on a character boundary. A list of players is
    /// never cut: [`BridgeAction::kicks`] splits it. A ban made or lifted, told to txAdmin's
    /// listeners, first loses its tokens, then its identifiers past the first, from the last.
    #[must_use]
    pub fn fitted(mut self) -> Self {
        while self.encode().len() > 2 * MAX_BRIDGE_JSON {
            let held = match &mut self {
                Self::Banned { event } => Some((&mut event.target_hwids, &mut event.target_ids)),
                Self::Lifted { event } => Some((&mut event.player_hwids, &mut event.player_ids)),
                _ => None,
            };
            if let Some((hwids, ids)) = held
                && (hwids.pop().is_some() || (ids.len() > 1 && ids.pop().is_some()))
            {
                continue;
            }
            let texts: Vec<&mut String> = match &mut self {
                Self::Verdict { refusal: None, .. } | Self::Positions { .. } => return self,
                Self::Verdict {
                    refusal: Some(Refusal { message, event }),
                    ..
                }
                | Self::Drop { message, event, .. } => [message, &mut event.reason]
                    .into_iter()
                    .chain(event.name.as_mut())
                    .collect(),
                Self::Kick { message, event, .. } => vec![message, &mut event.reason],
                Self::Banned { event } => {
                    vec![&mut event.kick_message, &mut event.reason, &mut event.target_name]
                }
                Self::Lifted { event } => [&mut event.action_reason]
                    .into_iter()
                    .chain(event.player_name.as_mut())
                    .collect(),
            };
            let longest = texts
                .into_iter()
                .max_by_key(|text| text.len())
                .expect("a message is always there");
            if longest.is_empty() {
                return self;
            }
            let mut end = longest.len() * 3 / 4;
            while !longest.is_char_boundary(end) {
                end -= 1;
            }
            longest.truncate(end);
        }
        self
    }
}

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
    /// One page of every resource FXServer knows, in name order: once the cfg has run, and again
    /// after each `refresh`. The pages of one list carry its number, as `list`.
    ResourceList {
        #[serde(rename = "list")]
        number: u64,
        page: u32,
        last: bool,
        resources: Vec<Reported>,
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
    use observer_domain::moderation::MAX_TEXT_CHARS;
    use observer_domain::players::{MAX_LABEL_ID_CHARS, MAX_LABEL_NAME_CHARS};
    use serde_json::json;

    use super::*;
    use crate::layout::Flavour;

    /// The contract as it ships, next to the bridge, and the example resource that follows it.
    const CONTRACT: &str = include_str!("../../../resource/CONTRACT.md");
    const EXAMPLE: &str = include_str!("../../../resource/observer_example/server.lua");

    fn every_event() -> [BridgeEvent; 7] {
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
            BridgeEvent::Announcement {
                message: String::new(),
                by: String::new(),
            },
            BridgeEvent::DirectMessage {
                player: 0,
                message: String::new(),
                by: String::new(),
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
    fn resources_find_the_bridge_through_its_convar_and_never_by_a_fixed_name() {
        let lookup = format!("GetConvar('{NAME_CONVAR}', '')");

        assert!(CONTRACT.contains(&lookup));
        assert!(CONTRACT.contains(&format!("exports[{lookup}]:getContractVersion()")));
        assert!(EXAMPLE.contains(&lookup));
        for fixed in ["exports.observer", "'observer'", "\"observer\""] {
            assert!(!CONTRACT.contains(fixed), "the contract names the bridge {fixed}");
            assert!(!EXAMPLE.contains(fixed), "the example names the bridge {fixed}");
        }
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

    fn ban_event(player: Option<u32>) -> BanEvent {
        BanEvent {
            ban: "K7QM-2XPA".to_owned(),
            reason: "Aimbot".to_owned(),
            ends_at: Some(1_794_000_000_000),
            by: "maxime".to_owned(),
            name: Some("kaito".to_owned()),
            license: Some(format!("license:{}", "a".repeat(40))),
            player,
        }
    }

    fn kick_event() -> KickEvent {
        KickEvent {
            reason: "RDM".to_owned(),
            by: "maxime".to_owned(),
            cause: KickCause::Kick,
        }
    }

    fn txadmin_ban() -> TxAdminBan {
        TxAdminBan {
            author: "maxime".to_owned(),
            reason: "Aimbot".to_owned(),
            action_id: "K7QM-2XPA".to_owned(),
            expiration: Some(1_794_000_000),
            duration_input: "2 days".to_owned(),
            duration_translated: Some("2 days".to_owned()),
            target_net_id: Some(3),
            target_ids: vec![
                format!("license:{}", "a".repeat(40)),
                "discord:123456789012345678".to_owned(),
            ],
            target_hwids: vec![format!("4:{}", "ab".repeat(32))],
            target_name: "kaito".to_owned(),
            kick_message: "Banned".to_owned(),
        }
    }

    fn txadmin_revocation() -> TxAdminRevocation {
        TxAdminRevocation {
            action_id: "K7QM-2XPA".to_owned(),
            action_type: TxAdminAction::Ban,
            action_reason: "Aimbot".to_owned(),
            action_author: "maxime".to_owned(),
            player_name: Some("kaito".to_owned()),
            player_ids: vec![
                format!("license:{}", "a".repeat(40)),
                "discord:123456789012345678".to_owned(),
            ],
            player_hwids: vec![format!("4:{}", "ab".repeat(32))],
            revoked_by: "sarah".to_owned(),
        }
    }

    fn every_action() -> [BridgeAction; 6] {
        [
            BridgeAction::Verdict {
                request: 7,
                refusal: Some(Refusal {
                    message: "Banned".to_owned(),
                    event: ban_event(None),
                }),
            },
            BridgeAction::Drop {
                player: 3,
                message: "Banned".to_owned(),
                event: ban_event(Some(3)),
            },
            BridgeAction::Positions {
                every_ms: 2000,
                for_ms: 30_000,
            },
            BridgeAction::Kick {
                players: vec![3],
                all: false,
                message: "Kicked".to_owned(),
                event: kick_event(),
            },
            BridgeAction::Banned { event: txadmin_ban() },
            BridgeAction::Lifted {
                event: txadmin_revocation(),
            },
        ]
    }

    #[test]
    fn the_resource_takes_exactly_the_actions_observer_can_send() {
        let lua = FILES[1].1;
        let start = lua
            .find(
                "local ACTIONS = {
",
            )
            .expect("the bridge has its actions");
        let actions = &lua[start..];
        let actions = &actions[..actions
            .find(
                "
}
",
            )
            .expect("the actions end")];

        for action in every_action() {
            assert!(actions.contains(&format!(
                "
    {} = function(payload)
",
                action.name()
            )));
            assert_eq!(decode(&action.encode())["action"], action.name());
        }
        assert_eq!(
            actions
                .matches(
                    " = function(payload)
"
                )
                .count(),
            every_action().len()
        );
        assert_eq!(
            decode(
                &BridgeAction::Verdict {
                    request: 9,
                    refusal: None
                }
                .encode()
            ),
            json!({ "action": "verdict", "payload": { "request": 9 } })
        );
    }

    #[test]
    fn the_contract_and_the_example_cover_the_ban_events_with_every_field() {
        for (name, player) in [("playerRefused", None), ("playerBanned", Some(3))] {
            let wire = serde_json::to_value(ban_event(player)).unwrap();
            let heading = format!("### `observer:{name}`");
            let start = CONTRACT
                .find(&heading)
                .unwrap_or_else(|| panic!("CONTRACT.md has no section {heading}"));
            let section = &CONTRACT[start + heading.len()..];
            let section = &section[..section
                .find(
                    "
#",
                )
                .unwrap_or(section.len())];

            assert!(CONTRACT.contains(&format!("| `observer:{name}` |")));
            assert!(FILES[1].1.contains(&format!("TriggerEvent('observer:{name}'")));
            assert!(EXAMPLE.contains(&format!("AddEventHandler('observer:{name}'")));
            for field in wire.as_object().unwrap().keys() {
                assert!(
                    section.contains(&format!("| `{field}` |")),
                    "{heading} does not document `{field}`"
                );
            }
        }
    }

    #[test]
    fn the_contract_and_the_example_cover_the_kick_event_with_every_field() {
        let mut wire = serde_json::to_value(kick_event()).unwrap();
        wire["player"] = json!(3);
        let heading = "### `observer:playerKicked`";
        let start = CONTRACT
            .find(heading)
            .unwrap_or_else(|| panic!("CONTRACT.md has no section {heading}"));
        let section = &CONTRACT[start + heading.len()..];
        let section = &section[..section.find("\n#").unwrap_or(section.len())];

        assert!(CONTRACT.contains("| `observer:playerKicked` |"));
        assert!(FILES[1].1.contains("TriggerEvent('observer:playerKicked'"));
        assert!(FILES[1].1.contains("copy.player = id"));
        assert!(EXAMPLE.contains("AddEventHandler('observer:playerKicked'"));
        for field in wire.as_object().unwrap().keys() {
            assert!(
                section.contains(&format!("| `{field}` |")),
                "{heading} does not document `{field}`"
            );
        }
        for cause in [KickCause::Kick, KickCause::KickAll] {
            let wire = serde_json::to_value(cause).unwrap();
            assert!(
                section.contains(&format!("`{}`", wire.as_str().unwrap())),
                "{cause:?}"
            );
        }
    }

    #[test]
    fn a_kick_names_its_players_or_everyone_and_never_both() {
        let kicks = BridgeAction::kicks(&[3, 12], "Kicked".to_owned(), kick_event());
        let everyone = BridgeAction::kick_all(
            "Kicked".to_owned(),
            KickEvent {
                cause: KickCause::KickAll,
                ..kick_event()
            },
        );

        assert_eq!(kicks.len(), 1);
        assert_eq!(
            decode(&kicks[0].encode()),
            json!({
                "action": "kick",
                "payload": {
                    "players": [3, 12],
                    "message": "Kicked",
                    "event": { "reason": "RDM", "by": "maxime", "cause": "kick" }
                }
            })
        );
        assert_eq!(
            decode(&everyone.encode()),
            json!({
                "action": "kick",
                "payload": {
                    "all": true,
                    "message": "Kicked",
                    "event": { "reason": "RDM", "by": "maxime", "cause": "kickAll" }
                }
            })
        );
        assert_eq!(BridgeAction::kicks(&[], "Kicked".to_owned(), kick_event()), []);
    }

    #[test]
    fn a_long_list_of_players_is_kicked_in_as_many_actions_as_the_console_line_asks() {
        let players: Vec<u32> = (1..=2048).map(|id| id * 2_000_000).collect();
        let event = KickEvent {
            reason: "é".repeat(1500),
            ..kick_event()
        };

        let kicks = BridgeAction::kicks(&players, "\"".repeat(900), event);

        assert!(kicks.len() > 1);
        let mut listed: Vec<u32> = Vec::new();
        for kick in &kicks {
            assert!(crate::stdin::allowed(&format!("observer_cmd {}", kick.encode())));
            let BridgeAction::Kick {
                players,
                all,
                message,
                event,
            } = kick
            else {
                panic!("not a kick");
            };
            assert!(!all && !players.is_empty());
            assert_ne!(message, "");
            assert_ne!(event.reason, "");
            listed.extend(players.iter().copied());
        }
        assert_eq!(listed, players);
        let one_line = BridgeAction::kicks(&players[..40], "Kicked".to_owned(), kick_event());
        assert_eq!(one_line.len(), 1);
    }

    #[test]
    fn an_announcement_or_a_message_at_its_longest_fits_the_console_line() {
        let longest = "\u{1F697}".repeat(MAX_TEXT_CHARS);
        let by = "m".repeat(32);

        for event in [
            BridgeEvent::Announcement {
                message: longest.clone(),
                by: by.clone(),
            },
            BridgeEvent::DirectMessage {
                player: u32::MAX,
                message: longest.clone(),
                by: by.clone(),
            },
            BridgeEvent::Announcement {
                message: "\n".repeat(MAX_TEXT_CHARS),
                by,
            },
        ] {
            assert!(
                crate::stdin::allowed(&format!("observer_cmd {}", event.encode())),
                "{} bytes",
                event.encode().len() / 2
            );
        }
    }

    #[test]
    fn a_ban_is_told_to_txadmin_s_listeners_under_txadmin_s_own_names() {
        let temporary = BridgeAction::Banned { event: txadmin_ban() };
        let permanent = BridgeAction::Banned {
            event: TxAdminBan {
                expiration: None,
                duration_input: TxAdminBan::duration(None),
                duration_translated: None,
                target_net_id: None,
                target_ids: vec!["discord:123456789012345678".to_owned()],
                target_hwids: Vec::new(),
                target_name: "identifiers".to_owned(),
                ..txadmin_ban()
            },
        };

        assert_eq!(
            decode(&temporary.encode()),
            json!({
                "action": "banned",
                "payload": { "event": {
                    "author": "maxime",
                    "reason": "Aimbot",
                    "actionId": "K7QM-2XPA",
                    "expiration": 1_794_000_000,
                    "durationInput": "2 days",
                    "durationTranslated": "2 days",
                    "targetNetId": 3,
                    "targetIds": [format!("license:{}", "a".repeat(40)), "discord:123456789012345678"],
                    "targetHwids": [format!("4:{}", "ab".repeat(32))],
                    "targetName": "kaito",
                    "kickMessage": "Banned"
                } }
            })
        );
        assert_eq!(
            decode(&permanent.encode())["payload"]["event"],
            json!({
                "author": "maxime",
                "reason": "Aimbot",
                "actionId": "K7QM-2XPA",
                "expiration": false,
                "durationInput": "permanent",
                "targetIds": ["discord:123456789012345678"],
                "targetHwids": [],
                "targetName": "identifiers",
                "kickMessage": "Banned"
            })
        );
    }

    #[test]
    fn a_ban_s_length_reads_in_the_words_txadmin_s_panel_sends() {
        for (minutes, words) in [
            (None, "permanent"),
            (Some(60), "1 hour"),
            (Some(120), "2 hours"),
            (Some(24 * 60), "1 day"),
            (Some(3 * 24 * 60), "3 days"),
            (Some(7 * 24 * 60), "1 week"),
            (Some(14 * 24 * 60), "2 weeks"),
            (Some(30 * 24 * 60), "1 month"),
            (Some(365 * 24 * 60), "365 days"),
            (Some(90), "90 minutes"),
            (Some(1), "1 minute"),
        ] {
            assert_eq!(TxAdminBan::duration(minutes), words, "{minutes:?}");
        }
    }

    #[test]
    fn a_ban_told_to_txadmin_loses_tokens_then_identifiers_to_fit_the_console_line() {
        let token = |n: u32| format!("4:{n:064x}");
        let mut event = txadmin_ban();
        event.target_hwids = (0..200).map(token).collect();
        let many_tokens = BridgeAction::Banned { event: event.clone() }.fitted();
        event.target_ids = (0..64).map(|n| format!("discord:{n:018}")).collect();
        event.kick_message = "é".repeat(600);
        let many_of_both = BridgeAction::Banned { event }.fitted();

        let (BridgeAction::Banned { event: tokens_cut }, BridgeAction::Banned { event: both_cut }) =
            (&many_tokens, &many_of_both)
        else {
            panic!("not bans");
        };
        for fitted in [&many_tokens, &many_of_both] {
            assert!(crate::stdin::allowed(&format!(
                "observer_cmd {}",
                fitted.encode()
            )));
        }
        assert_eq!(tokens_cut.target_ids, txadmin_ban().target_ids);
        assert!((1..200).contains(&tokens_cut.target_hwids.len()));
        assert_eq!(tokens_cut.target_hwids[0], token(0));
        assert_eq!(both_cut.target_hwids, Vec::<String>::new());
        assert!((1..64).contains(&both_cut.target_ids.len()));
        assert_eq!(both_cut.target_ids[0], format!("discord:{:018}", 0));
        assert_eq!(both_cut.kick_message, "é".repeat(600));
    }

    #[test]
    fn a_lifted_ban_is_told_to_txadmin_s_listeners_under_txadmin_s_own_names() {
        let named = BridgeAction::Lifted {
            event: txadmin_revocation(),
        };
        let unnamed = BridgeAction::Lifted {
            event: TxAdminRevocation {
                player_name: None,
                player_ids: vec!["discord:123456789012345678".to_owned()],
                player_hwids: Vec::new(),
                ..txadmin_revocation()
            },
        };

        assert_eq!(
            decode(&named.encode()),
            json!({
                "action": "lifted",
                "payload": { "event": {
                    "actionId": "K7QM-2XPA",
                    "actionType": "ban",
                    "actionReason": "Aimbot",
                    "actionAuthor": "maxime",
                    "playerName": "kaito",
                    "playerIds": [format!("license:{}", "a".repeat(40)), "discord:123456789012345678"],
                    "playerHwids": [format!("4:{}", "ab".repeat(32))],
                    "revokedBy": "sarah"
                } }
            })
        );
        assert_eq!(
            decode(&unnamed.encode())["payload"]["event"]["playerName"],
            json!(false)
        );
    }

    #[test]
    fn a_lifted_ban_told_to_txadmin_loses_tokens_then_identifiers_to_fit_the_console_line() {
        let mut event = txadmin_revocation();
        event.player_hwids = (0..200).map(|n| format!("4:{n:064x}")).collect();
        event.player_ids = (0..64).map(|n| format!("discord:{n:018}")).collect();
        event.action_reason = "é".repeat(200);

        let fitted = BridgeAction::Lifted { event }.fitted();

        assert!(crate::stdin::allowed(&format!(
            "observer_cmd {}",
            fitted.encode()
        )));
        let BridgeAction::Lifted { event } = fitted else {
            panic!("not a lift");
        };
        assert_eq!(event.player_hwids, Vec::<String>::new());
        assert!((1..64).contains(&event.player_ids.len()));
        assert_eq!(event.player_ids[0], format!("discord:{:018}", 0));
        assert_eq!(event.action_reason, "é".repeat(200));
    }

    #[test]
    fn the_contract_names_every_field_of_txadmin_s_ban_events() {
        for (name, wire) in [
            ("playerBanned", serde_json::to_value(txadmin_ban()).unwrap()),
            (
                "actionRevoked",
                serde_json::to_value(txadmin_revocation()).unwrap(),
            ),
        ] {
            let start = CONTRACT
                .find(&format!("`txAdmin:events:{name}` |"))
                .unwrap_or_else(|| panic!("CONTRACT.md does not map txAdmin's {name}"));
            let row = &CONTRACT[start..];
            let row = &row[..row.find('\n').unwrap_or(row.len())];

            assert!(
                FILES[1]
                    .1
                    .contains(&format!("txadminEvent('txAdmin:events:{name}', payload)")),
                "the bridge does not raise txAdmin's {name}"
            );
            for field in wire.as_object().unwrap().keys() {
                assert!(
                    row.contains(&format!("`{field}`")),
                    "CONTRACT.md does not name {name}'s `{field}`"
                );
            }
        }
    }

    #[test]
    fn an_action_is_cut_to_fit_the_console_line() {
        let mut event = ban_event(Some(3));
        event.reason = "r".repeat(3000);
        let long = BridgeAction::Drop {
            player: 3,
            message: "é".repeat(2000),
            event,
        };

        let fitted = long.fitted();

        assert!(crate::stdin::allowed(&format!(
            "observer_cmd {}",
            fitted.encode()
        )));
        let BridgeAction::Drop { message, event, .. } = fitted else {
            panic!("not a drop");
        };
        assert_ne!(message, "");
        assert!(message.chars().all(|c| c == 'é'));
        assert_ne!(event.reason, "");
        assert_eq!(every_action()[0].clone().fitted(), every_action()[0]);
    }

    #[test]
    fn the_contract_and_the_example_call_every_export_of_the_bridge() {
        let exports: Vec<&str> = FILES[1]
            .1
            .lines()
            .filter_map(|line| Some(line.strip_prefix("exports('")?.split_once('\'')?.0))
            .collect();

        assert_eq!(
            exports,
            ["getContractVersion", "setPlayerLabel", "clearPlayerLabel"]
        );
        for export in exports {
            let call = format!("]:{export}(");
            assert!(CONTRACT.contains(&call), "CONTRACT.md does not show {export}");
            assert!(EXAMPLE.contains(&call), "the example does not call {export}");
        }
    }

    #[test]
    fn the_contract_gives_the_label_fields_and_limits_the_bridge_and_observer_check() {
        let lua = FILES[1].1;
        let start = CONTRACT
            .find("## Player labels")
            .expect("CONTRACT.md documents labels");
        let section = &CONTRACT[start..];
        let section = &section[..section[1..].find("\n## ").map_or(section.len(), |end| end + 1)];

        for (field, max, constant) in [
            ("name", MAX_LABEL_NAME_CHARS, "LABEL_NAME_MAX"),
            ("id", MAX_LABEL_ID_CHARS, "LABEL_ID_MAX"),
        ] {
            assert!(lua.contains(&format!("label.{field}, {constant})")), "{field}");
            assert!(lua.contains(&format!("local {constant} = {max}\n")), "{constant}");
            assert!(section.contains(&format!("| `{field}` |")), "{field}");
            assert!(section.contains(&format!("up to {max} characters")), "{field}");
        }
        for refusal in ["notConnected", "invalid", "tooLong"] {
            assert!(lua.contains(&format!("'{refusal}'")), "{refusal}");
            assert!(section.contains(&format!("`{refusal}`")), "{refusal}");
        }
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
    fn a_page_of_the_resource_list_is_read_with_what_each_manifest_says() {
        let page = Report::from_trace(
            &TraceKind::Script {
                resource: BRIDGE.to_owned(),
                payload: json!({
                    "type": "observer",
                    "kind": "resource_list",
                    "list": 2,
                    "page": 1,
                    "last": false,
                    "resources": [
                        {
                            "name": "ox_inventory",
                            "state": "started",
                            "path": "C:/srv/resources//[ox]/ox_inventory",
                            "version": "2.44.1",
                            "needs": ["/onesync", "ox_lib"],
                            "provides": ["inventory"]
                        },
                        { "name": "taxi", "state": "stopped" }
                    ]
                }),
            },
            BRIDGE,
        );

        assert_eq!(
            page,
            Some(Report::ResourceList {
                number: 2,
                page: 1,
                last: false,
                resources: vec![
                    Reported {
                        name: "ox_inventory".to_owned(),
                        state: "started".to_owned(),
                        path: Some("C:/srv/resources//[ox]/ox_inventory".to_owned()),
                        version: Some("2.44.1".to_owned()),
                        needs: vec!["/onesync".to_owned(), "ox_lib".to_owned()],
                        provides: vec!["inventory".to_owned()],
                    },
                    Reported {
                        name: "taxi".to_owned(),
                        state: "stopped".to_owned(),
                        ..Reported::default()
                    },
                ],
            })
        );
    }

    #[test]
    fn the_resource_reports_the_list_once_the_cfg_has_run_and_after_each_refresh() {
        let lua = FILES[1].1;

        assert!(lua.contains("report('resource_list', { list = list, page = page, last = page == pages"));
        assert!(lua.contains("    reportResourceList()\n    reportPlayers()\n"));
        assert!(lua.contains("AddEventHandler('onResourceListRefresh', function()"));
        for native in [
            "GetResourceState(name)",
            "GetResourcePath(name)",
            "GetResourceMetadata(name, 'version', 0)",
            "'dependency', 'dependencie'",
            "'provide'",
        ] {
            assert!(lua.contains(native), "{native}");
        }
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
