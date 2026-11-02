//! FXServer's restart schedule: its settings, the one-off changes users make to it, and the task
//! that follows FXServer and the clock to warn resources and restart the server on time.

mod engine;

use std::time::Duration;

use jiff::Timestamp;
use jiff::tz::TimeZone;
use observer_domain::audit::{ActorKind, AuditAction, Outcome};
use observer_domain::error::{ApiError, ErrorCode};
use observer_domain::schedule::{
    OneOffRequest, PlannedDelay, PlannedRestart, RestartSource, ScheduleChange, ScheduleSettings,
    ScheduleStatus, StoredSchedule,
};
use observer_domain::server::ServerStatus;
use observer_store::{Database, schedule};
use serde_json::{Map, Value, json};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

pub(crate) use self::engine::fixed_occurrences;
use self::engine::{Applied, DropReason, Effect, Input, Miss, ServerView, Settings, State, Target};
use crate::audit::Record;
use crate::supervision::{ScheduledRefusal, ScheduledRestart};
use crate::{Caller, Clock, Error, Servers, Supervision};

const COMMANDS: usize = 16;

/// What [`Schedule::start`] works with.
pub struct ScheduleSetup {
    pub db: Database,
    pub clock: Clock,
    pub supervision: Supervision,
}

#[derive(Debug, Clone, Copy)]
pub struct ScheduleTimings {
    /// The longest the task sleeps between two looks at the clock. Its sleeps run on a monotonic
    /// clock while restarts are wall-clock instants: waking often absorbs a clock that jumps and a
    /// host that slept.
    pub longest_sleep: Duration,
}

impl Default for ScheduleTimings {
    fn default() -> Self {
        Self {
            longest_sleep: Duration::from_secs(30),
        }
    }
}

/// The schedule of the default server. Clones talk to the same task.
#[derive(Clone)]
pub struct Schedule {
    commands: mpsc::Sender<(Command, Reply)>,
    status: watch::Receiver<ScheduleStatus>,
}

/// The schedule's task; [`ScheduleTask::close`] ends it.
pub struct ScheduleTask {
    task: JoinHandle<()>,
    stop: CancellationToken,
}

impl ScheduleTask {
    pub async fn close(self) {
        self.stop.cancel();
        if let Err(error) = self.task.await {
            warn!(%error, "the schedule task failed");
        }
    }
}

type Reply = oneshot::Sender<Result<ScheduleStatus, Error>>;

enum Command {
    Change { caller: Caller, change: ScheduleChange },
    Skip { caller: Caller },
    Restore { caller: Caller },
    Delay { caller: Caller, minutes: u32 },
    RestartOnce { caller: Caller, request: OneOffRequest },
    CancelOnce { caller: Caller },
}

impl Schedule {
    /// Reads the stored settings and plan, then follows FXServer. Starting it before FXServer
    /// boots makes Observer's own start count as one for the recent start rule. `tune` adjusts the
    /// timings, for tests.
    pub async fn start(
        setup: ScheduleSetup,
        tune: impl FnOnce(&mut ScheduleTimings),
    ) -> Result<(Self, ScheduleTask), Error> {
        let mut timings = ScheduleTimings::default();
        tune(&mut timings);
        let (stored, plan) = setup
            .db
            .read(|conn| {
                Ok::<_, Error>((
                    schedule::settings(conn, Servers::DEFAULT)?,
                    schedule::plan(conn, Servers::DEFAULT)?,
                ))
            })
            .await?;
        let settings = resolve(stored);
        let state = State::restored(plan);
        let now = setup.clock.now();
        let (status_tx, status) = watch::channel(engine::status(&state, &settings, now));
        let (commands, commands_rx) = mpsc::channel(COMMANDS);
        let stop = CancellationToken::new();
        let driver = Driver {
            db: setup.db,
            clock: setup.clock,
            supervision: setup.supervision,
            timings,
            settings,
            state,
            status: status_tx,
            seen: None,
            due: None,
            retry: None,
        };
        let task = tokio::spawn(driver.run(commands_rx, stop.clone()));
        Ok((Self { commands, status }, ScheduleTask { task, stop }))
    }

