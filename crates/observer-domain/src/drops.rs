//! Why a player's session ended: FiveM's drop category first, the reason's text second, as
//! FiveM's `ClientDropReasons.h` and txAdmin's rules have it.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::metrics::RunMarker;

pub mod signature;
pub mod stats;

/// The resource name FXServer gives the drops it makes itself: no resource dropped the player.
const INTERNAL_RESOURCE: &str = "__cfx_internal:server";
/// The longest kicking resource's name kept, in bytes.
pub const MAX_RESOURCE_BYTES: usize = 256;

/// The crash prefix, as the client translates it in the 14 languages it ships the most.
const CRASH_PREFIXES: [&str; 14] = [
    "game crashed: ",
    "o jogo crashou: ",
    "le jeu a cessé de fonctionner : ",
    "spielabsturz: ",
    "el juego crasheó: ",
    "تعطلت العبة: ",
    "spel werkt niet meer: ",
    "oyun çöktü: ",
    "a játék összeomlott: ",
    "il gioco ha smesso di funzionare: ",
    "游戏发生崩溃：",
    "遊戲已崩潰: ",
    "pád hry: ",
    "spelet kraschade: ",
];

/// "Unhandled exception: " as the client translates it, turned back into English.
const EXCEPTION_PREFIXES: [&str; 13] = [
    "unhandled exception: ",
    "exceção não tratada: ",
    "exception non-gérée : ",
    "unbehandelte ausnahme: ",
    "excepción no manejada: ",
    "استثناء غير معالج: ",
    "i̇şlenemeyen özel durum: ",
    "nem kezelt kivétel: ",
    "eccezione non gestita: ",
    "未处理的异常：",
    "未處理的異常： ",
    "neošetřená výjimka: ",
    "okänt fel: ",
];

const QUIT_TEXTS: [(&str, DropKind); 8] = [
    ("exiting", DropKind::Exit),
    ("disconnected.", DropKind::Disconnect),
    ("connecting to another server", DropKind::OtherServer),
    ("could not find requested level", DropKind::Other),
    ("entering rockstar editor", DropKind::Editor),
    ("quit:", DropKind::QuitMessage),
    ("reconnecting", DropKind::Reconnect),
    ("reloading game", DropKind::Reload),
];

/// Texts of builds that give no category, as txAdmin reads them.
const TIMEOUT_TEXTS: [(&str, DropKind); 3] = [
    ("server->client connection timed out", DropKind::Connection),
    ("connection timed out", DropKind::Connection),
    ("timed out after 60 seconds", DropKind::MissedFrames),
];
const SECURITY_TEXTS: [&str; 8] = [
    "reliable network event overflow",
    "reliable network event size overflow:",
    "reliable server command overflow",
    "reliable state bag packet overflow",
    "unreliable network event overflow",
    "connection to cnl timed out",
    "server command overflow",
    "invalid client configuration. restart your game and reconnect",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DropCategory {
    Quit,
    Kick,
    Timeout,
    Crash,
    Security,
    /// The server stopped: never in the players' rates.
    ServerStop,
    /// The server crashed or hung: never in the players' rates either.
    ServerCrash,
    Unknown,
}

impl DropCategory {
    pub const ALL: [Self; 8] = [
        Self::Quit,
        Self::Kick,
        Self::Timeout,
        Self::Crash,
        Self::Security,
        Self::ServerStop,
        Self::ServerCrash,
        Self::Unknown,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Quit => "quit",
            Self::Kick => "kick",
            Self::Timeout => "timeout",
            Self::Crash => "crash",
            Self::Security => "security",
            Self::ServerStop => "serverStop",
            Self::ServerCrash => "serverCrash",
            Self::Unknown => "unknown",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|category| category.as_str() == name)
    }
}

