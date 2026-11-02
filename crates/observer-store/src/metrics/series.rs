//! The series the panel charts, on a regular grid: the Overview's fixed ranges up to a day read
//! the five-second samples, a week the minute aggregates, a month the hour aggregates; the
//! timeline's windows the finest table that still keeps them. Beside them, svMain's percentiles
//! from its tick histograms and the players' usual range of the weeks before.

use std::collections::{BTreeMap, HashMap};

use observer_domain::metrics::{MetricsSeries, Range, Series, SeriesData};
use observer_domain::statistics::{
    FrameBands, LEAST_POINTS, LONGEST_WINDOW, MOST_POINTS, SHORTEST_WINDOW, Timeline, USUAL_WEEKS,
    UsualRange, quantile, usual as usual_of,
};
use rusqlite::{Connection, params};

use super::rollups::{MINUTES_KEPT, RAW_KEPT};
use super::{Column, Kind, runs};
use crate::database::StoreError;

/// Where a series' values come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Gauge(Column),
    Count(Column),
    /// Milliseconds per tick.
    TickMean {
        ticks: Column,
        ms: Column,
    },
    /// The share of ticks over budget, in percent.
    OverBudget {
        over: Column,
        ticks: Column,
    },
}

fn source(series: Series) -> Source {
    use Column as C;
    use Source::{Count, Gauge, OverBudget, TickMean};
    match series {
        Series::Players => Gauge(C::Players),
        Series::Slots => Gauge(C::Slots),
        Series::PingAvg => Gauge(C::PingAvgMs),
        Series::PingP95 => Gauge(C::PingP95Ms),
        Series::PingMax => Gauge(C::PingMaxMs),
        Series::Joins => Count(C::Joins),
        Series::Leaves => Count(C::Leaves),
        Series::MainTick => TickMean {
            ticks: C::MainTicks,
            ms: C::MainTickMs,
        },
        Series::NetworkTick => TickMean {
            ticks: C::NetworkTicks,
            ms: C::NetworkTickMs,
        },
        Series::SyncTick => TickMean {
            ticks: C::SyncTicks,
            ms: C::SyncTickMs,
        },
        Series::MainOverBudget => OverBudget {
            over: C::MainOver,
            ticks: C::MainTicks,
        },
        Series::NetworkOverBudget => OverBudget {
            over: C::NetworkOver,
            ticks: C::NetworkTicks,
        },
        Series::SyncOverBudget => OverBudget {
            over: C::SyncOver,
            ticks: C::SyncTicks,
        },
        Series::HitchesMain => Count(C::HitchMain),
        Series::HitchesNetwork => Count(C::HitchNetwork),
        Series::HitchesSync => Count(C::HitchSync),
        Series::Barks => Count(C::Barks),
        Series::HitchMax => Gauge(C::HitchMaxMs),
        Series::Resources => Gauge(C::Resources),
        Series::JsHeapUsed => Gauge(C::JsHeapUsed),
        Series::JsHeapTotal => Gauge(C::JsHeapTotal),
        Series::FxsCpu => Gauge(C::FxsCpu),
        Series::FxsWorkingSet => Gauge(C::FxsWs),
        Series::FxsPrivate => Gauge(C::FxsPrivate),
        Series::FxsThreads => Gauge(C::FxsThreads),
        Series::FxsHandles => Gauge(C::FxsHandles),
        Series::FxsRead => Gauge(C::FxsReadBps),
        Series::FxsWrite => Gauge(C::FxsWriteBps),
        Series::Children => Gauge(C::Children),
        Series::ChildrenCpu => Gauge(C::ChildrenCpu),
        Series::ChildrenWorkingSet => Gauge(C::ChildrenWs),
        Series::ChildrenPrivate => Gauge(C::ChildrenPrivate),
        Series::HostCpu => Gauge(C::HostCpu),
        Series::HostMemoryUsed => Gauge(C::HostMemUsed),
        Series::HostMemoryTotal => Gauge(C::HostMemTotal),
        Series::HostDiskFree => Gauge(C::HostDiskFree),
        Series::HostDiskTotal => Gauge(C::HostDiskTotal),
        Series::HostReceive => Gauge(C::HostRxBps),
        Series::HostSend => Gauge(C::HostTxBps),
        Series::ObserverCpu => Gauge(C::ObsCpu),
        Series::ObserverWorkingSet => Gauge(C::ObsWs),
        Series::ObserverPrivate => Gauge(C::ObsPrivate),
        Series::StateDatabase => Gauge(C::ObsStateDb),
        Series::MetricsDatabase => Gauge(C::ObsMetricsDb),
        Series::DashboardClients => Gauge(C::ObsWsClients),
        Series::SampleTime => Gauge(C::CollectMs),
        Series::FxsKernel => Gauge(C::FxsCpuKernel),
        Series::FxsFaults => Count(C::FxsFaults),
        Series::FxsHardFaults => Count(C::FxsHardFaults),
        Series::FxsWorkingSetPeak => Gauge(C::FxsWsPeak),
        Series::MainThreadCpu => Gauge(C::ThrMainCpu),
        Series::NetworkThreadCpu => Gauge(C::ThrNetworkCpu),
        Series::SyncThreadCpu => Gauge(C::ThrSyncCpu),
        Series::JsThreadsCpu => Gauge(C::ThrJsCpu),
        Series::MonoThreadsCpu => Gauge(C::ThrMonoCpu),
        Series::OtherThreadsCpu => Gauge(C::ThrOtherCpu),
        Series::TreeCpu => Gauge(C::TreeCpu),
        Series::TreeStarted => Count(C::TreeStarted),
        Series::TreePrivatePeak => Gauge(C::TreePrivatePeak),
        Series::HostClock => Gauge(C::HostCpuMhz),
        Series::HostClockMax => Gauge(C::HostCpuMaxMhz),
        Series::HostClockLimit => Gauge(C::HostCpuLimit),
        Series::HostDiskLatency => Gauge(C::HostDiskMs),
        Series::HostDiskTransfers => Count(C::HostDiskOps),
        Series::HostSteal => Gauge(C::HostSteal),
        Series::HostPressureCpu => Gauge(C::HostPsiCpu),
        Series::HostPressureMemory => Gauge(C::HostPsiMemory),
        Series::HostPressureIo => Gauge(C::HostPsiIo),
    }
}

