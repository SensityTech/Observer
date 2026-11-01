//! The probation of a new build: once its run is ready, it must stay up for a while before it is
//! the last healthy build. A crash or a hang before then fails it; a stop someone asked for, a
//! port already taken or a refused licence key say nothing about the build and leave it open.
//! A run whose heartbeat or HTTP is late when the window ends does not pass until it answers
//! again: the hang delay is the owner's, up to an hour, so the hang it may be heading for can come
//! long after the window. Pure: the artifacts task feeds it what the supervisor reports and does
//! what it says.

use std::time::Duration;

use jiff::{SignedDuration, Timestamp};
use observer_domain::artifacts::{KVP_BUILD, NoticeReason};
use observer_store::artifacts::OpenProbation;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Probation {
    /// Kept in the artifact state: a probation outlives an Observer restart.
    pub open: Option<OpenProbation>,
    /// The run that counts, in memory only: after an Observer restart the next run on the build
    /// starts the window over.
    pub run: Option<Watched>,
    pub last_healthy: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Watched {
    pub generation: u32,
    /// When the run got ready: online, or degraded, which counts as up.
    pub ready_at: Option<Timestamp>,
    /// Its heartbeat or HTTP is late, which a hang starts with: no pass is due until it answers.
    pub silent: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Input {
    Spawned {
        generation: u32,
        build: u32,
    },
    /// The run is up: online, or degraded, `silent` when its heartbeat or HTTP is late.
    Ready {
        generation: u32,
        silent: bool,
    },
    Ended {
        generation: u32,
        end: End,
    },
    Tick,
}

/// How a run ended, as the supervisor reported it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum End {
    Crashed,
    /// Not ready in time, or no longer answering: the supervisor's limits.
    Hung,
    BindError,
    LicenceRejected,
    /// A stop, a restart, a scheduled restart, `quit` or Observer stopping.
    Requested,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Verdict {
    Opened {
        build: u32,
        from: Option<u32>,
    },
    Passed {
        build: u32,
    },
    Failed {
        build: u32,
        from: Option<u32>,
        reason: Failure,
    },
    /// A run of another build ended it without a verdict.
    Closed {
        build: u32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Failure {
    Crashed,
    Hung,
}

impl Failure {
    pub fn reason(self) -> NoticeReason {
        match self {
            Self::Crashed => NoticeReason::Crashed,
            Self::Hung => NoticeReason::Hung,
        }
    }
}

/// One step of the probation. Whatever the input, a run that has been up long enough passes, so
/// that the deadline is always ahead of `now` afterwards.
pub(crate) fn step(
    mut state: Probation,
    input: Input,
    now: Timestamp,
    stable_for: Duration,
) -> (Probation, Vec<Verdict>) {
    let mut verdicts = Vec::new();
    let current =
        |state: &Probation, generation: u32| state.run.is_some_and(|run| run.generation == generation);
    match input {
        Input::Spawned { generation, build } => {
            if let Some(open) = state.open.filter(|open| open.build != build) {
                verdicts.push(Verdict::Closed { build: open.build });
                state.open = None;
                state.run = None;
            }
            if state.last_healthy == Some(build) {
                state.run = None;
            } else {
                if state.open.is_none() {
                    let from = state.last_healthy;
                    state.open = Some(OpenProbation { build, from });
                    verdicts.push(Verdict::Opened { build, from });
                }
                state.run = Some(Watched {
                    generation,
                    ready_at: None,
                    silent: false,
                });
            }
        }
        Input::Ready { generation, silent } => {
            if let Some(run) = state.run.as_mut().filter(|run| run.generation == generation) {
                run.ready_at.get_or_insert(now);
                run.silent = silent;
            }
        }
        Input::Ended { generation, end } if current(&state, generation) => {
            state.run = None;
            let failure = match end {
                End::Crashed => Some(Failure::Crashed),
                End::Hung => Some(Failure::Hung),
                End::BindError | End::LicenceRejected | End::Requested => None,
            };
            if let (Some(reason), Some(open)) = (failure, state.open) {
                state.open = None;
                verdicts.push(Verdict::Failed {
                    build: open.build,
                    from: open.from,
                    reason,
                });
            }
        }
        Input::Ended { .. } | Input::Tick => {}
    }
    if let (Some(open), Some(at)) = (state.open, passes_at(&state, stable_for))
        && at <= now
    {
        state.open = None;
        state.run = None;
        state.last_healthy = Some(open.build);
        verdicts.push(Verdict::Passed { build: open.build });
    }
    (state, verdicts)
}

/// When the run on probation passes, once it is ready and while it answers.
pub(crate) fn passes_at(state: &Probation, stable_for: Duration) -> Option<Timestamp> {
    state.open?;
    let run = state.run.filter(|run| !run.silent)?;
    let ready_at = run.ready_at?;
    let window = SignedDuration::try_from(stable_for).unwrap_or(SignedDuration::MAX);
    ready_at.checked_add(window).ok()
}

/// Why a failed build is kept rather than left for the last healthy one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kept {
    /// The automatic return is off.
    Disabled,
    NoHealthyBuild,
    /// The last healthy build is broken or gone.
    Broken,
    /// It is below the KVP build while a newer one ran: an automatic return never crosses it.
    Kvp,
}

impl Kept {
    pub fn reason(self) -> NoticeReason {
        match self {
            Self::Disabled => NoticeReason::Disabled,
            Self::NoHealthyBuild => NoticeReason::NoHealthyBuild,
            Self::Broken => NoticeReason::Broken,
            Self::Kvp => NoticeReason::Kvp,
        }
    }
}

/// The build FXServer goes back to after a failed probation.
pub(crate) fn rollback_target(
    auto_rollback: bool,
    last_healthy: Option<u32>,
    last_healthy_valid: bool,
    highest_run: Option<u32>,
) -> Result<u32, Kept> {
    if !auto_rollback {
        return Err(Kept::Disabled);
    }
    let target = last_healthy.ok_or(Kept::NoHealthyBuild)?;
    if !last_healthy_valid {
        return Err(Kept::Broken);
    }
    if target < KVP_BUILD && highest_run.is_some_and(|highest| highest >= KVP_BUILD) {
        return Err(Kept::Kvp);
    }
    Ok(target)
}