/// What a category says in more detail. A crash has none until crash signatures exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DropKind {
    // Quit.
    Exit,
    Disconnect,
    OtherServer,
    Reconnect,
    /// The Rockstar Editor.
    Editor,
    Reload,
    /// "Quit: ..." with the player's own text, which is not kept.
    QuitMessage,
    // Kick.
    /// A resource dropped the player: the session names it.
    Resource,
    // Timeout.
    Connection,
    PendingCommands,
    /// OneSync's too many missed frames.
    MissedFrames,
    /// The player reconnected while the old connection was still dying: the tail of a crash or
    /// of a network loss.
    Replaced,
    // Security.
    StateBagRate,
    NetEventRate,
    LatentNetEventRate,
    CommandRate,
    NetGameEventRate,
    ServerInitiated,
    // Server stop: why Observer stopped it, or an operator's `quit`.
    Stop,
    Restart,
    Scheduled,
    Shutdown,
    CrashLoop,
    Console,
    // Server crash.
    #[serde(rename = "crash")]
    ServerCrashed,
    Hang,
    // Unknown.
    /// Observer stopped without seeing the end: closed at its next start.
    Lost,
    /// Gone from FXServer's player list without a drop report.
    Missed,
    Other,
}

impl DropKind {
    pub const ALL: [Self; 29] = [
        Self::Exit,
        Self::Disconnect,
        Self::OtherServer,
        Self::Reconnect,
        Self::Editor,
        Self::Reload,
        Self::QuitMessage,
        Self::Resource,
        Self::Connection,
        Self::PendingCommands,
        Self::MissedFrames,
        Self::Replaced,
        Self::StateBagRate,
        Self::NetEventRate,
        Self::LatentNetEventRate,
        Self::CommandRate,
        Self::NetGameEventRate,
        Self::ServerInitiated,
        Self::Stop,
        Self::Restart,
        Self::Scheduled,
        Self::Shutdown,
        Self::CrashLoop,
        Self::Console,
        Self::ServerCrashed,
        Self::Hang,
        Self::Lost,
        Self::Missed,
        Self::Other,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Exit => "exit",
            Self::Disconnect => "disconnect",
            Self::OtherServer => "otherServer",
            Self::Reconnect => "reconnect",
            Self::Editor => "editor",
            Self::Reload => "reload",
            Self::QuitMessage => "quitMessage",
            Self::Resource => "resource",
            Self::Connection => "connection",
            Self::PendingCommands => "pendingCommands",
            Self::MissedFrames => "missedFrames",
            Self::Replaced => "replaced",
            Self::StateBagRate => "stateBagRate",
            Self::NetEventRate => "netEventRate",
            Self::LatentNetEventRate => "latentNetEventRate",
            Self::CommandRate => "commandRate",
            Self::NetGameEventRate => "netGameEventRate",
            Self::ServerInitiated => "serverInitiated",
            Self::Stop => "stop",
            Self::Restart => "restart",
            Self::Scheduled => "scheduled",
            Self::Shutdown => "shutdown",
            Self::CrashLoop => "crashLoop",
            Self::Console => "console",
            Self::ServerCrashed => "crash",
            Self::Hang => "hang",
            Self::Lost => "lost",
            Self::Missed => "missed",
            Self::Other => "other",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == name)
    }
}

/// A drop as the bridge reported it: FXServer's reason, the resource that dropped the player and
/// FiveM's category, absent on very old builds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReportedDrop<'a> {
    pub reason: &'a str,
    pub resource: Option<&'a str>,
    pub category: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classified {
    pub category: DropCategory,
    /// `None` for a crash, and for a server stop, whose kind is why Observer stopped it.
    pub kind: Option<DropKind>,
    /// The resource that kicked the player.
    pub kicker: Option<String>,
    /// FiveM's own category, as reported.
    pub native: Option<i64>,
    /// For a crash, the text after the translated prefix, "Unhandled exception: " in English.
    pub crash_text: Option<String>,
    /// The reason worth keeping for review: a kick's, or an unknown drop's.
    pub keeps_reason: bool,
}

