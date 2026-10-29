//! The live updates websocket, `GET /api/ws`: what the panel and Observer say to each other once
//! it is open. Every message is a JSON object with a `type`; the panel follows topics, each with
//! its own permission, and gets a snapshot of the topic, then its events.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::artifacts::ArtifactsStatus;
use crate::audit::AuditEntry;
use crate::error::ErrorCode;
use crate::metrics::{LiveSample, MetricsSnapshot};
use crate::schedule::ScheduleStatus;
use crate::server::{ConsoleBatch, ConsoleSnapshot, ServerStatus};

/// The websocket subprotocol. A change older panels cannot read takes a new one.
pub const PROTOCOL: &str = "observer.v1";
/// Sent in `hello`, so the panel knows it speaks the same contract as this Observer.
pub const CONTRACT: u32 = 1;
/// The close code of a socket whose session ended: signed out, expired, revoked or changed.
pub const CLOSE_SESSION_ENDED: u16 = 4401;

/// `POST /api/auth/ws-ticket`: what opens the websocket, once, within 30 seconds, next to the
/// cookie of the session that asked for it.
#[derive(Serialize, TS)]
#[ts(export)]
pub struct WsTicket {
    pub ticket: String,
}

/// What the panel can follow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Topic {
    /// Observer's own state. Needs the dashboard permission.
    Status,
    /// Audit entries as they are written. Needs the audit log permission.
    Audit,
    /// FXServer under the supervisor. Needs the dashboard permission.
    #[serde(rename = "server.status")]
    ServerStatus,
    /// FXServer's console, resumed from `since`, the number of the last line the panel holds.
    /// Needs the console permission.
    Console,
    /// A sample of the statistics every five seconds. Needs the dashboard permission.
    #[serde(rename = "metrics.live")]
    MetricsLive,
    /// FXServer's restart schedule and its next restart. Needs the dashboard, schedule or server
    /// control permission.
    #[serde(rename = "server.schedule")]
    ServerSchedule,
    /// FXServer's builds, the install job and the probation of a new build. Needs the dashboard
    /// or the artifacts permission.
    #[serde(rename = "server.artifacts")]
    ServerArtifacts,
}

impl Topic {
    pub const ALL: [Self; 7] = [
        Self::Status,
        Self::Audit,
        Self::ServerStatus,
        Self::Console,
        Self::MetricsLive,
        Self::ServerSchedule,
        Self::ServerArtifacts,
    ];
}

/// What the panel sends.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(export)]
pub enum ClientMessage {
    /// Follows a topic, or starts it over. `since` resumes a topic whose events are numbered.
    Sub {
        topic: Topic,
        #[serde(default)]
        #[ts(optional)]
        since: Option<u64>,
    },
    Unsub {
        topic: Topic,
    },
    /// Answered with `pong`, which tells the panel the socket still works.
    Ping,
}