    /// The status as it changes.
    pub fn status(&self) -> watch::Receiver<ScheduleStatus> {
        self.status.clone()
    }

    /// Stores settings that pass every check, audited with what changed. `version` must be the
    /// one the change was made from, `None` before the first save. Saving the same settings again
    /// changes and audits nothing.
    pub async fn change_settings(
        &self,
        caller: &Caller,
        change: ScheduleChange,
    ) -> Result<ScheduleStatus, Error> {
        self.ask(Command::Change {
            caller: caller.clone(),
            change,
        })
        .await
    }

    /// Skips the schedule's next occurrence.
    pub async fn skip(&self, caller: &Caller) -> Result<ScheduleStatus, Error> {
        self.ask(Command::Skip {
            caller: caller.clone(),
        })
        .await
    }

    /// Brings back the skipped occurrence while its time is ahead.
    pub async fn restore(&self, caller: &Caller) -> Result<ScheduleStatus, Error> {
        self.ask(Command::Restore {
            caller: caller.clone(),
        })
        .await
    }

    /// Moves the schedule's next restart `minutes` later.
    pub async fn delay(&self, caller: &Caller, minutes: u32) -> Result<ScheduleStatus, Error> {
        self.ask(Command::Delay {
            caller: caller.clone(),
            minutes,
        })
        .await
    }

    /// Plans a single restart of the running server, in place of any planned before.
    pub async fn restart_once(
        &self,
        caller: &Caller,
        request: OneOffRequest,
    ) -> Result<ScheduleStatus, Error> {
        self.ask(Command::RestartOnce {
            caller: caller.clone(),
            request,
        })
        .await
    }

    pub async fn cancel_once(&self, caller: &Caller) -> Result<ScheduleStatus, Error> {
        self.ask(Command::CancelOnce {
            caller: caller.clone(),
        })
        .await
    }

    async fn ask(&self, command: Command) -> Result<ScheduleStatus, Error> {
        let (reply, answer) = oneshot::channel();
        self.commands
            .send((command, reply))
            .await
            .map_err(|_| ErrorCode::ShuttingDown)?;
        answer.await.map_err(|_| ErrorCode::ShuttingDown)?
    }
}

struct Driver {
    db: Database,
    clock: Clock,
    supervision: Supervision,
    timings: ScheduleTimings,
    settings: Settings,
    state: State,
    status: watch::Sender<ScheduleStatus>,
    /// FXServer as last handed to the engine.
    seen: Option<ServerView>,
    /// When the engine next has something to do, as worked out at its last step. A clock that
    /// jumped past it makes it due at once, at the next turn of the loop, whatever woke it.
    due: Option<Timestamp>,
    /// When to try again after a step that failed. It is a monotonic instant, as the loop's sleeps
    /// are, so neither the loop's other turns nor a clock set back put it off.
    retry: Option<Instant>,
}

