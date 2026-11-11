//! Who the server lets in. One task answers each player the bridge holds at `playerConnecting`: a
//! ban refuses first, then maintenance, then the allowlist while it is on. The bans check again
//! who joined.

mod allowlist;
mod maintenance;

use std::sync::Arc;

use observer_domain::admission::AdmissionStatus;
use observer_domain::allowlist::AllowlistStatus;
use observer_domain::schedule::ScheduleStatus;
use observer_fxserver::bridge::{BridgeAction, Refusal, Refused};
use observer_fxserver::players::{Connecting, PlayerReport, PlayerReportKind};
use observer_fxserver::supervisor::{Cause, Event, EventKind, State, Supervisor};
use observer_store::Database;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{broadcast, watch};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use self::maintenance::Door;
use crate::bans::Seen;
use crate::{Bans, Clock, Error, Supervision};

pub struct AdmissionSetup {
    pub db: Database,
    pub clock: Clock,
    pub supervision: Supervision,
    pub bans: Bans,
    /// The schedule's status, whose time zone the panel reads the lists in.
    pub schedule: watch::Receiver<ScheduleStatus>,
}

/// The admission service. Clones share it.
#[derive(Clone)]
pub struct Admission {
    inner: Arc<Inner>,
}

struct Inner {
    db: Database,
    clock: Clock,
    supervisor: Supervisor,
    bans: Bans,
    schedule: watch::Receiver<ScheduleStatus>,
    status: watch::Sender<AdmissionStatus>,
}

/// The admission's task; [`AdmissionTask::close`] ends it.
pub struct AdmissionTask {
    stop: CancellationToken,
    task: JoinHandle<()>,
}

impl AdmissionTask {
    pub async fn close(self) {
        self.stop.cancel();
        if let Err(error) = self.task.await {
            warn!(%error, "the admission's task failed");
        }
    }
}

impl Admission {
    /// Opens the service and starts its task, before FXServer boots so that no player gets past
    /// it.
    pub async fn start(setup: AdmissionSetup) -> Result<(Self, AdmissionTask), Error> {
        let supervisor = setup.supervision.supervisor().clone();
        let (reports, events) = (supervisor.player_reports(), supervisor.events());
        let admission = Self {
            inner: Arc::new(Inner {
                db: setup.db,
                clock: setup.clock,
                supervisor,
                bans: setup.bans,
                schedule: setup.schedule,
                status: watch::Sender::new(AdmissionStatus::default()),
            }),
        };
        admission.refresh().await?;
        let stop = CancellationToken::new();
        let task = tokio::spawn(admission.clone().run(stop.clone(), reports, events));
        Ok((admission, AdmissionTask { stop, task }))
    }

    async fn run(
        self,
        stop: CancellationToken,
        mut reports: broadcast::Receiver<PlayerReport>,
        mut events: broadcast::Receiver<Event>,
    ) {
        loop {
            tokio::select! {
                () = stop.cancelled() => return,
                report = reports.recv() => match report {
                    Ok(PlayerReport { kind: PlayerReportKind::Connecting(connecting), .. }) => {
                        self.answer(connecting).await;
                    }
                    Ok(_) => {}
                    Err(RecvError::Lagged(missed)) => {
                        warn!(missed, "the admission fell behind the bridge's reports; players it did not answer get in");
                    }
                    Err(RecvError::Closed) => return,
                },
                event = events.recv() => match event {
                    Ok(Event { kind: EventKind::State { to: State::Online, cause: Cause::Ready, .. }, .. }) => {
                        self.started().await;
                    }
                    // A start it missed hears of maintenance at the next one.
                    Ok(_) | Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => return,
                },
            }
        }
    }

    /// Answers the bridge about one connecting player: in, or refused with the owner's message.
    async fn answer(&self, connecting: Connecting) {
        let verdict = BridgeAction::Verdict {
            request: connecting.request,
            refusal: self.refusal(&Seen::of(&connecting.shown)).await,
        };
        // A server that stopped meanwhile has no one left to answer.
        let _ = self.inner.supervisor.act(verdict.fitted()).await;
    }

    /// What refuses the player, if anything does: a ban first, then maintenance, then the
    /// allowlist.
    async fn refusal(&self, seen: &Seen) -> Option<Refusal> {
        if let Some((message, event)) = self.inner.bans.refusal(seen, None).await {
            return Some(Refusal {
                message,
                event: Refused::Ban(event),
            });
        }
        match self.door(seen).await {
            Door::Closed(refusal) => Some(refusal),
            // Whoever is on the maintenance list gets in, approved or not.
            Door::Passed => None,
            Door::Open => self.unapproved(seen).await,
        }
    }

    /// Who gets in, as it changes, for the live topic.
    pub fn status(&self) -> watch::Receiver<AdmissionStatus> {
        self.inner.status.subscribe()
    }

    /// Reads the status again and publishes it when it moved.
    async fn refresh(&self) -> Result<(), Error> {
        let status = self
            .inner
            .db
            .read(|conn| -> Result<_, Error> {
                let (waiting, approved) = observer_store::allowlist::counts(conn)?;
                Ok(AdmissionStatus {
                    allowlist: AllowlistStatus {
                        enabled: observer_store::allowlist::settings(conn)?.enabled,
                        waiting,
                        approved,
                    },
                    maintenance: observer_store::maintenance::status(conn)?,
                })
            })
            .await?;
        self.inner.status.send_if_modified(|current| {
            let moved = *current != status;
            *current = status;
            moved
        });
        Ok(())
    }

    /// The same after a change already made: the panel only shows it late if this fails.
    async fn refreshed(&self) {
        if let Err(error) = self.refresh().await {
            warn!(%error, "the admission's status could not be read again");
        }
    }
}

#[cfg(test)]
mod tests;
