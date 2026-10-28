use observer_domain::schedule::{DEFAULT_WARNINGS, ScheduleSettings};

use super::*;

fn ts(text: &str) -> Timestamp {
    text.parse().unwrap()
}

fn fixed(zone: &str, times: &[&str]) -> Settings {
    Settings {
        stored: ScheduleSettings {
            enabled: true,
            times: times.iter().map(|&time| time.to_owned()).collect(),
            skip_after_start: 0,
            ..ScheduleSettings::default()
        },
        version: Some(1),
        zone: TimeZone::get(zone).unwrap(),
        zone_name: zone.to_owned(),
    }
}

fn paris(times: &[&str]) -> Settings {
    fixed("Europe/Paris", times)
}

fn interval(minutes: u32) -> Settings {
    let mut settings = paris(&[]);
    settings.stored.mode = ScheduleMode::Interval;
    settings.stored.interval_minutes = minutes;
    settings
}

fn view(state: ServerState, generation: u32, started_at: Option<&str>) -> ServerView {
    ServerView {
        state,
        generation,
        started_at: started_at.map(ts),
    }
}

fn scheduled(at: &str, seconds_remaining: u64) -> BridgeEvent {
    BridgeEvent::RestartScheduled {
        at: ts(at).as_millisecond(),
        seconds_remaining,
        reason: RestartSource::Schedule,
    }
}

/// The engine as the driver runs it, without the 30-second cap: a step at each deadline.
struct Sim {
    state: State,
    settings: Settings,
    now: Timestamp,
}

impl Sim {
    fn new(settings: Settings, now: &str) -> Self {
        Self::restored(settings, now, SchedulePlan::default())
    }

    fn restored(settings: Settings, now: &str, plan: SchedulePlan) -> Self {
        Self {
            state: State::restored(plan),
            settings,
            now: ts(now),
        }
    }

    /// A server online since `started_at`, as generation 1.
    fn running(settings: Settings, started_at: &str, now: &str) -> Self {
        let mut sim = Self::new(settings, now);
        sim.send(Input::Server(view(ServerState::Online, 1, Some(started_at))));
        sim
    }

    fn send(&mut self, input: Input) -> Vec<Effect> {
        let (state, effects) = step(self.state.clone(), &self.settings, input, self.now);
        self.state = state;
        if let Some(deadline) = deadline(&self.state, &self.settings, self.now) {
            assert!(deadline > self.now, "a deadline at or before now loops");
        }
        effects
    }

    fn at(&mut self, now: &str, input: Input) -> Vec<Effect> {
        self.now = ts(now);
        self.send(input)
    }

    fn tick(&mut self, now: &str) -> Vec<Effect> {
        self.at(now, Input::Tick)
    }

    fn server(
        &mut self,
        now: &str,
        state: ServerState,
        generation: u32,
        started_at: Option<&str>,
    ) -> Vec<Effect> {
        self.at(now, Input::Server(view(state, generation, started_at)))
    }

    /// Steps at every deadline up to `until`, then at `until`.
    fn run_until(&mut self, until: &str) -> Vec<(Timestamp, Effect)> {
        let until = ts(until);
        let mut seen = Vec::new();
        while let Some(next) = deadline(&self.state, &self.settings, self.now).filter(|next| *next <= until) {
            self.now = next;
            seen.extend(self.send(Input::Tick).into_iter().map(|effect| (next, effect)));
        }
        self.now = until;
        seen.extend(self.send(Input::Tick).into_iter().map(|effect| (until, effect)));
        seen
    }

    fn next(&self) -> Option<Timestamp> {
        self.state.armed.as_ref().map(|target| target.at)
    }

    fn status(&self) -> ScheduleStatus {
        status(&self.state, &self.settings, self.now)
    }
}

fn restarts(log: &[(Timestamp, Effect)]) -> Vec<Timestamp> {
    log.iter()
        .filter_map(|(now, effect)| matches!(effect, Effect::Restart(_)).then_some(*now))
        .collect()
}

fn announcements(log: &[(Timestamp, Effect)]) -> Vec<(Timestamp, BridgeEvent)> {
    log.iter()
        .filter_map(|(now, effect)| match effect {
            Effect::Announce(event) => Some((*now, event.clone())),
            _ => None,
        })
        .collect()
}

fn events(effects: &[Effect]) -> Vec<BridgeEvent> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Announce(event) => Some(event.clone()),
            _ => None,
        })
        .collect()
}

fn missed(effects: &[Effect]) -> Vec<Miss> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Missed { reason, .. } => Some(*reason),
            _ => None,
        })
        .collect()
}

fn refusal(effects: &[Effect]) -> Option<ErrorCode> {
    match effects {
        [Effect::Refused(error)] => Some(error.code),
        _ => None,
    }
}

fn persisted(effects: &[Effect]) -> bool {
    effects.contains(&Effect::Persist)
}

fn occurrences(settings: &Settings, after: &str, count: usize) -> Vec<Timestamp> {
    fixed_occurrences(&settings.stored.times, &settings.zone, ts(after), count)
        .into_iter()
        .map(|(at, _)| at)
        .collect()
}

#[test]
fn a_time_in_the_hour_the_clock_skips_fires_after_the_gap_with_its_warning_half_an_hour_before() {
    let mut sim = Sim::running(paris(&["02:30"]), "2026-03-28T20:00:00Z", "2026-03-28T22:00:00Z");

    let log = sim.run_until("2026-03-29T02:00:00Z");

    assert_eq!(restarts(&log), [ts("2026-03-29T01:30:00Z")]);
    assert_eq!(
        announcements(&log)[0],
        (
            ts("2026-03-29T01:00:00Z"),
            scheduled("2026-03-29T01:30:00Z", 1800)
        )
    );
    assert_eq!(announcements(&log).len(), DEFAULT_WARNINGS.len());
}

#[test]
fn two_times_the_clock_change_puts_on_one_instant_fire_once() {
    let mut sim = Sim::running(
        paris(&["02:30", "03:30"]),
        "2026-03-28T20:00:00Z",
        "2026-03-28T22:00:00Z",
    );

    let log = sim.run_until("2026-03-29T12:00:00Z");

    assert_eq!(restarts(&log), [ts("2026-03-29T01:30:00Z")]);
    assert_eq!(sim.next(), Some(ts("2026-03-30T00:30:00Z")));
}

#[test]
fn the_day_the_clock_goes_forward_is_23_hours_long() {
    let at_four = occurrences(&paris(&["04:00"]), "2026-03-28T00:00:00Z", 2);

    assert_eq!(at_four, [ts("2026-03-28T03:00:00Z"), ts("2026-03-29T02:00:00Z")]);
    assert_eq!(
        at_four[1].duration_since(at_four[0]),
        SignedDuration::from_hours(23)
    );
}

