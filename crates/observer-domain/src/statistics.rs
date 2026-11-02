//! The Statistics page beyond the series: the percentiles of the frame time, the players' usual
//! range, the timeline's events and the links between a crash wave and a change, and the insights:
//! frame time distribution, players by day and hour, memory run after run, how long sessions last,
//! and what moved stability.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::changes::{ChangeCounts, LiveChangeKind, NumberChange};
use crate::drops::{DropsRange, Trend};
use crate::metrics::{Fit, Growth, SeriesData, fit};

/// The most series one timeline request may ask for: its lanes and the ones a viewer adds.
pub const MAX_TIMELINE_SERIES: usize = 24;
/// Buckets a timeline answers at most, and at least when asked for fewer.
pub const MOST_POINTS: u32 = 720;
pub const LEAST_POINTS: u32 = 24;
/// The shortest window a timeline reads, and the longest, in seconds.
pub const SHORTEST_WINDOW: i64 = 60;
pub const LONGEST_WINDOW: i64 = 400 * 86_400;
/// svMain's budget, in milliseconds: 20 ticks a second.
pub const FRAME_BUDGET_MS: f64 = 50.0;

/// `GET /api/metrics/timeline`: a window of series on a regular grid, bucket `i` starting at
/// `from + i × step`, with what the timeline draws beside them.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Timeline {
    /// Unix seconds.
    pub from: i64,
    /// Seconds per bucket.
    pub step: i64,
    pub buckets: i64,
    /// The newest slot the buckets include.
    pub last: Option<i64>,
    pub series: Vec<SeriesData>,
    /// The samples Observer took in each bucket, and those taken while the server was online.
    pub observed: Vec<u32>,
    pub online: Vec<u32>,
    /// svMain's percentiles, when `mainTick` is asked for.
    pub frames: Option<FrameBands>,
    /// Where the players usually were at the same time of the 4 weeks before, when `players` is
    /// asked for.
    pub usual: Option<UsualRange>,
}

/// svMain's frame time percentiles per bucket, in milliseconds, estimated within FXServer's bounds.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FrameBands {
    pub p50: Vec<Option<f64>>,
    pub p95: Vec<Option<f64>>,
    pub p99: Vec<Option<f64>>,
    /// The ticks behind each bucket's percentiles.
    pub ticks: Vec<u32>,
    /// The lowest bound read: a percentile at or under it only says "under".
    pub floor: Option<f64>,
    /// The highest finite bound: a percentile equal to it says "at least".
    pub top: Option<f64>,
    /// The first histogram kept, Unix seconds: before it the window has means only.
    pub since: Option<i64>,
}

/// The lowest and highest players of the same time in each of the 4 weeks before, per bucket.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UsualRange {
    pub low: Vec<Option<f64>>,
    pub high: Vec<Option<f64>>,
}

/// The weeks the usual range looks back, and how many of them it needs.
pub const USUAL_WEEKS: i64 = 4;
pub const USUAL_LEAST_WEEKS: usize = 2;

/// The lowest and highest of the weeks' values, when enough weeks have one.
pub fn usual(weeks: &[Option<f64>]) -> Option<(f64, f64)> {
    let values: Vec<f64> = weeks.iter().flatten().copied().collect();
    if values.len() < USUAL_LEAST_WEEKS {
        return None;
    }
    let low = values.iter().copied().fold(f64::INFINITY, f64::min);
    let high = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    Some((low, high))
}

/// The value under which a share `q` of the ticks fell, from the ticks of each interval:
/// interval 0 is under the first bound, the last is beyond the last bound. Linear within the
/// interval where the rank falls; the last bound for a rank beyond it. `None` without ticks.
pub fn quantile(bounds: &[f64], ticks: &[u64], q: f64) -> Option<f64> {
    let total: u64 = ticks.iter().sum();
    if total == 0 || ticks.len() != bounds.len() + 1 {
        return None;
    }
    #[allow(clippy::cast_precision_loss)]
    let rank = q.clamp(0.0, 1.0) * total as f64;
    let mut below = 0.0;
    for (index, &count) in ticks.iter().enumerate() {
        #[allow(clippy::cast_precision_loss)]
        let count = count as f64;
        if count > 0.0 && below + count >= rank {
            let Some(&upper) = bounds.get(index) else {
                return bounds.last().copied();
            };
            let lower = if index == 0 { 0.0 } else { bounds[index - 1] };
            return Some(lower + (upper - lower) * ((rank - below) / count).clamp(0.0, 1.0));
        }
        below += count;
    }
    bounds.last().copied()
}

