//! The topics a socket can follow, each with the permission it needs, its snapshot and where its
//! events come from. A new topic is a new arm here; the socket does the rest.

use std::pin::Pin;
use std::sync::Arc;

use futures_util::Stream;
use futures_util::stream;
use observer_app::Error;
use observer_domain::audit::AuditEntry;
use observer_domain::live::{AuditSnapshot, ObserverStatus, Topic, TopicEvent, TopicSnapshot};
use observer_domain::permissions::Permissions;
use tokio::sync::broadcast::{self, error::RecvError};

use crate::AppState;

pub(super) type Events = Pin<Box<dyn Stream<Item = Delivery> + Send>>;

pub(super) enum Delivery {
    Event(TopicEvent),
    /// Events were lost before they reached the socket: the client must start over.
    Missed,
}

/// A topic as one socket follows it.
pub(super) struct Feed {
    pub snapshot: TopicSnapshot,
    /// `None` for a topic that only has a snapshot.
    pub events: Option<Events>,
}

pub(super) fn permission(topic: Topic) -> Permissions {
    match topic {
        Topic::Status => Permissions::DASHBOARD_VIEW,
        Topic::Audit => Permissions::AUDIT_VIEW,
    }
}

/// Opens a topic for one socket. Its events are subscribed to before its snapshot is taken, so
/// that nothing falls between the two.
pub(super) async fn open(topic: Topic, state: &AppState) -> Result<Feed, Error> {
    match topic {
        Topic::Status => Ok(Feed {
            snapshot: TopicSnapshot::Status(ObserverStatus {
                version: state.version.to_owned(),
                started_at: state.live.started_at,
            }),
            events: None,
        }),
        Topic::Audit => {
            let entries = state.audit.follow();
            let latest_id = state.audit.latest_id().await?;
            Ok(Feed {
                snapshot: TopicSnapshot::Audit(AuditSnapshot { latest_id }),
                events: Some(audit_events(entries, latest_id)),
            })
        }
    }
}

/// The entries written after `latest_id`, which the snapshot already accounts for.
fn audit_events(entries: broadcast::Receiver<Arc<AuditEntry>>, latest_id: Option<i64>) -> Events {
    Box::pin(stream::unfold(entries, move |mut entries| async move {
        loop {
            let delivery = match entries.recv().await {
                Ok(entry) if latest_id.is_some_and(|latest| entry.id <= latest) => continue,
                Ok(entry) => Delivery::Event(TopicEvent::Audit(AuditEntry::clone(&entry))),
                Err(RecvError::Lagged(_)) => Delivery::Missed,
                Err(RecvError::Closed) => return None,
            };
            return Some((delivery, entries));
        }
    }))
}