#[test]
fn a_time_in_the_hour_the_clock_repeats_fires_on_its_first_pass_only() {
    let mut sim = Sim::running(paris(&["02:30"]), "2026-10-24T20:00:00Z", "2026-10-24T21:00:00Z");

    let log = sim.run_until("2026-10-25T12:00:00Z");

    assert_eq!(restarts(&log), [ts("2026-10-25T00:30:00Z")]);
    assert_eq!(sim.next(), Some(ts("2026-10-26T01:30:00Z")));
}

#[test]
fn the_day_the_clock_goes_back_is_25_hours_long() {
    let at_four = occurrences(&paris(&["04:00"]), "2026-10-24T00:00:00Z", 2);

    assert_eq!(at_four, [ts("2026-10-24T02:00:00Z"), ts("2026-10-25T03:00:00Z")]);
    assert_eq!(
        at_four[1].duration_since(at_four[0]),
        SignedDuration::from_hours(25)
    );
}

#[test]
fn other_zones_follow_their_own_clock_changes() {
    let new_york = fixed("America/New_York", &["02:30", "13:30"]);
    let new_york_fall = fixed("America/New_York", &["01:30"]);
    let lord_howe = fixed("Australia/Lord_Howe", &["02:15"]);

    assert_eq!(
        occurrences(&new_york, "2026-03-08T05:00:00Z", 1),
        [ts("2026-03-08T07:30:00Z")]
    );
    assert_eq!(
        occurrences(&new_york_fall, "2026-11-01T04:00:00Z", 2),
        [ts("2026-11-01T05:30:00Z"), ts("2026-11-02T06:30:00Z")]
    );
    assert_eq!(
        occurrences(&lord_howe, "2026-10-03T00:00:00Z", 1),
        [ts("2026-10-03T15:45:00Z")],
        "02:15 falls in a half-hour gap and happens at 02:45 +11:00"
    );
    assert_eq!(
        occurrences(&fixed("Asia/Kolkata", &["04:00"]), "2026-10-28T00:00:00Z", 1),
        [ts("2026-10-28T22:30:00Z")]
    );
    assert_eq!(
        occurrences(&fixed("UTC", &["04:00"]), "2026-10-28T00:00:00Z", 1),
        [ts("2026-10-28T04:00:00Z")]
    );
}

#[test]
fn an_interval_counts_real_hours_across_both_clock_changes() {
    let spring = Sim::running(interval(360), "2026-03-28T23:00:00Z", "2026-03-28T23:00:01Z");
    let autumn = Sim::running(interval(360), "2026-10-24T22:00:00Z", "2026-10-24T22:00:01Z");

    assert_eq!(spring.next(), Some(ts("2026-03-29T05:00:00Z")), "07:00 CEST");
    assert_eq!(autumn.next(), Some(ts("2026-10-25T04:00:00Z")), "05:00 CET");
}

#[test]
fn a_crash_recovery_starts_the_interval_over() {
    let mut sim = Sim::running(interval(360), "2026-10-28T00:00:00Z", "2026-10-28T00:00:01Z");

    sim.server("2026-10-28T03:00:00Z", ServerState::Crashed, 1, None);
    let crashed = sim.next();
    sim.server(
        "2026-10-28T03:00:05Z",
        ServerState::Starting,
        2,
        Some("2026-10-28T03:00:05Z"),
    );
    sim.server(
        "2026-10-28T03:00:20Z",
        ServerState::Online,
        2,
        Some("2026-10-28T03:00:05Z"),
    );

    assert_eq!(crashed, None, "no run, no occurrence");
    assert_eq!(sim.next(), Some(ts("2026-10-28T09:00:05Z")));
}

#[test]
fn an_interval_has_nothing_ahead_while_the_server_is_stopped() {
    let mut sim = Sim::new(interval(360), "2026-10-28T00:00:00Z");
    sim.send(Input::Server(view(ServerState::Stopped, 0, None)));

    let log = sim.run_until("2026-10-30T00:00:00Z");

    assert!(log.is_empty(), "{log:?}");
    assert_eq!(sim.next(), None);
    assert_eq!(sim.status().upcoming, []);
}

#[test]
fn a_skip_of_an_interval_occurrence_is_dropped_by_a_new_run() {
    let mut sim = Sim::running(interval(360), "2026-10-28T00:00:00Z", "2026-10-28T01:00:00Z");
    sim.send(Input::Skip { by: "ada".to_owned() });
    assert!(sim.state.plan.skip.is_some());

    sim.server(
        "2026-10-28T02:00:00Z",
        ServerState::Starting,
        2,
        Some("2026-10-28T02:00:00Z"),
    );
    let effects = sim.server(
        "2026-10-28T02:00:10Z",
        ServerState::Online,
        2,
        Some("2026-10-28T02:00:00Z"),
    );

    assert_eq!(sim.state.plan.skip, None);
    assert!(
        !persisted(&effects),
        "the drop was written with the step that saw the run"
    );
    assert_eq!(sim.next(), Some(ts("2026-10-28T08:00:00Z")));
}

#[test]
fn a_clock_jumping_two_minutes_past_the_time_fires_at_once() {
    let mut sim = Sim::running(paris(&["04:00"]), "2026-10-27T20:00:00Z", "2026-10-28T02:59:30Z");

    let effects = sim.tick("2026-10-28T03:02:00Z");

    assert!(
        matches!(effects.as_slice(), [Effect::Restart(target), Effect::Persist] if target.at == ts("2026-10-28T03:00:00Z")),
        "{effects:?}"
    );
}

#[test]
fn a_clock_jumping_ten_minutes_past_the_time_misses_it_and_arms_the_next() {
    let mut sim = Sim::running(
        paris(&["04:00", "16:00"]),
        "2026-10-27T20:00:00Z",
        "2026-10-28T02:59:30Z",
    );

    let effects = sim.tick("2026-10-28T03:10:00Z");

    assert_eq!(
        missed(&effects),
        [Miss::Late {
            by: SignedDuration::from_mins(10)
        }]
    );
    assert!(!effects.iter().any(|effect| matches!(effect, Effect::Restart(_))));
    assert_eq!(sim.next(), Some(ts("2026-10-28T15:00:00Z")));
}

#[test]
fn a_clock_set_back_after_the_restart_replays_nothing() {
    let mut sim = Sim::running(paris(&["04:00"]), "2026-10-27T20:00:00Z", "2026-10-28T02:00:00Z");
    let first = sim.run_until("2026-10-28T03:00:30Z");

    sim.now = ts("2026-10-28T02:40:00Z");
    let again = sim.run_until("2026-10-28T03:30:00Z");

    assert_eq!(restarts(&first).len(), 1);
    assert_eq!(again, []);
    assert_eq!(sim.next(), Some(ts("2026-10-29T03:00:00Z")));
}