impl Source {
    fn columns(self) -> Vec<Column> {
        match self {
            Self::Gauge(column) | Self::Count(column) => vec![column],
            Self::TickMean { ticks, ms } => vec![ticks, ms],
            Self::OverBudget { over, ticks } => vec![over, ticks],
        }
    }
}

/// The value of one series from one sample, as the live topic sends it.
pub fn value(series: Series, sample: &super::Values) -> Option<f64> {
    match source(series) {
        Source::Gauge(column) | Source::Count(column) => sample.get(column),
        Source::TickMean { ticks, ms } => ratio(sample.get(ms)?, sample.get(ticks)?),
        Source::OverBudget { over, ticks } => {
            ratio(sample.get(over)?, sample.get(ticks)?).map(|share| share * 100.0)
        }
    }
}

fn ratio(part: f64, whole: f64) -> Option<f64> {
    (whole > 0.0).then(|| part / whole)
}

/// One column over one bucket: the average of a gauge or the total of a count, its extremes, and
/// how many samples measured it.
#[derive(Debug, Clone, Copy, Default)]
struct Aggregate {
    value: Option<f64>,
    min: Option<f64>,
    max: Option<f64>,
    samples: i64,
}

/// The samples of a bucket, and those of them taken while the server was online or degraded.
const RAW_COVERAGE: &str = "count(*), sum(state IN ('online', 'degraded'))";
const ROLLED_UP_COVERAGE: &str = "sum(samples), sum(online)";

/// The steps a window may take, in seconds: each a multiple of the one its table is kept at.
const STEPS: [i64; 15] = [
    5, 10, 30, 60, 120, 300, 600, 900, 1800, 3600, 7200, 10_800, 21_600, 43_200, 86_400,
];
/// The longest step that still reads the five-second samples, as a day's range does.
const RAW_STEP: i64 = 120;
const WEEK: i64 = 7 * 86_400;

/// The table a window reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Table {
    Raw,
    Minute,
    Hour,
}

impl Table {
    fn samples(self) -> &'static str {
        match self {
            Self::Raw => "sample_5s",
            Self::Minute => "sample_1m",
            Self::Hour => "sample_1h",
        }
    }

    pub(crate) fn ticks(self) -> &'static str {
        match self {
            Self::Raw => "tick_5s",
            Self::Minute => "tick_1m",
            Self::Hour => "tick_1h",
        }
    }

    /// The seconds one row covers.
    fn span(self) -> i64 {
        match self {
            Self::Raw => 5,
            Self::Minute => 60,
            Self::Hour => 3600,
        }
    }
}

/// A regular grid of buckets: bucket `i` starts at `from + i × step`, read from `table`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    pub from: i64,
    pub step: i64,
    pub buckets: i64,
    pub table: Table,
}

impl Window {
    /// A fixed range ending at `now`, as the Overview charts it.
    pub fn of_range(range: Range, now: i64) -> Self {
        let (from, _) = range.grid(now);
        let table = match range {
            Range::Hour | Range::SixHours | Range::Day => Table::Raw,
            Range::Week => Table::Minute,
            Range::Month => Table::Hour,
        };
        Self {
            from,
            step: range.step(),
            buckets: range.buckets(),
            table,
        }
    }

    /// The grid for `from` to `to` (Unix seconds) at `now` in at most `points` buckets: the finest
    /// step that fits, from the finest table that still keeps the window. `None` for a window that
    /// is empty, shorter than a minute, longer than [`LONGEST_WINDOW`] or that ends more than a day
    /// from now.
    pub fn plan(from: i64, to: i64, points: u32, now: i64) -> Option<Self> {
        let span = to.checked_sub(from)?;
        if !(SHORTEST_WINDOW..=LONGEST_WINDOW).contains(&span) || to > now + 86_400 {
            return None;
        }
        let points = i64::from(points.clamp(LEAST_POINTS, MOST_POINTS));
        let fitting = STEPS
            .into_iter()
            .find(|&step| (span + step - 1) / step <= points)
            .unwrap_or(STEPS[STEPS.len() - 1]);
        let (table, step) = if fitting <= RAW_STEP && from >= now - RAW_KEPT {
            (Table::Raw, fitting)
        } else if fitting < 3600 && from >= now - MINUTES_KEPT {
            (Table::Minute, fitting.max(60))
        } else {
            (Table::Hour, fitting.max(3600))
        };
        let start = from.div_euclid(step) * step;
        let end = to.div_euclid(step) * step + if to.rem_euclid(step) == 0 { 0 } else { step };
        Some(Self {
            from: start,
            step,
            buckets: (end - start) / step,
            table,
        })
    }