/// The share of ticks over `limit`, which must be one of the bounds.
pub fn share_over(bounds: &[f64], ticks: &[u64], limit: f64) -> Option<f64> {
    let at = bounds.iter().position(|bound| (bound - limit).abs() < 1e-9)?;
    let total: u64 = ticks.iter().sum();
    if total == 0 {
        return None;
    }
    let over: u64 = ticks.iter().skip(at + 1).sum();
    #[allow(clippy::cast_precision_loss)]
    Some(over as f64 / total as f64)
}

/// `GET /api/metrics/events`: what happened over a window, oldest first, and what the caller's
/// permissions left out.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TimelineEvents {
    pub events: Vec<TimelineEvent>,
    pub links: Vec<EventLink>,
    pub omitted: Vec<EventSource>,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TimelineEvent {
    /// The same at every read: what a link names.
    pub key: String,
    /// Unix seconds.
    pub at: i64,
    /// The last moment of an event that lasts: a wave's last crash, a group's last write.
    pub until: Option<i64>,
    pub detail: EventDetail,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum EventDetail {
    Start {
        build: Option<u32>,
    },
    /// A restart someone or the schedule asked for.
    Restart {
        build: Option<u32>,
    },
    Stop,
    Crash {
        hang: bool,
        /// When the automatic restart brought the server back, Unix seconds.
        back_online_at: Option<i64>,
        players_dropped: Option<i64>,
    },
    /// A start where something changed against the start before it.
    Changes {
        counts: ChangeCounts,
        build: Option<NumberChange>,
        /// The first resources that changed, by name.
        resources: Vec<String>,
    },
    /// A resource started, stopped or restarted while the server ran.
    Resource {
        name: String,
        change: LiveChangeKind,
        by: Option<String>,
    },
    /// Files written, deleted or renamed over SFTP, close together.
    Files {
        files: u32,
        resources: Vec<String>,
        by: Vec<String>,
    },
    /// Crashes of one signature, close together, from several players, and unusual for it.
    CrashWave {
        signature: Option<String>,
        label: Option<String>,
        crashes: u32,
        players: u32,
        /// The resource the crashes name, when they name one.
        resource: Option<String>,
    },
    /// A burst of drops.
    Incident {
        drops: i64,
        hitch_ms: Option<i64>,
        players: Option<i64>,
    },
    /// A backup of the game's database, once backups are recorded.
    Backup {
        failed: bool,
    },
}

/// A crash wave that came soon after a change, its signature unseen in the weeks before.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EventLink {
    /// The change's key and the wave's: what "not related" records.
    pub key: String,
    pub change: String,
    pub wave: String,
    /// Seconds from the change to the wave's first crash.
    pub after: i64,
    /// A changed resource the crashes name.
    pub names: Option<String>,
    /// Someone said it is not related.
    pub dismissed: bool,
}

/// Where events come from, beyond the runs every timeline reader sees.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum EventSource {
    /// What changed at starts, resources while running, SFTP writes: `CHANGES_VIEW`.
    Changes,
    /// Crash waves and bursts of drops: `DROPS_VIEW`.
    Drops,
}

/// `PUT /api/metrics/links`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct LinkDismissal {
    pub link: String,
    pub dismissed: bool,
}

/// A link's key: the change's and the wave's.
pub fn link_key(change: &str, wave: &str) -> String {
    format!("{change}>{wave}")
}

