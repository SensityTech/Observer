//! Minute and hour aggregates, always computed from the five-second samples, never from another
//! tier, and the retention of each table.

use rusqlite::{Connection, Transaction, params};

use super::{Column, Kind};
use crate::database::StoreError;

/// Five-second samples are kept three days, minute aggregates thirty; hour aggregates and runs
/// are kept.
pub const RAW_KEPT: i64 = 72 * 3600;
pub const MINUTES_KEPT: i64 = 30 * 86_400;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Minute,
    Hour,
}

impl Tier {
    pub fn seconds(self) -> i64 {
        match self {
            Self::Minute => 60,
            Self::Hour => 3600,
        }
    }

    fn table(self) -> &'static str {
        match self {
            Self::Minute => "sample_1m",
            Self::Hour => "sample_1h",
        }
    }

    fn ticks(self) -> &'static str {
        match self {
            Self::Minute => "tick_1m",
            Self::Hour => "tick_1h",
        }
    }
}

/// The buckets that end at or before `closed` (Unix seconds), have samples, and have no
/// aggregate yet: a gap the upkeep did not cover, Observer having been down or the upkeep late.
pub fn missing(conn: &Connection, tier: Tier, closed: i64) -> Result<Vec<i64>, StoreError> {
    let size = tier.seconds();
    let sql = format!(
        "SELECT DISTINCT (ts / ?1) * ?1 AS bucket FROM sample_5s WHERE ts < ?2 \
         EXCEPT SELECT ts FROM {} ORDER BY bucket",
        tier.table()
    );
    let mut statement = conn.prepare_cached(&sql)?;
    let buckets = statement
        .query_map(params![size, closed - closed.rem_euclid(size)], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    Ok(buckets)
}

/// (Re)computes the aggregate of each bucket from the samples it holds. A bucket without samples
/// is left as it is.
pub fn compute(tx: &Transaction<'_>, tier: Tier, buckets: &[i64]) -> Result<(), StoreError> {
    let mut names = vec!["ts", "samples", "online"]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let mut expressions = vec![
        "?1".to_owned(),
        "count(*)".to_owned(),
        "sum(state IN ('online', 'degraded'))".to_owned(),
    ];
    for column in Column::ALL {
        let name = column.name();
        match column.kind() {
            // SQL's avg, min and max skip NULL: a gauge averages the samples that measured it.
            Kind::Gauge => {
                names.extend([
                    format!("{name}_avg"),
                    format!("{name}_min"),
                    format!("{name}_max"),
                ]);
                expressions.extend([
                    format!("avg({name})"),
                    format!("min({name})"),
                    format!("max({name})"),
                ]);
            }
            Kind::Count => {
                names.push(format!("{name}_sum"));
                expressions.push(format!("sum({name})"));
            }
        }
        names.push(format!("{name}_n"));
        expressions.push(format!("count({name})"));
    }
    let sql = format!(
        "INSERT OR REPLACE INTO {} ({}) SELECT {} FROM sample_5s WHERE ts >= ?1 AND ts < ?1 + ?2 \
         HAVING count(*) > 0",
        tier.table(),
        names.join(", "),
        expressions.join(", ")
    );
    let mut statement = tx.prepare_cached(&sql)?;
    let mut cleared = tx.prepare_cached(&format!("DELETE FROM {} WHERE ts = ?1", tier.ticks()))?;
    let mut ticks = tx.prepare_cached(&format!(
        "INSERT INTO {} (ts, bounds, bucket, ticks) SELECT ?1, bounds, bucket, sum(ticks) FROM tick_5s          WHERE ts >= ?1 AND ts < ?1 + ?2 GROUP BY bounds, bucket",
        tier.ticks()
    ))?;
    for &bucket in buckets {
        statement.execute(params![bucket, tier.seconds()])?;
        cleared.execute([bucket])?;
        ticks.execute(params![bucket, tier.seconds()])?;
    }
    Ok(())
}

/// Deletes the samples, their thread and tick rows and the minute aggregates past their
/// retention at `now`, then the thread names no row refers to any more, and returns how many
/// rows went.
pub fn expire(tx: &Transaction<'_>, now: i64) -> Result<usize, StoreError> {
    let raw = tx.execute("DELETE FROM sample_5s WHERE ts < ?1", [now - RAW_KEPT])?;
    let threads = tx.execute("DELETE FROM thread_5s WHERE ts < ?1", [now - RAW_KEPT])?;
    let ticks = tx.execute("DELETE FROM tick_5s WHERE ts < ?1", [now - RAW_KEPT])?
        + tx.execute("DELETE FROM tick_1m WHERE ts < ?1", [now - MINUTES_KEPT])?;
    let names = tx.execute(
        "DELETE FROM thread_name WHERE id NOT IN (SELECT name FROM thread_5s)",
        [],
    )?;
    let minutes = tx.execute("DELETE FROM sample_1m WHERE ts < ?1", [now - MINUTES_KEPT])?;
    Ok(raw + threads + ticks + names + minutes)
}

/// Gives the pages that deletions freed back to the file system.
pub fn vacuum(conn: &Connection) -> Result<(), StoreError> {
    // The pragma frees one page per step, and answers a row for each.
    let mut statement = conn.prepare("PRAGMA incremental_vacuum")?;
    let mut freed = statement.query([])?;
    while freed.next()?.is_some() {}
    Ok(())
}

pub fn optimize(conn: &Connection) -> Result<(), StoreError> {
    conn.execute_batch("PRAGMA optimize")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use observer_domain::server::ServerState;

    use super::*;
    use crate::metrics::memory;
    use crate::metrics::samples::tests::{sample, store, tick_rows, with_frames, with_threads};

    const MINUTE: i64 = 1_761_642_000;

    fn compute_all(conn: &mut Connection, tier: Tier, buckets: &[i64]) {
        let tx = conn.transaction().unwrap();
        compute(&tx, tier, buckets).unwrap();
        tx.commit().unwrap();
    }

    #[test]
    fn a_minute_keeps_average_minimum_maximum_and_count_of_measured_samples_and_sums_counts() {
        let mut conn = memory();
        store(
            &mut conn,
            &[
                sample(
                    MINUTE,
                    ServerState::Starting,
                    &[(Column::FxsCpu, 10.0), (Column::Joins, 2.0)],
                ),
                sample(MINUTE + 5, ServerState::Online, &[(Column::FxsCpu, 30.0)]),
                sample(
                    MINUTE + 10,
                    ServerState::Degraded,
                    &[(Column::FxsCpu, 20.0), (Column::Joins, 3.0)],
                ),
                sample(MINUTE + 15, ServerState::Online, &[]),
                sample(MINUTE + 60, ServerState::Online, &[(Column::FxsCpu, 99.0)]),
            ],
        );

        compute_all(&mut conn, Tier::Minute, &[MINUTE]);

        let row: (i64, i64, f64, f64, f64, i64, i64, i64, Option<f64>, i64) = conn
            .query_row(
                "SELECT samples, online, fxs_cpu_avg, fxs_cpu_min, fxs_cpu_max, fxs_cpu_n, joins_sum, joins_n, \
                 players_avg, players_n FROM sample_1m WHERE ts = ?1",
                [MINUTE],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                        row.get(8)?,
                        row.get(9)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(row, (4, 3, 20.0, 10.0, 30.0, 3, 5, 2, None, 0));
    }

    #[test]
    fn a_bucket_without_samples_gets_no_aggregate() {
        let mut conn = memory();

        compute_all(&mut conn, Tier::Hour, &[MINUTE]);

        let rows: i64 = conn
            .query_row("SELECT count(*) FROM sample_1h", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, 0);
    }

    #[test]
    fn closed_buckets_with_samples_and_no_aggregate_are_missing() {
        let mut conn = memory();
        let samples: Vec<_> = [0, 60, 125, 600, 3605]
            .into_iter()
            .map(|offset| sample(MINUTE + offset, ServerState::Online, &[]))
            .collect();
        store(&mut conn, &samples);
        compute_all(&mut conn, Tier::Minute, &[MINUTE + 60]);

        // The minute of `MINUTE + 3605` is still open at `MINUTE + 3630`.
        assert_eq!(
            missing(&conn, Tier::Minute, MINUTE + 3630).unwrap(),
            [MINUTE, MINUTE + 120, MINUTE + 600]
        );
        assert_eq!(
            missing(&conn, Tier::Hour, MINUTE + 3630).unwrap(),
            [MINUTE - MINUTE % 3600]
        );
    }

    #[test]
    fn a_late_sample_is_absorbed_when_its_minute_is_computed_again() {
        let mut conn = memory();
        store(
            &mut conn,
            &[sample(MINUTE, ServerState::Online, &[(Column::Players, 4.0)])],
        );
        compute_all(&mut conn, Tier::Minute, &[MINUTE]);
        store(
            &mut conn,
            &[sample(
                MINUTE + 55,
                ServerState::Online,
                &[(Column::Players, 8.0)],
            )],
        );

        compute_all(&mut conn, Tier::Minute, &[MINUTE]);

        let (samples, average): (i64, f64) = conn
            .query_row("SELECT samples, players_avg FROM sample_1m", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert_eq!((samples, average), (2, 6.0));
    }

    #[test]
    fn retention_drops_old_samples_and_minutes_and_keeps_hours() {
        let mut conn = memory();
        let now = MINUTE + 40 * 86_400;
        store(
            &mut conn,
            &[
                sample(now - RAW_KEPT - 5, ServerState::Online, &[]),
                sample(now - RAW_KEPT, ServerState::Online, &[]),
                sample(now - 31 * 86_400, ServerState::Online, &[]),
            ],
        );
        compute_all(
            &mut conn,
            Tier::Minute,
            &[now - 31 * 86_400 - (now - 31 * 86_400) % 60],
        );
        compute_all(
            &mut conn,
            Tier::Hour,
            &[now - 31 * 86_400 - (now - 31 * 86_400) % 3600],
        );

        let tx = conn.transaction().unwrap();
        assert_eq!(expire(&tx, now).unwrap(), 3);
        tx.commit().unwrap();

        let count = |table: &str| -> i64 {
            conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| row.get(0))
                .unwrap()
        };
        assert_eq!(
            (count("sample_5s"), count("sample_1m"), count("sample_1h")),
            (1, 0, 1)
        );
        vacuum(&conn).unwrap();
        optimize(&conn).unwrap();
    }

    #[test]
    fn a_minute_and_an_hour_add_up_their_slots_ticks_interval_by_interval() {
        let mut conn = memory();
        store(
            &mut conn,
            &[
                with_frames(sample(MINUTE, ServerState::Online, &[]), &[(1, 98), (9, 2)]),
                with_frames(sample(MINUTE + 5, ServerState::Online, &[]), &[(1, 100)]),
                with_frames(sample(MINUTE + 60, ServerState::Online, &[]), &[(2, 100)]),
            ],
        );

        compute_all(&mut conn, Tier::Minute, &[MINUTE, MINUTE + 60]);
        compute_all(&mut conn, Tier::Hour, &[MINUTE - MINUTE % 3600]);

        assert_eq!(
            tick_rows(&conn, "tick_1m"),
            [(MINUTE, 1, 198), (MINUTE, 9, 2), (MINUTE + 60, 2, 100)]
        );
        let hour = MINUTE - MINUTE % 3600;
        assert_eq!(
            tick_rows(&conn, "tick_1h"),
            [(hour, 1, 198), (hour, 2, 100), (hour, 9, 2)]
        );

        // A slot written again with other ticks: the minute computed again follows it.
        store(
            &mut conn,
            &[with_frames(
                sample(MINUTE + 5, ServerState::Online, &[]),
                &[(3, 100)],
            )],
        );
        compute_all(&mut conn, Tier::Minute, &[MINUTE]);
        assert_eq!(
            tick_rows(&conn, "tick_1m"),
            [
                (MINUTE, 1, 98),
                (MINUTE, 3, 100),
                (MINUTE, 9, 2),
                (MINUTE + 60, 2, 100)
            ]
        );
    }

    #[test]
    fn retention_drops_tick_rows_with_their_tier_and_keeps_hours() {
        let mut conn = memory();
        let now = MINUTE + 40 * 86_400;
        let month_ago = now - 31 * 86_400;
        store(
            &mut conn,
            &[
                with_frames(sample(now - RAW_KEPT - 5, ServerState::Online, &[]), &[(1, 100)]),
                with_frames(sample(month_ago, ServerState::Online, &[]), &[(1, 100)]),
                with_frames(sample(now - 5, ServerState::Online, &[]), &[(1, 100)]),
            ],
        );
        compute_all(&mut conn, Tier::Minute, &[month_ago - month_ago % 60]);
        compute_all(&mut conn, Tier::Hour, &[month_ago - month_ago % 3600]);

        let tx = conn.transaction().unwrap();
        expire(&tx, now).unwrap();
        tx.commit().unwrap();

        assert_eq!(tick_rows(&conn, "tick_5s"), [(now - 5, 1, 100)]);
        assert_eq!(tick_rows(&conn, "tick_1m"), []);
        assert_eq!(tick_rows(&conn, "tick_1h").len(), 1);
    }

    #[test]
    fn retention_drops_old_thread_rows_and_the_names_only_they_used() {
        let mut conn = memory();
        let now = MINUTE + 40 * 86_400;
        store(
            &mut conn,
            &[
                with_threads(
                    sample(now - RAW_KEPT - 5, ServerState::Online, &[]),
                    &[("UV loop: svMain", 50.0), ("DevCon Thread", 0.1)],
                ),
                with_threads(
                    sample(now - 5, ServerState::Online, &[]),
                    &[("UV loop: svMain", 52.0)],
                ),
            ],
        );

        let tx = conn.transaction().unwrap();
        assert_eq!(expire(&tx, now).unwrap(), 4, "a sample, two thread rows, a name");
        tx.commit().unwrap();

        let names: Vec<String> = conn
            .prepare("SELECT name FROM thread_name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(names, ["UV loop: svMain"]);
        let rows: i64 = conn
            .query_row("SELECT count(*) FROM thread_5s", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, 1);
    }

    #[test]
    fn the_vacuum_gives_back_every_page_the_retention_freed() {
        let mut conn = memory();
        let now = MINUTE + 40 * 86_400;
        let old: Vec<_> = (0..2000)
            .map(|index| {
                sample(
                    now - RAW_KEPT - 5 * (index + 1),
                    ServerState::Online,
                    &[
                        (Column::Players, 187.0),
                        (Column::FxsCpu, 182.4),
                        (Column::HostCpu, 41.0),
                    ],
                )
            })
            .collect();
        store(&mut conn, &old);
        let pages = |conn: &Connection| -> (i64, i64) {
            let read = |pragma: &str| conn.pragma_query_value(None, pragma, |row| row.get(0)).unwrap();
            (read("page_count"), read("freelist_count"))
        };
        let tx = conn.transaction().unwrap();
        expire(&tx, now).unwrap();
        tx.commit().unwrap();
        let (before, freed) = pages(&conn);
        assert!(freed > 10, "{freed} pages freed");

        vacuum(&conn).unwrap();

        assert_eq!(pages(&conn), (before - freed, 0));
    }
}