    pub fn end(&self) -> i64 {
        self.from + self.step * self.buckets
    }

    fn len(&self) -> usize {
        usize::try_from(self.buckets).unwrap_or_default()
    }

    /// The bucket a row of `ts` falls in.
    fn bucket(&self, ts: i64) -> Option<usize> {
        usize::try_from((ts - self.from).div_euclid(self.step))
            .ok()
            .filter(|&bucket| bucket < self.len())
    }
}

/// What a window's buckets hold: each series, and how many samples were taken.
struct Grid {
    series: Vec<SeriesData>,
    observed: Vec<u32>,
    online: Vec<u32>,
    last: Option<i64>,
}

/// The range ending at `now` (Unix seconds).
pub fn read(
    conn: &Connection,
    range: Range,
    series: &[Series],
    now: i64,
) -> Result<MetricsSeries, StoreError> {
    let window = Window::of_range(range, now);
    let grid = read_grid(conn, &window, series)?;
    Ok(MetricsSeries {
        range,
        step: window.step,
        from: window.from,
        buckets: window.buckets,
        last: grid.last,
        series: grid.series,
        markers: runs::markers(conn, window.from, window.end())?,
        observed: grid.observed,
        online: grid.online,
    })
}

/// A window of the timeline: its series, svMain's percentiles when `mainTick` is among them, and
/// the players' usual range when `players` is.
pub fn timeline(conn: &Connection, window: &Window, series: &[Series]) -> Result<Timeline, StoreError> {
    let grid = read_grid(conn, window, series)?;
    let frames = series
        .contains(&Series::MainTick)
        .then(|| frames(conn, window))
        .transpose()?;
    let usual = series
        .contains(&Series::Players)
        .then(|| usual(conn, window))
        .transpose()?;
    Ok(Timeline {
        from: window.from,
        step: window.step,
        buckets: window.buckets,
        last: grid.last,
        series: grid.series,
        observed: grid.observed,
        online: grid.online,
        frames,
        usual,
    })
}

fn read_grid(conn: &Connection, window: &Window, series: &[Series]) -> Result<Grid, StoreError> {
    let mut columns: Vec<Column> = series
        .iter()
        .flat_map(|&series| source(series).columns())
        .collect();
    columns.sort_unstable();
    columns.dedup();

    let rolled_up = window.table != Table::Raw;
    let (coverage, expressions) = if rolled_up {
        (ROLLED_UP_COVERAGE, rolled_up_expressions(&columns))
    } else {
        (RAW_COVERAGE, raw_expressions(&columns))
    };
    let sql = format!(
        "SELECT (ts - ?1) / ?2, max(ts), {coverage}, {} FROM {} WHERE ts >= ?1 AND ts < ?3 GROUP BY 1",
        expressions.join(", "),
        window.table.samples()
    );
    let buckets = window.len();
    let mut grid: Vec<BTreeMap<Column, Aggregate>> = vec![BTreeMap::new(); buckets];
    let mut observed = vec![0; buckets];
    let mut online = vec![0; buckets];
    let mut newest: Option<i64> = None;
    let mut statement = conn.prepare_cached(&sql)?;
    let mut rows = statement.query(params![window.from, window.step, window.end()])?;
    while let Some(row) = rows.next()? {
        let Some(bucket) = usize::try_from(row.get::<_, i64>(0)?)
            .ok()
            .filter(|&bucket| bucket < buckets)
        else {
            continue;
        };
        let ts: i64 = row.get(1)?;
        newest = newest.max(Some(ts));
        let count = |samples: Option<i64>| u32::try_from(samples.unwrap_or(0)).unwrap_or(u32::MAX);
        observed[bucket] = count(row.get(2)?);
        online[bucket] = count(row.get(3)?);
        for (index, &column) in columns.iter().enumerate() {
            let at = 4 + index * 4;
            let mut aggregate = Aggregate {
                value: row.get(at)?,
                min: row.get(at + 1)?,
                max: row.get(at + 2)?,
                samples: row.get::<_, Option<i64>>(at + 3)?.unwrap_or(0),
            };
            if aggregate.samples == 0 {
                aggregate.value = None;
            } else if rolled_up && column.kind() == Kind::Gauge {
                // The weighted sum of the averages, over the samples behind them.
                #[allow(clippy::cast_precision_loss)]
                let samples = aggregate.samples as f64;
                aggregate.value = aggregate.value.map(|sum| sum / samples);
            }
            grid[bucket].insert(column, aggregate);
        }
    }

    // A bucket of one five-second sample has no extremes to tell.
    let extremes = window.table != Table::Raw || window.step > 5;
    Ok(Grid {
        series: series
            .iter()
            .map(|&series| series_data(series, &grid, extremes))
            .collect(),
        observed,
        online,
        last: newest.map(|ts| ts + window.table.span() - 5),
    })
}

