//! The topics a socket can follow, each with the permission it needs, its snapshot and where its
//! events come from. A new topic is a new arm here; the socket does the rest.

use std::pin::Pin;
use std::sync::Arc;

use futures_util::Stream;
use futures_util::stream;
use observer_app::{ConsoleLines, Error};
use observer_domain::audit::AuditEntry;
use observer_domain::diagnostics::DiagnosticsEvent;
use observer_domain::live::{AuditSnapshot, ObserverStatus, Topic, TopicEvent, TopicSnapshot};
use observer_domain::metrics::{LiveSample, MetricsSnapshot};
use observer_domain::permissions::Permissions;
use observer_domain::server::ConsoleBatch;
use tokio::sync::broadcast::{self, error::RecvError};
use tokio::sync::watch;

use crate::AppState;

pub(super) type Events = Pin<Box<dyn Stream<Item = Delivery> + Send>>;

pub(super) enum Delivery {
    Event(Box<TopicEvent>),
    /// Events were lost before they reached the socket: the client must start over.
    Missed,
}

/// A topic as one socket follows it.
pub(super) struct Feed {
    pub snapshot: TopicSnapshot,
    /// `None` for a topic that only has a snapshot.
    pub events: Option<Events>,
}

/// Whether `held` lets a user follow `topic`. FXServer's state is the dashboard's, and also what
/// the console and the server controls go by to show whether they can act.
pub(super) fn allows(topic: Topic, held: Permissions) -> bool {
    let any = |permissions: &[Permissions]| permissions.iter().any(|&permission| held.contains(permission));
    match topic {
        Topic::Status | Topic::MetricsLive => any(&[Permissions::DASHBOARD_VIEW]),
        Topic::ServerStatus => any(&[
            Permissions::DASHBOARD_VIEW,
            Permissions::CONSOLE_VIEW,
            Permissions::SERVER_CONTROL,
        ]),
        Topic::Audit => any(&[Permissions::AUDIT_VIEW]),
        Topic::Console => any(&[Permissions::CONSOLE_VIEW]),
        Topic::ServerSchedule => any(&[
            Permissions::DASHBOARD_VIEW,
            Permissions::SCHEDULE_MANAGE,
            Permissions::SERVER_CONTROL,
        ]),
        Topic::ServerArtifacts => any(&[Permissions::DASHBOARD_VIEW, Permissions::ARTIFACTS_VIEW]),
        // The alert list names programs and modules, never a command line or a path.
        Topic::ServerAlerts => any(&[Permissions::DASHBOARD_VIEW, Permissions::DIAGNOSTICS_VIEW]),
        Topic::ServerDiagnostics => any(&[Permissions::DIAGNOSTICS_VIEW]),
        Topic::ServerChanges => any(&[Permissions::CHANGES_VIEW]),
    }
}

