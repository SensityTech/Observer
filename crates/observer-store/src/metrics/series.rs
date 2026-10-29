//! The series the panel charts, on the fixed grid of a range: ranges up to a day read the
//! five-second samples, a week the minute aggregates, a month the hour aggregates.

use std::collections::BTreeMap;

use observer_domain::metrics::{MetricsSeries, Range, Series, SeriesData};
use rusqlite::{Connection, params};

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

/// The range ending at `now` (Unix seconds).
pub fn read(
    conn: &Connection,
    range: Range,
    series: &[Series],
    now: i64,
) -> Result<MetricsSeries, StoreError> {
    let (from, end) = range.grid(now);
    let step = range.step();
    let mut columns: Vec<Column> = series
        .iter()
        .flat_map(|&series| source(series).columns())
        .collect();
    columns.sort_unstable();
    columns.dedup();

    let (table, slot_span, coverage, expressions) = match range {
        Range::Hour | Range::SixHours | Range::Day => {
            ("sample_5s", 5, RAW_COVERAGE, raw_expressions(&columns))
        }
        Range::Week => (
            "sample_1m",
            60,
            ROLLED_UP_COVERAGE,
            rolled_up_expressions(&columns),
        ),
        Range::Month => (
            "sample_1h",
            3600,
            ROLLED_UP_COVERAGE,
            rolled_up_expressions(&columns),
        ),
    };
    let rolled_up = table != "sample_5s";
    let sql = format!(
        "SELECT (ts - ?1) / ?2, max(ts), {coverage}, {} FROM {table} WHERE ts >= ?1 AND ts < ?3 GROUP BY 1",
        expressions.join(", ")
    );
    let buckets = usize::try_from(range.buckets()).unwrap_or_default();
    let mut grid: Vec<BTreeMap<Column, Aggregate>> = vec![BTreeMap::new(); buckets];
    let mut observed = vec![0; buckets];
    let mut online = vec![0; buckets];
    let mut newest: Option<i64> = None;
    let mut statement = conn.prepare_cached(&sql)?;
    let mut rows = statement.query(params![from, step, end])?;
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

    let data = series
        .iter()
        .map(|&series| series_data(series, &grid, range))
        .collect();
    Ok(MetricsSeries {
        range,
        step,
        from,
        buckets: range.buckets(),
        last: newest.map(|ts| ts + slot_span - 5),
        series: data,
        markers: runs::markers(conn, from, end)?,
        observed,
        online,
    })
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

fn series_data(series: Series, grid: &[BTreeMap<Column, Aggregate>], range: Range) -> SeriesData {
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
        && range != Range::Hour
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