/// svMain's ticks of each set of bounds in each bucket of `window`, interval by interval.
pub(crate) fn tick_grid(
    conn: &Connection,
    window: &Window,
    known: &HashMap<i64, Vec<f64>>,
) -> Result<Vec<BTreeMap<i64, Vec<u64>>>, StoreError> {
    let sql = format!(
        "SELECT ts, bounds, bucket, ticks FROM {} WHERE ts >= ?1 AND ts < ?2",
        window.table.ticks()
    );
    let mut per_bucket: Vec<BTreeMap<i64, Vec<u64>>> = vec![BTreeMap::new(); window.len()];
    let mut statement = conn.prepare_cached(&sql)?;
    let mut rows = statement.query(params![window.from, window.end()])?;
    while let Some(row) = rows.next()? {
        let id: i64 = row.get(1)?;
        let (Some(bucket), Some(bounds)) = (window.bucket(row.get(0)?), known.get(&id)) else {
            continue;
        };
        let Some(interval) = usize::try_from(row.get::<_, i64>(2)?)
            .ok()
            .filter(|&interval| interval <= bounds.len())
        else {
            continue;
        };
        let ticks = per_bucket[bucket]
            .entry(id)
            .or_insert_with(|| vec![0; bounds.len() + 1]);
        ticks[interval] += u64::try_from(row.get::<_, i64>(3)?).unwrap_or(0);
    }
    Ok(per_bucket)
}

/// svMain's percentiles in each bucket, from the bounds that hold most of its ticks there.
fn frames(conn: &Connection, window: &Window) -> Result<FrameBands, StoreError> {
    let known = tick_bounds(conn)?;
    let per_bucket = tick_grid(conn, window, &known)?;
    let mut bands = FrameBands {
        p50: Vec::with_capacity(per_bucket.len()),
        p95: Vec::with_capacity(per_bucket.len()),
        p99: Vec::with_capacity(per_bucket.len()),
        ticks: Vec::with_capacity(per_bucket.len()),
        floor: None,
        top: None,
        since: first_frames(conn)?,
    };
    let mut ticks_by_bounds: BTreeMap<i64, u64> = BTreeMap::new();
    for sets in &per_bucket {
        let chosen = sets
            .iter()
            .max_by_key(|(_, ticks)| ticks.iter().sum::<u64>())
            .and_then(|(id, ticks)| Some((*id, known.get(id)?, ticks)));
        let Some((id, bounds, ticks)) = chosen else {
            bands.p50.push(None);
            bands.p95.push(None);
            bands.p99.push(None);
            bands.ticks.push(0);
            continue;
        };
        let total: u64 = ticks.iter().sum();
        *ticks_by_bounds.entry(id).or_default() += total;
        bands.p50.push(quantile(bounds, ticks, 0.50));
        bands.p95.push(quantile(bounds, ticks, 0.95));
        bands.p99.push(quantile(bounds, ticks, 0.99));
        bands.ticks.push(u32::try_from(total).unwrap_or(u32::MAX));
    }
    if let Some(bounds) = ticks_by_bounds
        .iter()
        .max_by_key(|(_, ticks)| **ticks)
        .and_then(|(id, _)| known.get(id))
    {
        bands.floor = bounds.first().copied();
        bands.top = bounds.last().copied();
    }
    Ok(bands)
}

/// Every set of bounds stored, by its id.
pub fn tick_bounds(conn: &Connection) -> Result<HashMap<i64, Vec<f64>>, StoreError> {
    let mut statement = conn.prepare_cached("SELECT id, bounds FROM tick_bounds")?;
    let rows = statement.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))?;
    let mut known = HashMap::new();
    for row in rows {
        let (id, bounds) = row?;
        if let Ok(bounds) = serde_json::from_str::<Vec<f64>>(&bounds) {
            known.insert(id, bounds);
        }
    }
    Ok(known)
}

/// The first slot, minute or hour with a histogram, Unix seconds.
pub fn first_frames(conn: &Connection) -> Result<Option<i64>, StoreError> {
    Ok(conn
        .prepare_cached(
            "SELECT min(first) FROM (SELECT min(ts) AS first FROM tick_1h UNION ALL \
             SELECT min(ts) FROM tick_1m UNION ALL SELECT min(ts) FROM tick_5s)",
        )?
        .query_row([], |row| row.get(0))?)
}

/// For each bucket, the lowest and highest players at the same time in each of the 4 weeks
/// before, from the hour aggregates: the hour that holds the bucket's start for buckets shorter
/// than an hour, the hours inside it, weighted by their samples, for longer ones.
fn usual(conn: &Connection, window: &Window) -> Result<UsualRange, StoreError> {
    let first = window.from - USUAL_WEEKS * WEEK;
    let first = first - first.rem_euclid(3600);
    let mut statement = conn.prepare_cached(
        "SELECT ts, players_avg, players_n FROM sample_1h WHERE ts >= ?1 AND ts < ?2 AND players_n > 0",
    )?;
    let hours: BTreeMap<i64, (f64, f64)> = statement
        .query_map(params![first, window.end() - WEEK], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                (row.get::<_, f64>(1)?, row.get::<_, f64>(2)?),
            ))
        })?
        .collect::<Result<_, _>>()?;
    let average = |from: i64, to: i64| {
        let from = from - from.rem_euclid(3600);
        let (sum, weight) = hours
            .range(from..to.max(from + 1))
            .fold((0.0, 0.0), |(sum, weight), (_, (players, samples))| {
                (sum + players * samples, weight + samples)
            });
        (weight > 0.0).then(|| sum / weight)
    };
    let mut range = UsualRange {
        low: Vec::with_capacity(window.len()),
        high: Vec::with_capacity(window.len()),
    };
    for bucket in 0..window.buckets {
        let start = window.from + bucket * window.step;
        let weeks: Vec<Option<f64>> = (1..=USUAL_WEEKS)
            .map(|week| average(start - week * WEEK, start + window.step - week * WEEK))
            .collect();
        let (low, high) = usual_of(&weeks).unzip();
        range.low.push(low);
        range.high.push(high);
    }
    Ok(range)
}