#[test]
fn a_clock_set_back_before_the_restart_sends_no_tier_twice_and_fires_once() {
    let mut sim = Sim::running(paris(&["04:00"]), "2026-10-27T20:00:00Z", "2026-10-28T02:20:00Z");
    let before = sim.run_until("2026-10-28T02:55:30Z");

    sim.now = ts("2026-10-28T02:35:00Z");
    let after = sim.run_until("2026-10-28T03:05:00Z");

    let tiers = |log: &[(Timestamp, Effect)]| -> Vec<u64> {
        announcements(log)
            .into_iter()
            .map(|(_, event)| match event {
                BridgeEvent::RestartScheduled {
                    seconds_remaining, ..
                } => seconds_remaining,
                other => panic!("{other:?}"),
            })
            .collect()
    };
    assert_eq!(tiers(&before), [1800, 900, 600, 300]);
    assert_eq!(tiers(&after), [240, 180, 120, 60]);
    assert_eq!(restarts(&after), [ts("2026-10-28T03:00:00Z")]);
}

#[test]
fn a_wake_three_minutes_before_sends_only_the_nearest_tier() {
    let mut sim = Sim::running(paris(&["04:00"]), "2026-10-27T20:00:00Z", "2026-10-28T02:20:00Z");
    sim.tick("2026-10-28T02:20:00Z");

    let wake = sim.tick("2026-10-28T02:57:00Z");
    let next = sim.tick("2026-10-28T02:58:00Z");

    assert_eq!(events(&wake), [scheduled("2026-10-28T03:00:00Z", 180)]);
    assert_eq!(events(&next), [scheduled("2026-10-28T03:00:00Z", 120)]);
}

#[test]
fn a_wake_thirty_seconds_before_says_thirty() {
    let mut sim = Sim::running(paris(&["04:00"]), "2026-10-27T20:00:00Z", "2026-10-28T02:20:00Z");

    let wake = sim.tick("2026-10-28T02:59:30Z");

    assert_eq!(events(&wake), [scheduled("2026-10-28T03:00:00Z", 30)]);
}

#[test]
fn a_one_off_seven_minutes_ahead_is_announced_at_once_with_the_time_left() {
    let mut sim = Sim::running(paris(&[]), "2026-10-27T20:00:00Z", "2026-10-28T09:00:00Z");

    let effects = sim.send(Input::OneOff {
        at: ts("2026-10-28T09:07:00Z"),
        by: "ada".to_owned(),
    });

    assert_eq!(
        events(&effects),
        [BridgeEvent::RestartScheduled {
            at: ts("2026-10-28T09:07:00Z").as_millisecond(),
            seconds_remaining: 420,
            reason: RestartSource::OneOff,
        }]
    );
    assert!(persisted(&effects));
}

#[test]
fn a_one_off_planned_in_minutes_warns_once_per_tier_whatever_the_milliseconds() {
    for (planned, count, expected) in [
        ("2026-10-28T17:45:22.630Z", 1, vec![60]),
        ("2026-10-28T17:45:22.300Z", 1, vec![60]),
        ("2026-10-28T17:45:22.700Z", 5, vec![300, 240, 180, 120, 60]),
    ] {
        let mut sim = Sim::running(paris(&[]), "2026-10-28T17:00:00Z", planned);
        let request = OneOffRequest {
            in_minutes: Some(count),
            at: None,
        };
        let at = resolve_one_off(&request, &sim.settings.zone, sim.now).unwrap();
        let mut log: Vec<(Timestamp, Effect)> = sim
            .send(Input::OneOff {
                at,
                by: "ada".to_owned(),
            })
            .into_iter()
            .map(|effect| (ts(planned), effect))
            .collect();
        log.extend(sim.run_until(&(at - SignedDuration::from_secs(1)).to_string()));

        let seconds: Vec<u64> = announcements(&log)
            .into_iter()
            .filter_map(|(_, event)| match event {
                BridgeEvent::RestartScheduled {
                    seconds_remaining, ..
                } => Some(seconds_remaining),
                _ => None,
            })
            .collect();
        assert_eq!(seconds, expected, "planned at {planned} for {count} min");
    }
}

#[test]
fn warnings_wait_for_the_bridge_then_send_the_nearest_only() {
    let mut sim = Sim::new(paris(&["04:00"]), "2026-10-28T02:40:00Z");
    sim.server(
        "2026-10-28T02:40:00Z",
        ServerState::Starting,
        1,
        Some("2026-10-28T02:40:00Z"),
    );
    let starting = sim.run_until("2026-10-28T02:52:00Z");

    let online = sim.server(
        "2026-10-28T02:52:00Z",
        ServerState::Online,
        1,
        Some("2026-10-28T02:40:00Z"),
    );

    assert_eq!(announcements(&starting), []);
    assert_eq!(events(&online), [scheduled("2026-10-28T03:00:00Z", 480)]);
}

#[test]
fn a_stopped_or_crash_looping_server_is_never_restarted_and_nothing_is_audited() {
    for state in [ServerState::Stopped, ServerState::CrashLoop] {
        let mut sim = Sim::new(paris(&["04:00"]), "2026-10-28T02:00:00Z");
        sim.server("2026-10-28T02:00:00Z", state, 3, None);

        let log = sim.run_until("2026-10-28T04:00:00Z");

        assert_eq!(log, [(ts("2026-10-28T03:00:00Z"), Effect::Persist)], "{state:?}");
        assert_eq!(sim.next(), Some(ts("2026-10-29T03:00:00Z")));
    }
}

#[test]
fn a_server_busy_at_the_time_misses_the_restart() {
    for state in [
        ServerState::Starting,
        ServerState::Stopping,
        ServerState::Hung,
        ServerState::Crashed,
    ] {
        let mut sim = Sim::running(paris(&["04:00"]), "2026-10-27T20:00:00Z", "2026-10-28T02:00:00Z");
        let started = (state != ServerState::Crashed).then_some("2026-10-27T20:00:00Z");
        sim.server("2026-10-28T02:59:50Z", state, 1, started);

        let log = sim.run_until("2026-10-28T03:01:00Z");

        let effects: Vec<Effect> = log.into_iter().map(|(_, effect)| effect).collect();
        assert_eq!(
            missed(&effects),
            [Miss::ServerBusy { state: Some(state) }],
            "{state:?}"
        );
        assert!(!effects.iter().any(|effect| matches!(effect, Effect::Restart(_))));
    }
}