/// Crashes of one signature this far apart belong to separate waves.
pub const WAVE_GAP_MS: i64 = 15 * 60_000;
pub const WAVE_CRASHES: usize = 5;
pub const WAVE_PLAYERS: usize = 3;
/// A wave is this unlikely from the signature's rate over the weeks before.
pub const WAVE_UNLIKELY: f64 = 0.001;
/// The weeks before a wave its rate is read from, and the least time and the least expected
/// crashes the test takes.
pub const WAVE_BASELINE_MS: i64 = 28 * 86_400_000;
pub const WAVE_LEAST_SPAN_MS: i64 = 10 * 60_000;
pub const WAVE_LEAST_EXPECTED: f64 = 0.1;
/// A wave is linked to the last change recorded at most this long before it.
pub const LINK_WINDOW_MS: i64 = 30 * 60_000;

/// The runs of `times` (ascending) with no gap over [`WAVE_GAP_MS`], as index ranges.
pub fn bursts(times: &[i64]) -> Vec<std::ops::Range<usize>> {
    let mut runs = Vec::new();
    let mut start = 0;
    for index in 1..=times.len() {
        if index == times.len() || times[index] - times[index - 1] > WAVE_GAP_MS {
            if index > start {
                runs.push(start..index);
            }
            start = index;
        }
    }
    runs
}

/// The chance of at least `k` events where `expected` were.
pub fn at_least(k: u64, expected: f64) -> f64 {
    if k == 0 {
        return 1.0;
    }
    if expected <= 0.0 {
        return 0.0;
    }
    // 1 − P(X < k), the terms summed in log space so that a large `expected` does not underflow.
    let mut log_term = -expected;
    let mut below = log_term.exp();
    for i in 1..k {
        #[allow(clippy::cast_precision_loss)]
        let i = i as f64;
        log_term += expected.ln() - i.ln();
        below += log_term.exp();
    }
    (1.0 - below).clamp(0.0, 1.0)
}

/// Whether `crashes` of a signature from `players` players are a wave, against `expected` from
/// its rate in the weeks before.
pub fn is_wave(crashes: usize, players: usize, expected: f64) -> bool {
    crashes >= WAVE_CRASHES
        && players >= WAVE_PLAYERS
        && at_least(crashes as u64, expected.max(WAVE_LEAST_EXPECTED)) < WAVE_UNLIKELY
}

/// The last of `changes` (Unix milliseconds, ascending) at or before `wave` and at most
/// [`LINK_WINDOW_MS`] before it.
pub fn linked(changes: &[i64], wave: i64) -> Option<usize> {
    changes
        .iter()
        .rposition(|&change| change <= wave && wave - change <= LINK_WINDOW_MS)
}

/// `GET /api/metrics/frames`: how svMain's frames spread over the bounds, this period and the one
/// before.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FrameDistribution {
    pub range: DropsRange,
    /// Milliseconds, ascending; the shares have one interval more, beyond the last bound.
    pub bounds: Vec<f64>,
    pub budget: f64,
    pub now: FramePeriod,
    pub before: FramePeriod,
    /// The first histogram kept, Unix seconds.
    pub since: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FramePeriod {
    /// Unix seconds.
    pub from: i64,
    pub to: i64,
    pub ticks: u64,
    /// Of the ticks, the share in each interval; `None` without ticks on these bounds.
    pub shares: Option<Vec<f64>>,
    pub p50: Option<f64>,
    pub p95: Option<f64>,
    pub p99: Option<f64>,
    /// The share over the budget.
    pub over_budget: Option<f64>,
}

impl FramePeriod {
    /// A period's figures from its ticks on `bounds`.
    pub fn of(from: i64, to: i64, bounds: &[f64], ticks: &[u64]) -> Self {
        let total: u64 = ticks.iter().sum();
        #[allow(clippy::cast_precision_loss)]
        let shares = (total > 0 && ticks.len() == bounds.len() + 1)
            .then(|| ticks.iter().map(|&count| count as f64 / total as f64).collect());
        Self {
            from,
            to,
            ticks: total,
            shares,
            p50: quantile(bounds, ticks, 0.50),
            p95: quantile(bounds, ticks, 0.95),
            p99: quantile(bounds, ticks, 0.99),
            over_budget: share_over(bounds, ticks, FRAME_BUDGET_MS),
        }
    }
}