/// What Observer sends.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(export)]
pub enum ServerMessage {
    /// The first message on every socket.
    #[serde(rename_all = "camelCase")]
    Hello {
        connection_id: u64,
        contract: u32,
    },
    /// A topic's whole state, first after each `sub`.
    Snapshot(TopicSnapshot),
    /// What changed in a topic since its snapshot.
    Event(TopicEvent),
    /// The panel fell behind and some events of the topic are gone: it subscribes again for a
    /// fresh snapshot.
    Resync {
        topic: Topic,
    },
    /// The topic stopped, or never started: the session does not hold its permission.
    Unsubscribed {
        topic: Topic,
        reason: UnsubscribeReason,
    },
    Pong,
    /// A message Observer could not act on.
    Error {
        code: ErrorCode,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(tag = "topic", content = "data", rename_all = "camelCase")]
#[ts(export)]
pub enum TopicSnapshot {
    Status(ObserverStatus),
    Audit(AuditSnapshot),
    #[serde(rename = "server.status")]
    ServerStatus(ServerStatus),
    Console(ConsoleSnapshot),
    #[serde(rename = "metrics.live")]
    MetricsLive(MetricsSnapshot),
    #[serde(rename = "server.schedule")]
    ServerSchedule(ScheduleStatus),
    #[serde(rename = "server.artifacts")]
    ServerArtifacts(Box<ArtifactsStatus>),
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(tag = "topic", content = "data", rename_all = "camelCase")]
#[ts(export)]
pub enum TopicEvent {
    Audit(AuditEntry),
    /// The whole status again, whenever any of it changes.
    #[serde(rename = "server.status")]
    ServerStatus(ServerStatus),
    Console(ConsoleBatch),
    #[serde(rename = "metrics.live")]
    MetricsLive(LiveSample),
    /// The whole schedule status again, whenever any of it changes.
    #[serde(rename = "server.schedule")]
    ServerSchedule(ScheduleStatus),
    /// The whole artifacts status again, whenever any of it changes: during a job, at most
    /// four times a second.
    #[serde(rename = "server.artifacts")]
    ServerArtifacts(Box<ArtifactsStatus>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum UnsubscribeReason {
    Forbidden,
}

/// Observer itself, as the status topic shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ObserverStatus {
    pub version: String,
    pub started_at: Timestamp,
}

/// Where the audit log stood when the subscription began. Later entries come as events; a panel
/// that read up to an older entry reads the log again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AuditSnapshot {
    pub latest_id: Option<i64>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn messages_are_objects_typed_by_their_type() {
        let messages = [
            (
                ServerMessage::Hello {
                    connection_id: 7,
                    contract: CONTRACT,
                },
                json!({ "type": "hello", "connectionId": 7, "contract": 1 }),
            ),
            (
                ServerMessage::Snapshot(TopicSnapshot::Audit(AuditSnapshot { latest_id: Some(12) })),
                json!({ "type": "snapshot", "topic": "audit", "data": { "latestId": 12 } }),
            ),
            (
                ServerMessage::Resync { topic: Topic::Audit },
                json!({ "type": "resync", "topic": "audit" }),
            ),
            (
                ServerMessage::Unsubscribed {
                    topic: Topic::Status,
                    reason: UnsubscribeReason::Forbidden,
                },
                json!({ "type": "unsubscribed", "topic": "status", "reason": "forbidden" }),
            ),
            (ServerMessage::Pong, json!({ "type": "pong" })),
            (
                ServerMessage::Error {
                    code: ErrorCode::InvalidRequest,
                },
                json!({ "type": "error", "code": "invalid_request" }),
            ),
        ];

        for (message, expected) in messages {
            assert_eq!(serde_json::to_value(&message).unwrap(), expected);
        }
    }

    #[test]
    fn the_panel_subscribes_by_topic_name_with_an_optional_position() {
        let parse = |text: &str| serde_json::from_str::<ClientMessage>(text).ok();

        assert_eq!(
            parse(r#"{"type":"sub","topic":"audit"}"#),
            Some(ClientMessage::Sub {
                topic: Topic::Audit,
                since: None
            })
        );
        assert_eq!(
            parse(r#"{"type":"sub","topic":"status","since":120344}"#),
            Some(ClientMessage::Sub {
                topic: Topic::Status,
                since: Some(120_344)
            })
        );
        assert_eq!(
            parse(r#"{"type":"unsub","topic":"status"}"#),
            Some(ClientMessage::Unsub { topic: Topic::Status })
        );
        assert_eq!(
            parse(r#"{"type":"sub","topic":"console","since":1761642000000001}"#),
            Some(ClientMessage::Sub {
                topic: Topic::Console,
                since: Some(1_761_642_000_000_001)
            })
        );
        assert_eq!(
            parse(r#"{"type":"sub","topic":"server.status"}"#),
            Some(ClientMessage::Sub {
                topic: Topic::ServerStatus,
                since: None
            })
        );
        assert_eq!(
            parse(r#"{"type":"sub","topic":"metrics.live"}"#),
            Some(ClientMessage::Sub {
                topic: Topic::MetricsLive,
                since: None
            })
        );
        assert_eq!(
            parse(r#"{"type":"sub","topic":"server.schedule"}"#),
            Some(ClientMessage::Sub {
                topic: Topic::ServerSchedule,
                since: None
            })
        );
        assert_eq!(parse(r#"{"type":"ping"}"#), Some(ClientMessage::Ping));
        for invalid in [
            r#"{"type":"sub","topic":"console.secret"}"#,
            r#"{"type":"sub"}"#,
            r#"{"type":"shutdown"}"#,
            r#"{"topic":"audit"}"#,
            "[]",
        ] {
            assert_eq!(parse(invalid), None, "{invalid}");
        }
    }
}