/// Per column: the value, minimum, maximum and samples of a bucket of five-second samples.
fn raw_expressions(columns: &[Column]) -> Vec<String> {
    columns
        .iter()
        .map(|column| {
            let name = column.name();
            match column.kind() {
                Kind::Gauge => format!("avg({name}), min({name}), max({name}), count({name})"),
                Kind::Count => format!("sum({name}), NULL, NULL, count({name})"),
            }
        })
        .collect()
}

/// The same from aggregates, a gauge's average weighted by the samples behind it.
fn rolled_up_expressions(columns: &[Column]) -> Vec<String> {
    columns
        .iter()
        .map(|column| {
            let name = column.name();
            match column.kind() {
                Kind::Gauge => {
                    format!("sum({name}_avg * {name}_n), min({name}_min), max({name}_max), sum({name}_n)")
                }
                Kind::Count => format!("sum({name}_sum), NULL, NULL, sum({name}_n)"),
            }
        })
        .collect()
}

fn series_data(series: Series, grid: &[BTreeMap<Column, Aggregate>], extremes: bool) -> SeriesData {
    let get = |bucket: &BTreeMap<Column, Aggregate>, column: Column| {
        bucket.get(&column).copied().unwrap_or_default()
    };
    let source = source(series);
    let values = grid
        .iter()
        .map(|bucket| match source {
            Source::Gauge(column) | Source::Count(column) => get(bucket, column).value,
            Source::TickMean { ticks, ms } => ratio(get(bucket, ms).value?, get(bucket, ticks).value?),
            // Ticks without an over count mean bounds without the budget: no share then, rather
            // than a guess from the samples that had one.
            Source::OverBudget { over, ticks } => {
                let (over, ticks) = (get(bucket, over), get(bucket, ticks));
                if over.samples < ticks.samples {
                    return None;
                }
                ratio(over.value?, ticks.value?).map(|share| share * 100.0)
            }
        })
        .collect();
    let mut data = SeriesData {
        series,
        values,
        min: None,
        max: None,
        samples: None,
    };
    if let Source::Gauge(column) = source
        && extremes
    {
        data.min = Some(grid.iter().map(|bucket| get(bucket, column).min).collect());
        data.max = Some(grid.iter().map(|bucket| get(bucket, column).max).collect());
        data.samples = Some(
            grid.iter()
                .map(|bucket| u32::try_from(get(bucket, column).samples).unwrap_or(u32::MAX))
                .collect(),
        );
    }
    data
}

#[cfg(test)]
mod tests {
    use observer_domain::metrics::MarkerKind;
    use observer_domain::server::ServerState;

    use super::*;
    use crate::metrics::memory;
    use crate::metrics::rollups::{self, Tier};
    use crate::metrics::samples::tests::{sample, store};

    /// 2025-10-28T09:00:00Z, on every grid.
    const T: i64 = 1_761_642_000;

    fn values(data: &MetricsSeries, series: Series) -> &[Option<f64>] {
        &data
            .series
            .iter()
            .find(|data| data.series == series)
            .unwrap()
            .values
    }

    #[test]
    fn an_hour_is_720_slots_with_nulls_where_nothing_was_measured() {
        let mut conn = memory();
        let now = T + 3599;
        // Observer ran for the first ten minutes and the last two, the server for part of it.
        let mut samples: Vec<_> = (0..120)
            .map(|slot| {
                let players = if slot < 60 {
                    vec![(Column::Players, 10.0)]
                } else {
                    vec![]
                };
                sample(T + slot * 5, ServerState::Online, &players)
            })
            .collect();
        samples.extend(
            (696..720).map(|slot| sample(T + slot * 5, ServerState::Stopped, &[(Column::ObsWs, 1e6)])),
        );
        store(&mut conn, &samples);

        let data = read(
            &conn,
            Range::Hour,
            &[Series::Players, Series::ObserverWorkingSet],
            now,
        )
        .unwrap();

        assert_eq!((data.from, data.step, data.buckets), (T, 5, 720));
        assert_eq!(data.last, Some(T + 3595));
        let players = values(&data, Series::Players);
        assert_eq!(players.len(), 720);
        assert!(players[..60].iter().all(|value| *value == Some(10.0)));
        assert!(players[60..].iter().all(Option::is_none));
        let observer = values(&data, Series::ObserverWorkingSet);
        assert!(
            observer[120..696].iter().all(Option::is_none),
            "Observer was down"
        );
        assert_eq!(observer[719], Some(1e6));
        assert!(data.series[0].min.is_none(), "a five-second bucket is one sample");
    }