/// Classifies a reported drop. The texts it keeps still need redacting.
pub fn classify(drop: ReportedDrop<'_>) -> Classified {
    let lower = drop.reason.trim().to_lowercase();
    let native = drop.category.filter(|&category| (1..=64).contains(&category));
    let simple = |category, kind| Classified {
        category,
        kind,
        kicker: None,
        native,
        crash_text: None,
        keeps_reason: false,
    };
    let Some(category) = drop.category else {
        return classify_text(drop.reason, &lower);
    };
    match category {
        1 => Classified {
            kicker: drop
                .resource
                .map(str::trim)
                .filter(|resource| !resource.is_empty() && *resource != INTERNAL_RESOURCE)
                .map(|resource| cut(resource, MAX_RESOURCE_BYTES).to_owned()),
            keeps_reason: true,
            ..simple(DropCategory::Kick, Some(DropKind::Resource))
        },
        // A player's own quit message is a quit, whatever it says: "Quit: Game crashed: ..." is
        // not a crash.
        2 if QUIT_TEXTS.iter().any(|(text, _)| lower.starts_with(text)) => {
            simple(DropCategory::Quit, Some(quit_kind(&lower)))
        }
        2 => match crash_text(drop.reason, &lower) {
            Some(text) => Classified {
                crash_text: Some(text),
                ..simple(DropCategory::Crash, None)
            },
            None => simple(DropCategory::Quit, Some(quit_kind(&lower))),
        },
        3 => simple(DropCategory::Security, Some(DropKind::ServerInitiated)),
        4 => simple(DropCategory::Timeout, Some(DropKind::Replaced)),
        5 => simple(DropCategory::Timeout, Some(DropKind::Connection)),
        6 => simple(DropCategory::Timeout, Some(DropKind::PendingCommands)),
        7 => simple(DropCategory::ServerStop, None),
        8 => simple(DropCategory::Security, Some(DropKind::StateBagRate)),
        9 => simple(DropCategory::Security, Some(DropKind::NetEventRate)),
        10 => simple(DropCategory::Security, Some(DropKind::LatentNetEventRate)),
        11 => simple(DropCategory::Security, Some(DropKind::CommandRate)),
        12 => simple(DropCategory::Timeout, Some(DropKind::MissedFrames)),
        13 => simple(DropCategory::Security, Some(DropKind::NetGameEventRate)),
        _ => Classified {
            keeps_reason: true,
            ..simple(DropCategory::Unknown, Some(DropKind::Other))
        },
    }
}

/// The text rules, for builds that report no category.
fn classify_text(reason: &str, lower: &str) -> Classified {
    let classified = |category, kind| Classified {
        category,
        kind: Some(kind),
        kicker: None,
        native: None,
        crash_text: None,
        keeps_reason: false,
    };
    if QUIT_TEXTS.iter().any(|(text, _)| lower.starts_with(text)) {
        return classified(DropCategory::Quit, quit_kind(lower));
    }
    if lower.starts_with("server shutting down:") {
        return Classified {
            kind: None,
            ..classified(DropCategory::ServerStop, DropKind::Other)
        };
    }
    if lower.starts_with("disconnected by server:") {
        return classified(DropCategory::Security, DropKind::ServerInitiated);
    }
    if let Some((_, kind)) = TIMEOUT_TEXTS.iter().find(|(text, _)| lower.contains(text)) {
        return classified(DropCategory::Timeout, *kind);
    }
    if SECURITY_TEXTS.iter().any(|text| lower.contains(text)) {
        return classified(DropCategory::Security, DropKind::ServerInitiated);
    }
    if let Some(text) = crash_text(reason, lower) {
        return Classified {
            kind: None,
            crash_text: Some(text),
            ..classified(DropCategory::Crash, DropKind::Other)
        };
    }
    Classified {
        keeps_reason: true,
        ..classified(DropCategory::Unknown, DropKind::Other)
    }
}

fn cut(text: &str, bytes: usize) -> &str {
    let mut end = text.len().min(bytes);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn quit_kind(lower: &str) -> DropKind {
    QUIT_TEXTS
        .iter()
        .find(|(text, _)| lower.starts_with(text))
        .map_or(DropKind::Other, |(_, kind)| *kind)
}

/// The crash's own text when the reason has a crash's shape: one of the translated prefixes, or,
/// in a language Observer does not know, what only a crash says (a code name with its module, a
/// module and offset, an error code).
fn crash_text(reason: &str, lower: &str) -> Option<String> {
    let reason = reason.trim();
    let after = if CRASH_PREFIXES.iter().any(|prefix| lower.contains(prefix)) {
        (0..reason.len())
            .filter(|&at| reason.is_char_boundary(at))
            .find_map(|at| {
                CRASH_PREFIXES
                    .iter()
                    .find_map(|prefix| strip_lowercase(&reason[at..], prefix))
            })?
    } else if crash_shaped(reason) {
        reason.split_once(": ").map_or(reason, |(_, rest)| rest)
    } else {
        return None;
    };
    let text = match EXCEPTION_PREFIXES
        .iter()
        .find_map(|prefix| strip_lowercase(after, prefix))
    {
        Some(rest) => format!("Unhandled exception: {rest}"),
        None => after.to_owned(),
    };
    Some(text)
}

/// `text` without a prefix that lowercases to `prefix`. The comparison runs on the original
/// characters, whose lowercase may be longer (`İ` gives two).
fn strip_lowercase<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let mut lowered = String::with_capacity(prefix.len());
    for (at, c) in text.char_indices() {
        if lowered.len() >= prefix.len() {
            return (lowered == prefix).then(|| &text[at..]);
        }
        lowered.extend(c.to_lowercase());
        if !prefix.starts_with(&lowered) {
            return None;
        }
    }
    (lowered == prefix).then_some("")
}

