//! The statistics of the Player drops page, from the metrics database alone: what holds no player
//! identity. Names and reason groups are added per caller afterwards.

use std::collections::{BTreeMap, HashMap};

use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan as _, Zoned};
use observer_domain::drops::stats::{self, Comparison, Verdict};
use observer_domain::drops::{
    BeforeAfter, CategoryCount, CategoryShare, ChangeEvent, ChangeKind, ChangeMarker, ChartCategory,
    Coverage, CrashForm, CrashGroup, DropCategory, DropChanges, DropKind, DropSeries, DropTile, DropUptime,
    DropsRange, DropsSummary, ReasonGroup, ResourceEntry, ResourceTable, SignatureEntry, SignatureTable,
    TileKind, Trend, TrendVerdict, UptimeBin, UptimeVerdict,
};
use observer_store::Connection;
use observer_store::StoreError;
use observer_store::metrics::drops::{self as stored, Counts, Group, HOUR_MS};
use observer_store::metrics::runs;

const DAY_MS: i64 = 24 * HOUR_MS;
/// The days whose spread tells how noisy this server's drops are.
const DISPERSION_DAYS: i64 = 28;
/// A day with less play than this says nothing about the noise.
const LEAST_DAY_MS: i64 = HOUR_MS;
/// One player making more than this share of a category's drops is named as the cause, not the server.
const ONE_PLAYER: f64 = 0.3;
const ONE_PLAYER_LEAST_DROPS: i64 = 5;
/// Repeat crashers: this many crashes in the last 7 days.
const REPEAT_CRASHES: i64 = 3;
const REPEAT_SHOWN: u32 = 20;
const SIGNATURES_SHOWN: usize = 50;
const RESOURCES_SHOWN: usize = 30;
const REASONS_SHOWN: usize = 5;
const EXAMPLES_SHOWN: u32 = 3;
/// A change is compared on this much time each side, cut at the next change.
const CHANGE_SIDE_MS: i64 = 72 * HOUR_MS;
const BASELINE_WEEKS: i64 = 4;
const UPTIME_BINS: [(u32, Option<u32>); 5] = [
    (0, Some(1)),
    (1, Some(3)),
    (3, Some(6)),
    (6, Some(12)),
    (12, None),
];

/// The times a range reads: what it shows, and the two windows its trends compare.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Windows {
    pub from_ms: i64,
    pub to_ms: i64,
    /// Now, and before: a day compares today since midnight with yesterday's same hours.
    pub now: (i64, i64),
    pub before: (i64, i64),
}

impl Windows {
    pub(super) fn of(range: DropsRange, now: Timestamp, zone: &TimeZone) -> Self {
        let now_ms = now.as_millisecond();
        let days = range.days();
        let from_ms = now_ms - days * DAY_MS;
        if range == DropsRange::Day {
            let here = now.to_zoned(zone.clone());
            let midnight = here
                .start_of_day()
                .map_or(from_ms, |start| start.timestamp().as_millisecond());
            let yesterday = |zoned: &Zoned| {
                zoned
                    .checked_sub(1.day())
                    .map_or(zoned.timestamp().as_millisecond() - DAY_MS, |day| {
                        day.timestamp().as_millisecond()
                    })
            };
            let midnight_zoned = Timestamp::from_millisecond(midnight)
                .unwrap_or(now)
                .to_zoned(zone.clone());
            return Self {
                from_ms,
                to_ms: now_ms,
                now: (midnight, now_ms),
                before: (yesterday(&midnight_zoned), yesterday(&here)),
            };
        }
        Self {
            from_ms,
            to_ms: now_ms,
            now: (from_ms, now_ms),
            before: (from_ms - days * DAY_MS, from_ms),
        }
    }
}

/// Which drops a tile, a chart or a test counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pick {
    Category(DropCategory),
    Unexpected,
}

impl Pick {
    fn takes(self, group: &Group) -> bool {
        match self {
            Self::Category(category) => group.category == category,
            Self::Unexpected => group.unexpected(),
        }
    }