#[test]
fn a_restart_the_supervisor_refuses_is_missed_with_its_reason() {
    let mut sim = Sim::running(paris(&["04:00"]), "2026-10-27T20:00:00Z", "2026-10-28T02:59:00Z");
    let fired = sim.run_until("2026-10-28T03:00:00Z");
    assert_eq!(restarts(&fired), [ts("2026-10-28T03:00:00Z")]);

    let refused = sim.send(Input::RestartRefused {
        reason: Miss::LaunchInvalid,
    });

    assert_eq!(missed(&refused), [Miss::LaunchInvalid]);
    assert_eq!(
        events(&refused),
        [BridgeEvent::RestartSkipped {
            at: ts("2026-10-28T03:00:00Z").as_millisecond(),
            by: OBSERVER.to_owned(),
            reason: RestartSource::Schedule,
        }],
        "resources that heard of it learn it will not happen"
    );
    assert_eq!(sim.next(), Some(ts("2026-10-29T03:00:00Z")));
}

fn recent_start(window: u32) -> Settings {
    let mut settings = paris(&["04:00", "16:00"]);
    settings.stored.skip_after_start = window;
    settings
}

#[test]
fn a_start_by_hand_shortly_before_a_fixed_time_skips_it() {
    let mut sim = Sim::new(recent_start(120), "2026-10-28T02:50:00Z");

    let started = sim.server(
        "2026-10-28T02:50:00Z",
        ServerState::Starting,
        1,
        Some("2026-10-28T02:50:00Z"),
    );
    sim.server(
        "2026-10-28T02:50:20Z",
        ServerState::Online,
        1,
        Some("2026-10-28T02:50:00Z"),
    );
    let status = sim.status();
    let log = sim.run_until("2026-10-28T04:00:00Z");

    assert_eq!(
        started,
        [Effect::AutoSkipped {
            at: ts("2026-10-28T03:00:00Z"),
            started_at: ts("2026-10-28T02:50:00Z"),
        }]
    );
    assert_eq!(
        status.upcoming[0].skipped,
        Some(Skipped::RecentStart {
            started_at: ts("2026-10-28T02:50:00Z")
        })
    );
    assert_eq!(restarts(&log), []);
    assert_eq!(sim.next(), Some(ts("2026-10-28T15:00:00Z")));
    assert_eq!(sim.state.plan.settled, Some(ts("2026-10-28T03:00:00Z")));
}

#[test]
fn the_window_after_a_start_is_the_setting() {
    let skipped = |window: u32, started_at: &str| {
        let mut sim = Sim::new(recent_start(window), started_at);
        let effects = sim.server(started_at, ServerState::Starting, 1, Some(started_at));
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::AutoSkipped { .. }))
    };

    assert!(
        !skipped(120, "2026-10-28T00:30:00Z"),
        "01:30, two and a half hours before"
    );
    assert!(
        skipped(120, "2026-10-28T01:30:00Z"),
        "02:30, an hour and a half before"
    );
    assert!(!skipped(60, "2026-10-28T01:30:00Z"));
    assert!(!skipped(0, "2026-10-28T02:50:00Z"), "0 turns the rule off");
}

#[test]
fn the_schedules_own_restart_never_skips_the_next_time() {
    let mut settings = paris(&["04:00", "04:30"]);
    settings.stored.skip_after_start = 120;
    let mut sim = Sim::running(settings, "2026-10-27T20:00:00Z", "2026-10-28T02:59:00Z");
    sim.run_until("2026-10-28T03:00:00Z");

    sim.server(
        "2026-10-28T03:00:05Z",
        ServerState::Stopping,
        1,
        Some("2026-10-27T20:00:00Z"),
    );
    let started = sim.server(
        "2026-10-28T03:00:20Z",
        ServerState::Starting,
        2,
        Some("2026-10-28T03:00:20Z"),
    );
    sim.server(
        "2026-10-28T03:00:40Z",
        ServerState::Online,
        2,
        Some("2026-10-28T03:00:20Z"),
    );
    let log = sim.run_until("2026-10-28T03:31:00Z");

    assert!(
        !started
            .iter()
            .any(|effect| matches!(effect, Effect::AutoSkipped { .. }))
    );
    assert_eq!(restarts(&log), [ts("2026-10-28T03:30:00Z")]);
}

#[test]
fn a_start_by_hand_after_a_scheduled_restart_that_ended_stopped_skips_the_next_time() {
    // A stop overtook the restart during its notice; or the restart's start failed, after the
    // supervisor counted a new generation.
    for stopped_generation in [1, 2] {
        let mut sim = Sim::running(recent_start(120), "2026-10-27T20:00:00Z", "2026-10-28T02:59:00Z");
        let fired = sim.run_until("2026-10-28T03:00:00Z");
        sim.server(
            "2026-10-28T03:00:01Z",
            ServerState::Stopping,
            1,
            Some("2026-10-27T20:00:00Z"),
        );
        sim.server(
            "2026-10-28T03:00:20Z",
            ServerState::Stopped,
            stopped_generation,
            None,
        );
        sim.run_until("2026-10-28T14:50:00Z");

        let started = sim.server(
            "2026-10-28T14:50:00Z",
            ServerState::Starting,
            stopped_generation + 1,
            Some("2026-10-28T14:50:00Z"),
        );

        assert_eq!(restarts(&fired), [ts("2026-10-28T03:00:00Z")]);
        assert_eq!(
            started,
            [Effect::AutoSkipped {
                at: ts("2026-10-28T15:00:00Z"),
                started_at: ts("2026-10-28T14:50:00Z"),
            }],
            "stopped as generation {stopped_generation}"
        );
    }
}

#[test]
fn a_one_off_restart_counts_as_a_start_by_hand() {
    let mut sim = Sim::running(recent_start(120), "2026-10-27T20:00:00Z", "2026-10-28T02:30:00Z");
    sim.send(Input::OneOff {
        at: ts("2026-10-28T02:50:00Z"),
        by: "ada".to_owned(),
    });
    let fired = sim.run_until("2026-10-28T02:50:00Z");

    sim.server(
        "2026-10-28T02:50:05Z",
        ServerState::Stopping,
        1,
        Some("2026-10-27T20:00:00Z"),
    );
    let started = sim.server(
        "2026-10-28T02:50:20Z",
        ServerState::Starting,
        2,
        Some("2026-10-28T02:50:20Z"),
    );

    assert_eq!(restarts(&fired), [ts("2026-10-28T02:50:00Z")]);
    assert!(
        started.iter().any(
            |effect| matches!(effect, Effect::AutoSkipped { at, .. } if *at == ts("2026-10-28T03:00:00Z"))
        )
    );
}