fn crash_shaped(reason: &str) -> bool {
    let lower = reason.to_ascii_lowercase();
    lower.contains(".exe!") || lower.contains(".dll+") || has_error_code(reason) || has_code_name(reason)
}

/// `ERR_` followed by capitals, digits and underscores, as the game's error codes are.
fn has_error_code(reason: &str) -> bool {
    reason.match_indices("ERR_").any(|(at, _)| {
        let rest = &reason[at + 4..];
        let code = rest
            .bytes()
            .take_while(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || *byte == b'_')
            .count();
        code >= 3
    })
}

/// A crash's code name and blamed module: `kilo-ohio-lemon (GTA5_b3095.exe+1A2B3C)`.
fn has_code_name(reason: &str) -> bool {
    reason.match_indices(" (").any(|(at, _)| {
        let name = reason[..at].rsplit(' ').next().unwrap_or("");
        let words: Vec<&str> = name.split('-').collect();
        let named = words.len() == 3
            && words
                .iter()
                .all(|word| !word.is_empty() && word.bytes().all(|byte| byte.is_ascii_lowercase()));
        let module = reason[at + 2..].split(')').next().unwrap_or("");
        named && module.contains('+')
    })
}

/// The ranges of the Player drops page. Each ends now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum DropsRange {
    #[serde(rename = "24h")]
    Day,
    #[serde(rename = "7d")]
    Week,
    #[serde(rename = "30d")]
    Month,
    #[serde(rename = "90d")]
    Quarter,
}

impl DropsRange {
    pub const ALL: [Self; 4] = [Self::Day, Self::Week, Self::Month, Self::Quarter];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Day => "24h",
            Self::Week => "7d",
            Self::Month => "30d",
            Self::Quarter => "90d",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|range| range.name() == name)
    }

    pub const fn days(self) -> i64 {
        match self {
            Self::Day => 1,
            Self::Week => 7,
            Self::Month => 30,
            Self::Quarter => 90,
        }
    }

    /// The chart's bucket, in seconds: an hour up to a week, a day beyond.
    pub const fn step(self) -> i64 {
        match self {
            Self::Day | Self::Week => 3600,
            Self::Month | Self::Quarter => 86_400,
        }
    }
}

/// What a comparison of two windows says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum TrendVerdict {
    Rising,
    Falling,
    NoClearChange,
    NotEnoughDrops,
}

/// A rate now against the rate before, with what the test made of it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Trend {
    pub verdict: TrendVerdict,
    /// The change, `0.38` for +38 %, and its 95 % interval.
    pub change: Option<f64>,
    pub low: Option<f64>,
    pub high: Option<f64>,
    /// Drops per 100 player-hours now and before.
    pub now: Option<f64>,
    pub before: Option<f64>,
    /// Drops counted now and before, an incident counting once.
    pub events_now: u64,
    pub events_before: u64,
}

/// One of the four tiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum TileKind {
    Crashes,
    Timeouts,
    Kicks,
    /// Every way a player's session ends badly: timeouts, crashes, security and unknown drops.
    Unexpected,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DropTile {
    pub kind: TileKind,
    pub drops: i64,
    /// Drops per 100 player-hours.
    pub rate: Option<f64>,
    /// "1 session in N" ended this way.
    pub one_in: Option<f64>,
    pub trend: Trend,
    /// The rate of each bucket of the range, oldest first, for the sparkline.
    pub spark: Vec<Option<f64>>,
    /// When one player made more than 30 % of these drops: their share, never who.
    pub one_player_share: Option<f64>,
}

/// A category in the breakdown.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CategoryShare {
    pub category: DropCategory,
    /// Quits, kicks and server stops are expected; the others are not.
    pub expected: bool,
    pub drops: i64,
    /// Of every drop in the range.
    pub share: f64,
    pub rate: Option<f64>,
    pub trend: Trend,
    /// Of these drops, the share of players still loading, when FXServer could tell.
    pub loading_share: Option<f64>,
}