/// Opens a topic for one socket. Its events are subscribed to before its snapshot is taken, so
/// that nothing falls between the two. `since` resumes the console after the line of that number.
pub(super) async fn open(topic: Topic, since: Option<u64>, state: &AppState) -> Result<Feed, Error> {
    match topic {
        Topic::Status => Ok(Feed {
            snapshot: TopicSnapshot::Status(ObserverStatus {
                version: state.build.version.to_owned(),
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
                events: Some(status_events(status, TopicEvent::ServerStatus)),
            })
        }
        Topic::ServerSchedule => {
            let mut status = state.schedule.status();
            let snapshot = status.borrow_and_update().clone();
            Ok(Feed {
                snapshot: TopicSnapshot::ServerSchedule(snapshot),
                events: Some(status_events(status, TopicEvent::ServerSchedule)),
            })
        }
        Topic::ServerArtifacts => {
            let mut status = state.artifacts.status();
            let snapshot = status.borrow_and_update().clone();
            Ok(Feed {
                snapshot: TopicSnapshot::ServerArtifacts(Box::new(snapshot)),
                events: Some(status_events(status, |status| {
                    TopicEvent::ServerArtifacts(Box::new(status))
                })),
            })
        }
        Topic::Console => {
            let (snapshot, lines) = state.supervision.console(since);
            Ok(Feed {
                snapshot: TopicSnapshot::Console(snapshot),
                events: Some(console_events(lines)),
            })
        }
        Topic::MetricsLive => {
            let (latest, samples) = state.metrics.live();
            Ok(Feed {
                snapshot: TopicSnapshot::MetricsLive(MetricsSnapshot { latest }),
                events: Some(metrics_events(samples)),
            })
        }
        Topic::ServerAlerts => {
            let mut alerts = state.diagnostics.alerts();
            let snapshot = alerts.borrow_and_update().clone();
            Ok(Feed {
                snapshot: TopicSnapshot::ServerAlerts(snapshot),
                events: Some(status_events(alerts, TopicEvent::ServerAlerts)),
            })
        }
        Topic::ServerDiagnostics => {
            let (snapshot, events) = state.diagnostics.follow().await?;
            Ok(Feed {
                snapshot: TopicSnapshot::ServerDiagnostics(snapshot),
                events: Some(diagnostics_events(events)),
            })
        }
        Topic::ServerChanges => {
            let mut live = state.changes.live();
            let snapshot = live.borrow_and_update().clone();
            Ok(Feed {
                snapshot: TopicSnapshot::ServerChanges(snapshot),
                events: Some(status_events(live, TopicEvent::ServerChanges)),
            })
        }
    }
}

/// Each change of the records as it is made. A socket that falls behind starts over from a new
/// snapshot.
fn diagnostics_events(events: broadcast::Receiver<DiagnosticsEvent>) -> Events {
    Box::pin(stream::unfold(events, |mut events| async move {
        let delivery = match events.recv().await {
            Ok(event) => Delivery::Event(Box::new(TopicEvent::ServerDiagnostics(event))),
            Err(RecvError::Lagged(_)) => Delivery::Missed,
            Err(RecvError::Closed) => return None,
        };
        Some((delivery, events))
    }))
}

/// Each sample as it is taken. One that falls more than a few samples behind starts over from the
/// newest.
fn metrics_events(samples: broadcast::Receiver<LiveSample>) -> Events {
    Box::pin(stream::unfold(samples, |mut samples| async move {
        let delivery = match samples.recv().await {
            Ok(sample) => Delivery::Event(Box::new(TopicEvent::MetricsLive(sample))),
            Err(RecvError::Lagged(_)) => Delivery::Missed,
            Err(RecvError::Closed) => return None,
        };
        Some((delivery, samples))
    }))
}

/// The whole status each time it changes. A client too slow to see every change only misses the
/// ones a newer status already replaced.
fn status_events<T>(status: watch::Receiver<T>, event: fn(T) -> TopicEvent) -> Events
where
    T: Clone + Send + Sync + 'static,
{
    Box::pin(stream::unfold(status, move |mut status| async move {
        status.changed().await.ok()?;
        let current = status.borrow_and_update().clone();
        Some((Delivery::Event(Box::new(event(current))), status))
    }))
}

fn console_events(lines: ConsoleLines) -> Events {
    Box::pin(stream::unfold(lines, |mut lines| async move {
        let delivery = match lines.next().await? {
            Ok(lines) => Delivery::Event(Box::new(TopicEvent::Console(ConsoleBatch { lines }))),
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
                Ok(entry) => Delivery::Event(Box::new(TopicEvent::Audit(AuditEntry::clone(&entry)))),
                Err(RecvError::Lagged(_)) => Delivery::Missed,
                Err(RecvError::Closed) => return None,
            };
            return Some((delivery, entries));
        }
    }))
}

#[cfg(test)]
mod tests {
    use futures_util::StreamExt;
    use observer_domain::server::ServerState;

    use super::*;

    fn sample(ts: i64) -> LiveSample {
        LiveSample {
            ts,
            state: ServerState::Online,
            values: std::collections::BTreeMap::new(),
            children: Vec::new(),
            cores: Some(8),
        }
    }

    #[tokio::test]
    async fn a_socket_that_fell_behind_the_samples_starts_over() {
        let (samples, receiver) = broadcast::channel(2);
        for ts in [0, 5, 10] {
            samples.send(sample(ts)).unwrap();
        }
        let mut events = metrics_events(receiver);

        assert!(matches!(events.next().await, Some(Delivery::Missed)));
        assert!(matches!(
            events.next().await,
            Some(Delivery::Event(event)) if matches!(*event, TopicEvent::MetricsLive(LiveSample { ts: 5, .. }))
        ));
        drop(samples);
        assert!(events.next().await.is_some(), "the last one queued");
        assert!(events.next().await.is_none(), "the samples ended");
    }
}