    fn of_chart(category: ChartCategory) -> Self {
        match category {
            ChartCategory::Unexpected => Self::Unexpected,
            ChartCategory::Crash => Self::Category(DropCategory::Crash),
            ChartCategory::Timeout => Self::Category(DropCategory::Timeout),
            ChartCategory::Kick => Self::Category(DropCategory::Kick),
            ChartCategory::Security => Self::Category(DropCategory::Security),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Tally {
    drops: i64,
    events: i64,
    phased: i64,
    loading: i64,
}

fn tally(counts: &Counts, keep: impl Fn(&Group) -> bool) -> Tally {
    counts
        .groups
        .iter()
        .filter(|group| keep(group))
        .fold(Tally::default(), |tally, group| Tally {
            drops: tally.drops + group.drops,
            events: tally.events + group.events(),
            phased: tally.phased + group.phased,
            loading: tally.loading + group.loading,
        })
}

fn count(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

#[expect(clippy::cast_precision_loss, reason = "milliseconds of play far below 2^52")]
fn hours(ms: i64) -> f64 {
    ms as f64 / 3_600_000.0
}

#[expect(clippy::cast_precision_loss, reason = "counts far below 2^52")]
fn share(part: i64, whole: i64) -> f64 {
    if whole > 0 {
        part as f64 / whole as f64
    } else {
        0.0
    }
}

fn rate(drops: i64, player_ms: i64) -> Option<f64> {
    stats::per_hundred_hours(count(drops), player_ms)
}

fn compare(now: (i64, i64), before: (i64, i64), phi: f64) -> Comparison {
    stats::compare(count(now.0), hours(now.1), count(before.0), hours(before.1), phi)
}

fn trend(comparison: &Comparison, verdict: Verdict, now: (i64, i64), before: (i64, i64)) -> Trend {
    let change = |value: Option<f64>| value.filter(|value| value.is_finite()).map(|value| value - 1.0);
    Trend {
        verdict: match verdict {
            Verdict::Rising => TrendVerdict::Rising,
            Verdict::Falling => TrendVerdict::Falling,
            Verdict::NoClearChange => TrendVerdict::NoClearChange,
            Verdict::NotEnoughDrops => TrendVerdict::NotEnoughDrops,
        },
        change: change(comparison.ratio),
        low: change(comparison.low),
        high: change(comparison.high),
        now: rate(now.0, now.1),
        before: rate(before.0, before.1),
        events_now: count(now.0),
        events_before: count(before.0),
    }
}

/// The trend of what `pick` counts between the two windows, widened by this server's noise.
fn trend_of(now: &Counts, before: &Counts, phi: f64, keep: impl Fn(&Group) -> bool) -> Trend {
    let now = (tally(now, &keep).events, now.player_ms);
    let before_tally = (tally(before, &keep).events, before.player_ms);
    let comparison = compare(now, before_tally, phi);
    trend(&comparison, stats::verdict(&comparison), now, before_tally)
}

/// The hours from `from_ms` to `to_ms`, read once for what several answers need.
struct Hours {
    hours: BTreeMap<i64, Counts>,
}

impl Hours {
    fn read(conn: &Connection, from_ms: i64, now_ms: i64) -> Result<Self, StoreError> {
        Ok(Self {
            hours: stored::hourly(
                conn,
                stored::hour_of(from_ms),
                stored::hour_of(now_ms) + 3600,
                now_ms,
            )?,
        })
    }

    /// The counts of the hours that start from `from_ms` to before `to_ms`.
    fn between(&self, from_ms: i64, to_ms: i64) -> Counts {
        let mut counts = Counts::default();
        for hour in self
            .hours
            .range(from_ms.div_euclid(1000)..to_ms.div_euclid(1000))
            .map(|(_, hour)| hour)
        {
            counts.add(hour);
        }
        counts
    }
}

/// The starts of the days of `zone` from the one that holds `from_ms` to the one after `to_ms`.
fn day_starts(zone: &TimeZone, from_ms: i64, to_ms: i64) -> Vec<i64> {
    let Ok(first) = Timestamp::from_millisecond(from_ms) else {
        return Vec::new();
    };
    let Ok(mut day) = first.to_zoned(zone.clone()).start_of_day() else {
        return Vec::new();
    };
    let mut starts = Vec::new();
    loop {
        let start = day.timestamp().as_millisecond();
        starts.push(start);
        if start > to_ms {
            return starts;
        }
        match day.checked_add(1.day()) {
            Ok(next) => day = next,
            Err(_) => return starts,
        }
    }
}

/// How much more this server's days vary than chance allows, from the 28 days before today.
fn noise(hours: &Hours, zone: &TimeZone, now_ms: i64, keep: impl Fn(&Group) -> bool) -> f64 {
    let starts = day_starts(zone, now_ms - DISPERSION_DAYS * DAY_MS, now_ms);
    let days: Vec<(u64, f64)> = starts
        .windows(2)
        .filter(|day| day[1] <= now_ms)
        .map(|day| hours.between(day[0], day[1]))
        .filter(|day| day.player_ms >= LEAST_DAY_MS)
        .map(|day| (count(tally(&day, &keep).events), self::hours(day.player_ms)))
        .collect();
    stats::dispersion(&days)
}

/// `GET /drops/summary`, without names: repeat crashers come as pseudonyms.
pub(super) struct Summary {
    pub summary: DropsSummary,
    pub repeat: Vec<([u8; 16], i64)>,
}

/// What a range's answers read: its counts, the two windows its trends compare, and its hours.
struct Reads {
    windows: Windows,
    shown: Counts,
    current: Counts,
    before: Counts,
}

impl Reads {
    fn of(conn: &Connection, range: DropsRange, now: Timestamp, zone: &TimeZone) -> Result<Self, StoreError> {
        let windows = Windows::of(range, now, zone);
        let now_ms = windows.to_ms;
        Ok(Self {
            windows,
            shown: stored::window(conn, windows.from_ms, now_ms, now_ms)?,
            current: stored::window(conn, windows.now.0, windows.now.1, now_ms)?,
            before: stored::window(conn, windows.before.0, windows.before.1, now_ms)?,
        })
    }
}

pub(super) fn summary(
    conn: &Connection,
    range: DropsRange,
    now: Timestamp,
    zone: &TimeZone,
) -> Result<Summary, StoreError> {
    let reads = Reads::of(conn, range, now, zone)?;
    let windows = reads.windows;
    let now_ms = windows.to_ms;
    let shown = &reads.shown;
    let hours = Hours::read(
        conn,
        windows.from_ms.min(now_ms - (DISPERSION_DAYS + 1) * DAY_MS),
        now_ms,
    )?;
    let tiles = tiles(conn, range, &reads, &hours, zone)?;
    let categories = categories(&reads, &hours, zone);
    let unclosed = |kind| tally(shown, |group| group.kind == Some(kind)).drops;
    let repeat = stored::top_players(
        conn,
        Some(DropCategory::Crash),
        now_ms - 7 * DAY_MS,
        now_ms,
        REPEAT_CRASHES,
        REPEAT_SHOWN,
    )?;
    let at = |ms: i64| Timestamp::from_millisecond(ms).unwrap_or(now);
    Ok(Summary {
        summary: DropsSummary {
            range,
            zone: zone.iana_name().unwrap_or("UTC").to_owned(),
            from: at(windows.from_ms),
            to: now,
            trend_from: at(windows.now.0),
            before_from: at(windows.before.0),
            before_to: at(windows.before.1),
            tiles,
            categories,
            played_ms: shown.player_ms,
            sessions: shown.sessions,
            repeat_crashers: Vec::new(),
            coverage: Coverage {
                sessions: shown.sessions,
                missed: unclosed(DropKind::Missed),
                lost: unclosed(DropKind::Lost),
            },
        },
        repeat,
    })
}

fn tiles(
    conn: &Connection,
    range: DropsRange,
    reads: &Reads,
    hours: &Hours,
    zone: &TimeZone,
) -> Result<Vec<DropTile>, StoreError> {
    let Reads {
        windows,
        shown,
        current,
        before,
    } = reads;
    let now_ms = windows.to_ms;
    let buckets = buckets(range, now_ms, zone);
    let mut tiles = Vec::new();
    for (kind, pick) in [
        (TileKind::Crashes, Pick::Category(DropCategory::Crash)),
        (TileKind::Timeouts, Pick::Category(DropCategory::Timeout)),
        (TileKind::Kicks, Pick::Category(DropCategory::Kick)),
        (TileKind::Unexpected, Pick::Unexpected),
    ] {
        let keep = |group: &Group| pick.takes(group);
        let total = tally(shown, keep);
        let phi = noise(hours, zone, now_ms, keep);
        let top = stored::top_players(
            conn,
            match pick {
                Pick::Category(category) => Some(category),
                Pick::Unexpected => None,
            },
            windows.from_ms,
            now_ms,
            1,
            1,
        )?;
        tiles.push(DropTile {
            kind,
            drops: total.drops,
            rate: rate(total.drops, shown.player_ms),
            one_in: (total.drops > 0)
                .then(|| share(shown.sessions, total.drops))
                .filter(|one_in| *one_in >= 1.0),
            trend: trend_of(current, before, phi, keep),
            spark: buckets
                .windows(2)
                .map(|bucket| {
                    let counts = hours.between(bucket[0], bucket[1]);
                    rate(tally(&counts, keep).drops, counts.player_ms)
                })
                .collect(),
            one_player_share: top
                .first()
                .filter(|_| total.drops >= ONE_PLAYER_LEAST_DROPS)
                .map(|(_, drops)| share(*drops, total.drops))
                .filter(|share| *share > ONE_PLAYER),
        });
    }
    Ok(tiles)
}

fn categories(reads: &Reads, hours: &Hours, zone: &TimeZone) -> Vec<CategoryShare> {
    let Reads {
        windows,
        shown,
        current,
        before,
    } = reads;
    let all = tally(shown, |_| true).drops;
    DropCategory::ALL
        .into_iter()
        .map(|category| {
            let keep = |group: &Group| group.category == category;
            let total = tally(shown, keep);
            let phi = noise(hours, zone, windows.to_ms, keep);
            CategoryShare {
                category,
                expected: matches!(
                    category,
                    DropCategory::Quit | DropCategory::Kick | DropCategory::ServerStop
                ),
                drops: total.drops,
                share: share(total.drops, all),
                rate: rate(total.drops, shown.player_ms),
                trend: trend_of(current, before, phi, keep),
                loading_share: (total.phased > 0).then(|| share(total.loading, total.phased)),
            }
        })
        .collect()
}

/// The starts of a range's buckets, and the end of the last: hours up to a week, days of `zone`
/// beyond.
fn buckets(range: DropsRange, now_ms: i64, zone: &TimeZone) -> Vec<i64> {
    if range.step() == 3600 {
        let count = range.days() * 24;
        let last = stored::hour_of(now_ms) * 1000;
        return (0..=count)
            .map(|step| last - (count - 1 - step) * HOUR_MS)
            .collect();
    }
    let starts = day_starts(zone, now_ms - (range.days() - 1) * DAY_MS, now_ms);
    starts
        .into_iter()
        .take_while(|&start| start <= now_ms + DAY_MS)
        .collect()
}

/// `GET /drops/series`.
pub(super) fn series(
    conn: &Connection,
    range: DropsRange,
    category: ChartCategory,
    now: Timestamp,
    zone: &TimeZone,
) -> Result<DropSeries, StoreError> {
    let now_ms = now.as_millisecond();
    let pick = Pick::of_chart(category);
    let keep = |group: &Group| pick.takes(group);
    let starts = buckets(range, now_ms, zone);
    let first = starts.first().copied().unwrap_or(now_ms);
    let hours = Hours::read(conn, first - BASELINE_WEEKS * 7 * DAY_MS, now_ms)?;
    let point = |from: i64, to: i64| {
        let counts = hours.between(from, to);
        (tally(&counts, keep).drops, counts.player_ms)
    };
    let mut rates = Vec::new();
    let mut baseline = Vec::new();
    let mut drops = Vec::new();
    for bucket in starts.windows(2) {
        let (dropped, played) = point(bucket[0], bucket[1]);
        drops.push(dropped);
        rates.push(rate(dropped, played));
        let mut earlier: Vec<f64> = (1..=BASELINE_WEEKS)
            .filter_map(|week| {
                let back = week * 7 * DAY_MS;
                let (dropped, played) = point(bucket[0] - back, bucket[1] - back);
                rate(dropped, played)
            })
            .collect();
        baseline.push(median(&mut earlier));
    }
    let runs = stored::runs(conn, first, now_ms)?;
    let changes = runs
        .windows(2)
        .filter(|pair| pair[1].started_ms >= first && build_changed(pair[0].build, pair[1].build))
        .map(|pair| ChangeMarker {
            at: pair[1].started_ms.div_euclid(1000),
            build: pair[1].build,
        })
        .collect();
    Ok(DropSeries {
        range,
        category,
        from: first.div_euclid(1000),
        step: range.step(),
        rates,
        baseline,
        drops,
        markers: runs::markers(conn, first.div_euclid(1000), now_ms.div_euclid(1000) + 1)?,
        changes,
    })
}

fn build_changed(before: Option<u32>, after: Option<u32>) -> bool {
    matches!((before, after), (Some(before), Some(after)) if before != after)
}

/// The median of at least two values.
fn median(values: &mut [f64]) -> Option<f64> {
    if values.len() < 2 {
        return None;
    }
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    Some(if values.len().is_multiple_of(2) {
        f64::midpoint(values[middle - 1], values[middle])
    } else {
        values[middle]
    })
}

/// `GET /drops/signatures`, with every signature shown and how many players ever produced each.
pub(super) struct Signatures {
    pub table: SignatureTable,
    /// Over every session kept: a client's free text is shown to statistics viewers from 2.
    pub players_ever: HashMap<i64, i64>,
}

pub(super) fn signatures(
    conn: &Connection,
    range: DropsRange,
    now: Timestamp,
    zone: &TimeZone,
) -> Result<Signatures, StoreError> {
    let Reads {
        windows,
        shown,
        current,
        before,
    } = Reads::of(conn, range, now, zone)?;
    let now_ms = windows.to_ms;
    let crash = |group: &Group| group.category == DropCategory::Crash;
    let crashes = tally(&shown, crash).drops;
    let mut subjects: Vec<(i64, Tally)> = Vec::new();
    for group in shown
        .groups
        .iter()
        .filter(|group| crash(group) && group.subject != 0)
    {
        let subject = group.subject;
        subjects.push((
            subject,
            tally(&shown, |other| crash(other) && other.subject == subject),
        ));
    }
    subjects.sort_by_key(|(subject, tally)| (std::cmp::Reverse(tally.drops), *subject));
    subjects.dedup_by_key(|(subject, _)| *subject);

    let ids: Vec<i64> = subjects.iter().map(|(subject, _)| *subject).collect();
    let info: HashMap<i64, stored::SignatureRow> = stored::signatures(conn, &ids)?
        .into_iter()
        .map(|row| (row.id, row))
        .collect();
    let players = stored::players_by_subject(conn, DropCategory::Crash, Some((windows.from_ms, now_ms)))?;
    let players_ever = stored::players_by_subject(conn, DropCategory::Crash, None)?;

    let comparisons: Vec<Comparison> = subjects
        .iter()
        .map(|(subject, _)| {
            let of =
                |counts: &Counts| tally(counts, |group| crash(group) && group.subject == *subject).events;
            compare(
                (of(&current), current.player_ms),
                (of(&before), before.player_ms),
                1.0,
            )
        })
        .collect();
    let verdicts = stats::list_verdicts(&comparisons);

    let mut modules: BTreeMap<Option<String>, (i64, i64)> = BTreeMap::new();
    let mut builds: BTreeMap<Option<String>, (i64, i64)> = BTreeMap::new();
    let mut entries = Vec::new();
    for (((subject, total), comparison), verdict) in subjects.iter().zip(&comparisons).zip(&verdicts) {
        let Some(row) = info.get(subject) else {
            continue;
        };
        let module = modules.entry(row.module.clone()).or_default();
        module.0 += total.drops;
        module.1 += 1;
        let build = builds
            .entry(row.game_build.map(|build| build.to_string()))
            .or_default();
        build.0 += total.drops;
        build.1 += 1;
        if entries.len() >= SIGNATURES_SHOWN {
            continue;
        }
        let of = |counts: &Counts| tally(counts, |group| crash(group) && group.subject == *subject).events;
        entries.push(SignatureEntry {
            id: row.id,
            signature: Some(row.signature.clone()),
            form: CrashForm::parse(&row.form).unwrap_or(CrashForm::Text),
            label: row.label.clone(),
            module: row.module.clone(),
            game_build: row.game_build,
            resource: row.resource.clone(),
            drops: total.drops,
            share: share(total.drops, crashes),
            players: players.get(subject).copied().unwrap_or(0),
            first_seen: Timestamp::from_millisecond(row.first_seen_ms).unwrap_or(now),
            new: row.first_seen_ms >= windows.from_ms,
            loading_share: (total.phased > 0).then(|| share(total.loading, total.phased)),
            trend: trend(
                comparison,
                *verdict,
                (of(&current), current.player_ms),
                (of(&before), before.player_ms),
            ),
            examples: stored::examples(conn, row.id, EXAMPLES_SHOWN)?,
        });
    }
    Ok(Signatures {
        table: SignatureTable {
            range,
            crashes,
            signatures: entries,
            modules: crash_groups(modules, crashes),
            builds: crash_groups(builds, crashes),
        },
        players_ever,
    })
}

/// Crashes gathered by a key, the largest first.
fn crash_groups(map: BTreeMap<Option<String>, (i64, i64)>, crashes: i64) -> Vec<CrashGroup> {
    let mut groups: Vec<CrashGroup> = map
        .into_iter()
        .map(|(key, (drops, signatures))| CrashGroup {
            key,
            drops,
            share: share(drops, crashes),
            signatures,
        })
        .collect();
    groups.sort_by_key(|group| std::cmp::Reverse(group.drops));
    groups
}

/// `GET /drops/resources`, every reason group included.
pub(super) fn resources(
    conn: &Connection,
    range: DropsRange,
    now: Timestamp,
    zone: &TimeZone,
) -> Result<ResourceTable, StoreError> {
    let windows = Windows::of(range, now, zone);
    let now_ms = windows.to_ms;
    let kick = |group: &Group| group.category == DropCategory::Kick;
    let shown = stored::window(conn, windows.from_ms, now_ms, now_ms)?;
    let current = stored::window(conn, windows.now.0, windows.now.1, now_ms)?;
    let before = stored::window(conn, windows.before.0, windows.before.1, now_ms)?;
    let kicks = tally(&shown, kick).drops;
    let mut subjects: Vec<(i64, i64)> = Vec::new();
    for group in shown.groups.iter().filter(|group| kick(group)) {
        match subjects.iter_mut().find(|(subject, _)| *subject == group.subject) {
            Some((_, drops)) => *drops += group.drops,
            None => subjects.push((group.subject, group.drops)),
        }
    }
    subjects.sort_by_key(|(subject, drops)| (std::cmp::Reverse(*drops), *subject));
    subjects.truncate(RESOURCES_SHOWN);
    let ids: Vec<i64> = subjects
        .iter()
        .map(|(subject, _)| *subject)
        .filter(|&id| id != 0)
        .collect();
    let names = stored::resource_names(conn, &ids)?;
    let players = stored::players_by_subject(conn, DropCategory::Kick, Some((windows.from_ms, now_ms)))?;
    let mut reasons: HashMap<i64, Vec<ReasonGroup>> = HashMap::new();
    for reason in stored::reason_groups(conn, windows.from_ms, now_ms)? {
        let list = reasons.entry(reason.resource).or_default();
        if list.len() < REASONS_SHOWN {
            list.push(ReasonGroup {
                template: reason.template,
                kicks: reason.kicks,
            });
        }
    }
    let of =
        |counts: &Counts, subject: i64| tally(counts, |group| kick(group) && group.subject == subject).events;
    let comparisons: Vec<Comparison> = subjects
        .iter()
        .map(|(subject, _)| {
            compare(
                (of(&current, *subject), current.player_ms),
                (of(&before, *subject), before.player_ms),
                1.0,
            )
        })
        .collect();
    let verdicts = stats::list_verdicts(&comparisons);
    let resources = subjects
        .iter()
        .zip(&comparisons)
        .zip(&verdicts)
        .map(|(((subject, drops), comparison), verdict)| ResourceEntry {
            resource: names.get(subject).cloned(),
            kicks: *drops,
            share: share(*drops, kicks),
            players: players.get(subject).copied().unwrap_or(0),
            trend: trend(
                comparison,
                *verdict,
                (of(&current, *subject), current.player_ms),
                (of(&before, *subject), before.player_ms),
            ),
            reasons: Some(reasons.remove(subject).unwrap_or_default()),
        })
        .collect();
    Ok(ResourceTable {
        range,
        kicks,
        resources,
    })
}

/// A build change at `start`: each unexpected category in the 72 hours before against the 72
/// after, both cut at the next change, and the signatures first seen after it.
fn build_change(
    conn: &Connection,
    start: i64,
    build_starts: &[i64],
    now_ms: i64,
) -> Result<(Vec<BeforeAfter>, Vec<String>), StoreError> {
    let since = build_starts
        .iter()
        .copied()
        .filter(|&change| change < start)
        .max()
        .map_or(start - CHANGE_SIDE_MS, |change| {
            change.max(start - CHANGE_SIDE_MS)
        });
    let until = build_starts
        .iter()
        .copied()
        .filter(|&change| change > start)
        .min()
        .map_or(start + CHANGE_SIDE_MS, |change| {
            change.min(start + CHANGE_SIDE_MS)
        })
        .min(now_ms);
    let before = stored::window(conn, since, start, now_ms)?;
    let after = stored::window(conn, start, until, now_ms)?;
    let before_after = [
        DropCategory::Crash,
        DropCategory::Timeout,
        DropCategory::Security,
        DropCategory::Unknown,
    ]
    .into_iter()
    .map(|category| BeforeAfter {
        category,
        trend: trend_of(&after, &before, 1.0, |group| {
            group.category == category && group.unexpected()
        }),
    })
    .collect();
    let ids: Vec<i64> = after
        .groups
        .iter()
        .filter(|group| group.category == DropCategory::Crash && group.subject != 0)
        .map(|group| group.subject)
        .collect();
    // A client's free text stays out: this list is shown to statistics viewers.
    let new_signatures = stored::signatures(conn, &ids)?
        .into_iter()
        .filter(|row| {
            row.first_seen_ms >= start && !CrashForm::parse(&row.form).is_none_or(CrashForm::free_text)
        })
        .map(|row| row.signature)
        .collect();
    Ok((before_after, new_signatures))
}

/// `GET /drops/changes`: what changed, newest first.
pub(super) fn changes(
    conn: &Connection,
    range: DropsRange,
    now: Timestamp,
    zone: &TimeZone,
) -> Result<DropChanges, StoreError> {
    let windows = Windows::of(range, now, zone);
    let now_ms = windows.to_ms;
    let at = |ms: i64| Timestamp::from_millisecond(ms).unwrap_or(now);
    let runs = stored::runs(conn, windows.from_ms, now_ms)?;
    let build_starts: Vec<i64> = runs
        .windows(2)
        .filter(|pair| build_changed(pair[0].build, pair[1].build))
        .map(|pair| pair[1].started_ms)
        .collect();
    let mut events = Vec::new();
    for (index, run) in runs.iter().enumerate() {
        let event = |kind| ChangeEvent {
            at: at(run.started_ms),
            kind,
            run: run.id,
            generation: run.generation,
            build_from: None,
            build_to: run.build,
            players_dropped: None,
            hang: false,
            incident_categories: Vec::new(),
            hitch_ms: None,
            players_online: None,
            before_after: Vec::new(),
            new_signatures: Vec::new(),
        };
        if run.started_ms >= windows.from_ms {
            let previous = index.checked_sub(1).map(|index| &runs[index]);
            match previous.filter(|previous| build_changed(previous.build, run.build)) {
                Some(previous) => {
                    let (before_after, new_signatures) =
                        build_change(conn, run.started_ms, &build_starts, now_ms)?;
                    events.push(ChangeEvent {
                        build_from: previous.build,
                        before_after,
                        new_signatures,
                        ..event(ChangeKind::Build)
                    });
                }
                None => events.push(event(ChangeKind::Restart)),
            }
        }
        if let (Some(ended), Some(cause)) = (run.ended_ms, run.end_cause.as_deref())
            && matches!(cause, "crashed" | "hung")
            && ended >= windows.from_ms
        {
            events.push(ChangeEvent {
                at: at(ended),
                players_dropped: Some(run.crash_drops),
                hang: cause == "hung",
                ..event(ChangeKind::ServerCrash)
            });
        }
    }
    for incident in stored::incidents(conn, windows.from_ms, now_ms)? {
        let run = runs.iter().find(|run| run.id == incident.run);
        events.push(ChangeEvent {
            at: at(incident.started_ms),
            kind: ChangeKind::Incident,
            run: incident.run,
            generation: run.map_or(0, |run| run.generation),
            build_from: None,
            build_to: run.and_then(|run| run.build),
            players_dropped: Some(incident.drops),
            hang: false,
            incident_categories: incident
                .categories
                .into_iter()
                .map(|(category, drops)| CategoryCount { category, drops })
                .collect(),
            hitch_ms: incident.hitch_max_ms,
            players_online: incident.players,
            before_after: Vec::new(),
            new_signatures: Vec::new(),
        });
    }
    events.sort_by_key(|event| std::cmp::Reverse(event.at));
    Ok(DropChanges {
        range,
        snapshots: false,
        events,
    })
}

/// `GET /drops/uptime`: unexpected drops by time since the run started.
pub(super) fn uptime(
    conn: &Connection,
    range: DropsRange,
    now: Timestamp,
    zone: &TimeZone,
) -> Result<DropUptime, StoreError> {
    let windows = Windows::of(range, now, zone);
    let now_ms = windows.to_ms;
    let mut played = [0i64; UPTIME_BINS.len()];
    let mut drops = [0i64; UPTIME_BINS.len()];
    let bin_of = |since_start: i64| {
        UPTIME_BINS
            .iter()
            .rposition(|(from, _)| since_start >= i64::from(*from) * HOUR_MS)
            .unwrap_or(0)
    };
    for session in stored::played(conn, windows.from_ms, now_ms, now_ms)? {
        let from = session.seen_ms.max(windows.from_ms);
        let until = session.until_ms.min(now_ms);
        for (index, (bin_from, bin_to)) in UPTIME_BINS.iter().enumerate() {
            let start = session.run_started_ms + i64::from(*bin_from) * HOUR_MS;
            let end = bin_to.map_or(i64::MAX, |to| session.run_started_ms + i64::from(to) * HOUR_MS);
            played[index] += (until.min(end) - from.max(start)).max(0);
        }
        if let Some(ended) = session
            .unexpected_ms
            .filter(|&ended| ended >= windows.from_ms && ended < now_ms)
        {
            drops[bin_of(ended - session.run_started_ms)] += 1;
        }
    }
    let total_drops: i64 = drops.iter().sum();
    let total_played: i64 = played.iter().sum();
    let comparisons: Vec<Comparison> = (0..UPTIME_BINS.len())
        .map(|index| {
            compare(
                (drops[index], played[index]),
                (total_drops - drops[index], total_played - played[index]),
                1.0,
            )
        })
        .collect();
    let verdicts = stats::list_verdicts(&comparisons);
    let bins = UPTIME_BINS
        .iter()
        .enumerate()
        .map(|(index, (from_hours, to_hours))| UptimeBin {
            from_hours: *from_hours,
            to_hours: *to_hours,
            drops: drops[index],
            played_ms: played[index],
            rate: rate(drops[index], played[index]),
            verdict: match verdicts[index] {
                Verdict::Rising => UptimeVerdict::Higher,
                Verdict::NotEnoughDrops => UptimeVerdict::NotEnoughDrops,
                Verdict::Falling | Verdict::NoClearChange => UptimeVerdict::Usual,
            },
        })
        .collect();
    Ok(DropUptime { range, bins })
}

/// A day's drops and play, for the rates export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DayRates {
    /// `YYYY-MM-DD` in the schedule's time zone.
    pub day: String,
    pub played_ms: i64,
    pub sessions: i64,
    pub drops: Vec<(DropCategory, i64)>,
}

pub(super) fn day_rates(
    conn: &Connection,
    from_ms: i64,
    to_ms: i64,
    now_ms: i64,
    zone: &TimeZone,
) -> Result<Vec<DayRates>, StoreError> {
    let hours = Hours::read(conn, from_ms, now_ms.min(to_ms))?;
    let starts = day_starts(zone, from_ms, to_ms);
    let mut days = Vec::new();
    for day in starts.windows(2).filter(|day| day[0] < to_ms && day[0] <= now_ms) {
        let counts = hours.between(day[0], day[1]);
        let date = Timestamp::from_millisecond(day[0])
            .map(|start| start.to_zoned(zone.clone()).date().to_string())
            .unwrap_or_default();
        days.push(DayRates {
            day: date,
            played_ms: counts.player_ms,
            sessions: counts.sessions,
            drops: DropCategory::ALL
                .into_iter()
                .map(|category| (category, tally(&counts, |group| group.category == category).drops))
                .collect(),
        });
    }
    Ok(days)
}

#[cfg(test)]
mod tests;
