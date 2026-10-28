//! The schedule's rules, with no I/O and no clock of their own: [`step`] takes what the schedule
//! knows, its settings, what happened and the time, and says what to do about it. The driver in
//! `schedule.rs` turns the effects into stdin lines, supervisor calls, store writes and audit
//! entries.

use std::collections::BTreeMap;

use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp, Unit};
use observer_domain::error::{ApiError, ErrorCode};
use observer_domain::schedule::{
    Delayed, NextRestart, Occurrence, OneOffRequest, OneOffRestart, PlannedDelay, PlannedRestart,
    RestartSource, ScheduleMode, SchedulePlan, ScheduleSettings, ScheduleStatus, Skipped, parse_time,
};
use observer_domain::server::ServerState;
use observer_fxserver::bridge::BridgeEvent;

/// A restart this late still happens at once; a later one is missed.
pub(crate) const GRACE: SignedDuration = SignedDuration::from_mins(5);
/// A `settled` this far ahead was written while the clock was wrong.
const SETTLED_AHEAD: SignedDuration = SignedDuration::from_hours(24);
const MAX_DELAY: SignedDuration = SignedDuration::from_hours(12);
pub(crate) const MAX_DELAY_MINUTES: u32 = 720;
pub(crate) const MAX_ONE_OFF_MINUTES: u32 = 1440;
const UPCOMING: usize = 5;
/// Who the bridge's events name for what the schedule does by itself.
pub(crate) const OBSERVER: &str = "observer";

/// The stored settings with the zone they resolve to.
#[derive(Debug, Clone)]
pub(crate) struct Settings {
    pub stored: ScheduleSettings,
    pub version: Option<i64>,
    pub zone: TimeZone,
    /// The zone's IANA name, which the status shows.
    pub zone_name: String,
}

/// FXServer as the schedule needs to see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServerView {
    pub state: ServerState,
    pub generation: u32,
    /// Set while a process runs.
    pub started_at: Option<Timestamp>,
}

/// A restart the schedule means to make.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Target {
    pub at: Timestamp,
    pub source: RestartSource,
    /// The schedule's own time for it, before any delay; a one-off's time.
    pub occurrence: Timestamp,
    /// Who planned the one-off, or delayed the occurrence.
    pub by: Option<String>,
}

