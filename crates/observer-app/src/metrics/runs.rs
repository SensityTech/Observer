//! FXServer's runs as the supervisor's events tell them, written to the metrics database for the
//! charts' markers.

use observer_fxserver::supervisor::{Cause, Event, EventKind, State};
use observer_store::metrics::runs::{self, EndCause, RunKey, StartCause};
use observer_store::{MetricsDatabase, StoreError};
use tracing::warn;

/// What one event changes in a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Change {
    Started {
        at_ms: i64,
        cause: StartCause,
    },
    Ready {
        at_ms: i64,
    },
    Spawned {
        at_ms: i64,
    },
    Ended {
        at_ms: i64,
        cause: EndCause,
    },
    /// The crash just recorded was a hang.
    Hung,
    Build(u32),
}

impl Change {
    pub(crate) fn of(event: &Event) -> Option<Self> {
        match &event.kind {
            EventKind::State {
                to: State::Starting,
                cause,
                ..
            } => {
                let cause = match cause {
                    Cause::Requested { .. } => StartCause::Requested,
                    Cause::Restart { .. } | Cause::Scheduled { .. } => StartCause::Restart,
                    Cause::Recovery => StartCause::Recovery,
                    _ => return None,
                };
                Some(Self::Started {
                    at_ms: event.at_ms,
                    cause,
                })
            }
            EventKind::State {
                to: State::Online,
                cause: Cause::Ready,
                ..
            } => Some(Self::Ready { at_ms: event.at_ms }),
            EventKind::Spawned { .. } => Some(Self::Spawned { at_ms: event.at_ms }),
            EventKind::Exited { expected, .. } => Some(Self::Ended {
                at_ms: event.at_ms,
                cause: if *expected {
                    EndCause::Stopped
                } else {
                    EndCause::Crashed
                },
            }),
            EventKind::State {
                to: State::Crashed,
                cause: Cause::Hang { .. },
                ..
            } => Some(Self::Hung),
            _ => None,
        }
    }
}

pub(super) struct Runs {
    db: MetricsDatabase,
    observer_started: i64,
    /// The generation whose build is recorded.
    built: Option<u32>,
    failing: bool,
}

impl Runs {
    pub(super) fn new(db: MetricsDatabase, observer_started: i64) -> Self {
        Self {
            db,
            observer_started,
            built: None,
            failing: false,
        }
    }

    /// Records the build of `generation` once the supervisor knows it.
    pub(super) async fn build(&mut self, generation: u32, build: u32) {
        if self.built != Some(generation) {
            self.built = Some(generation);
            self.apply(generation, Change::Build(build)).await;
        }
    }

    pub(super) async fn apply(&mut self, generation: u32, change: Change) {
        let run = RunKey {
            observer_started: self.observer_started,
            generation,
        };
        let written = self
            .db
            .write(move |tx| -> Result<(), StoreError> {
                match change {
                    Change::Started { at_ms, cause } => runs::start(tx, run, at_ms, cause),
                    Change::Ready { at_ms } => runs::ready(tx, run, at_ms),
                    Change::Spawned { at_ms } => runs::spawned(tx, run, at_ms),
                    Change::Ended { at_ms, cause } => runs::end(tx, run, at_ms, cause),
                    Change::Hung => runs::hung(tx, run),
                    Change::Build(build) => runs::build(tx, run, build),
                }
            })
            .await;
        match written {
            Ok(()) => self.failing = false,
            Err(error) if !self.failing => {
                self.failing = true;
                warn!(%error, generation, "a run of FXServer could not be recorded");
            }
            Err(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use observer_fxserver::health::HangCause;

    use super::*;

    fn event(kind: EventKind) -> Event {
        Event {
            at_ms: 1_761_642_000_000,
            generation: 2,
            kind,
        }
    }

    fn state(to: State, cause: Cause) -> Event {
        event(EventKind::State {
            from: State::Stopped,
            to,
            cause,
        })
    }

    #[test]
    fn starts_say_who_asked_and_ends_whether_anyone_did() {
        let at_ms = 1_761_642_000_000;
        let by = || "ada".to_owned();

        assert_eq!(
            Change::of(&state(State::Starting, Cause::Requested { by: by() })),
            Some(Change::Started {
                at_ms,
                cause: StartCause::Requested
            })
        );
        assert_eq!(
            Change::of(&state(State::Starting, Cause::Restart { by: by() })),
            Some(Change::Started {
                at_ms,
                cause: StartCause::Restart
            })
        );
        assert_eq!(
            Change::of(&state(State::Starting, Cause::Recovery)),
            Some(Change::Started {
                at_ms,
                cause: StartCause::Recovery
            })
        );
        assert_eq!(
            Change::of(&state(State::Online, Cause::Ready)),
            Some(Change::Ready { at_ms })
        );
        assert_eq!(Change::of(&state(State::Online, Cause::Recovered)), None);
        assert_eq!(
            Change::of(&event(EventKind::Spawned {
                pid: 1,
                probe: "127.0.0.1:30120".parse().unwrap(),
                build: "builds/1".into(),
                launch: Box::new(observer_domain::launch::LaunchSettings {
                    data_path: "/srv/fivem".to_owned(),
                    cfg_file: "server.cfg".to_owned(),
                    onesync: observer_domain::launch::OneSync::On,
                    auto_start: false,
                    txadmin_events: false,
                    extra_arguments: Vec::new(),
                }),
            })),
            Some(Change::Spawned { at_ms })
        );
        let exited = |expected| {
            event(EventKind::Exited {
                code: Some(1),
                signal: None,
                description: "exit code 1".to_owned(),
                expected,
            })
        };
        assert_eq!(
            Change::of(&exited(true)),
            Some(Change::Ended {
                at_ms,
                cause: EndCause::Stopped
            })
        );
        assert_eq!(
            Change::of(&exited(false)),
            Some(Change::Ended {
                at_ms,
                cause: EndCause::Crashed
            })
        );
        assert_eq!(
            Change::of(&state(
                State::Crashed,
                Cause::Hang {
                    cause: HangCause::NoHeartbeat
                }
            )),
            Some(Change::Hung)
        );
    }

    #[test]
    fn a_restart_the_schedule_made_starts_a_run_as_a_restart() {
        for by in ["observer", "ada"] {
            assert_eq!(
                Change::of(&state(State::Starting, Cause::Scheduled { by: by.to_owned() })),
                Some(Change::Started {
                    at_ms: 1_761_642_000_000,
                    cause: StartCause::Restart
                }),
                "{by}"
            );
        }
    }
}