#[test]
fn skip_restore_and_delay_change_the_plan_and_are_written() {
    let mut sim = Sim::running(
        paris(&["04:00", "16:00"]),
        "2026-10-27T20:00:00Z",
        "2026-10-28T01:00:00Z",
    );

    let skipped = sim.send(Input::Skip { by: "ada".to_owned() });
    let next_after_skip = sim.next();
    let restored = sim.send(Input::Restore { by: "ada".to_owned() });
    let delayed = sim.send(Input::Delay {
        minutes: 30,
        by: "grace".to_owned(),
    });
    let again = sim.send(Input::Delay {
        minutes: 15,
        by: "grace".to_owned(),
    });

    assert!(skipped.contains(&Effect::Applied(Applied::Skipped {
        at: ts("2026-10-28T03:00:00Z")
    })));
    assert!(persisted(&skipped));
    assert_eq!(next_after_skip, Some(ts("2026-10-28T15:00:00Z")));
    assert!(restored.contains(&Effect::Applied(Applied::Restored {
        at: ts("2026-10-28T03:00:00Z")
    })));
    assert!(persisted(&restored));
    assert!(persisted(&delayed));
    assert!(again.contains(&Effect::Applied(Applied::Delayed {
        occurrence: ts("2026-10-28T03:00:00Z"),
        at: ts("2026-10-28T03:45:00Z"),
        previous_at: ts("2026-10-28T03:30:00Z"),
    })));
    let status = sim.status();
    assert_eq!(
        status.next,
        Some(NextRestart {
            at: ts("2026-10-28T03:45:00Z"),
            source: RestartSource::Schedule,
            by: Some("grace".to_owned()),
            delayed_from: Some(ts("2026-10-28T03:00:00Z")),
        })
    );
    let log = sim.run_until("2026-10-28T04:00:00Z");
    assert_eq!(restarts(&log), [ts("2026-10-28T03:45:00Z")]);
    assert_eq!(sim.state.plan.delay, None);
}

#[test]
fn a_delay_stays_before_the_following_time_and_within_twelve_hours() {
    let mut close = Sim::running(
        paris(&["04:00", "05:00"]),
        "2026-10-27T20:00:00Z",
        "2026-10-28T01:00:00Z",
    );
    let mut far = Sim::running(paris(&["04:00"]), "2026-10-27T20:00:00Z", "2026-10-28T01:00:00Z");

    let past_the_next = close.send(Input::Delay {
        minutes: 60,
        by: "ada".to_owned(),
    });
    let just_before = close.send(Input::Delay {
        minutes: 59,
        by: "ada".to_owned(),
    });
    far.send(Input::Delay {
        minutes: 700,
        by: "ada".to_owned(),
    });
    let past_twelve_hours = far.send(Input::Delay {
        minutes: 30,
        by: "ada".to_owned(),
    });

    assert!(matches!(past_the_next.as_slice(), [Effect::Refused(error)]
        if error.code == ErrorCode::DelayTooLong && error.params["maxMinutes"] == 59));
    assert!(persisted(&just_before));
    assert!(matches!(past_twelve_hours.as_slice(), [Effect::Refused(error)]
        if error.code == ErrorCode::DelayTooLong && error.params["maxMinutes"] == 20));
    assert_eq!(far.next(), Some(ts("2026-10-28T14:40:00Z")));
}

#[test]
fn a_restart_before_a_delayed_time_takes_the_delay_with_it() {
    let mut sim = Sim::running(
        paris(&["04:00", "16:00"]),
        "2026-10-27T20:00:00Z",
        "2026-10-28T02:50:00Z",
    );
    sim.send(Input::Delay {
        minutes: 30,
        by: "ada".to_owned(),
    });
    sim.send(Input::OneOff {
        at: ts("2026-10-28T03:10:00Z"),
        by: "grace".to_owned(),
    });

    let log = sim.run_until("2026-10-28T03:10:00Z");
    sim.server(
        "2026-10-28T03:10:20Z",
        ServerState::Starting,
        2,
        Some("2026-10-28T03:10:20Z"),
    );
    let after = sim.run_until("2026-10-28T04:00:00Z");

    assert_eq!(restarts(&log), [ts("2026-10-28T03:10:00Z")]);
    assert_eq!(restarts(&after), []);
    assert_eq!(
        sim.state.plan.delay, None,
        "the plan keeps no delay of a settled occurrence"
    );
    assert_eq!(sim.next(), Some(ts("2026-10-28T15:00:00Z")));
}

#[test]
fn a_one_off_replaces_the_previous_one_and_can_be_cancelled() {
    let mut sim = Sim::running(paris(&["04:00"]), "2026-10-27T20:00:00Z", "2026-10-28T01:00:00Z");

    let first = sim.send(Input::OneOff {
        at: ts("2026-10-28T02:00:00Z"),
        by: "ada".to_owned(),
    });
    let second = sim.send(Input::OneOff {
        at: ts("2026-10-28T02:30:00Z"),
        by: "grace".to_owned(),
    });
    let next = sim.status().next;
    let cancelled = sim.send(Input::CancelOneOff { by: "ada".to_owned() });

    assert!(first.contains(&Effect::Applied(Applied::Planned {
        at: ts("2026-10-28T02:00:00Z"),
        previous_at: None
    })));
    assert!(second.contains(&Effect::Applied(Applied::Planned {
        at: ts("2026-10-28T02:30:00Z"),
        previous_at: Some(ts("2026-10-28T02:00:00Z"))
    })));
    assert_eq!(
        next,
        Some(NextRestart {
            at: ts("2026-10-28T02:30:00Z"),
            source: RestartSource::OneOff,
            by: Some("grace".to_owned()),
            delayed_from: None,
        })
    );
    assert!(cancelled.contains(&Effect::Applied(Applied::Cancelled {
        at: ts("2026-10-28T02:30:00Z")
    })));
    assert!(persisted(&cancelled));
    assert_eq!(sim.next(), Some(ts("2026-10-28T03:00:00Z")));
}