/// A player who crashed again and again: always counted, named only for `PLAYERS_VIEW`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RepeatCrasher {
    pub crashes: i64,
    pub player: Option<DropPlayer>,
}

/// A player as the drop pages name them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DropPlayer {
    /// The profile's id, for its page.
    pub id: i64,
    pub license: String,
    pub name: Option<String>,
}

/// How much of what happened Observer saw: sessions it closed itself because their drop never came.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Coverage {
    pub sessions: i64,
    /// Gone from FXServer's list without a drop.
    pub missed: i64,
    /// Left open when Observer stopped.
    pub lost: i64,
}

/// `GET /servers/default/drops/summary`.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DropsSummary {
    pub range: DropsRange,
    /// The schedule's time zone, in which days start.
    pub zone: String,
    pub from: Timestamp,
    pub to: Timestamp,
    /// The trends compare from here to `to` with the window before: a day's, today since midnight
    /// with yesterday's same hours.
    pub trend_from: Timestamp,
    pub before_from: Timestamp,
    pub before_to: Timestamp,
    pub tiles: Vec<DropTile>,
    pub categories: Vec<CategoryShare>,
    pub played_ms: i64,
    pub sessions: i64,
    /// Players with at least 3 crashes in the last 7 days.
    pub repeat_crashers: Vec<RepeatCrasher>,
    pub coverage: Coverage,
}

/// What the chart shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ChartCategory {
    Unexpected,
    Crash,
    Timeout,
    Kick,
    Security,
}

impl ChartCategory {
    pub const ALL: [Self; 5] = [
        Self::Unexpected,
        Self::Crash,
        Self::Timeout,
        Self::Kick,
        Self::Security,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Unexpected => "unexpected",
            Self::Crash => "crash",
            Self::Timeout => "timeout",
            Self::Kick => "kick",
            Self::Security => "security",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|category| category.name() == name)
    }
}

/// `GET /servers/default/drops/series`: buckets from `from`, `step` seconds each.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DropSeries {
    pub range: DropsRange,
    pub category: ChartCategory,
    /// Unix seconds.
    pub from: i64,
    pub step: i64,
    /// Drops per 100 player-hours; `null` where nobody played, which the chart leaves as a gap.
    pub rates: Vec<Option<f64>>,
    /// The median rate of the same hour (or weekday) over the 4 weeks before.
    pub baseline: Vec<Option<f64>>,
    pub drops: Vec<i64>,
    pub markers: Vec<RunMarker>,
    /// FXServer builds that changed at a start.
    pub changes: Vec<ChangeMarker>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChangeMarker {
    /// Unix seconds.
    pub at: i64,
    pub build: Option<u32>,
}

/// How a crash text was turned into its signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum CrashForm {
    /// A code name and the blamed module's offset.
    Hash,
    Symbol,
    Error,
    /// An exception while loading a resource's asset.
    Load,
    Exception,
    /// Anything else the client wrote.
    Text,
}

impl CrashForm {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "hash" => Self::Hash,
            "symbol" => Self::Symbol,
            "error" => Self::Error,
            "load" => Self::Load,
            "exception" => Self::Exception,
            "text" => Self::Text,
            _ => return None,
        })
    }

    /// Written by the client: shown to statistics viewers only once two players produced it.
    pub const fn free_text(self) -> bool {
        matches!(self, Self::Exception | Self::Text)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SignatureEntry {
    pub id: i64,
    /// `None` when a client's free text only one player produced is hidden from this viewer.
    pub signature: Option<String>,
    pub form: CrashForm,
    pub label: Option<String>,
    pub module: Option<String>,
    pub game_build: Option<u32>,
    /// For a crash while loading an asset.
    pub resource: Option<String>,
    pub drops: i64,
    /// Of every crash in the range.
    pub share: f64,
    pub players: i64,
    pub first_seen: Timestamp,
    /// First seen in the range, after 28 days without it.
    pub new: bool,
    pub loading_share: Option<f64>,
    pub trend: Trend,
    /// The latest redacted crash texts; none when the signature is hidden.
    pub examples: Vec<String>,
}

/// Crashes gathered by blamed module or by game build.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CrashGroup {
    /// The module's name, or the build's number; `None` for crashes that name none.
    pub key: Option<String>,
    pub drops: i64,
    pub share: f64,
    pub signatures: i64,
}