/// The days the players' hours look back.
pub const HOURS_DAYS: i64 = 28;

/// `GET /api/metrics/hours`: players on average by day of the week and hour of the day.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PlayerHours {
    /// The schedule's time zone, in which days and hours are.
    pub zone: String,
    /// Unix seconds.
    pub from: i64,
    pub to: i64,
    /// 168 cells, Monday 00:00 first, then by hour.
    pub players: Vec<Option<f64>>,
    /// The hours measured behind each cell.
    pub measured: Vec<u32>,
    /// Each hour of the day over every day.
    pub daily: Vec<Option<f64>>,
    /// The hour of the day with the fewest players, once every day has a measure of it.
    pub quietest: Option<u8>,
}

/// The hour whose every day was measured and whose daily average is the lowest.
pub fn quietest(players: &[Option<f64>], daily: &[Option<f64>]) -> Option<u8> {
    (0..24usize)
        .filter(|&hour| (0..7).all(|day| players.get(day * 24 + hour).copied().flatten().is_some()))
        .filter_map(|hour| Some((hour, daily.get(hour).copied().flatten()?)))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .and_then(|(hour, _)| u8::try_from(hour).ok())
}

/// `GET /api/metrics/runs`: FXServer's memory, run after run.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RunMemory {
    pub range: DropsRange,
    /// Unix seconds.
    pub from: i64,
    pub to: i64,
    /// Oldest first.
    pub runs: Vec<RunFit>,
    /// Runs that started before this have no minutes left to fit.
    pub fitted_from: i64,
    pub acceleration: Option<Acceleration>,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RunFit {
    /// Unix seconds.
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub end: Option<RunEnd>,
    pub build: Option<u32>,
    pub private: Option<Growth>,
    /// The most private bytes a minute averaged.
    pub peak: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum RunEnd {
    Stopped,
    Crashed,
    Hung,
    /// Observer stopped without seeing its end.
    Lost,
}

impl RunEnd {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "stopped" => Self::Stopped,
            "crashed" => Self::Crashed,
            "hung" => Self::Hung,
            "lost" => Self::Lost,
            _ => return None,
        })
    }
}

/// How each run's growth compares with the one before.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Acceleration {
    pub runs: u32,
    /// Bytes per hour each run grows faster than the one before.
    pub per_run: f64,
    pub fit: f64,
    /// Steadily faster, and the newest past the leak's floor.
    pub accelerating: bool,
}

/// The runs an acceleration looks at, and the fewest it needs.
pub const ACCELERATION_RUNS: usize = 6;
pub const ACCELERATION_LEAST_RUNS: usize = 3;

/// The line through the runs' growths per hour (oldest first), over the last
/// [`ACCELERATION_RUNS`]. `None` under [`ACCELERATION_LEAST_RUNS`].
pub fn acceleration(per_hour: &[f64], floor: f64) -> Option<Acceleration> {
    let recent = &per_hour[per_hour.len().saturating_sub(ACCELERATION_RUNS)..];
    if recent.len() < ACCELERATION_LEAST_RUNS {
        return None;
    }
    #[allow(clippy::cast_precision_loss)]
    let points: Vec<(f64, f64, f64)> = recent
        .iter()
        .enumerate()
        .map(|(index, &growth)| (index as f64, growth, 1.0))
        .collect();
    let Fit { slope, r2, .. } = fit(&points)?;
    let newest = recent.last().copied().unwrap_or_default();
    Some(Acceleration {
        runs: u32::try_from(recent.len()).unwrap_or(u32::MAX),
        per_run: slope,
        fit: r2,
        accelerating: slope > 0.0 && r2 >= crate::metrics::LEAK_FIT && newest >= floor,
    })
}

