//! What the bridge reports about players: a join, a drop, the list of players after it starts, and
//! the labels the server's resources give them.
//! The reports go to the players service on a bus of their own, never with the supervisor's events,
//! which are printed and logged: a join carries the player's address.
//!
//! Every field is optional when it is read, and each identifier is taken or dropped on its own, so
//! one odd element never loses a whole report. What is dropped is counted, never kept.

use std::fmt;
use std::time::Instant;

use serde_json::Value;

use crate::bridge::REPORT_TYPE;
use crate::health::is_address;
use crate::trace::TraceKind;

/// More than any player shows: FiveM gives about ten identifiers and a dozen tokens, none longer
/// than 70 bytes.
const MAX_IDENTIFIERS: usize = 64;
const MAX_TOKENS: usize = 64;
const MAX_IDENTIFIER_BYTES: usize = 128;
/// The longest text kept before Observer's own limits apply.
const MAX_TEXT_BYTES: usize = 4096;
/// The bridge sends at most 100 players a page.
const MAX_PAGE: usize = 200;
/// Past any map: a position beyond is not one the game holds.
const MAX_COORDINATE: f64 = 100_000.0;

/// A report and when Observer received it.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerReport {
    pub generation: u32,
    /// Unix milliseconds.
    pub at_ms: i64,
    /// The same instant on the monotonic clock, which bounds what the game timer may claim.
    pub received: Instant,
    pub kind: PlayerReportKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PlayerReportKind {
    Join(Joined),
    Drop(Dropped),
    Roster(RosterPage),
    /// The latest label of each player whose label changed in the last second.
    Labels(Vec<LabelChange>),
    /// A player connecting, whom the bridge holds until Observer's verdict on `request`.
    Connecting(Connecting),
    /// A page of where the players stand, while Observer asks.
    Positions(PositionsPage),
}

#[derive(Debug, Clone, PartialEq)]
pub struct PositionsPage {
    /// From 1.
    pub page: u32,
    pub last: bool,
    /// Without OneSync the server has no position, and the page none.
    pub onesync: bool,
    pub players: Vec<Position>,
}

/// Where a player stands, in game units, and which way they face, in degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Position {
    pub net_id: u32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub heading: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Connecting {
    pub request: u32,
    pub shown: Shown,
}