/// `GET /servers/default/drops/signatures`.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SignatureTable {
    pub range: DropsRange,
    pub crashes: i64,
    pub signatures: Vec<SignatureEntry>,
    pub modules: Vec<CrashGroup>,
    pub builds: Vec<CrashGroup>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ReasonGroup {
    /// The reason with its numbers, names and quoted words taken out.
    pub template: String,
    pub kicks: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ResourceEntry {
    /// `None` for kicks no resource is named for.
    pub resource: Option<String>,
    pub kicks: i64,
    pub share: f64,
    pub players: i64,
    pub trend: Trend,
    /// Only for `PLAYERS_VIEW`: what the resource said.
    pub reasons: Option<Vec<ReasonGroup>>,
}

/// `GET /servers/default/drops/resources`.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ResourceTable {
    pub range: DropsRange,
    pub kicks: i64,
    pub resources: Vec<ResourceEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ChangeKind {
    /// FXServer started again after a stop or a restart.
    Restart,
    /// A start on another FXServer build.
    Build,
    /// FXServer crashed or hung, and dropped its players.
    ServerCrash,
    /// A burst of drops.
    Incident,
}

/// A category's rate in the 72 hours before a change and the 72 after.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BeforeAfter {
    pub category: DropCategory,
    pub trend: Trend,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChangeEvent {
    pub at: Timestamp,
    pub kind: ChangeKind,
    /// The run it belongs to, for the log's filter.
    pub run: i64,
    pub generation: u32,
    pub build_from: Option<u32>,
    pub build_to: Option<u32>,
    /// The players the server's crash, or the incident, dropped.
    pub players_dropped: Option<i64>,
    pub hang: bool,
    /// An incident's drops by category, and the longest hitch of its seconds.
    pub incident_categories: Vec<CategoryCount>,
    pub hitch_ms: Option<i64>,
    pub players_online: Option<i64>,
    /// For a build change: the unexpected categories before and after.
    pub before_after: Vec<BeforeAfter>,
    /// Signatures first seen in the 72 hours after a build change.
    pub new_signatures: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CategoryCount {
    pub category: DropCategory,
    pub drops: i64,
}

/// `GET /servers/default/drops/changes`.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DropChanges {
    pub range: DropsRange,
    /// Whether file changes' snapshots feed the timeline. Until then it holds builds, restarts,
    /// server crashes and incidents only.
    pub snapshots: bool,
    /// Newest first.
    pub events: Vec<ChangeEvent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum UptimeVerdict {
    /// Significantly above the rest of the run's time.
    Higher,
    Usual,
    NotEnoughDrops,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UptimeBin {
    /// Hours since the run started.
    pub from_hours: u32,
    pub to_hours: Option<u32>,
    pub drops: i64,
    pub played_ms: i64,
    pub rate: Option<f64>,
    pub verdict: UptimeVerdict,
}

/// `GET /servers/default/drops/uptime`.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DropUptime {
    pub range: DropsRange,
    pub bins: Vec<UptimeBin>,
}

/// A row of the drop log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DropLogRow {
    pub id: i64,
    pub at: Timestamp,
    /// `None` when the identity was never confirmed, or the player was forgotten.
    pub player: Option<DropPlayer>,
    pub category: DropCategory,
    pub kind: Option<DropKind>,
    pub resource: Option<String>,
    pub signature: Option<String>,
    pub label: Option<String>,
    /// The redacted reason a resource gave, or an unknown drop's text.
    pub reason: Option<String>,
    pub length_ms: Option<i64>,
    pub loading: Option<bool>,
    pub run: i64,
    pub generation: u32,
    /// Closed by Observer rather than reported.
    pub synthesized: bool,
    pub incident: bool,
}

/// `GET /servers/default/drops/log`: newest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DropLog {
    pub rows: Vec<DropLogRow>,
    /// The `before` of the next page.
    pub next: Option<String>,
}

/// `GET /servers/default/drops/crash`: the players a crash of FXServer dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CrashDrops {
    pub run: i64,
    pub players: i64,
}

/// How long players' sessions and their texts are kept, in days.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DropsSettings {
    pub session_days: u32,
}

impl DropsSettings {
    pub const LEAST_DAYS: u32 = 14;
    pub const MOST_DAYS: u32 = 365;

