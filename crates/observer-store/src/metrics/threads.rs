//! CPU per thread name of FXServer's main process over the last hour, from the slots' busiest
//! names.

use observer_domain::metrics::{SLOT_MS, ThreadGroup, ThreadShare, ThreadShares};
use rusqlite::{Connection, params};

use crate::database::StoreError;

/// Names under this share of one core, in percent, are left out: of a slot by the sampler, of the
/// hour here.
pub const SHOWN_FROM: f64 = 0.05;
/// The most names a slot keeps, and the hour shows.
pub const MOST_NAMES: usize = 24;
const SPAN: i64 = 3600;

/// The hour of slots ending with the newest sample at or before `now` (Unix seconds). A name's
/// share is its rows' total over the slots whose threads were measured: a slot without a row for
/// it saw it idle.
pub fn read(conn: &Connection, now: i64) -> Result<ThreadShares, StoreError> {
    let slot = SLOT_MS / 1000;
    let newest: Option<i64> =
        conn.query_row("SELECT max(ts) FROM sample_5s WHERE ts <= ?1", [now], |row| {
            row.get(0)
        })?;
    let to = newest.map_or_else(|| (now.div_euclid(slot) + 1) * slot, |ts| ts + slot);
    let from = to - SPAN;
    let (measured, process): (i64, Option<f64>) = conn.query_row(
        "SELECT count(*), avg(fxs_cpu) FROM sample_5s WHERE ts >= ?1 AND ts < ?2 AND thr_other_cpu IS NOT NULL",
        params![from, to],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let measured = u32::try_from(measured).unwrap_or(u32::MAX);
    let mut threads = Vec::new();
    if measured > 0 {
        let mut statement = conn.prepare_cached(
            "SELECT n.name, sum(t.cpu) FROM thread_5s t JOIN thread_name n ON n.id = t.name \
             WHERE t.ts >= ?1 AND t.ts < ?2 GROUP BY n.name",
        )?;
        let rows = statement.query_map(params![from, to], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
        })?;
        for row in rows {
            let (name, total) = row?;
            let cpu = total / f64::from(measured);
            if cpu >= SHOWN_FROM {
                threads.push(ThreadShare {
                    group: ThreadGroup::of(&name),
                    name,
                    cpu,
                });
            }
        }
        threads.sort_by(|a, b| b.cpu.total_cmp(&a.cpu).then_with(|| a.name.cmp(&b.name)));
        threads.truncate(MOST_NAMES);
    }
    Ok(ThreadShares {
        from,
        to,
        measured,
        process: process.filter(|_| measured > 0),
        threads,
    })
}

#[cfg(test)]
mod tests {
    use observer_domain::server::ServerState;

    use super::*;
    use crate::metrics::Column;
    use crate::metrics::memory;
    use crate::metrics::samples::tests::{sample, store, with_threads};

    const T: i64 = 1_761_642_000;

    fn measured(ts: i64, fxs_cpu: f64, threads: &[(&str, f64)]) -> crate::metrics::samples::Sample {
        with_threads(
            sample(
                ts,
                ServerState::Online,
                &[(Column::FxsCpu, fxs_cpu), (Column::ThrOtherCpu, 0.0)],
            ),
            threads,
        )
    }

    #[test]
    fn a_name_s_share_is_its_total_over_the_measured_slots_of_the_last_hour() {
        let mut conn = memory();
        store(
            &mut conn,
            &[
                // Before the hour.
                measured(T - 3600, 400.0, &[("UV loop: svMain", 400.0)]),
                measured(T, 60.0, &[("UV loop: svMain", 50.0), ("libuv-worker", 0.75)]),
                measured(T + 5, 90.0, &[("UV loop: svMain", 60.0), ("DevCon Thread", 0.09)]),
                // FXServer's first listing: its threads were not measured.
                sample(T + 10, ServerState::Starting, &[(Column::FxsCpu, 500.0)]),
                measured(
                    T + 15,
                    120.0,
                    &[("UV loop: svMain", 70.0), ("UV loop: svNetw", 9.0)],
                ),
            ],
        );

        let shares = read(&conn, T + 3000).unwrap();

        assert_eq!((shares.from, shares.to), (T + 20 - 3600, T + 20));
        assert_eq!(shares.measured, 3);
        assert_eq!(shares.process, Some(90.0));
        let rows: Vec<(&str, ThreadGroup, f64)> = shares
            .threads
            .iter()
            .map(|thread| (thread.name.as_str(), thread.group, thread.cpu))
            .collect();
        assert_eq!(
            rows,
            [
                ("UV loop: svMain", ThreadGroup::Main, 60.0),
                ("UV loop: svNetw", ThreadGroup::Network, 3.0),
                ("libuv-worker", ThreadGroup::Other, 0.25),
            ],
            "DevCon Thread averages 0.03%"
        );
    }

    #[test]
    fn at_most_twenty_four_names_are_kept_the_busiest_first_then_by_name() {
        let mut conn = memory();
        let names: Vec<String> = (0..30).map(|index| format!("worker {index:02}")).collect();
        let threads: Vec<(&str, f64)> = names
            .iter()
            .zip(0..)
            .map(|(name, index)| (name.as_str(), if index < 10 { 1.0 } else { f64::from(index) }))
            .collect();
        store(&mut conn, &[measured(T, 100.0, &threads)]);

        let shares = read(&conn, T).unwrap();

        assert_eq!(shares.threads.len(), 24);
        assert_eq!(shares.threads[0].name, "worker 29");
        assert_eq!(shares.threads[19].name, "worker 10");
        assert_eq!(shares.threads[20].name, "worker 00", "ties by name");
        assert_eq!(shares.threads[23].name, "worker 03");
    }

    #[test]
    fn without_a_measured_slot_there_is_no_name_and_no_process_share() {
        let mut conn = memory();

        let empty = read(&conn, T + 2).unwrap();
        assert_eq!((empty.from, empty.to, empty.measured), (T + 5 - 3600, T + 5, 0));
        assert_eq!((empty.process, empty.threads.len()), (None, 0));

        store(
            &mut conn,
            &[sample(T, ServerState::Stopped, &[(Column::FxsCpu, 3.0)])],
        );
        let stopped = read(&conn, T + 2).unwrap();
        assert_eq!((stopped.measured, stopped.process), (0, None));
    }
}