/// `GET /servers/default/drops/sessions`: the share of sessions still connected after a given
/// time, those that started in the range against those of the 4 weeks before.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SessionSurvival {
    pub range: DropsRange,
    pub now: Survival,
    pub before: Survival,
    /// Sessions are kept since then: a side that starts earlier was cut there.
    pub kept_from: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Survival {
    pub from: Timestamp,
    pub to: Timestamp,
    pub sessions: u32,
    /// Of those, the ones a player ended: the others are still open or the server ended them.
    pub ended: u32,
    /// The share still connected every [`SURVIVAL_STEP`] minutes from 0 to [`SURVIVAL_MINUTES`].
    pub points: Vec<f64>,
    /// Minutes after which half the sessions had ended.
    pub median: Option<f64>,
    /// The share ended within [`EARLY_MINUTES`].
    pub early: Option<f64>,
    /// Of the crashes with a known length, the share in the first [`EARLY_MINUTES`].
    pub early_crashes: Option<f64>,
}

pub const SURVIVAL_STEP: u32 = 2;
pub const SURVIVAL_MINUTES: u32 = 240;
pub const EARLY_MINUTES: f64 = 10.0;
/// The weeks before the range its sessions are compared with.
pub const SURVIVAL_BEFORE_DAYS: i64 = 28;

/// Kaplan–Meier over `(minutes, ended)` sessions: a session not ended by its player (still open,
/// or ended by the server) leaves the count without taking the share down. Returns the share
/// still there at each of `marks`, ascending, and the median.
pub fn survival(sessions: &mut [(f64, bool)], marks: &[f64]) -> (Vec<f64>, Option<f64>) {
    sessions.sort_by(|a, b| a.0.total_cmp(&b.0).then(b.1.cmp(&a.1)));
    let mut at_risk = sessions.len();
    let mut share = 1.0;
    // (minutes, share after the ends at that time)
    let mut steps: Vec<(f64, f64)> = Vec::new();
    let mut index = 0;
    while index < sessions.len() {
        let time = sessions[index].0;
        let mut ended = 0;
        let mut left = 0;
        while index < sessions.len() && sessions[index].0.total_cmp(&time).is_eq() {
            if sessions[index].1 {
                ended += 1;
            } else {
                left += 1;
            }
            index += 1;
        }
        if ended > 0 && at_risk > 0 {
            #[allow(clippy::cast_precision_loss)]
            let fraction = ended as f64 / at_risk as f64;
            share *= 1.0 - fraction;
            steps.push((time, share));
        }
        at_risk -= ended + left;
    }
    let share_at = |mark: f64| {
        steps
            .iter()
            .take_while(|(time, _)| *time <= mark)
            .last()
            .map_or(1.0, |(_, share)| *share)
    };
    let points = marks.iter().map(|&mark| share_at(mark)).collect();
    let median = steps
        .iter()
        .find(|(_, share)| *share <= 0.5)
        .map(|(time, _)| *time);
    (points, median)
}

/// `GET /servers/default/drops/effects`: each recorded change of the last 90 days, the
/// unexpected drop rate in the 7 days before it against the 7 after.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StabilityEffects {
    pub from: Timestamp,
    pub to: Timestamp,
    /// Newest first.
    pub rows: Vec<EffectRow>,
    /// Whether changes of resources, the cfg and the launch settings were read: they need
    /// `CHANGES_VIEW`, builds alone do not.
    pub changes: bool,
}