    pub fn check(&self) -> bool {
        (Self::LEAST_DAYS..=Self::MOST_DAYS).contains(&self.session_days)
    }

    /// A value edited by hand out of its range, brought back into it.
    #[must_use]
    pub fn clamped(self) -> Self {
        Self {
            session_days: self.session_days.clamp(Self::LEAST_DAYS, Self::MOST_DAYS),
        }
    }
}

impl Default for DropsSettings {
    fn default() -> Self {
        Self { session_days: 90 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drop(reason: &str, category: Option<i64>) -> Classified {
        classify(ReportedDrop {
            reason,
            resource: None,
            category,
        })
    }

    fn summary(classified: &Classified) -> (DropCategory, Option<DropKind>) {
        (classified.category, classified.kind)
    }

    #[test]
    fn fivems_category_comes_before_the_text() {
        let cases = [
            (2, "Exiting", DropCategory::Quit, Some(DropKind::Exit)),
            (2, "Disconnected.", DropCategory::Quit, Some(DropKind::Disconnect)),
            (
                2,
                "Connecting to another server.",
                DropCategory::Quit,
                Some(DropKind::OtherServer),
            ),
            (2, "Reconnecting", DropCategory::Quit, Some(DropKind::Reconnect)),
            (
                2,
                "Entering Rockstar Editor",
                DropCategory::Quit,
                Some(DropKind::Editor),
            ),
            (2, "Reloading game.", DropCategory::Quit, Some(DropKind::Reload)),
            (
                2,
                "Quit: see you",
                DropCategory::Quit,
                Some(DropKind::QuitMessage),
            ),
            (2, "Bye", DropCategory::Quit, Some(DropKind::Other)),
            (
                3,
                "Connection timed out",
                DropCategory::Security,
                Some(DropKind::ServerInitiated),
            ),
            (4, "Exiting", DropCategory::Timeout, Some(DropKind::Replaced)),
            (
                5,
                "Server->client connection timed out.",
                DropCategory::Timeout,
                Some(DropKind::Connection),
            ),
            (6, "x", DropCategory::Timeout, Some(DropKind::PendingCommands)),
            (7, "Server shutting down: restart", DropCategory::ServerStop, None),
            (8, "x", DropCategory::Security, Some(DropKind::StateBagRate)),
            (9, "x", DropCategory::Security, Some(DropKind::NetEventRate)),
            (
                10,
                "x",
                DropCategory::Security,
                Some(DropKind::LatentNetEventRate),
            ),
            (11, "x", DropCategory::Security, Some(DropKind::CommandRate)),
            (12, "x", DropCategory::Timeout, Some(DropKind::MissedFrames)),
            (13, "x", DropCategory::Security, Some(DropKind::NetGameEventRate)),
            (14, "x", DropCategory::Unknown, Some(DropKind::Other)),
            (0, "Exiting", DropCategory::Unknown, Some(DropKind::Other)),
        ];

        for (category, reason, expected, kind) in cases {
            assert_eq!(
                summary(&drop(reason, Some(category))),
                (expected, kind),
                "{category} {reason}"
            );
        }
    }

    #[test]
    fn a_kick_names_the_resource_unless_fxserver_made_it() {
        let kick = |resource| {
            classify(ReportedDrop {
                reason: "You were kicked",
                resource,
                category: Some(1),
            })
        };

        assert_eq!(kick(Some("anticheat")).kicker.as_deref(), Some("anticheat"));
        assert_eq!(kick(Some("__cfx_internal:server")).kicker, None);
        assert_eq!(kick(Some(" ")).kicker, None);
        assert_eq!(kick(None).kicker, None);
        assert!(kick(Some("anticheat")).keeps_reason);
        assert_eq!(
            summary(&kick(None)),
            (DropCategory::Kick, Some(DropKind::Resource))
        );
    }

    #[test]
    fn a_crash_keeps_its_text_after_the_prefix() {
        let crash = drop("Game crashed: kilo-ohio-lemon (GTA5_b3095.exe+1A2B3C)", Some(2));

        assert_eq!(summary(&crash), (DropCategory::Crash, None));
        assert_eq!(
            crash.crash_text.as_deref(),
            Some("kilo-ohio-lemon (GTA5_b3095.exe+1A2B3C)")
        );
    }

    #[test]
    fn every_translated_prefix_is_a_crash() {
        for prefix in CRASH_PREFIXES {
            let reason = format!("{prefix}ERR_GFX_D3D_INIT");

            let crash = drop(&reason, Some(2));

            assert_eq!(crash.category, DropCategory::Crash, "{prefix}");
            assert_eq!(crash.crash_text.as_deref(), Some("ERR_GFX_D3D_INIT"), "{prefix}");
        }
    }

    #[test]
    fn a_translated_exception_comes_back_in_english() {
        let crash = drop(
            "Le jeu a cessé de fonctionner : Exception non-gérée : std::bad_alloc",
            Some(2),
        );

        assert_eq!(
            crash.crash_text.as_deref(),
            Some("Unhandled exception: std::bad_alloc")
        );
    }

    #[test]
    fn a_prefix_whose_lowercase_is_longer_is_still_cut_in_the_right_place() {
        let crash = drop("Oyun çöktü: İşlenemeyen özel durum: std::bad_alloc", Some(2));

        assert_eq!(
            crash.crash_text.as_deref(),
            Some("Unhandled exception: std::bad_alloc")
        );
    }

    #[test]
    fn a_crash_in_an_unknown_locale_is_still_a_crash() {
        for reason in [
            "Gra uległa awarii: kilo-ohio-lemon (GTA5_b3095.exe+1A2B3C)",
            "Игра вылетела: GTA5_b3095.exe!sub_140A1B2C3 (0x1c)",
            "Hra spadla: ERR_MEM_EMBEDDEDALLOC_ALLOC",
            "Peli kaatui: gta-streaming-five.dll+4A2B1",
        ] {
            let crash = drop(reason, Some(2));

            assert_eq!(crash.category, DropCategory::Crash, "{reason}");
            assert!(!crash.crash_text.unwrap().contains(": "), "{reason}");
        }
    }

    #[test]
    fn a_quit_message_that_mentions_a_crash_is_a_quit() {
        let quit = drop("Quit: Game crashed: you all suck", Some(2));

        assert_eq!(summary(&quit), (DropCategory::Quit, Some(DropKind::QuitMessage)));
        assert_eq!(quit.crash_text, None);
    }

    #[test]
    fn a_long_kicking_resource_name_is_cut_on_a_character_boundary() {
        let name = "é".repeat(300);

        let kick = classify(ReportedDrop {
            reason: "Kicked",
            resource: Some(&name),
            category: Some(1),
        });

        assert_eq!(kick.kicker.unwrap().len(), MAX_RESOURCE_BYTES);
    }

    #[test]
    fn an_ordinary_text_with_brackets_is_not_a_crash() {
        assert_eq!(
            drop("Left (see you soon+1)", Some(2)).category,
            DropCategory::Quit
        );
        assert_eq!(drop("ERR_", Some(2)).category, DropCategory::Quit);
    }

    #[test]
    fn without_a_category_the_text_decides() {
        let cases = [
            ("Exiting", DropCategory::Quit, Some(DropKind::Exit)),
            ("Server shutting down: restart", DropCategory::ServerStop, None),
            (
                "Disconnected by server: banned",
                DropCategory::Security,
                Some(DropKind::ServerInitiated),
            ),
            (
                "Server->client connection timed out. Last seen 2 msec ago.",
                DropCategory::Timeout,
                Some(DropKind::Connection),
            ),
            (
                "Timed out after 60 seconds (1)",
                DropCategory::Timeout,
                Some(DropKind::MissedFrames),
            ),
            (
                "Reliable network event overflow.",
                DropCategory::Security,
                Some(DropKind::ServerInitiated),
            ),
            ("Game crashed: ERR_GFX_D3D_INIT", DropCategory::Crash, None),
            ("Something new", DropCategory::Unknown, Some(DropKind::Other)),
        ];

        for (reason, category, kind) in cases {
            assert_eq!(summary(&drop(reason, None)), (category, kind), "{reason}");
        }
        assert!(drop("Something new", None).keeps_reason);
    }

    #[test]
    fn names_round_trip() {
        for category in DropCategory::ALL {
            assert_eq!(DropCategory::from_name(category.as_str()), Some(category));
        }
        for kind in DropKind::ALL {
            assert_eq!(DropKind::from_name(kind.as_str()), Some(kind));
            assert_eq!(
                serde_json::to_value(kind).unwrap(),
                serde_json::Value::from(kind.as_str())
            );
        }
    }
}