#[test]
fn actions_that_do_not_apply_are_refused_and_change_nothing() {
    let mut off = Sim::running(paris(&["04:00"]), "2026-10-27T20:00:00Z", "2026-10-28T01:00:00Z");
    off.settings.stored.enabled = false;
    let mut sim = Sim::running(paris(&["04:00"]), "2026-10-27T20:00:00Z", "2026-10-28T01:00:00Z");
    let mut stopped = Sim::new(paris(&["04:00"]), "2026-10-28T01:00:00Z");
    stopped.send(Input::Server(view(ServerState::Stopped, 0, None)));
    let by = || "ada".to_owned();

    assert_eq!(
        refusal(&off.send(Input::Skip { by: by() })),
        Some(ErrorCode::NothingToSkip)
    );
    assert_eq!(
        refusal(&sim.send(Input::Restore { by: by() })),
        Some(ErrorCode::NothingToRestore)
    );
    sim.send(Input::Skip { by: by() });
    sim.send(Input::Skip { by: by() });
    let before = sim.state.clone();
    assert_eq!(
        refusal(&sim.send(Input::Skip { by: by() })),
        Some(ErrorCode::AlreadySkipped)
    );
    assert_eq!(sim.state, before);
    assert_eq!(
        refusal(&sim.send(Input::CancelOneOff { by: by() })),
        Some(ErrorCode::NoOneOff)
    );
    sim.send(Input::OneOff {
        at: ts("2026-10-28T02:00:00Z"),
        by: by(),
    });
    assert_eq!(
        refusal(&sim.send(Input::Delay {
            minutes: 10,
            by: by()
        })),
        Some(ErrorCode::NothingToDelay),
        "the next restart is the one-off"
    );
    assert_eq!(
        refusal(&stopped.send(Input::OneOff {
            at: ts("2026-10-28T02:00:00Z"),
            by: by()
        })),
        Some(ErrorCode::ServerNotRunning)
    );
}

#[test]
fn a_one_off_is_refused_while_the_server_stops_rather_than_dropped_by_the_stop() {
    let mut sim = Sim::running(paris(&[]), "2026-10-27T20:00:00Z", "2026-10-28T01:00:00Z");
    sim.server(
        "2026-10-28T01:00:00Z",
        ServerState::Stopping,
        1,
        Some("2026-10-27T20:00:00Z"),
    );

    let planned = sim.send(Input::OneOff {
        at: ts("2026-10-28T03:00:00Z"),
        by: "ada".to_owned(),
    });

    assert_eq!(refusal(&planned), Some(ErrorCode::ServerStopping));
    assert_eq!(sim.state.plan.one_off, None);
}

/// A server online since the day before with 04:00 announced down to the 5-minute tier.
fn announced() -> Sim {
    let mut sim = Sim::running(
        paris(&["04:00", "16:00"]),
        "2026-10-27T20:00:00Z",
        "2026-10-28T02:20:00Z",
    );
    sim.run_until("2026-10-28T02:55:00Z");
    sim
}

#[test]
fn a_delay_of_an_announced_restart_reschedules_it_then_announces_the_nearest_tier() {
    let mut sim = announced();

    let effects = sim.at(
        "2026-10-28T02:56:00Z",
        Input::Delay {
            minutes: 15,
            by: "ada".to_owned(),
        },
    );

    assert_eq!(
        events(&effects),
        [
            BridgeEvent::RestartRescheduled {
                at: ts("2026-10-28T03:15:00Z").as_millisecond(),
                previous_at: ts("2026-10-28T03:00:00Z").as_millisecond(),
                by: "ada".to_owned(),
                reason: RestartSource::Schedule,
            },
            scheduled("2026-10-28T03:15:00Z", 19 * 60),
        ]
    );
}

#[test]
fn a_one_off_before_the_announced_restart_reschedules_it() {
    let mut sim = announced();

    let effects = sim.at(
        "2026-10-28T02:56:00Z",
        Input::OneOff {
            at: ts("2026-10-28T02:58:00Z"),
            by: "grace".to_owned(),
        },
    );

    assert_eq!(
        events(&effects),
        [
            BridgeEvent::RestartRescheduled {
                at: ts("2026-10-28T02:58:00Z").as_millisecond(),
                previous_at: ts("2026-10-28T03:00:00Z").as_millisecond(),
                by: "grace".to_owned(),
                reason: RestartSource::OneOff,
            },
            BridgeEvent::RestartScheduled {
                at: ts("2026-10-28T02:58:00Z").as_millisecond(),
                seconds_remaining: 120,
                reason: RestartSource::OneOff,
            },
        ]
    );
}

#[test]
fn skipping_or_turning_off_an_announced_restart_tells_resources_it_will_not_happen() {
    let skipped_event = |by: &str| BridgeEvent::RestartSkipped {
        at: ts("2026-10-28T03:00:00Z").as_millisecond(),
        by: by.to_owned(),
        reason: RestartSource::Schedule,
    };
    let mut skipped = announced();
    let mut turned_off = announced();

    let skip = skipped.at("2026-10-28T02:56:00Z", Input::Skip { by: "ada".to_owned() });
    turned_off.settings.stored.enabled = false;
    let off = turned_off.at(
        "2026-10-28T02:56:00Z",
        Input::Settings {
            by: "grace".to_owned(),
        },
    );

    assert_eq!(events(&skip), [skipped_event("ada")]);
    assert_eq!(events(&off), [skipped_event("grace")]);
    assert_eq!(turned_off.next(), None);
}

#[test]
fn a_restart_told_skipped_then_brought_back_is_announced_again() {
    // Within the 5-minute tier, which went out at 02:55 and is not due again by itself.
    let mut restored = announced();
    let mut turned_on = announced();
    restored.at("2026-10-28T02:55:10Z", Input::Skip { by: "ada".to_owned() });
    turned_on.settings.stored.enabled = false;
    turned_on.at("2026-10-28T02:55:10Z", Input::Settings { by: "ada".to_owned() });
    turned_on.settings.stored.enabled = true;

    let back = restored.at("2026-10-28T02:55:30Z", Input::Restore { by: "ada".to_owned() });
    let on = turned_on.at("2026-10-28T02:55:30Z", Input::Settings { by: "ada".to_owned() });
    let fired = restored.run_until("2026-10-28T03:00:00Z");

    assert_eq!(events(&back), [scheduled("2026-10-28T03:00:00Z", 270)]);
    assert_eq!(events(&on), [scheduled("2026-10-28T03:00:00Z", 270)]);
    assert_eq!(restarts(&fired), [ts("2026-10-28T03:00:00Z")]);
}

#[test]
fn changes_before_any_announcement_tell_resources_nothing() {
    let mut sim = Sim::running(
        paris(&["04:00", "16:00"]),
        "2026-10-27T20:00:00Z",
        "2026-10-28T01:00:00Z",
    );

    let skip = sim.send(Input::Skip { by: "ada".to_owned() });
    let one_off = sim.send(Input::OneOff {
        at: ts("2026-10-28T05:00:00Z"),
        by: "ada".to_owned(),
    });

    assert_eq!(events(&skip), []);
    assert_eq!(events(&one_off), []);
}