impl Driver {
    async fn run(mut self, mut commands: mpsc::Receiver<(Command, Reply)>, stop: CancellationToken) {
        let mut server = self.supervision.status();
        let first = view(&server.borrow_and_update());
        self.server(first).await;
        loop {
            let sleep = if let Some(at) = self.retry {
                at.saturating_duration_since(Instant::now())
            } else {
                let now = self.clock.now();
                self.due.map_or(self.timings.longest_sleep, |at| {
                    Duration::try_from(now.duration_until(at))
                        .unwrap_or(Duration::ZERO)
                        .min(self.timings.longest_sleep)
                })
            };
            tokio::select! {
                () = stop.cancelled() => return,
                command = commands.recv() => match command {
                    Some((command, reply)) => {
                        let _ = reply.send(self.command(command).await);
                    }
                    None => return,
                },
                changed = server.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    let next = view(&server.borrow_and_update());
                    self.server(next).await;
                }
                () = tokio::time::sleep(sleep) => {
                    if let Err(error) = self.apply(Input::Tick, None, None).await {
                        warn!(%error, "the schedule could not catch up with the clock");
                        // Tried again after the longest sleep, not in a loop.
                        self.retry = Some(Instant::now() + self.timings.longest_sleep);
                    }
                }
            }
        }
    }

    async fn server(&mut self, view: ServerView) {
        if self.seen.as_ref() == Some(&view) {
            return;
        }
        self.seen = Some(view.clone());
        if let Err(error) = self.apply(Input::Server(view), None, None).await {
            warn!(%error, "the schedule could not follow FXServer");
        }
    }

    async fn command(&mut self, command: Command) -> Result<ScheduleStatus, Error> {
        let (caller, input) = match command {
            Command::Change { caller, change } => return self.change(&caller, change).await,
            Command::Skip { caller } => {
                let by = caller.username.clone();
                (caller, Input::Skip { by })
            }
            Command::Restore { caller } => {
                let by = caller.username.clone();
                (caller, Input::Restore { by })
            }
            Command::Delay { caller, minutes } => {
                let by = caller.username.clone();
                (caller, Input::Delay { minutes, by })
            }
            Command::RestartOnce { caller, request } => {
                let at = engine::resolve_one_off(&request, &self.settings.zone, self.clock.now())?;
                let by = caller.username.clone();
                (caller, Input::OneOff { at, by })
            }
            Command::CancelOnce { caller } => {
                let by = caller.username.clone();
                (caller, Input::CancelOneOff { by })
            }
        };
        self.apply(input, Some(&caller), None).await
    }

    async fn change(&mut self, caller: &Caller, change: ScheduleChange) -> Result<ScheduleStatus, Error> {
        let issues = change.settings.issues();
        if !issues.is_empty() {
            return Err(ApiError::new(ErrorCode::InvalidSchedule)
                .with("issues", serde_json::to_value(issues).unwrap_or_default())
                .into());
        }
        // This task is the only writer of the settings: the version it holds is the stored one.
        if change.version != self.settings.version {
            return Err(ErrorCode::Conflict.into());
        }
        let settings = change.settings.normalized();
        if settings == self.settings.stored {
            return Ok(self.publish(self.clock.now()));
        }
        let stored = StoredSchedule {
            settings,
            version: self.settings.version.map_or(1, |version| version + 1),
        };
        let next = resolve(Some(stored));
        let by = caller.username.clone();
        self.apply(Input::Settings { by }, Some(caller), Some(next)).await
    }

    /// Runs one step of the engine and does what it says. Its writes commit before anything else
    /// happens, and the new state is kept only once they have.
    async fn apply(
        &mut self,
        input: Input,
        caller: Option<&Caller>,
        new_settings: Option<Settings>,
    ) -> Result<ScheduleStatus, Error> {
        let mut input = input;
        let mut caller = caller;
        let mut new_settings = new_settings;
        loop {
            let now = self.clock.now();
            let settings = new_settings.as_ref().unwrap_or(&self.settings);
            let (state, effects) = engine::step(self.state.clone(), settings, input, now);
            if let [Effect::Refused(error)] = effects.as_slice() {
                return Err(error.clone().into());
            }

            let mut records: Vec<Record> = effects
                .iter()
                .filter_map(|effect| record(effect, caller))
                .collect();
            let stored = new_settings.as_ref().map(|settings| StoredSchedule {
                settings: settings.stored.clone(),
                version: settings.version.unwrap_or(1),
            });
            if let (Some(caller), Some(settings)) = (caller, &new_settings) {
                let dropped = effects.iter().find_map(|effect| match effect {
                    Effect::Dropped { skip, delay } => Some(dropped(skip.as_ref(), delay.as_ref())),
                    _ => None,
                });
                let details = changes(&self.settings.stored, &settings.stored, dropped);
                records.insert(
                    0,
                    Record::by(caller, AuditAction::ScheduleChanged)
                        .target(Servers::DEFAULT)
                        .details(details),
                );
            }
            let plan = effects.contains(&Effect::Persist).then(|| state.plan.clone());
            if plan.is_some() || stored.is_some() || !records.is_empty() {
                self.db
                    .write(move |tx| -> Result<(), Error> {
                        if let Some(stored) = &stored {
                            schedule::save_settings(tx, Servers::DEFAULT, stored, now)?;
                        }
                        if let Some(plan) = &plan {
                            schedule::save_plan(tx, Servers::DEFAULT, plan, now)?;
                        }
                        for record in records {
                            record.write(tx, now)?;
                        }
                        Ok(())
                    })
                    .await?;
            }
            self.state = state;
            if let Some(settings) = new_settings.take() {
                self.settings = settings;
            }
            self.due = engine::deadline(&self.state, &self.settings, now);
            self.retry = None;

            let mut refused = None;
            for effect in effects {
                match effect {
                    Effect::Announce(event) => {
                        if let Err(error) = self.supervision.announce(event).await {
                            debug!(%error, "a schedule event did not reach the bridge");
                        }
                    }
                    Effect::Restart(target) => refused = self.restart(&target).await,
                    Effect::SettledAhead(at) => warn!(
                        settled = %at,
                        "the schedule's last restart was recorded more than a day ahead of the clock; \
                         the clock was wrong then, so the mark is dropped"
                    ),
                    _ => {}
                }
            }
            match refused {
                Some(reason) => {
                    input = Input::RestartRefused { reason };
                    caller = None;
                }
                None => return Ok(self.publish(now)),
            }
        }
    }

    async fn restart(&self, target: &Target) -> Option<Miss> {
        let local = target.at.to_zoned(self.settings.zone.clone());
        let restart = ScheduledRestart {
            at: target.at,
            source: target.source,
            by: target.made_by().to_owned(),
            planned_by: target
                .by
                .clone()
                .filter(|_| target.source == RestartSource::OneOff),
            local: format!(
                "{:02}:{:02} {}",
                local.hour(),
                local.minute(),
                self.settings.zone_name
            ),
        };
        match self.supervision.restart_on_schedule(&restart).await {
            Err(ScheduledRefusal::LaunchInvalid) => Some(Miss::LaunchInvalid),
            Err(ScheduledRefusal::Busy) => Some(Miss::ServerBusy { state: None }),
            // A refusal while Observer stops says nothing: FXServer stops with it.
            Ok(()) | Err(ScheduledRefusal::ShuttingDown) => None,
        }
    }

    fn publish(&self, now: Timestamp) -> ScheduleStatus {
        let next = engine::status(&self.state, &self.settings, now);
        self.status.send_if_modified(|current| {
            let changed = *current != next;
            if changed {
                current.clone_from(&next);
            }
            changed
        });
        next
    }
}