/// A player who joined: their net id, FXServer's game timer, and what they showed.
#[derive(Debug, Clone, PartialEq)]
pub struct Joined {
    pub net_id: u32,
    /// Milliseconds since FXServer started.
    pub timer_ms: Option<i64>,
    pub shown: Shown,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Shown {
    /// `kind:value` as FXServer gives them, `ip:` left out.
    pub identifiers: Vec<String>,
    pub address: Option<Address>,
    pub tokens: Vec<String>,
    pub name: Option<String>,
    /// Elements that were not strings, or past the limits.
    pub dropped: usize,
}

/// A player's `ip:` identifier. It prints as `[ip]` and cannot be serialised: it reaches no log
/// and no response before the players service turns it into a keyed fingerprint.
#[derive(Clone, PartialEq, Eq)]
pub struct Address(String);

impl Address {
    pub fn new(identifier: impl Into<String>) -> Self {
        Self(identifier.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[ip]")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dropped {
    pub net_id: u32,
    pub timer_ms: Option<i64>,
    pub reason: String,
    /// The resource FXServer names for the drop.
    pub resource: Option<String>,
    /// FiveM's drop category, absent on very old builds.
    pub category: Option<i64>,
    /// Whether the player's ped existed, with OneSync only.
    pub ped: Option<bool>,
}

/// A label a resource gave a player through the bridge, or took away. The texts are as the resource
/// wrote them, within the bridge's limits: the players service cleans them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelChange {
    pub net_id: u32,
    /// `None` once cleared.
    pub label: Option<GivenLabel>,
    /// The resource that called the bridge.
    pub resource: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GivenLabel {
    pub name: Option<String>,
    pub id: Option<String>,
}

/// One page of the players listed right after the bridge started.
#[derive(Debug, Clone, PartialEq)]
pub struct RosterPage {
    pub timer_ms: Option<i64>,
    /// From 1.
    pub page: u32,
    pub last: bool,
    pub players: Vec<(u32, Shown)>,
    /// Entries without a net id.
    pub dropped: usize,
}

impl PlayerReportKind {
    /// The bridge's player report in a trace, if the trace is one. Like the other reports, only
    /// the bridge of this start, named `bridge`, may send it.
    pub fn from_trace(trace: &TraceKind, bridge: &str) -> Option<Self> {
        let TraceKind::Script { resource, payload } = trace else {
            return None;
        };
        if resource != bridge || payload.get("type").and_then(Value::as_str) != Some(REPORT_TYPE) {
            return None;
        }
        match payload.get("kind").and_then(Value::as_str)? {
            "join" => Some(Self::Join(Joined {
                net_id: net_id(payload.get("id"))?,
                timer_ms: timer(payload),
                shown: shown(payload),
            })),
            "drop" => Some(Self::Drop(Dropped {
                net_id: net_id(payload.get("id"))?,
                timer_ms: timer(payload),
                reason: text(payload.get("reason")).unwrap_or_default(),
                resource: text(payload.get("resource")),
                category: payload.get("category").and_then(Value::as_i64),
                ped: payload.get("ped").and_then(Value::as_bool),
            })),
            "players" => {
                let entries = payload.get("players").and_then(Value::as_array);
                let mut players = Vec::new();
                let mut dropped = 0;
                for entry in entries.into_iter().flatten() {
                    match net_id(entry.get("id")) {
                        Some(id) if players.len() < MAX_PAGE => players.push((id, shown(entry))),
                        _ => dropped += 1,
                    }
                }
                Some(Self::Roster(RosterPage {
                    timer_ms: timer(payload),
                    page: payload
                        .get("page")
                        .and_then(Value::as_u64)
                        .and_then(|page| u32::try_from(page).ok())?,
                    last: payload.get("last").and_then(Value::as_bool).unwrap_or(false),
                    players,
                    dropped,
                }))
            }
            "positions" => Some(Self::Positions(PositionsPage {
                page: payload
                    .get("page")
                    .and_then(Value::as_u64)
                    .and_then(|page| u32::try_from(page).ok())?,
                last: payload.get("last").and_then(Value::as_bool).unwrap_or(false),
                onesync: payload.get("onesync").and_then(Value::as_bool).unwrap_or(false),
                players: payload
                    .get("players")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(position)
                    .take(MAX_PAGE)
                    .collect(),
            })),
            "connecting" => Some(Self::Connecting(Connecting {
                request: payload
                    .get("request")
                    .and_then(Value::as_u64)
                    .and_then(|request| u32::try_from(request).ok())?,
                shown: shown(payload),
            })),
            "labels" => Some(Self::Labels(
                payload
                    .get("labels")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|entry| {
                        Some(LabelChange {
                            net_id: net_id(entry.get("id"))?,
                            label: entry.get("label").filter(|label| label.is_object()).map(|label| {
                                GivenLabel {
                                    name: text(label.get("name")),
                                    id: text(label.get("id")),
                                }
                            }),
                            resource: text(entry.get("resource")),
                        })
                    })
                    .take(MAX_PAGE)
                    .collect(),
            )),
            _ => None,
        }
    }
}

/// A position the game could hold: finite, and within a margin of any map's size.
fn position(entry: &Value) -> Option<Position> {
    let coordinate = |key| {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "positions in tenths of a game unit"
        )]
        entry
            .get(key)
            .and_then(Value::as_f64)
            .filter(|value| value.is_finite() && value.abs() <= MAX_COORDINATE)
            .map(|value| value as f32)
    };
    Some(Position {
        net_id: net_id(entry.get("id"))?,
        x: coordinate("x")?,
        y: coordinate("y")?,
        z: coordinate("z")?,
        heading: coordinate("h").unwrap_or(0.0),
    })
}

fn net_id(value: Option<&Value>) -> Option<u32> {
    value
        .and_then(Value::as_u64)
        .and_then(|id| u32::try_from(id).ok())
        .filter(|&id| id > 0)
}

fn timer(payload: &Value) -> Option<i64> {
    payload.get("t").and_then(Value::as_i64).filter(|&t| t >= 0)
}