#[test]
fn a_settings_change_that_keeps_the_time_keeps_the_tiers_sent() {
    let mut sim = announced();
    sim.settings.stored.times = vec!["04:00".to_owned(), "12:00".to_owned()];

    let changed = sim.at("2026-10-28T02:55:30Z", Input::Settings { by: "ada".to_owned() });
    let next = sim.tick("2026-10-28T02:56:00Z");

    assert_eq!(events(&changed), []);
    assert_eq!(events(&next), [scheduled("2026-10-28T03:00:00Z", 240)]);
}

#[test]
fn a_settings_change_that_removes_a_skipped_or_delayed_time_drops_its_plan() {
    let mut sim = Sim::running(
        paris(&["04:00", "16:00"]),
        "2026-10-27T20:00:00Z",
        "2026-10-28T01:00:00Z",
    );
    sim.send(Input::Skip { by: "ada".to_owned() });
    sim.send(Input::Delay {
        minutes: 30,
        by: "ada".to_owned(),
    });
    sim.settings.stored.times = vec!["10:00".to_owned()];

    let effects = sim.send(Input::Settings {
        by: "grace".to_owned(),
    });

    assert!(effects.iter().any(|effect| matches!(effect,
        Effect::Dropped { skip: Some(skip), delay: Some(delay) }
            if skip.at == ts("2026-10-28T03:00:00Z") && delay.occurrence == ts("2026-10-28T15:00:00Z"))));
    assert!(persisted(&effects));
    assert_eq!(sim.next(), Some(ts("2026-10-28T09:00:00Z")));
}

#[test]
fn a_skip_whose_time_passed_is_forgotten_and_settles_its_occurrence() {
    let mut sim = Sim::running(
        paris(&["04:00", "16:00"]),
        "2026-10-27T20:00:00Z",
        "2026-10-28T01:00:00Z",
    );
    sim.send(Input::Skip { by: "ada".to_owned() });

    let log = sim.run_until("2026-10-28T03:00:01Z");

    assert_eq!(restarts(&log), []);
    assert_eq!(sim.state.plan.skip, None);
    assert_eq!(sim.state.plan.settled, Some(ts("2026-10-28T03:00:00Z")));
    assert!(log.iter().any(|(_, effect)| *effect == Effect::Persist));
}

#[test]
fn a_skipped_delayed_restart_can_be_restored_until_its_delayed_time() {
    let mut sim = Sim::running(
        paris(&["04:00", "16:00"]),
        "2026-10-27T20:00:00Z",
        "2026-10-28T02:00:00Z",
    );
    sim.send(Input::Delay {
        minutes: 30,
        by: "ada".to_owned(),
    });

    let skipped = sim.at(
        "2026-10-28T03:10:00Z",
        Input::Skip {
            by: "grace".to_owned(),
        },
    );
    let status = sim.status();
    let restored = sim.at(
        "2026-10-28T03:11:00Z",
        Input::Restore {
            by: "grace".to_owned(),
        },
    );
    let next = sim.next();
    let log = sim.run_until("2026-10-28T04:00:00Z");

    assert!(skipped.contains(&Effect::Applied(Applied::Skipped {
        at: ts("2026-10-28T03:00:00Z")
    })));
    assert_eq!(
        status.upcoming[0].skipped,
        Some(Skipped::Manual {
            by: "grace".to_owned()
        })
    );
    assert!(restored.contains(&Effect::Applied(Applied::Restored {
        at: ts("2026-10-28T03:00:00Z")
    })));
    assert_eq!(next, Some(ts("2026-10-28T03:30:00Z")), "back at its delayed time");
    assert_eq!(restarts(&log), [ts("2026-10-28T03:30:00Z")]);
}

#[test]
fn a_skipped_delayed_restart_is_forgotten_once_its_delayed_time_passes() {
    let mut sim = Sim::running(
        paris(&["04:00", "16:00"]),
        "2026-10-27T20:00:00Z",
        "2026-10-28T02:00:00Z",
    );
    sim.send(Input::Delay {
        minutes: 30,
        by: "ada".to_owned(),
    });
    sim.send(Input::Skip {
        by: "grace".to_owned(),
    });

    let log = sim.run_until("2026-10-28T03:30:01Z");

    assert_eq!(restarts(&log), []);
    assert_eq!(sim.state.plan.skip, None);
    assert_eq!(sim.state.plan.delay, None);
    assert_eq!(
        refusal(&sim.send(Input::Restore {
            by: "grace".to_owned()
        })),
        Some(ErrorCode::NothingToRestore)
    );
    assert_eq!(sim.next(), Some(ts("2026-10-28T15:00:00Z")));
}

#[test]
fn a_plan_read_back_after_observer_restarted_still_fires_the_one_off_the_skip_and_the_delay() {
    let plan = SchedulePlan {
        skip: Some(PlannedRestart {
            at: ts("2026-10-28T03:00:00Z"),
            by: "ada".to_owned(),
        }),
        delay: Some(PlannedDelay {
            occurrence: ts("2026-10-28T09:00:00Z"),
            to: ts("2026-10-28T09:20:00Z"),
            by: "grace".to_owned(),
        }),
        one_off: Some(PlannedRestart {
            at: ts("2026-10-28T02:30:00Z"),
            by: "ada".to_owned(),
        }),
        settled: Some(ts("2026-10-27T15:00:00Z")),
    };
    let mut sim = Sim::restored(paris(&["04:00", "10:00", "16:00"]), "2026-10-28T02:00:00Z", plan);
    sim.server("2026-10-28T02:00:00Z", ServerState::Stopped, 0, None);
    sim.server(
        "2026-10-28T02:00:01Z",
        ServerState::Starting,
        1,
        Some("2026-10-28T02:00:01Z"),
    );
    sim.server(
        "2026-10-28T02:00:20Z",
        ServerState::Online,
        1,
        Some("2026-10-28T02:00:01Z"),
    );

    let one_off = sim.run_until("2026-10-28T02:30:00Z");
    sim.server(
        "2026-10-28T02:30:20Z",
        ServerState::Starting,
        2,
        Some("2026-10-28T02:30:20Z"),
    );
    sim.server(
        "2026-10-28T02:30:40Z",
        ServerState::Online,
        2,
        Some("2026-10-28T02:30:20Z"),
    );
    let rest = sim.run_until("2026-10-28T10:00:00Z");

    assert_eq!(restarts(&one_off), [ts("2026-10-28T02:30:00Z")]);
    assert_eq!(
        restarts(&rest),
        [ts("2026-10-28T09:20:00Z")],
        "04:00 skipped, 10:00 delayed"
    );
}