fn view(status: &ServerStatus) -> ServerView {
    ServerView {
        state: status.state,
        generation: status.generation,
        started_at: status.started_at,
    }
}

/// The settings in force, with the zone they name, or the host's. A host zone without an IANA
/// name, as some Linux containers have, falls back to UTC.
fn resolve(stored: Option<StoredSchedule>) -> Settings {
    let (stored, version) = match stored {
        Some(stored) => (stored.settings, Some(stored.version)),
        None => (ScheduleSettings::default(), None),
    };
    let named = stored
        .time_zone
        .as_deref()
        .and_then(|name| match TimeZone::get(name) {
            Ok(zone) => Some((zone, name.to_owned())),
            Err(error) => {
                warn!(zone = name, %error, "the schedule's time zone is unknown here; it follows the host's");
                None
            }
        });
    let (zone, zone_name) = named.unwrap_or_else(|| {
        let host = TimeZone::system();
        if let Some(name) = host.iana_name().map(str::to_owned) {
            (host, name)
        } else {
            warn!("the host's time zone has no IANA name; the schedule follows UTC");
            (TimeZone::UTC, "UTC".to_owned())
        }
    });
    info!(zone = %zone_name, "schedule time zone");
    Settings {
        stored,
        version,
        zone,
        zone_name,
    }
}