fn text(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?;
    let mut end = text.len().min(MAX_TEXT_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    Some(text[..end].to_owned())
}

fn shown(entry: &Value) -> Shown {
    let mut shown = Shown {
        name: text(entry.get("name")),
        ..Shown::default()
    };
    for value in entry
        .get("identifiers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        match value.as_str() {
            Some(identifier) if identifier.len() > MAX_IDENTIFIER_BYTES => shown.dropped += 1,
            Some(identifier) if is_address(identifier) && shown.address.is_none() => {
                shown.address = Some(Address(identifier.to_owned()));
            }
            Some(identifier) if !is_address(identifier) && shown.identifiers.len() < MAX_IDENTIFIERS => {
                shown.identifiers.push(identifier.to_owned());
            }
            _ => shown.dropped += 1,
        }
    }
    for value in entry
        .get("tokens")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        match value.as_str() {
            Some(token) if token.len() <= MAX_IDENTIFIER_BYTES && shown.tokens.len() < MAX_TOKENS => {
                shown.tokens.push(token.to_owned());
            }
            _ => shown.dropped += 1,
        }
    }
    shown
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const BRIDGE: &str = "sys_k3jx7qmw2pza";

    fn report(resource: &str, payload: Value) -> Option<PlayerReportKind> {
        PlayerReportKind::from_trace(
            &TraceKind::Script {
                resource: resource.to_owned(),
                payload,
            },
            BRIDGE,
        )
    }

    #[test]
    fn a_join_keeps_the_address_apart_and_never_prints_it() {
        let join = report(
            BRIDGE,
            json!({
                "type": "observer", "kind": "join", "id": 7, "t": 120_000,
                "identifiers": ["license:abc", "ip:192.0.2.7", "discord:1", 4, null],
                "tokens": ["4:ab", {}], "name": "Ada",
            }),
        );

        let Some(PlayerReportKind::Join(joined)) = join else {
            panic!("not a join: {join:?}");
        };
        assert_eq!(joined.net_id, 7);
        assert_eq!(joined.timer_ms, Some(120_000));
        assert_eq!(joined.shown.identifiers, ["license:abc", "discord:1"]);
        assert_eq!(
            joined.shown.address.as_ref().map(Address::expose),
            Some("ip:192.0.2.7")
        );
        assert_eq!(joined.shown.tokens, ["4:ab"]);
        assert_eq!(joined.shown.name.as_deref(), Some("Ada"));
        assert_eq!(joined.shown.dropped, 3);
        assert!(!format!("{joined:?}").contains("192.0.2.7"));
    }

    #[test]
    fn a_drop_takes_what_it_can_read() {
        assert_eq!(
            report(
                BRIDGE,
                json!({ "type": "observer", "kind": "drop", "id": 7, "t": 5,
                        "reason": "Exiting", "resource": "", "category": 2, "ped": true }),
            ),
            Some(PlayerReportKind::Drop(Dropped {
                net_id: 7,
                timer_ms: Some(5),
                reason: "Exiting".to_owned(),
                resource: Some(String::new()),
                category: Some(2),
                ped: Some(true),
            }))
        );
        assert_eq!(
            report(
                BRIDGE,
                json!({ "type": "observer", "kind": "drop", "id": 7, "reason": 3 })
            ),
            Some(PlayerReportKind::Drop(Dropped {
                net_id: 7,
                timer_ms: None,
                reason: String::new(),
                resource: None,
                category: None,
                ped: None,
            }))
        );
    }

    #[test]
    fn a_report_without_a_net_id_is_none() {
        for id in [
            json!(0),
            json!(-1),
            json!("7"),
            json!(null),
            json!(5_000_000_000_u64),
        ] {
            assert_eq!(
                report(BRIDGE, json!({ "type": "observer", "kind": "drop", "id": id })),
                None
            );
        }
    }

    #[test]
    fn a_roster_page_keeps_numbered_players() {
        let page = report(
            BRIDGE,
            json!({ "type": "observer", "kind": "players", "t": 9, "page": 2, "last": true,
                    "players": [{ "id": 3, "identifiers": ["license:abc"], "name": "Grace" }, { "name": "?" }] }),
        );

        let Some(PlayerReportKind::Roster(page)) = page else {
            panic!("not a roster page");
        };
        assert_eq!((page.page, page.last, page.timer_ms), (2, true, Some(9)));
        assert_eq!(page.players.len(), 1);
        assert_eq!(page.players[0].0, 3);
        assert_eq!(page.dropped, 1);
    }

    #[test]
    fn labels_are_given_or_cleared_per_net_id() {
        let labels = report(
            BRIDGE,
            json!({ "type": "observer", "kind": "labels", "labels": [
                { "id": 3, "label": { "name": "John Doe", "id": "CHAR-1234" }, "resource": "characters" },
                { "id": 4, "label": { "id": 12 }, "resource": "characters" },
                { "id": 5, "resource": "characters" },
                { "label": { "name": "nobody" } },
            ] }),
        );

        assert_eq!(
            labels,
            Some(PlayerReportKind::Labels(vec![
                LabelChange {
                    net_id: 3,
                    label: Some(GivenLabel {
                        name: Some("John Doe".to_owned()),
                        id: Some("CHAR-1234".to_owned()),
                    }),
                    resource: Some("characters".to_owned()),
                },
                LabelChange {
                    net_id: 4,
                    label: Some(GivenLabel::default()),
                    resource: Some("characters".to_owned()),
                },
                LabelChange {
                    net_id: 5,
                    label: None,
                    resource: Some("characters".to_owned()),
                },
            ]))
        );
    }

    #[test]
    fn positions_keep_what_the_game_could_hold() {
        let page = report(
            BRIDGE,
            json!({
                "type": "observer", "kind": "positions", "t": 1, "page": 1, "last": true, "onesync": true,
                "players": [
                    { "id": 3, "x": 412.6, "y": -1208.3, "z": 32.1, "h": 90.0 },
                    { "id": 4, "x": 1e9, "y": 0, "z": 0 },
                    { "id": 5, "x": "far", "y": 0, "z": 0 },
                    { "x": 1, "y": 2, "z": 3 },
                    { "id": 6, "x": 1, "y": 2, "z": 3 },
                ],
            }),
        );

        let Some(PlayerReportKind::Positions(page)) = page else {
            panic!("not a positions report: {page:?}");
        };
        assert!(page.last && page.onesync);
        assert_eq!(
            page.players,
            [
                Position {
                    net_id: 3,
                    x: 412.6,
                    y: -1208.3,
                    z: 32.1,
                    heading: 90.0
                },
                Position {
                    net_id: 6,
                    x: 1.0,
                    y: 2.0,
                    z: 3.0,
                    heading: 0.0
                },
            ]
        );
    }

    #[test]
    fn a_connecting_player_carries_the_request_to_answer() {
        let token = format!("4:{}", "ab".repeat(32));
        let connecting = report(
            BRIDGE,
            json!({
                "type": "observer", "kind": "connecting", "request": 7, "name": "kaito",
                "identifiers": ["license:abc", "ip:192.0.2.1"], "tokens": [token],
            }),
        );
        let unnumbered = report(
            BRIDGE,
            json!({ "type": "observer", "kind": "connecting", "name": "kaito" }),
        );

        let Some(PlayerReportKind::Connecting(connecting)) = connecting else {
            panic!("not a connecting report: {connecting:?}");
        };
        assert_eq!(connecting.request, 7);
        assert_eq!(connecting.shown.identifiers, ["license:abc"]);
        assert_eq!(connecting.shown.tokens, [token]);
        assert!(connecting.shown.address.is_some());
        assert_eq!(unnumbered, None);
    }

    #[test]
    fn only_the_bridge_of_this_start_reports_players() {
        let join = json!({ "type": "observer", "kind": "join", "id": 7 });
        let labels = json!({ "type": "observer", "kind": "labels", "labels": [{ "id": 7, "label": { "name": "John" } }] });

        assert!(report(BRIDGE, join.clone()).is_some());
        assert!(report(BRIDGE, labels.clone()).is_some());
        for impostor in ["mischief", "observer", "sys_aaaaaaaaaaaa"] {
            assert_eq!(report(impostor, join.clone()), None, "{impostor}");
            assert_eq!(report(impostor, labels.clone()), None, "{impostor}");
        }
        assert_eq!(
            report(BRIDGE, json!({ "type": "other", "kind": "join", "id": 7 })),
            None
        );
        assert_eq!(report(BRIDGE, json!({ "type": "observer", "kind": "hb" })), None);
    }

    #[test]
    fn long_texts_are_cut_on_a_character_boundary() {
        let reason = "é".repeat(MAX_TEXT_BYTES);

        let Some(PlayerReportKind::Drop(dropped)) = report(
            BRIDGE,
            json!({ "type": "observer", "kind": "drop", "id": 1, "reason": reason }),
        ) else {
            panic!("not a drop");
        };

        assert_eq!(dropped.reason.len(), MAX_TEXT_BYTES);
    }
}