#[test]
fn the_first_run_seen_keeps_a_read_back_one_off_and_the_next_drops_it() {
    let plan = SchedulePlan {
        one_off: Some(PlannedRestart {
            at: ts("2026-10-28T05:00:00Z"),
            by: "ada".to_owned(),
        }),
        ..SchedulePlan::default()
    };
    let mut sim = Sim::restored(paris(&[]), "2026-10-28T02:00:00Z", plan);
    sim.server("2026-10-28T02:00:00Z", ServerState::Stopped, 0, None);
    let first = sim.server(
        "2026-10-28T02:00:01Z",
        ServerState::Starting,
        1,
        Some("2026-10-28T02:00:01Z"),
    );
    sim.server(
        "2026-10-28T03:00:00Z",
        ServerState::Stopping,
        1,
        Some("2026-10-28T02:00:01Z"),
    );
    let second = sim.server(
        "2026-10-28T03:00:20Z",
        ServerState::Starting,
        2,
        Some("2026-10-28T03:00:20Z"),
    );

    assert!(sim.state.plan.one_off.is_none());
    assert!(
        !first
            .iter()
            .any(|effect| matches!(effect, Effect::OneOffDropped { .. }))
    );
    assert!(second.iter().any(|effect| matches!(effect,
        Effect::OneOffDropped { one_off, reason: DropReason::ServerRestarted } if one_off.by == "ada")));
    assert!(persisted(&second));
}

#[test]
fn a_stop_by_hand_drops_the_one_off_of_its_run() {
    let mut sim = Sim::running(paris(&[]), "2026-10-27T20:00:00Z", "2026-10-28T01:00:00Z");
    sim.send(Input::OneOff {
        at: ts("2026-10-28T02:00:00Z"),
        by: "ada".to_owned(),
    });

    sim.server(
        "2026-10-28T01:10:00Z",
        ServerState::Stopping,
        1,
        Some("2026-10-27T20:00:00Z"),
    );
    let stopped = sim.server("2026-10-28T01:10:20Z", ServerState::Stopped, 1, None);

    assert!(stopped.iter().any(|effect| matches!(
        effect,
        Effect::OneOffDropped {
            reason: DropReason::ServerStopped,
            ..
        }
    )));
    assert_eq!(sim.next(), None);
}

#[test]
fn a_settled_mark_two_days_ahead_is_dropped() {
    let plan = SchedulePlan {
        settled: Some(ts("2026-10-30T03:00:00Z")),
        ..SchedulePlan::default()
    };
    let mut sim = Sim::restored(paris(&["04:00"]), "2026-10-28T01:00:00Z", plan);

    let effects = sim.send(Input::Server(view(ServerState::Stopped, 0, None)));

    assert!(effects.contains(&Effect::SettledAhead(ts("2026-10-30T03:00:00Z"))));
    assert!(persisted(&effects));
    assert_eq!(sim.next(), Some(ts("2026-10-28T03:00:00Z")));
}

#[test]
fn the_deadline_is_the_next_tier_or_the_restart_and_always_after_now() {
    let mut sim = Sim::running(paris(&["04:00"]), "2026-10-27T20:00:00Z", "2026-10-28T02:20:00Z");
    sim.tick("2026-10-28T02:20:00Z");
    let first = deadline(&sim.state, &sim.settings, sim.now);
    sim.tick("2026-10-28T02:59:30Z");
    let last = deadline(&sim.state, &sim.settings, sim.now);
    sim.settings.stored.enabled = false;
    sim.send(Input::Settings { by: "ada".to_owned() });
    let none = deadline(&sim.state, &sim.settings, sim.now);

    assert_eq!(first, Some(ts("2026-10-28T02:30:00Z")));
    assert_eq!(last, Some(ts("2026-10-28T03:00:00Z")));
    assert_eq!(none, None);
}

#[test]
fn a_one_off_at_a_wall_time_is_the_first_such_time_at_least_a_minute_ahead() {
    let zone = TimeZone::get("Europe/Paris").unwrap();
    let at = |time: &str, now: &str| {
        let request = OneOffRequest {
            in_minutes: None,
            at: Some(time.to_owned()),
        };
        resolve_one_off(&request, &zone, ts(now)).map_err(|error| error.code)
    };
    let within = |count: u32| {
        let request = OneOffRequest {
            in_minutes: Some(count),
            at: None,
        };
        resolve_one_off(&request, &zone, ts("2026-10-28T08:59:59.7Z")).map_err(|error| error.code)
    };

    assert_eq!(
        at("02:30", "2026-03-29T00:00:00Z"),
        Ok(ts("2026-03-29T01:30:00Z")),
        "01:00 CET, in the gap"
    );
    assert_eq!(
        at("04:00", "2026-10-28T02:59:30Z"),
        Ok(ts("2026-10-29T03:00:00Z")),
        "less than a minute ahead"
    );
    assert_eq!(
        at("04:00", "2026-10-28T02:58:59Z"),
        Ok(ts("2026-10-28T03:00:00Z"))
    );
    assert_eq!(at("4:00", "2026-10-28T02:00:00Z"), Err(ErrorCode::InvalidRequest));
    assert_eq!(within(5), Ok(ts("2026-10-28T09:05:00Z")), "to the nearest second");
    assert_eq!(within(0), Err(ErrorCode::InvalidRequest));
    assert_eq!(within(1441), Err(ErrorCode::InvalidRequest));
    let both = OneOffRequest {
        in_minutes: Some(5),
        at: Some("04:00".to_owned()),
    };
    assert_eq!(
        resolve_one_off(&both, &zone, ts("2026-10-28T09:00:00Z")).map_err(|error| error.code),
        Err(ErrorCode::InvalidRequest)
    );
}

#[test]
fn the_status_lists_five_occurrences_with_the_time_each_comes_from() {
    let mut sim = Sim::running(
        paris(&["02:30", "12:00"]),
        "2026-03-28T20:00:00Z",
        "2026-03-28T20:00:01Z",
    );
    sim.send(Input::Skip { by: "ada".to_owned() });

    let status = sim.status();

    let listed: Vec<(Timestamp, Option<&str>)> = status
        .upcoming
        .iter()
        .map(|occurrence| (occurrence.at, occurrence.time.as_deref()))
        .collect();
    assert_eq!(
        listed,
        [
            (ts("2026-03-29T01:30:00Z"), Some("02:30")),
            (ts("2026-03-29T10:00:00Z"), Some("12:00")),
            (ts("2026-03-30T00:30:00Z"), Some("02:30")),
            (ts("2026-03-30T10:00:00Z"), Some("12:00")),
            (ts("2026-03-31T00:30:00Z"), Some("02:30")),
        ]
    );
    assert_eq!(
        status.upcoming[0].skipped,
        Some(Skipped::Manual { by: "ada".to_owned() })
    );
    assert_eq!(status.zone, "Europe/Paris");
    assert_eq!(status.next.map(|next| next.at), Some(ts("2026-03-29T10:00:00Z")));
}