    #[test]
    fn grouped_buckets_average_gauges_and_keep_their_extremes_and_samples() {
        let mut conn = memory();
        store(
            &mut conn,
            &[
                sample(
                    T,
                    ServerState::Online,
                    &[(Column::FxsCpu, 10.0), (Column::Joins, 1.0)],
                ),
                sample(T + 5, ServerState::Online, &[(Column::FxsCpu, 50.0)]),
                sample(T + 10, ServerState::Online, &[(Column::Joins, 2.0)]),
            ],
        );

        let data = read(&conn, Range::SixHours, &[Series::FxsCpu, Series::Joins], T + 10).unwrap();

        let bucket = usize::try_from((T - data.from) / 30).unwrap();
        let cpu = &data.series[0];
        assert_eq!(cpu.values[bucket], Some(30.0));
        assert_eq!(cpu.min.as_ref().unwrap()[bucket], Some(10.0));
        assert_eq!(cpu.max.as_ref().unwrap()[bucket], Some(50.0));
        assert_eq!(cpu.samples.as_ref().unwrap()[bucket], 2);
        assert_eq!(data.series[1].values[bucket], Some(3.0));
        assert!(data.series[1].min.is_none(), "a count has a total only");
    }

    #[test]
    fn tick_means_and_shares_over_budget_come_from_the_totals_and_never_guess() {
        let mut conn = memory();
        let ticks = |ticks: f64, ms: f64, over: Option<f64>| {
            let mut values = vec![(Column::MainTicks, ticks), (Column::MainTickMs, ms)];
            values.extend(over.map(|over| (Column::MainOver, over)));
            values
        };
        store(
            &mut conn,
            &[
                // One bucket of two minutes: 100 ticks of 12 ms, then 100 of 6 ms with 10 over budget.
                sample(T, ServerState::Online, &ticks(100.0, 1200.0, Some(0.0))),
                sample(T + 5, ServerState::Online, &ticks(100.0, 600.0, Some(10.0))),
                // The next: one sample whose bounds had no budget.
                sample(T + 120, ServerState::Online, &ticks(100.0, 600.0, Some(0.0))),
                sample(T + 125, ServerState::Online, &ticks(100.0, 600.0, None)),
            ],
        );

        let data = read(
            &conn,
            Range::Day,
            &[Series::MainTick, Series::MainOverBudget],
            T + 125,
        )
        .unwrap();

        let bucket = usize::try_from((T - data.from) / 120).unwrap();
        assert_eq!(values(&data, Series::MainTick)[bucket], Some(9.0));
        assert_eq!(values(&data, Series::MainOverBudget)[bucket], Some(5.0));
        assert_eq!(values(&data, Series::MainTick)[bucket + 1], Some(6.0));
        assert_eq!(values(&data, Series::MainOverBudget)[bucket + 1], None);
    }

    #[test]
    fn a_week_weights_each_minute_by_the_samples_behind_it() {
        let mut conn = memory();
        let mut samples = vec![sample(T, ServerState::Online, &[(Column::Players, 100.0)])];
        samples.extend(
            (0..12).map(|slot| sample(T + 60 + slot * 5, ServerState::Online, &[(Column::Players, 10.0)])),
        );
        store(&mut conn, &samples);
        let tx = conn.transaction().unwrap();
        rollups::compute(&tx, Tier::Minute, &[T, T + 60]).unwrap();
        tx.commit().unwrap();

        let data = read(&conn, Range::Week, &[Series::Players], T + 200).unwrap();

        let bucket = usize::try_from((T - data.from) / 900).unwrap();
        let players = &data.series[0];
        let expected = (100.0 + 12.0 * 10.0) / 13.0;
        assert!((players.values[bucket].unwrap() - expected).abs() < 1e-9);
        assert_eq!(players.samples.as_ref().unwrap()[bucket], 13);
        assert_eq!(players.max.as_ref().unwrap()[bucket], Some(100.0));
        assert_eq!(
            data.last,
            Some(T + 60 + 55),
            "the newest closed minute's last slot"
        );
    }

    #[test]
    fn a_month_reads_the_hours_and_markers_come_with_the_answer() {
        let mut conn = memory();
        store(
            &mut conn,
            &[sample(T, ServerState::Online, &[(Column::HostCpu, 41.0)])],
        );
        let tx = conn.transaction().unwrap();
        rollups::compute(&tx, Tier::Hour, &[T]).unwrap();
        let run = runs::RunKey {
            observer_started: T * 1000,
            generation: 1,
        };
        runs::start(&tx, run, T * 1000, runs::StartCause::Requested).unwrap();
        tx.commit().unwrap();

        let data = read(&conn, Range::Month, &[Series::HostCpu], T + 7200).unwrap();

        assert_eq!(data.buckets, 720);
        let bucket = usize::try_from((T - data.from) / 3600).unwrap();
        assert_eq!(data.series[0].values[bucket], Some(41.0));
        assert_eq!(data.markers.len(), 1);
        assert_eq!(data.markers[0].kind, MarkerKind::Start);
    }

    #[test]
    fn every_bucket_counts_the_samples_taken_and_those_taken_online() {
        use ServerState::{Crashed, Degraded, Online, Starting, Stopped};
        let mut conn = memory();
        let mut samples = Vec::new();
        let mut slots = |offset: i64, states: &[ServerState]| {
            for (slot, &state) in (0..).zip(states) {
                samples.push(sample(T + offset + 5 * slot, state, &[]));
            }
        };
        slots(0, &[Online; 6]);
        slots(30, &[Starting, Starting, Online, Degraded, Online, Online]);
        slots(60, &[Stopped; 6]);
        slots(90, &[Crashed, Starting, Starting]);
        store(&mut conn, &samples);

        let data = read(&conn, Range::SixHours, &[Series::Players], T + 150).unwrap();

        let bucket = usize::try_from((T - data.from) / 30).unwrap();
        assert_eq!((data.observed.len(), data.online.len()), (720, 720));
        assert_eq!(data.observed[bucket..bucket + 5], [6, 6, 6, 3, 0]);
        assert_eq!(
            data.online[bucket..bucket + 5],
            [6, 4, 0, 0, 0],
            "down while stopped and starting"
        );
        assert_eq!(data.observed[bucket - 1], 0, "Observer was not running");
    }

