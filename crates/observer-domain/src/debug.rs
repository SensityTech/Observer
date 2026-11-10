//! What the Debug pages read: the events, the state bag writes and the outgoing HTTP requests
//! the server's resources count themselves, once the owner turned Debug on. Names, counts and
//! sizes over the last minutes, kept in memory: never what an event, a bag or a request carries.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// How far back a page looks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum DebugWindow {
    #[serde(rename = "1m")]
    Minute,
    #[default]
    #[serde(rename = "5m")]
    FiveMinutes,
    #[serde(rename = "15m")]
    QuarterHour,
}

impl DebugWindow {
    pub const fn seconds(self) -> u32 {
        match self {
            Self::Minute => 60,
            Self::FiveMinutes => 300,
            Self::QuarterHour => 900,
        }
    }
}

/// `?window=` of the Debug routes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct DebugQuery {
    #[serde(default)]
    pub window: DebugWindow,
}

/// Where the counting stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DebugState {
    /// On, but FXServer started before it was, or does not run: it counts from the next start.
    Waiting,
    /// The server that runs was started to count.
    Counting,
    /// On, but the build's scheduler was left as it was at this start: nothing is counted.
    Refused,
}

/// What every Debug page says under its title.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DebugStatus {
    pub state: DebugState,
    /// Whether FXServer runs now.
    pub running: bool,
    /// When the first report of this start came in.
    pub since: Option<Timestamp>,
    /// How many resources report.
    pub reporting: u32,
    /// The started resources that do not: in JavaScript or C#, or with no script. The first
    /// hundred by name.
    pub silent: Vec<String>,
    pub silent_count: u32,
    /// Why the scheduler was left as it was, in the host's words.
    pub refused: Option<String>,
}

/// Which way an event went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum EventWay {
    FromClients,
    ToClient,
    ToAll,
    /// Between the server's resources.
    Local,
    /// From the engine, such as `playerJoining`.
    FromEngine,
}

/// A count of events with the bytes they carried.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct Flow {
    pub count: u64,
    pub bytes: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EventTotals {
    pub to_clients: Flow,
    pub from_clients: Flow,
    /// Events received from the server's own resources.
    pub local: u64,
    /// In the handlers of the events received, in microseconds.
    pub handler_us: u64,
    /// Events clients sent that no resource opened to them.
    pub refused: u64,
    pub refused_players: u32,
}

/// The bytes that went to and came from clients in one slice of the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TrafficPoint {
    pub at: Timestamp,
    pub seconds: u32,
    pub to_clients: u64,
    pub from_clients: u64,
}

/// A client that sent an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct EventSender {
    pub player: u32,
    pub count: u64,
    pub bytes: u64,
}

/// An event in one way, over the resources that sent or received it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EventRow {
    pub name: String,
    pub way: EventWay,
    /// The first three by traffic.
    pub resources: Vec<String>,
    pub resource_count: u32,
    pub count: u64,
    pub bytes: u64,
    /// The largest single payload.
    pub largest: u64,
    pub handler_us: u64,
    /// Who sent it most, for an event from clients: five at most.
    pub senders: Vec<EventSender>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EventResourceRow {
    pub resource: String,
    pub to_clients: Flow,
    pub from_clients: Flow,
    /// Events received from the server's resources and from the engine.
    pub local: u64,
    pub handler_us: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct EventPlayerRow {
    pub player: u32,
    pub count: u64,
    pub bytes: u64,
    /// The event they sent most.
    pub top: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct RefusedRow {
    pub name: String,
    /// Null when the scheduler did not say who.
    pub player: Option<u32>,
    pub resource: String,
    pub count: u64,
}

/// `GET /api/debug/events`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct DebugEvents {
    pub status: DebugStatus,
    /// The window's length.
    pub seconds: u32,
    pub totals: EventTotals,
    pub series: Vec<TrafficPoint>,
    /// By traffic, the first two hundred.
    pub events: Vec<EventRow>,
    pub resources: Vec<EventResourceRow>,
    pub players: Vec<EventPlayerRow>,
    pub refused: Vec<RefusedRow>,
    /// A resource or Observer kept less than there was.
    pub truncated: bool,
}

/// The bag a write went to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum BagKind {
    Global,
    Player,
    Entity,
    LocalEntity,
    Other,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BagTotals {
    pub writes: u64,
    pub bytes: u64,
    /// Of the writes that went out to clients.
    pub replicated_bytes: u64,
    pub keys: u32,
    pub resources: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BagRow {
    pub key: String,
    pub kind: BagKind,
    pub resources: Vec<String>,
    pub resource_count: u32,
    pub count: u64,
    pub bytes: u64,
    pub largest: u64,
    /// How many of the writes went out to clients.
    pub replicated: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BagResourceRow {
    pub resource: String,
    pub count: u64,
    pub bytes: u64,
    pub replicated_bytes: u64,
    pub keys: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct LargestWrite {
    /// The bag's own name, such as `player:41`.
    pub bag: String,
    pub key: String,
    pub resource: String,
    pub bytes: u64,
    pub replicated: bool,
}

/// `GET /api/debug/bags`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct DebugBags {
    pub status: DebugStatus,
    pub seconds: u32,
    pub totals: BagTotals,
    pub keys: Vec<BagRow>,
    pub resources: Vec<BagResourceRow>,
    pub largest: Vec<LargestWrite>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HttpTotals {
    pub requests: u64,
    /// Of the request bodies.
    pub bytes: u64,
    pub hosts: u32,
    pub resources: u32,
    /// The hosts no resource had called before, and nobody said they know.
    pub new_hosts: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HttpRow {
    pub host: String,
    pub method: String,
    pub resource: String,
    pub count: u64,
    pub bytes: u64,
    /// When this resource first called this host, as far as Observer saw.
    pub first_called: Option<Timestamp>,
    /// First called after Observer's day of learning, and not marked as known since.
    pub new: bool,
}

/// `GET /api/debug/http`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DebugHttp {
    pub status: DebugStatus,
    pub seconds: u32,
    pub totals: HttpTotals,
    pub requests: Vec<HttpRow>,
    /// Until when every host is taken as known: the first day Debug counted.
    pub learning_until: Option<Timestamp>,
    pub truncated: bool,
}

/// `POST /api/debug/http/known`: the owner knows why this resource calls this host.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct KnownHost {
    pub resource: String,
    pub host: String,
}