impl Target {
    /// Who the restart is made by: Observer for the schedule's own, the planner for a one-off.
    pub(crate) fn made_by(&self) -> &str {
        match (self.source, &self.by) {
            (RestartSource::OneOff, Some(by)) => by,
            _ => OBSERVER,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AutoSkip {
    at: Timestamp,
    started_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Asked {
    generation: u32,
    target: Target,
}

/// Everything the schedule knows. Only the plan outlives Observer; the rest is rebuilt from what
/// FXServer does next.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct State {
    pub plan: SchedulePlan,
    server: Option<ServerView>,
    /// The generation of the run being watched.
    run: Option<u32>,
    /// The run the one-off belongs to. `None` gives it to the next run seen, which is what a
    /// plan read back after Observer restarted needs.
    one_off_run: Option<u32>,
    auto_skip: Option<AutoSkip>,
    /// The restart of the last step: once its time passes, it fires or is given up on.
    armed: Option<Target>,
    /// The smallest tier sent or passed for each `at`, in this run.
    warned: BTreeMap<Timestamp, u32>,
    /// The restart asked of the supervisor, until the run it starts shows up.
    asked: Option<Asked>,
}

impl State {
    pub(crate) fn restored(plan: SchedulePlan) -> Self {
        Self {
            plan,
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Input {
    /// A deadline came, or the longest sleep ended.
    Tick,
    /// New settings were saved.
    Settings {
        by: String,
    },
    Server(ServerView),
    Skip {
        by: String,
    },
    Restore {
        by: String,
    },
    Delay {
        minutes: u32,
        by: String,
    },
    /// Already resolved to an instant by [`resolve_one_off`].
    OneOff {
        at: Timestamp,
        by: String,
    },
    CancelOneOff {
        by: String,
    },
    /// The restart asked of the supervisor did not happen.
    RestartRefused {
        reason: Miss,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Miss {
    /// Observer saw the time pass too late: the host slept, or the clock jumped.
    Late { by: SignedDuration },
    /// Something else was already stopping or restarting FXServer.
    ServerBusy { state: Option<ServerState> },
    /// This host refuses the stored launch settings.
    LaunchInvalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DropReason {
    ServerRestarted,
    ServerStopped,
}

/// What a user's action did, for its audit entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Applied {
    Skipped {
        at: Timestamp,
    },
    Restored {
        at: Timestamp,
    },
    Delayed {
        occurrence: Timestamp,
        at: Timestamp,
        previous_at: Timestamp,
    },
    Planned {
        at: Timestamp,
        previous_at: Option<Timestamp>,
    },
    Cancelled {
        at: Timestamp,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Effect {
    Announce(BridgeEvent),
    Restart(Target),
    /// The plan changed: it is written before anything else is done.
    Persist,
    Applied(Applied),
    Missed {
        target: Target,
        reason: Miss,
    },
    AutoSkipped {
        at: Timestamp,
        started_at: Timestamp,
    },
    OneOffDropped {
        one_off: PlannedRestart,
        reason: DropReason,
    },
    /// What a settings change took away: a skip or a delay of an occurrence it removed.
    Dropped {
        skip: Option<PlannedRestart>,
        delay: Option<PlannedDelay>,
    },
    /// `settled` was too far ahead to trust, and was dropped.
    SettledAhead(Timestamp),
    /// The action did not apply; the state is as it was.
    Refused(ApiError),
}

/// How a user's action changed the next restart, for resources that heard of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Change {
    Moved,
    Removed,
}

/// Applies `input`, then catches up with `now`: a due restart fires or is given up on, passed
/// skips are forgotten, the due warning goes out.
pub(crate) fn step(
    mut state: State,
    settings: &Settings,
    input: Input,
    now: Timestamp,
) -> (State, Vec<Effect>) {
    let before = state.clone();
    let previous = state.armed.clone();
    let mut effects = Vec::new();

    if let Some(settled) = state
        .plan
        .settled
        .filter(|settled| *settled > now + SETTLED_AHEAD)
    {
        state.plan.settled = None;
        effects.push(Effect::SettledAhead(settled));
    }
    let settings_change = matches!(input, Input::Settings { .. });
    let applied = match input {
        Input::Tick => Ok(None),
        Input::Settings { by } => Ok(Some((by, Change::Removed, None))),
        Input::Server(view) => {
            state.observe(view, settings, &mut effects);
            Ok(None)
        }
        Input::Skip { by } => state
            .skip(settings, now, &by)
            .map(|applied| Some((by, Change::Removed, Some(applied)))),
        Input::Restore { by } => state
            .restore(now)
            .map(|applied| Some((by, Change::Moved, Some(applied)))),
        Input::Delay { minutes, by } => state
            .delay(settings, now, minutes, &by)
            .map(|applied| Some((by, Change::Moved, Some(applied)))),
        Input::OneOff { at, by } => state
            .plan_one_off(at, &by)
            .map(|applied| Some((by, Change::Moved, Some(applied)))),
        Input::CancelOneOff { by } => state
            .cancel_one_off()
            .map(|applied| Some((by, Change::Removed, Some(applied)))),
        Input::RestartRefused { reason } => {
            if let Some(asked) = state.asked.take() {
                state.miss(asked.target, reason, &mut effects);
            }
            Ok(None)
        }
    };
    let change = match applied {
        Ok(change) => change,
        Err(error) => return (before, vec![Effect::Refused(error)]),
    };
    if let Some((_, _, Some(applied))) = &change {
        effects.push(Effect::Applied(applied.clone()));
    }

    let (skip, delay) = state.drop_stale(settings);
    if settings_change && (skip.is_some() || delay.is_some()) {
        effects.push(Effect::Dropped { skip, delay });
    }
    state.catch_up(settings, now, &mut effects);
    if let (Some(previous), Some((by, change, _))) = (previous, change) {
        state.tell_of_change(&previous, &by, change, &mut effects);
    }
    state.warn(settings, now, &mut effects);
    if state.plan != before.plan {
        effects.push(Effect::Persist);
    }
    (state, effects)
}

/// The next instant something is due: a warning tier, the restart itself, a skip to forget.
/// Always after `now`.
pub(crate) fn deadline(state: &State, settings: &Settings, now: Timestamp) -> Option<Timestamp> {
    let mut due = Vec::new();
    if let Some(target) = &state.armed {
        due.push(target.at);
        let below = state.warned.get(&target.at).copied().unwrap_or(u32::MAX);
        due.extend(
            settings
                .stored
                .warnings
                .iter()
                .filter(|&&tier| tier < below)
                .map(|&tier| target.at - minutes(tier)),
        );
    }
    due.extend(state.plan.skip.as_ref().map(|skip| skip.at));
    due.extend(state.auto_skip.map(|skip| skip.at));
    due.into_iter().filter(|at| *at > now).min()
}

/// The `server.schedule` topic's value.
pub(crate) fn status(state: &State, settings: &Settings, now: Timestamp) -> ScheduleStatus {
    let upcoming = state
        .candidates(settings, now)
        .into_iter()
        .take(match settings.stored.mode {
            ScheduleMode::FixedTimes => UPCOMING,
            ScheduleMode::Interval => 1,
        })
        .map(|candidate| Occurrence {
            at: candidate.occurrence,
            time: candidate.time,
            skipped: candidate.skipped,
            delayed: candidate.delayed,
        })
        .collect();
    ScheduleStatus {
        settings: settings.stored.clone(),
        version: settings.version,
        zone: settings.zone_name.clone(),
        next: state.armed.as_ref().map(|target| NextRestart {
            at: target.at,
            source: target.source,
            by: target.by.clone(),
            delayed_from: (target.source == RestartSource::Schedule && target.occurrence != target.at)
                .then_some(target.occurrence),
        }),
        upcoming,
        one_off: state.plan.one_off.as_ref().map(|one_off| OneOffRestart {
            at: one_off.at,
            by: one_off.by.clone(),
        }),
    }
}

/// When a single restart asked for in minutes, or at a wall time, happens: the first such time in
/// `zone` at least a minute ahead, today or tomorrow. Minutes count to the nearest second.
pub(crate) fn resolve_one_off(
    request: &OneOffRequest,
    zone: &TimeZone,
    now: Timestamp,
) -> Result<Timestamp, ApiError> {
    match (request.in_minutes, request.at.as_deref()) {
        (Some(count), None) if (1..=MAX_ONE_OFF_MINUTES).contains(&count) => {
            let at = now + minutes(count);
            Ok(at.round(Unit::Second).unwrap_or(at))
        }
        (None, Some(time)) => {
            let minute = parse_time(time).ok_or(ErrorCode::InvalidRequest)?;
            let earliest = now + SignedDuration::from_mins(1);
            let today = now.to_zoned(zone.clone()).date();
            let mut date = today.yesterday().unwrap_or(today);
            for _ in 0..4 {
                if let Some(at) = wall_time(date, minute, zone).filter(|at| *at >= earliest) {
                    return Ok(at);
                }
                date = date.tomorrow().unwrap_or(date);
            }
            Err(ErrorCode::InvalidRequest.into())
        }
        _ => Err(ErrorCode::InvalidRequest.into()),
    }
}

/// One occurrence of the schedule as the plan sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Candidate {
    occurrence: Timestamp,
    /// When it happens: its own time, or the one it was delayed to.
    at: Timestamp,
    time: Option<String>,
    skipped: Option<Skipped>,
    delayed: Option<Delayed>,
}

impl State {
    fn observe(&mut self, view: ServerView, settings: &Settings, effects: &mut Vec<Effect>) {
        if matches!(view.state, ServerState::Stopped | ServerState::CrashLoop) && self.one_off_run.is_some() {
            self.drop_one_off(DropReason::ServerStopped, effects);
        }
        if let Some(started_at) = view.started_at
            && self.run != Some(view.generation)
        {
            let asked = self.asked.take_if(|asked| view.generation > asked.generation);
            let by_schedule = asked.is_some_and(|asked| asked.target.source == RestartSource::Schedule);
            self.run = Some(view.generation);
            self.warned.clear();
            if self.plan.one_off.is_some() {
                match self.one_off_run {
                    None => self.one_off_run = Some(view.generation),
                    Some(generation) if generation != view.generation => {
                        self.drop_one_off(DropReason::ServerRestarted, effects);
                    }
                    Some(_) => {}
                }
            }
            self.auto_skip = if by_schedule {
                None
            } else {
                self.recent_start(settings, started_at)
            };
            if let Some(skip) = self.auto_skip {
                effects.push(Effect::AutoSkipped {
                    at: skip.at,
                    started_at: skip.started_at,
                });
            }
        }
        self.server = Some(view);
    }

    fn drop_one_off(&mut self, reason: DropReason, effects: &mut Vec<Effect>) {
        self.one_off_run = None;
        if let Some(one_off) = self.plan.one_off.take() {
            effects.push(Effect::OneOffDropped { one_off, reason });
        }
    }

    /// The first fixed time after a start the schedule did not make, when it comes too soon after.
    fn recent_start(&self, settings: &Settings, started_at: Timestamp) -> Option<AutoSkip> {
        let stored = &settings.stored;
        if !stored.enabled || stored.mode != ScheduleMode::FixedTimes || stored.skip_after_start == 0 {
            return None;
        }
        let (at, _) = fixed_occurrences(&stored.times, &settings.zone, started_at, 1)
            .into_iter()
            .next()?;
        let soon = at.duration_since(started_at) < minutes(stored.skip_after_start);
        let skipped = self.plan.skip.as_ref().is_some_and(|skip| skip.at == at);
        let settled = self.plan.settled.is_some_and(|settled| at <= settled);
        (soon && !skipped && !settled).then_some(AutoSkip { at, started_at })
    }

    fn skip(&mut self, settings: &Settings, now: Timestamp, by: &str) -> Result<Applied, ApiError> {
        let first = self
            .candidates(settings, now)
            .into_iter()
            .find(|candidate| !matches!(candidate.skipped, Some(Skipped::RecentStart { .. })))
            .ok_or(ErrorCode::NothingToSkip)?;
        if first.skipped.is_some() {
            return Err(ErrorCode::AlreadySkipped.into());
        }
        self.plan.skip = Some(PlannedRestart {
            at: first.occurrence,
            by: by.to_owned(),
        });
        self.plan
            .delay
            .take_if(|delay| delay.occurrence == first.occurrence);
        Ok(Applied::Skipped { at: first.occurrence })
    }

    fn restore(&mut self, now: Timestamp) -> Result<Applied, ApiError> {
        let skip = self
            .plan
            .skip
            .take_if(|skip| skip.at > now)
            .ok_or(ErrorCode::NothingToRestore)?;
        Ok(Applied::Restored { at: skip.at })
    }

    fn delay(
        &mut self,
        settings: &Settings,
        now: Timestamp,
        count: u32,
        by: &str,
    ) -> Result<Applied, ApiError> {
        if !(1..=MAX_DELAY_MINUTES).contains(&count) {
            return Err(ErrorCode::InvalidRequest.into());
        }
        let next = self
            .target(settings, now)
            .filter(|target| target.source == RestartSource::Schedule)
            .ok_or(ErrorCode::NothingToDelay)?;
        let to = next.at + minutes(count);
        let mut limit = next.occurrence + MAX_DELAY;
        if let Some(following) = self.following(settings, next.occurrence) {
            limit = limit.min(following - SignedDuration::from_mins(1));
        }
        if to > limit {
            let left = limit.duration_since(next.at).as_mins().max(0);
            return Err(ApiError::new(ErrorCode::DelayTooLong).with("maxMinutes", left));
        }
        self.plan.delay = Some(PlannedDelay {
            occurrence: next.occurrence,
            to,
            by: by.to_owned(),
        });
        Ok(Applied::Delayed {
            occurrence: next.occurrence,
            at: to,
            previous_at: next.at,
        })
    }

    fn plan_one_off(&mut self, at: Timestamp, by: &str) -> Result<Applied, ApiError> {
        let view = self
            .server
            .as_ref()
            .filter(|view| !matches!(view.state, ServerState::Stopped | ServerState::CrashLoop))
            .ok_or(ErrorCode::ServerNotRunning)?;
        self.one_off_run = view.started_at.map(|_| view.generation);
        let previous = self.plan.one_off.replace(PlannedRestart {
            at,
            by: by.to_owned(),
        });
        Ok(Applied::Planned {
            at,
            previous_at: previous.map(|previous| previous.at),
        })
    }

    fn cancel_one_off(&mut self) -> Result<Applied, ApiError> {
        let one_off = self.plan.one_off.take().ok_or(ErrorCode::NoOneOff)?;
        self.one_off_run = None;
        Ok(Applied::Cancelled { at: one_off.at })
    }

    /// Drops a skip or a delay whose occurrence the schedule no longer has.
    fn drop_stale(&mut self, settings: &Settings) -> (Option<PlannedRestart>, Option<PlannedDelay>) {
        let skip_stale = self
            .plan
            .skip
            .as_ref()
            .is_some_and(|skip| !self.produced(settings, skip.at));
        let delay_stale = self
            .plan
            .delay
            .as_ref()
            .is_some_and(|delay| !self.produced(settings, delay.occurrence));
        (
            self.plan.skip.take_if(|_| skip_stale),
            self.plan.delay.take_if(|_| delay_stale),
        )
    }

    fn catch_up(&mut self, settings: &Settings, now: Timestamp, effects: &mut Vec<Effect>) {
        loop {
            match self.target(settings, now) {
                Some(target) if target.at <= now => self.resolve(target, now, effects),
                target => {
                    self.armed = target;
                    break;
                }
            }
        }
        if let Some(skip) = self.plan.skip.take_if(|skip| skip.at <= now) {
            self.settle(skip.at);
        }
        if let Some(skip) = self.auto_skip.take_if(|skip| skip.at <= now) {
            self.settle(skip.at);
        }
        // A restart settles every occurrence whose own time passed, a delayed one included: what
        // it was delayed for is done.
        let settled = self.plan.settled;
        self.plan
            .delay
            .take_if(|delay| settled.is_some_and(|settled| delay.occurrence <= settled));
    }

    /// A restart whose time came: it fires, or it is given up on.
    fn resolve(&mut self, target: Target, now: Timestamp, effects: &mut Vec<Effect>) {
        match target.source {
            RestartSource::Schedule => {
                self.settle(target.occurrence);
                self.plan
                    .delay
                    .take_if(|delay| delay.occurrence == target.occurrence);
            }
            RestartSource::OneOff => {
                self.plan.one_off = None;
                self.one_off_run = None;
            }
        }
        let state = self.server.as_ref().map(|view| view.state);
        let late = now.duration_since(target.at);
        match state {
            None | Some(ServerState::Stopped | ServerState::CrashLoop) => {}
            // A restart is under way already: what this one was for is done.
            _ if self.asked.is_some() => {
                if target.source == RestartSource::OneOff {
                    effects.push(Effect::OneOffDropped {
                        one_off: PlannedRestart {
                            at: target.at,
                            by: target.made_by().to_owned(),
                        },
                        reason: DropReason::ServerRestarted,
                    });
                }
            }
            _ if late > GRACE => self.miss(target, Miss::Late { by: late }, effects),
            Some(ServerState::Online | ServerState::Degraded) => {
                // Nothing that was due by now fires after this restart.
                self.settle(now);
                self.asked = Some(Asked {
                    generation: self.server.as_ref().map_or(0, |view| view.generation),
                    target: target.clone(),
                });
                effects.push(Effect::Restart(target));
            }
            Some(state) => self.miss(target, Miss::ServerBusy { state: Some(state) }, effects),
        }
    }

    fn miss(&mut self, target: Target, reason: Miss, effects: &mut Vec<Effect>) {
        if self.announced(target.at) && self.online() {
            effects.push(Effect::Announce(BridgeEvent::RestartSkipped {
                at: target.at.as_millisecond(),
                by: OBSERVER.to_owned(),
                reason: target.source,
            }));
        }
        effects.push(Effect::Missed { target, reason });
    }

    fn settle(&mut self, at: Timestamp) {
        if self.plan.settled.is_none_or(|settled| settled < at) {
            self.plan.settled = Some(at);
        }
    }

    /// Resources heard of the restart that was next before a user's action: they hear what
    /// became of it.
    fn tell_of_change(&self, previous: &Target, by: &str, change: Change, effects: &mut Vec<Effect>) {
        let resolved = effects.iter().any(|effect| {
            matches!(effect, Effect::Restart(target) | Effect::Missed { target, .. } if target.at == previous.at)
        });
        let current = self.armed.as_ref();
        if resolved
            || !self.announced(previous.at)
            || !self.online()
            || current.is_some_and(|current| current.at == previous.at)
        {
            return;
        }
        let event = match (change, current) {
            (Change::Moved, Some(current)) => BridgeEvent::RestartRescheduled {
                at: current.at.as_millisecond(),
                previous_at: previous.at.as_millisecond(),
                by: by.to_owned(),
                reason: current.source,
            },
            _ => BridgeEvent::RestartSkipped {
                at: previous.at.as_millisecond(),
                by: by.to_owned(),
                reason: previous.source,
            },
        };
        effects.push(Effect::Announce(event));
    }

    fn announced(&self, at: Timestamp) -> bool {
        self.warned.contains_key(&at)
    }

    fn online(&self) -> bool {
        self.server
            .as_ref()
            .is_some_and(|view| matches!(view.state, ServerState::Online | ServerState::Degraded))
    }

    fn warn(&mut self, settings: &Settings, now: Timestamp, effects: &mut Vec<Effect>) {
        self.warned.retain(|at, _| *at + GRACE > now);
        let Some((at, source)) = self.armed.as_ref().map(|target| (target.at, target.source)) else {
            return;
        };
        if !self.online() {
            return;
        }
        let left = at.duration_since(now);
        let below = self.warned.get(&at).copied().unwrap_or(u32::MAX);
        let Some(tier) = settings
            .stored
            .warnings
            .iter()
            .copied()
            .filter(|&tier| tier < below && left <= minutes(tier))
            .min()
        else {
            return;
        };
        self.warned.insert(at, tier);
        let seconds = (left.as_millis().max(0) + 500) / 1000;
        effects.push(Effect::Announce(BridgeEvent::RestartScheduled {
            at: at.as_millisecond(),
            seconds_remaining: u64::try_from(seconds).unwrap_or(u64::MAX),
            reason: source,
        }));
    }

    /// The next restart: the one-off, or the schedule's first occurrence neither skipped nor
    /// settled, at its delayed time when delayed. The one-off wins a tie.
    fn target(&self, settings: &Settings, now: Timestamp) -> Option<Target> {
        let schedule = self
            .candidates(settings, now)
            .into_iter()
            .find(|candidate| candidate.skipped.is_none())
            .map(|candidate| Target {
                at: candidate.at,
                source: RestartSource::Schedule,
                occurrence: candidate.occurrence,
                by: candidate.delayed.map(|delayed| delayed.by),
            });
        let one_off = self.plan.one_off.as_ref().map(|one_off| Target {
            at: one_off.at,
            source: RestartSource::OneOff,
            occurrence: one_off.at,
            by: Some(one_off.by.clone()),
        });
        match (schedule, one_off) {
            (Some(schedule), Some(one_off)) if schedule.at < one_off.at => Some(schedule),
            (schedule, one_off) => one_off.or(schedule),
        }
    }

    /// The occurrences that may still happen, in the order they would: those after `now` and
    /// `settled`, and the armed or delayed one even when its own time is past.
    fn candidates(&self, settings: &Settings, now: Timestamp) -> Vec<Candidate> {
        let stored = &settings.stored;
        if !stored.enabled {
            return Vec::new();
        }
        let floor = self.plan.settled.map_or(now, |settled| settled.max(now));
        let count = UPCOMING + 2;
        let mut found = match stored.mode {
            ScheduleMode::FixedTimes => fixed_occurrences(&stored.times, &settings.zone, floor, count),
            ScheduleMode::Interval => self
                .started_at()
                .map(|started_at| interval_occurrences(started_at, stored.interval_minutes, floor, count))
                .unwrap_or_default(),
        };
        let pending = [
            self.armed
                .as_ref()
                .filter(|armed| armed.source == RestartSource::Schedule)
                .map(|armed| armed.occurrence),
            self.plan.delay.as_ref().map(|delay| delay.occurrence),
        ];
        for occurrence in pending.into_iter().flatten() {
            let settled = self.plan.settled.is_some_and(|settled| occurrence <= settled);
            if occurrence <= floor && !settled && !found.iter().any(|(at, _)| *at == occurrence) {
                found.extend(self.occurrence_at(settings, occurrence));
            }
        }
        let mut candidates: Vec<Candidate> = found
            .into_iter()
            .map(|(occurrence, time)| {
                let delay = self
                    .plan
                    .delay
                    .as_ref()
                    .filter(|delay| delay.occurrence == occurrence);
                let manual = self.plan.skip.as_ref().filter(|skip| skip.at == occurrence);
                let recent = self
                    .auto_skip
                    .filter(|skip| skip.at == occurrence && stored.mode == ScheduleMode::FixedTimes);
                Candidate {
                    occurrence,
                    at: delay.map_or(occurrence, |delay| delay.to),
                    time,
                    skipped: manual
                        .map(|skip| Skipped::Manual { by: skip.by.clone() })
                        .or(recent.map(|skip| Skipped::RecentStart {
                            started_at: skip.started_at,
                        })),
                    delayed: delay.map(|delay| Delayed {
                        to: delay.to,
                        by: delay.by.clone(),
                    }),
                }
            })
            .collect();
        candidates.sort_by_key(|candidate| (candidate.at, candidate.occurrence));
        candidates
    }

    /// Whether the schedule, as set now, has an occurrence at `at`.
    fn produced(&self, settings: &Settings, at: Timestamp) -> bool {
        self.occurrence_at(settings, at).is_some()
    }

    /// The schedule's occurrence at `at`, with the time it comes from, if it has one.
    fn occurrence_at(&self, settings: &Settings, at: Timestamp) -> Option<(Timestamp, Option<String>)> {
        let stored = &settings.stored;
        if !stored.enabled {
            return None;
        }
        let before = at - SignedDuration::from_nanos(1);
        let first = match stored.mode {
            ScheduleMode::FixedTimes => fixed_occurrences(&stored.times, &settings.zone, before, 1),
            ScheduleMode::Interval => {
                interval_occurrences(self.started_at()?, stored.interval_minutes, before, 1)
            }
        };
        first.into_iter().next().filter(|(first, _)| *first == at)
    }

    /// The occurrence after `occurrence`, which a delay must stay before.
    fn following(&self, settings: &Settings, occurrence: Timestamp) -> Option<Timestamp> {
        let stored = &settings.stored;
        let next = match stored.mode {
            ScheduleMode::FixedTimes => fixed_occurrences(&stored.times, &settings.zone, occurrence, 1),
            ScheduleMode::Interval => {
                interval_occurrences(self.started_at()?, stored.interval_minutes, occurrence, 1)
            }
        };
        next.first().map(|(at, _)| *at)
    }

    fn started_at(&self) -> Option<Timestamp> {
        self.server.as_ref().and_then(|view| view.started_at)
    }
}

/// The first `count` instants after `after` at which one of `times` falls in `zone`, with the
/// time each comes from. A time a clock change skips happens as much later as the change lasts; a
/// time it repeats happens on its first pass. Two times on one instant are one occurrence.
fn fixed_occurrences(
    times: &[String],
    zone: &TimeZone,
    after: Timestamp,
    count: usize,
) -> Vec<(Timestamp, Option<String>)> {
    let minutes: Vec<(i32, &String)> = times
        .iter()
        .filter_map(|time| Some((parse_time(time)?, time)))
        .collect();
    if minutes.is_empty() || count == 0 {
        return Vec::new();
    }
    let days = count.div_ceil(minutes.len()) + 2;
    let start = after.to_zoned(zone.clone()).date();
    let mut date = start.yesterday().unwrap_or(start);
    let mut found: Vec<(Timestamp, bool, &String)> = Vec::new();
    for _ in 0..days {
        for &(minute, time) in &minutes {
            if let Some(at) = wall_time(date, minute, zone).filter(|at| *at > after) {
                let moved = local_time(at, zone) != *time;
                found.push((at, moved, time));
            }
        }
        date = match date.tomorrow() {
            Ok(next) => next,
            Err(_) => break,
        };
    }
    // Of two times on one instant, the one the clock change did not move names it.
    found.sort_unstable();
    found.dedup_by_key(|(at, _, _)| *at);
    found
        .into_iter()
        .take(count)
        .map(|(at, _, time)| (at, Some(time.clone())))
        .collect()
}

/// `started_at + k × interval`, k ≥ 1, in real time, the first `count` after `after`.
fn interval_occurrences(
    started_at: Timestamp,
    interval_minutes: u32,
    after: Timestamp,
    count: usize,
) -> Vec<(Timestamp, Option<String>)> {
    let interval = minutes(interval_minutes).as_nanos();
    if interval <= 0 {
        return Vec::new();
    }
    let elapsed = after.duration_since(started_at).as_nanos();
    let first = if elapsed < 0 { 1 } else { elapsed / interval + 1 };
    (0..count)
        .filter_map(|index| {
            let k = first + i128::try_from(index).ok()?;
            let offset = i64::try_from(k * interval).ok()?;
            started_at
                .checked_add(SignedDuration::from_nanos(offset))
                .ok()
                .map(|at| (at, None))
        })
        .collect()
}

fn wall_time(date: jiff::civil::Date, minute: i32, zone: &TimeZone) -> Option<Timestamp> {
    let hour = i8::try_from(minute / 60).ok()?;
    let minute = i8::try_from(minute % 60).ok()?;
    date.at(hour, minute, 0, 0)
        .to_zoned(zone.clone())
        .ok()
        .map(|zoned| zoned.timestamp())
}

fn local_time(at: Timestamp, zone: &TimeZone) -> String {
    let zoned = at.to_zoned(zone.clone());
    format!("{:02}:{:02}", zoned.hour(), zoned.minute())
}

fn minutes(count: u32) -> SignedDuration {
    SignedDuration::from_mins(i64::from(count))
}

#[cfg(test)]
mod tests;