    #[test]
    fn rolled_up_buckets_count_the_samples_behind_their_aggregates() {
        let mut conn = memory();
        let mut samples: Vec<_> = (0..12)
            .map(|slot| sample(T + slot * 5, ServerState::Online, &[]))
            .collect();
        samples.extend((0..6).map(|slot| sample(T + 60 + slot * 5, ServerState::Starting, &[])));
        store(&mut conn, &samples);
        let tx = conn.transaction().unwrap();
        rollups::compute(&tx, Tier::Minute, &[T, T + 60]).unwrap();
        rollups::compute(&tx, Tier::Hour, &[T]).unwrap();
        tx.commit().unwrap();

        let week = read(&conn, Range::Week, &[Series::Players], T + 200).unwrap();
        let month = read(&conn, Range::Month, &[Series::Players], T + 200).unwrap();

        let bucket = usize::try_from((T - week.from) / 900).unwrap();
        assert_eq!((week.observed[bucket], week.online[bucket]), (18, 12));
        let bucket = usize::try_from((T - month.from) / 3600).unwrap();
        assert_eq!((month.observed[bucket], month.online[bucket]), (18, 12));
    }

    #[test]
    fn the_deeper_series_read_their_own_columns() {
        let mut conn = memory();
        store(
            &mut conn,
            &[
                sample(
                    T,
                    ServerState::Online,
                    &[
                        (Column::FxsFaults, 1200.0),
                        (Column::ThrMainCpu, 50.0),
                        (Column::HostDiskMs, 0.5),
                    ],
                ),
                sample(
                    T + 5,
                    ServerState::Online,
                    &[
                        (Column::FxsFaults, 300.0),
                        (Column::ThrMainCpu, 70.0),
                        (Column::HostDiskOps, 9.0),
                    ],
                ),
            ],
        );

        let data = read(
            &conn,
            Range::SixHours,
            &[
                Series::FxsFaults,
                Series::MainThreadCpu,
                Series::HostDiskLatency,
                Series::HostDiskTransfers,
            ],
            T + 10,
        )
        .unwrap();

        let bucket = usize::try_from((T - data.from) / 30).unwrap();
        let at = |series| values(&data, series)[bucket];
        assert_eq!(at(Series::FxsFaults), Some(1500.0));
        assert_eq!(at(Series::MainThreadCpu), Some(60.0));
        assert_eq!(at(Series::HostDiskLatency), Some(0.5));
        assert_eq!(at(Series::HostDiskTransfers), Some(9.0));
    }

    #[test]
    fn a_live_value_is_computed_like_the_stored_series() {
        let mut values = super::super::Values::default();
        values.set(Column::SyncTicks, Some(200.0));
        values.set(Column::SyncTickMs, Some(300.0));
        values.set(Column::SyncOver, Some(20.0));

        assert_eq!(value(Series::SyncTick, &values), Some(1.5));
        assert_eq!(value(Series::SyncOverBudget, &values), Some(10.0));
        assert_eq!(value(Series::Players, &values), None);
    }
}

#[cfg(test)]
mod window_tests {
    use observer_domain::server::ServerState;

    use super::*;
    use crate::metrics::memory;
    use crate::metrics::rollups::{self, Tier};
    use crate::metrics::samples::Frames;
    use crate::metrics::samples::tests::{sample, store, with_frames};

    /// 2025-10-28T09:00:00Z, on every grid.
    const T: i64 = 1_761_642_000;
    const DAY: i64 = 86_400;

    fn plan(from: i64, to: i64, points: u32) -> Option<(i64, i64, Table)> {
        Window::plan(from, to, points, T).map(|window| (window.step, window.buckets, window.table))
    }

    #[test]
    fn a_window_takes_the_finest_step_and_the_finest_table_that_keeps_it() {
        assert_eq!(plan(T - 3600, T, 720), Some((5, 720, Table::Raw)));
        assert_eq!(plan(T - DAY, T, 720), Some((120, 720, Table::Raw)));
        assert_eq!(plan(T - 7 * DAY, T, 720), Some((900, 672, Table::Minute)));
        assert_eq!(plan(T - 30 * DAY, T, 720), Some((3600, 720, Table::Hour)));
        assert_eq!(plan(T - DAY, T, 48), Some((1800, 48, Table::Minute)));
        // An hour five days ago: the five-second samples are gone, the minutes are not.
        assert_eq!(
            plan(T - 5 * DAY - 3600, T - 5 * DAY, 720),
            Some((60, 60, Table::Minute))
        );
        assert_eq!(
            plan(T - 40 * DAY - 3600, T - 40 * DAY, 720),
            Some((3600, 1, Table::Hour))
        );
    }

