//! What a request to use a build may change: never the build FXServer runs while it runs. Pure.

use observer_domain::artifacts::{ApplyMode, BrokenReason, KVP_BUILD, SelectBuild};
use observer_domain::error::{ApiError, ErrorCode};

/// What the request finds when it arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Context {
    /// The build's health, `None` when it is not installed.
    pub installed: Option<Result<(), BrokenReason>>,
    pub selected: Option<u32>,
    pub pending: Option<u32>,
    pub running: Option<u32>,
    pub highest_run: Option<u32>,
    /// The cached catalog says the build is past its end of life.
    pub end_of_life: bool,
    pub server: Server,
    /// The schedule has a restart ahead, one-offs included.
    pub restart_ahead: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Server {
    Running,
    Stopping,
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Change {
    /// The build becomes the selected one and the pending one is cleared; `restart` restarts
    /// FXServer at once.
    Select { restart: bool },
    /// The build waits for the next scheduled restart, in place of any pending one.
    Pend,
    /// It is so already: nothing is changed or audited.
    Nothing,
}

pub(crate) fn decide(request: SelectBuild, context: &Context) -> Result<Change, ApiError> {
    let build = request.build;
    match context.installed {
        None => return Err(ErrorCode::BuildNotInstalled.into()),
        Some(Err(reason)) => {
            return Err(ApiError::new(ErrorCode::BuildBroken)
                .with("reason", serde_json::to_value(reason).unwrap_or_default()));
        }
        Some(Ok(())) => {}
    }
    if context.end_of_life && !request.allow_end_of_life {
        return Err(ErrorCode::EndOfLife.into());
    }
    // A build waiting for the scheduled restart comes after the selected one whenever FXServer
    // starts before it, after a crash or by hand: the selected build counts as run.
    let ran = match request.apply {
        ApplyMode::ScheduledRestart => context.highest_run.max(context.selected),
        ApplyMode::Now | ApplyMode::NextStart => context.highest_run,
    };
    if let Some(highest) = ran.filter(|highest| build < KVP_BUILD && *highest >= KVP_BUILD)
        && !request.confirm_kvp_rollback
    {
        return Err(ApiError::new(ErrorCode::KvpRollback).with("highestRun", highest));
    }
    let settled = context.selected == Some(build) && context.pending.is_none();
    match request.apply {
        ApplyMode::Now => match context.server {
            Server::Down => Err(ErrorCode::ServerNotRunning.into()),
            Server::Stopping => Err(ErrorCode::ServerStopping.into()),
            Server::Running if context.running == Some(build) && settled => Ok(Change::Nothing),
            // It runs already: it only needs to be the build that starts next again.
            Server::Running if context.running == Some(build) => Ok(Change::Select { restart: false }),
            Server::Running => Ok(Change::Select { restart: true }),
        },
        ApplyMode::NextStart if settled => Ok(Change::Nothing),
        ApplyMode::NextStart => Ok(Change::Select { restart: false }),
        ApplyMode::ScheduledRestart => {
            if !context.restart_ahead {
                Err(ErrorCode::NoScheduledRestart.into())
            } else if context.selected == Some(build) {
                Err(ErrorCode::AlreadySelected.into())
            } else if context.pending == Some(build) {
                Ok(Change::Nothing)
            } else {
                Ok(Change::Pend)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn context() -> Context {
        Context {
            installed: Some(Ok(())),
            selected: Some(37872),
            pending: None,
            running: Some(37872),
            highest_run: Some(37872),
            end_of_life: false,
            server: Server::Running,
            restart_ahead: true,
        }
    }

    fn request(build: u32, apply: ApplyMode) -> SelectBuild {
        SelectBuild {
            build,
            apply,
            confirm_kvp_rollback: false,
            allow_end_of_life: false,
        }
    }

    fn code(result: Result<Change, ApiError>) -> ErrorCode {
        result.unwrap_err().code
    }

    #[test]
    fn each_mode_changes_what_it_says() {
        assert_eq!(
            decide(request(37900, ApplyMode::Now), &context()),
            Ok(Change::Select { restart: true })
        );
        assert_eq!(
            decide(request(37900, ApplyMode::NextStart), &context()),
            Ok(Change::Select { restart: false })
        );
        assert_eq!(
            decide(request(37900, ApplyMode::ScheduledRestart), &context()),
            Ok(Change::Pend)
        );
    }

    #[test]
    fn now_needs_a_server_that_runs_and_is_not_stopping() {
        let down = Context {
            server: Server::Down,
            running: None,
            ..context()
        };
        let stopping = Context {
            server: Server::Stopping,
            ..context()
        };

        assert_eq!(
            code(decide(request(37900, ApplyMode::Now), &down)),
            ErrorCode::ServerNotRunning
        );
        assert_eq!(
            code(decide(request(37900, ApplyMode::Now), &stopping)),
            ErrorCode::ServerStopping
        );
        assert_eq!(
            decide(request(37900, ApplyMode::NextStart), &down),
            Ok(Change::Select { restart: false })
        );
    }

    #[test]
    fn the_scheduled_restart_needs_one_ahead_and_another_build() {
        let nothing_ahead = Context {
            restart_ahead: false,
            ..context()
        };

        assert_eq!(
            code(decide(
                request(37900, ApplyMode::ScheduledRestart),
                &nothing_ahead
            )),
            ErrorCode::NoScheduledRestart
        );
        assert_eq!(
            code(decide(request(37872, ApplyMode::ScheduledRestart), &context())),
            ErrorCode::AlreadySelected
        );
    }

    #[test]
    fn what_is_so_already_changes_nothing() {
        let pending = Context {
            pending: Some(37950),
            ..context()
        };

        assert_eq!(
            decide(request(37872, ApplyMode::Now), &context()),
            Ok(Change::Nothing)
        );
        assert_eq!(
            decide(request(37872, ApplyMode::NextStart), &context()),
            Ok(Change::Nothing)
        );
        assert_eq!(
            decide(request(37950, ApplyMode::ScheduledRestart), &pending),
            Ok(Change::Nothing)
        );
        assert_eq!(
            decide(request(37872, ApplyMode::NextStart), &pending),
            Ok(Change::Select { restart: false }),
            "the next start of the selected build drops the pending one"
        );
    }

    #[test]
    fn now_of_the_build_that_runs_takes_it_back_as_the_next_one_without_a_restart() {
        let moved_on = Context {
            selected: Some(37900),
            pending: Some(37950),
            ..context()
        };

        assert_eq!(
            decide(request(37872, ApplyMode::Now), &moved_on),
            Ok(Change::Select { restart: false })
        );
    }

    #[test]
    fn a_build_not_installed_or_broken_is_refused_whatever_the_mode() {
        let missing = Context {
            installed: None,
            ..context()
        };
        let broken = Context {
            installed: Some(Err(BrokenReason::NoManifest)),
            ..context()
        };

        for apply in [ApplyMode::Now, ApplyMode::NextStart, ApplyMode::ScheduledRestart] {
            assert_eq!(
                code(decide(request(37900, apply), &missing)),
                ErrorCode::BuildNotInstalled
            );
            let refused = decide(request(37900, apply), &broken).unwrap_err();
            assert_eq!(refused.code, ErrorCode::BuildBroken);
            assert_eq!(refused.params["reason"], json!("noManifest"));
        }
    }

    #[test]
    fn going_below_6449_after_a_newer_build_ran_needs_a_confirmation() {
        let after_6449 = Context {
            highest_run: Some(6449),
            ..context()
        };
        let before = Context {
            highest_run: Some(6448),
            ..context()
        };
        let nothing_ran = Context {
            highest_run: None,
            ..context()
        };

        let refused = decide(request(6448, ApplyMode::NextStart), &after_6449).unwrap_err();
        assert_eq!(refused.code, ErrorCode::KvpRollback);
        assert_eq!(refused.params["highestRun"], json!(6449));
        let confirmed = SelectBuild {
            confirm_kvp_rollback: true,
            ..request(6448, ApplyMode::NextStart)
        };
        assert!(decide(confirmed, &after_6449).is_ok());
        assert!(decide(request(6449, ApplyMode::NextStart), &after_6449).is_ok());
        assert!(decide(request(6448, ApplyMode::NextStart), &before).is_ok());
        assert!(decide(request(6448, ApplyMode::NextStart), &nothing_ran).is_ok());
    }

    #[test]
    fn a_build_pending_below_6449_needs_the_confirmation_when_the_selected_one_is_past_it() {
        let not_run_yet = Context {
            selected: Some(7000),
            running: None,
            highest_run: None,
            server: Server::Down,
            ..context()
        };

        let refused = decide(request(6000, ApplyMode::ScheduledRestart), &not_run_yet).unwrap_err();
        let confirmed = SelectBuild {
            confirm_kvp_rollback: true,
            ..request(6000, ApplyMode::ScheduledRestart)
        };

        assert_eq!(refused.code, ErrorCode::KvpRollback);
        assert_eq!(refused.params["highestRun"], json!(7000));
        assert_eq!(decide(confirmed, &not_run_yet), Ok(Change::Pend));
        assert_eq!(
            decide(request(6000, ApplyMode::NextStart), &not_run_yet),
            Ok(Change::Select { restart: false }),
            "the next start replaces 7000 before it ever runs"
        );
    }

    #[test]
    fn a_build_past_its_end_of_life_needs_the_allowance() {
        let end_of_life = Context {
            end_of_life: true,
            ..context()
        };
        let allowed = SelectBuild {
            allow_end_of_life: true,
            ..request(37900, ApplyMode::NextStart)
        };

        assert_eq!(
            code(decide(request(37900, ApplyMode::NextStart), &end_of_life)),
            ErrorCode::EndOfLife
        );
        assert!(decide(allowed, &end_of_life).is_ok());
    }
}