/// Each setting that differs, with its value before and after, and what the change dropped.
fn changes(before: &ScheduleSettings, after: &ScheduleSettings, dropped: Option<Value>) -> Value {
    let fields = |settings: &ScheduleSettings| match serde_json::to_value(settings) {
        Ok(Value::Object(fields)) => fields,
        _ => Map::new(),
    };
    let old = fields(before);
    let mut changed: Map<String, Value> = fields(after)
        .into_iter()
        .filter_map(|(name, value)| {
            let previous = old.get(&name).cloned().unwrap_or(Value::Null);
            (previous != value).then(|| (name, json!({ "before": previous, "after": value })))
        })
        .collect();
    if let Some(dropped) = dropped {
        changed.insert("dropped".to_owned(), dropped);
    }
    Value::Object(changed)
}

fn dropped(skip: Option<&PlannedRestart>, delay: Option<&PlannedDelay>) -> Value {
    let skip = skip.map(|skip| json!({ "kind": "skip", "at": skip.at, "by": skip.by }));
    let delay = delay.map(
        |delay| json!({ "kind": "delay", "occurrence": delay.occurrence, "to": delay.to, "by": delay.by }),
    );
    Value::Array(skip.into_iter().chain(delay).collect())
}

/// The audit entry an effect calls for, if any: a user's action, or what the schedule did by
/// itself.
fn record(effect: &Effect, caller: Option<&Caller>) -> Option<Record> {
    let system = |action| Record::operator(ActorKind::System, action).target(Servers::DEFAULT);
    match effect {
        Effect::Applied(applied) => {
            let (action, details) = match applied {
                Applied::Skipped { at } => (AuditAction::ScheduledRestartSkipped, json!({ "at": at })),
                Applied::Restored { at } => (AuditAction::ScheduledRestartRestored, json!({ "at": at })),
                Applied::Delayed {
                    occurrence,
                    at,
                    previous_at,
                } => (
                    AuditAction::ScheduledRestartDelayed,
                    json!({ "occurrence": occurrence, "at": at, "previousAt": previous_at }),
                ),
                Applied::Planned { at, previous_at } => {
                    let mut details = json!({ "at": at });
                    if let Some(previous_at) = previous_at {
                        details["previousAt"] = json!(previous_at);
                    }
                    (AuditAction::OneOffRestartPlanned, details)
                }
                Applied::Cancelled { at } => (AuditAction::OneOffRestartCancelled, json!({ "at": at })),
            };
            Some(
                Record::by(caller?, action)
                    .target(Servers::DEFAULT)
                    .details(details),
            )
        }
        Effect::Missed { target, reason } => {
            let mut details = json!({ "at": target.at, "source": target.source });
            match reason {
                Miss::Late { by } => {
                    details["reason"] = json!("late");
                    details["lateBySeconds"] = json!(by.as_secs());
                }
                Miss::ServerBusy { state } => {
                    details["reason"] = json!("serverBusy");
                    if let Some(state) = state {
                        details["state"] = json!(state);
                    }
                }
                Miss::LaunchInvalid => details["reason"] = json!("launchInvalid"),
            }
            Some(
                system(AuditAction::ScheduledRestartMissed)
                    .outcome(Outcome::Failure)
                    .details(details),
            )
        }
        Effect::AutoSkipped { at, started_at } => Some(
            system(AuditAction::ScheduledRestartSkipped)
                .details(json!({ "at": at, "reason": "recentStart", "startedAt": started_at })),
        ),
        Effect::OneOffDropped { one_off, reason } => {
            let reason = match reason {
                DropReason::ServerRestarted => "serverRestarted",
                DropReason::ServerStopped => "serverStopped",
            };
            Some(system(AuditAction::OneOffRestartCancelled).details(json!({
                "at": one_off.at,
                "plannedBy": one_off.by,
                "reason": reason,
            })))
        }
        Effect::Announce(_)
        | Effect::Restart(_)
        | Effect::Persist
        | Effect::Dropped { .. }
        | Effect::SettledAhead(_)
        | Effect::Refused(_) => None,
    }
}
