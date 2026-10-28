//! The topics a socket can follow, each with the permission it needs, its snapshot and where its
//! events come from. A new topic is a new arm here; the socket does the rest.

use std::pin::Pin;
use std::sync::Arc;

use futures_util::Stream;
use futures_util::stream;
use observer_app::{ConsoleLines, Error};
use observer_domain::audit::AuditEntry;
use observer_domain::live::{AuditSnapshot, ObserverStatus, Topic, TopicEvent, TopicSnapshot};
use observer_domain::permissions::Permissions;
use observer_domain::server::{ConsoleBatch, ServerStatus};
use tokio::sync::broadcast::{self, error::RecvError};
use tokio::sync::watch;

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
        Topic::Status | Topic::ServerStatus => Permissions::DASHBOARD_VIEW,
        Topic::Audit => Permissions::AUDIT_VIEW,
        Topic::Console => Permissions::CONSOLE_VIEW,
    }
}

/// Opens a topic for one socket. Its events are subscribed to before its snapshot is taken, so
/// that nothing falls between the two. `since` resumes the console after the line of that number.
pub(super) async fn open(topic: Topic, since: Option<u64>, state: &AppState) -> Result<Feed, Error> {
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
        Topic::ServerStatus => {
            let mut status = state.supervision.status();
            let snapshot = status.borrow_and_update().clone();
            Ok(Feed {
                snapshot: TopicSnapshot::ServerStatus(snapshot),
                events: Some(status_events(status)),
            })
        }
        Topic::Console => {
            let (snapshot, lines) = state.supervision.console(since);
            Ok(Feed {
                snapshot: TopicSnapshot::Console(snapshot),
                events: Some(console_events(lines)),
            })
        }
    }
}

/// The whole status each time it changes. A client too slow to see every change only misses the
/// ones a newer status already replaced.
fn status_events(status: watch::Receiver<ServerStatus>) -> Events {
    Box::pin(stream::unfold(status, |mut status| async move {
        status.changed().await.ok()?;
        let current = status.borrow_and_update().clone();
        Some((Delivery::Event(TopicEvent::ServerStatus(current)), status))
    }))
}

fn console_events(lines: ConsoleLines) -> Events {
    Box::pin(stream::unfold(lines, |mut lines| async move {
        let delivery = match lines.next().await? {
            Ok(lines) => Delivery::Event(TopicEvent::Console(ConsoleBatch { lines })),
            Err(_) => Delivery::Missed,
        };
        Some((delivery, lines))
    }))
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
