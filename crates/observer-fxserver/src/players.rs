//! What the bridge reports about players: a join, a drop, and the list of players after it starts.
//! The reports go to the players service on a bus of their own, never with the supervisor's events,
//! which are printed and logged: a join carries the player's address.
//!
//! Every field is optional when it is read, and each identifier is taken or dropped on its own, so
//! one odd element never loses a whole report. What is dropped is counted, never kept.

use std::fmt;
use std::time::Instant;

use serde_json::Value;

use crate::bridge::NAME;
use crate::health::is_address;
use crate::trace::TraceKind;

/// More than any player shows: FiveM gives about ten identifiers and a dozen tokens.
const MAX_IDENTIFIERS: usize = 64;
const MAX_TOKENS: usize = 64;
/// The longest text kept before Observer's own limits apply.
const MAX_TEXT_BYTES: usize = 4096;
/// The bridge sends at most 100 players a page.
const MAX_PAGE: usize = 200;

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
    /// the `observer` resource may send it.
    pub fn from_trace(trace: &TraceKind) -> Option<Self> {
        let TraceKind::Script { resource, payload } = trace else {
            return None;
        };
        if resource != NAME || payload.get("type").and_then(Value::as_str) != Some(NAME) {
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
            _ => None,
        }
    }
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
            Some(token) if shown.tokens.len() < MAX_TOKENS => shown.tokens.push(token.to_owned()),
            _ => shown.dropped += 1,
        }
    }
    shown
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn report(resource: &str, payload: Value) -> Option<PlayerReportKind> {
        PlayerReportKind::from_trace(&TraceKind::Script {
            resource: resource.to_owned(),
            payload,
        })
    }

    #[test]
    fn a_join_keeps_the_address_apart_and_never_prints_it() {
        let join = report(
            "observer",
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
                "observer",
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
                "observer",
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
                report(
                    "observer",
                    json!({ "type": "observer", "kind": "drop", "id": id })
                ),
                None
            );
        }
    }

    #[test]
    fn a_roster_page_keeps_numbered_players() {
        let page = report(
            "observer",
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
    fn only_the_bridge_reports_players() {
        let join = json!({ "type": "observer", "kind": "join", "id": 7 });

        assert!(report("observer", join.clone()).is_some());
        assert_eq!(report("mischief", join), None);
        assert_eq!(
            report("observer", json!({ "type": "other", "kind": "join", "id": 7 })),
            None
        );
        assert_eq!(
            report("observer", json!({ "type": "observer", "kind": "hb" })),
            None
        );
    }

    #[test]
    fn long_texts_are_cut_on_a_character_boundary() {
        let reason = "é".repeat(MAX_TEXT_BYTES);

        let Some(PlayerReportKind::Drop(dropped)) = report(
            "observer",
            json!({ "type": "observer", "kind": "drop", "id": 1, "reason": reason }),
        ) else {
            panic!("not a drop");
        };

        assert_eq!(dropped.reason.len(), MAX_TEXT_BYTES);
    }
}