    #[test]
    fn a_window_s_grid_covers_its_edges_on_multiples_of_its_step() {
        let window = Window::plan(T - 3600 + 7, T + 3, 720, T).unwrap();

        assert_eq!((window.from, window.step), (T - 3600 + 5, 5));
        assert_eq!(window.end(), T + 5);
    }

    #[test]
    fn a_window_too_short_too_long_reversed_or_in_the_future_is_refused() {
        assert_eq!(plan(T, T, 720), None);
        assert_eq!(plan(T, T - 3600, 720), None);
        assert_eq!(plan(T - 59, T, 720), None);
        assert_eq!(plan(T - 401 * DAY, T, 720), None);
        assert_eq!(plan(T, T + DAY + 1, 720), None);
        assert!(plan(T - 60, T, 720).is_some());
    }

    fn frames_of(conn: &Connection, from: i64, to: i64) -> FrameBands {
        let window = Window::plan(from, to, 720, T).unwrap();
        timeline(conn, &window, &[Series::MainTick])
            .unwrap()
            .frames
            .unwrap()
    }

    #[test]
    fn a_bucket_s_percentiles_come_from_its_ticks_interval_by_interval() {
        let mut conn = memory();
        store(
            &mut conn,
            &[
                // 98 ticks between 1 and 2 ms, 2 between 30 and 50.
                with_frames(sample(T - 3600, ServerState::Online, &[]), &[(1, 98), (9, 2)]),
                sample(T - 3595, ServerState::Online, &[]),
            ],
        );

        let frames = frames_of(&conn, T - 3600, T);

        assert!((frames.p50[0].unwrap() - (1.0 + 50.0 / 98.0)).abs() < 1e-9);
        assert_eq!(frames.p99[0], Some(40.0));
        assert_eq!(frames.ticks[0], 100);
        assert_eq!(
            (frames.p50[1], frames.ticks[1]),
            (None, 0),
            "no histogram in that slot"
        );
        assert_eq!((frames.floor, frames.top), (Some(1.0), Some(250.0)));
        assert_eq!(frames.since, Some(T - 3600));
    }

    #[test]
    fn a_bucket_with_two_sets_of_bounds_reads_the_one_with_most_ticks() {
        let mut conn = memory();
        let old = Frames {
            bounds: vec![5.0, 10.0, 25.0, 50.0, 75.0, 100.0, 250.0, 500.0],
            // 30 ticks between 25 and 50 ms.
            ticks: vec![0, 0, 0, 30, 0, 0, 0, 0, 0],
        };
        let mut before = sample(T - 120, ServerState::Online, &[]);
        before.frames = Some(old);
        store(
            &mut conn,
            &[
                before,
                with_frames(sample(T - 115, ServerState::Online, &[]), &[(2, 100)]),
            ],
        );

        let frames = frames_of(&conn, T - DAY, T);

        let last = frames.p50.len() - 1;
        assert_eq!(frames.ticks[last], 100);
        assert_eq!(
            frames.p50[last],
            Some(3.0),
            "between 2 and 4 ms, from the newer bounds"
        );
    }

    #[test]
    fn rolled_up_frames_read_the_same_percentiles_as_their_slots() {
        let mut conn = memory();
        let start = T - 10 * DAY;
        store(
            &mut conn,
            &[
                with_frames(sample(start, ServerState::Online, &[]), &[(2, 60)]),
                with_frames(sample(start + 5, ServerState::Online, &[]), &[(2, 40)]),
            ],
        );
        let tx = conn.transaction().unwrap();
        rollups::compute(&tx, Tier::Minute, &[start]).unwrap();
        tx.commit().unwrap();

        let frames = frames_of(&conn, start, start + 3600);

        assert_eq!(frames.ticks[0], 100);
        assert_eq!(frames.p50[0], Some(3.0));
    }

    #[test]
    fn the_usual_range_is_the_lowest_and_highest_of_the_same_hour_in_the_weeks_before() {
        let mut conn = memory();
        let week = 7 * DAY;
        let samples: Vec<_> = [(1, 40.0), (2, 55.0), (3, 48.0)]
            .into_iter()
            .flat_map(|(weeks, players)| {
                (0..12).map(move |slot| {
                    sample(
                        T - weeks * week + slot * 5,
                        ServerState::Online,
                        &[(Column::Players, players)],
                    )
                })
            })
            .collect();
        store(&mut conn, &samples);
        // A lone week elsewhere: one value is not a range.
        store(
            &mut conn,
            &[sample(
                T + 3600 - week,
                ServerState::Online,
                &[(Column::Players, 9.0)],
            )],
        );
        let hours: Vec<i64> = (1..=3)
            .map(|weeks| T - weeks * week)
            .chain([T + 3600 - week])
            .collect();
        let tx = conn.transaction().unwrap();
        rollups::compute(&tx, Tier::Hour, &hours).unwrap();
        tx.commit().unwrap();

        let window = Window::plan(T, T + 7200, 720, T).unwrap();
        let usual = timeline(&conn, &window, &[Series::Players])
            .unwrap()
            .usual
            .unwrap();

        assert_eq!((usual.low[0], usual.high[0]), (Some(40.0), Some(55.0)));
        assert_eq!(window.step, 10);
        assert_eq!(
            (usual.low[359], usual.high[359]),
            (Some(40.0), Some(55.0)),
            "the same hour"
        );
        assert_eq!(
            (usual.low[360], usual.high[360]),
            (None, None),
            "the next hour had one week"
        );
    }
}