/// The days a change is judged on each side, cut at the neighbouring change.
pub const EFFECT_DAYS: i64 = 7;
pub const EFFECTS_DAYS: i64 = 90;

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EffectRow {
    pub at: Timestamp,
    pub kinds: Vec<EffectKind>,
    pub build: Option<NumberChange>,
    /// The first resources that changed, and how many more did.
    pub resources: Vec<String>,
    pub more_resources: u32,
    /// After against before: `now` is after the change.
    pub trend: Trend,
    /// The two windows compared.
    pub before_from: Timestamp,
    pub after_to: Timestamp,
    /// The window after is not over yet.
    pub open: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum EffectKind {
    Build,
    GameBuild,
    Cfg,
    Launch,
    Resources,
}

#[cfg(test)]
mod tests {
    use super::*;

    const NEW_BOUNDS: [f64; 13] = [
        2.0, 4.0, 6.0, 8.0, 10.0, 15.0, 20.0, 30.0, 50.0, 70.0, 100.0, 150.0, 250.0,
    ];

    fn ticks(pairs: &[(usize, u64)]) -> Vec<u64> {
        let mut ticks = vec![0; NEW_BOUNDS.len() + 1];
        for &(index, count) in pairs {
            ticks[index] = count;
        }
        ticks
    }

    #[test]
    fn a_quantile_is_interpolated_within_the_interval_its_rank_falls_in() {
        // 100 ticks between 4 and 6 ms: the median halfway.
        let even = ticks(&[(2, 100)]);
        assert_eq!(quantile(&NEW_BOUNDS, &even, 0.5), Some(5.0));
        assert!((quantile(&NEW_BOUNDS, &even, 0.99).unwrap() - 5.98).abs() < 1e-9);

        // 90 fast ticks, 10 between 50 and 70 ms: p95 is halfway into the slow interval.
        let tail = ticks(&[(0, 90), (9, 10)]);
        assert_eq!(quantile(&NEW_BOUNDS, &tail, 0.95), Some(60.0));
    }

    #[test]
    fn a_quantile_under_the_first_bound_or_beyond_the_last_stays_at_the_bounds() {
        let fast = ticks(&[(0, 50)]);
        let slow = ticks(&[(0, 50), (13, 50)]);

        assert!(quantile(&NEW_BOUNDS, &fast, 0.99).unwrap() <= NEW_BOUNDS[0]);
        assert_eq!(quantile(&NEW_BOUNDS, &slow, 0.99), Some(250.0));
        assert_eq!(quantile(&NEW_BOUNDS, &ticks(&[]), 0.5), None);
        assert_eq!(
            quantile(&NEW_BOUNDS, &[1, 2], 0.5),
            None,
            "intervals for other bounds"
        );
    }

    #[test]
    fn the_share_over_the_budget_counts_the_intervals_past_it() {
        let ticks = ticks(&[(0, 990), (9, 6), (13, 4)]);

        assert_eq!(share_over(&NEW_BOUNDS, &ticks, FRAME_BUDGET_MS), Some(0.01));
        assert_eq!(share_over(&NEW_BOUNDS, &ticks, 55.0), None, "not a bound");
    }

    #[test]
    fn the_usual_range_needs_two_weeks_with_a_value() {
        assert_eq!(
            usual(&[Some(40.0), None, Some(55.0), Some(48.0)]),
            Some((40.0, 55.0))
        );
        assert_eq!(usual(&[Some(40.0), None, None, None]), None);
    }

    #[test]
    fn bursts_split_where_crashes_are_more_than_fifteen_minutes_apart() {
        let minute = 60_000;
        let times = [0, 5 * minute, 20 * minute, 36 * minute, 40 * minute];

        assert_eq!(bursts(&times), vec![0..3, 3..5]);
        assert_eq!(bursts(&[]), Vec::<std::ops::Range<usize>>::new());
    }

    #[test]
    fn the_poisson_tail_matches_its_closed_forms() {
        assert_eq!(at_least(0, 3.0), 1.0);
        assert!((at_least(1, 2.0) - (1.0 - (-2.0f64).exp())).abs() < 1e-12);
        // P(X ≥ 5 | 0.1) = 1 − Σ_{i<5} e^-0.1 0.1^i / i!
        let below: f64 = (0..5)
            .map(|i| (-0.1f64).exp() * 0.1f64.powi(i) / (1..=i).map(f64::from).product::<f64>())
            .sum();
        assert!((at_least(5, 0.1) - (1.0 - below)).abs() < 1e-15);
        assert!(at_least(10, 900.0) > 0.999_999, "no underflow");
    }

    #[test]
    fn a_wave_needs_five_crashes_three_players_and_an_unusual_count() {
        assert!(is_wave(5, 3, 0.0));
        assert!(!is_wave(4, 4, 0.0), "four crashes");
        assert!(!is_wave(9, 2, 0.0), "two players");
        // A signature that crashes this often anyway is no wave.
        assert!(!is_wave(8, 5, 4.0));
        assert!(is_wave(14, 6, 2.0));
    }

    #[test]
    fn a_wave_links_to_the_last_change_at_most_thirty_minutes_before_it() {
        let minute = 60_000;
        let changes = [0, 10 * minute, 40 * minute];

        assert_eq!(
            linked(&changes, 39 * minute),
            Some(1),
            "29 minutes after the second"
        );
        assert_eq!(linked(&changes, 41 * minute), Some(2));
        assert_eq!(linked(&changes, 71 * minute), None, "31 minutes after the last");
        assert_eq!(linked(&changes, -1), None, "before every change");
    }

    #[test]
    fn the_quietest_hour_needs_every_day_measured() {
        let mut players = vec![Some(50.0); 168];
        let mut daily = vec![Some(50.0); 24];
        players[6] = Some(4.0);
        daily[6] = Some(10.0);
        daily[5] = Some(8.0);
        // Hour 5 is lower on average but one day lacks it.
        players[2 * 24 + 5] = None;

        assert_eq!(quietest(&players, &daily), Some(6));
        assert_eq!(quietest(&[None; 168], &[None; 24]), None);
    }

    #[test]
    fn runs_that_grow_faster_each_time_accelerate() {
        let mb = 1_048_576.0;
        let steady = [80.0 * mb, 88.0 * mb, 96.0 * mb, 104.0 * mb];
        let rising = acceleration(&steady, 32.0 * mb).unwrap();
        assert!(rising.accelerating, "{rising:?}");
        assert!((rising.per_run - 8.0 * mb).abs() < 1.0);

        assert!(
            !acceleration(&[90.0 * mb, 70.0 * mb, 95.0 * mb, 60.0 * mb], 32.0 * mb)
                .unwrap()
                .accelerating
        );
        assert!(
            !acceleration(&[1.0 * mb, 2.0 * mb, 3.0 * mb], 32.0 * mb)
                .unwrap()
                .accelerating,
            "under the floor"
        );
        assert_eq!(acceleration(&[80.0 * mb, 88.0 * mb], 32.0 * mb), None);
    }

    #[test]
    fn survival_steps_down_at_each_end_and_keeps_sessions_the_server_ended_out() {
        // Four sessions: ends at 5 and 20 minutes, one open at 10, one ended by the server at 30.
        let mut sessions = [(5.0, true), (10.0, false), (20.0, true), (30.0, false)];

        let (points, median) = survival(&mut sessions, &[0.0, 5.0, 15.0, 25.0, 60.0]);

        // 1 − 1/4 at 5; after the open one leaves, 2 at risk: × (1 − 1/2) at 20.
        assert_eq!(points, vec![1.0, 0.75, 0.75, 0.375, 0.375]);
        assert_eq!(median, Some(20.0));
    }

    #[test]
    fn survival_without_ends_stays_at_one_and_has_no_median() {
        let mut sessions = [(12.0, false), (40.0, false)];

        assert_eq!(survival(&mut sessions, &[0.0, 60.0]), (vec![1.0, 1.0], None));
        assert_eq!(survival(&mut [], &[0.0]), (vec![1.0], None));
    }

    #[test]
    fn a_period_of_frames_gives_its_shares_and_percentiles() {
        let period = FramePeriod::of(0, 3600, &NEW_BOUNDS, &ticks(&[(1, 75), (2, 20), (10, 5)]));

        assert_eq!(period.ticks, 100);
        assert_eq!(period.shares.as_ref().map(Vec::len), Some(14));
        assert_eq!(period.over_budget, Some(0.05));
        assert!((period.p50.unwrap() - (2.0 + 2.0 * 50.0 / 75.0)).abs() < 1e-9);
        assert!(period.p99.unwrap() > 70.0);
        assert_eq!(FramePeriod::of(0, 1, &NEW_BOUNDS, &ticks(&[])).shares, None);
    }
}
