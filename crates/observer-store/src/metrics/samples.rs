use observer_domain::server::ServerState;
use rusqlite::types::Value;
use rusqlite::{Transaction, params, params_from_iter};

use super::{Affinity, Column, Values};
use crate::database::StoreError;

/// One slot's measures, stored under the slot's start in Unix seconds.
#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    pub ts: i64,
    pub state: ServerState,
    pub values: Values,
    /// The slot's busiest thread names of FXServer's main process, each name once, with its
    /// threads' CPU together in percent of one core.
    pub threads: Vec<(String, f64)>,
    /// svMain's ticks in the slot, interval by interval.
    pub frames: Option<Frames>,
}

/// Ticks within each interval of a set of bounds, in milliseconds: one interval more than the
/// bounds, the last beyond the last bound.
#[derive(Debug, Clone, PartialEq)]
pub struct Frames {
    pub bounds: Vec<f64>,
    pub ticks: Vec<u64>,
}

impl Frames {
    /// The bounds as the `tick_bounds` table spells them.
    pub fn key(bounds: &[f64]) -> String {
        serde_json::to_string(bounds).unwrap_or_default()
    }
}

/// Writes the sample and its thread rows, replacing what was already stored for its slot: the
/// clock went back.
pub fn insert(tx: &Transaction<'_>, sample: &Sample) -> Result<(), StoreError> {
    let names: Vec<&str> = Column::ALL.iter().map(|column| column.name()).collect();
    let placeholders: Vec<String> = (1..=names.len() + 2).map(|index| format!("?{index}")).collect();
    let sql = format!(
        "INSERT OR REPLACE INTO sample_5s (ts, state, {}) VALUES ({})",
        names.join(", "),
        placeholders.join(", ")
    );
    let mut values = vec![
        Value::Integer(sample.ts),
        Value::Text(state_name(sample.state).to_owned()),
    ];
    values.extend(
        Column::ALL.map(|column| match (sample.values.get(column), column.affinity()) {
            (None, _) => Value::Null,
            (Some(value), Affinity::Real) => Value::Real(value),
            #[allow(clippy::cast_possible_truncation)]
            (Some(value), Affinity::Integer) => Value::Integer(value.round() as i64),
        }),
    );
    tx.prepare_cached(&sql)?.execute(params_from_iter(values))?;

    tx.prepare_cached("DELETE FROM thread_5s WHERE ts = ?1")?
        .execute([sample.ts])?;
    let mut name =
        tx.prepare_cached("INSERT INTO thread_name (name) VALUES (?1) ON CONFLICT (name) DO NOTHING")?;
    let mut row = tx.prepare_cached(
        "INSERT INTO thread_5s (ts, name, cpu) SELECT ?1, id, ?3 FROM thread_name WHERE name = ?2",
    )?;
    for (thread, cpu) in &sample.threads {
        name.execute([thread])?;
        row.execute(params![sample.ts, thread, cpu])?;
    }

    tx.prepare_cached("DELETE FROM tick_5s WHERE ts = ?1")?
        .execute([sample.ts])?;
    if let Some(frames) = &sample.frames {
        let bounds = Frames::key(&frames.bounds);
        tx.prepare_cached("INSERT INTO tick_bounds (bounds) VALUES (?1) ON CONFLICT (bounds) DO NOTHING")?
            .execute([&bounds])?;
        let mut interval = tx.prepare_cached(
            "INSERT INTO tick_5s (ts, bounds, bucket, ticks) SELECT ?1, id, ?3, ?4 FROM tick_bounds WHERE bounds = ?2",
        )?;
        for (bucket, &ticks) in frames.ticks.iter().enumerate() {
            if ticks > 0 {
                interval.execute(params![
                    sample.ts,
                    bounds,
                    i64::try_from(bucket).unwrap_or(i64::MAX),
                    i64::try_from(ticks).unwrap_or(i64::MAX)
                ])?;
            }
        }
    }
    Ok(())
}

/// The state as the `state` column spells it.
pub fn state_name(state: ServerState) -> &'static str {
    match state {
        ServerState::Stopped => "stopped",
        ServerState::Starting => "starting",
        ServerState::Online => "online",
        ServerState::Degraded => "degraded",
        ServerState::Hung => "hung",
        ServerState::Stopping => "stopping",
        ServerState::Crashed => "crashed",
        ServerState::CrashLoop => "crash_loop",
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use rusqlite::Connection;

    use super::*;
    use crate::metrics::memory;

    pub(crate) fn sample(ts: i64, state: ServerState, values: &[(Column, f64)]) -> Sample {
        let mut sample = Sample {
            ts,
            state,
            values: Values::default(),
            threads: Vec::new(),
            frames: None,
        };
        for &(column, value) in values {
            sample.values.set(column, Some(value));
        }
        sample
    }

    pub(crate) fn store(conn: &mut Connection, samples: &[Sample]) {
        let tx = conn.transaction().unwrap();
        for sample in samples {
            insert(&tx, sample).unwrap();
        }
        tx.commit().unwrap();
    }

    #[test]
    fn a_sample_keeps_integers_reals_and_unmeasured_columns_apart() {
        let mut conn = memory();
        store(
            &mut conn,
            &[sample(
                1_761_642_000,
                ServerState::CrashLoop,
                &[
                    (Column::Players, 187.0),
                    (Column::FxsCpu, 182.5),
                    (Column::PingAvgMs, 41.25),
                ],
            )],
        );

        let row: (String, i64, f64, f64, Option<i64>) = conn
            .query_row(
                "SELECT state, players, fxs_cpu, ping_avg_ms, slots FROM sample_5s WHERE ts = 1761642000",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .unwrap();
        assert_eq!(row, ("crash_loop".to_owned(), 187, 182.5, 41.25, None));
    }

    pub(crate) fn with_threads(mut sample: Sample, threads: &[(&str, f64)]) -> Sample {
        sample.threads = threads
            .iter()
            .map(|&(name, cpu)| (name.to_owned(), cpu))
            .collect();
        sample
    }

    fn thread_rows(conn: &Connection) -> Vec<(i64, String, f64)> {
        let mut statement = conn
            .prepare(
                "SELECT t.ts, n.name, t.cpu FROM thread_5s t JOIN thread_name n ON n.id = t.name \
                 ORDER BY t.ts, n.name",
            )
            .unwrap();
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    #[test]
    fn a_sample_s_threads_are_stored_under_names_kept_once() {
        let mut conn = memory();
        store(
            &mut conn,
            &[
                with_threads(
                    sample(100, ServerState::Online, &[(Column::ThrMainCpu, 58.0)]),
                    &[("UV loop: svMain", 58.0), ("libuv-worker", 1.5)],
                ),
                with_threads(
                    sample(105, ServerState::Online, &[]),
                    &[("UV loop: svMain", 61.0)],
                ),
            ],
        );

        assert_eq!(
            thread_rows(&conn),
            [
                (100, "UV loop: svMain".to_owned(), 58.0),
                (100, "libuv-worker".to_owned(), 1.5),
                (105, "UV loop: svMain".to_owned(), 61.0),
            ]
        );
        let names: i64 = conn
            .query_row("SELECT count(*) FROM thread_name", [], |row| row.get(0))
            .unwrap();
        assert_eq!(names, 2);
    }

    #[test]
    fn a_slot_written_again_replaces_its_thread_rows() {
        let mut conn = memory();
        store(
            &mut conn,
            &[with_threads(
                sample(100, ServerState::Online, &[]),
                &[("UV loop: svMain", 58.0), ("libuv-worker", 1.5)],
            )],
        );

        store(
            &mut conn,
            &[with_threads(
                sample(100, ServerState::Online, &[]),
                &[("UV loop: svSync", 7.0)],
            )],
        );

        assert_eq!(thread_rows(&conn), [(100, "UV loop: svSync".to_owned(), 7.0)]);
    }

    /// The bounds of FXServer since mid-2024, in milliseconds.
    pub(crate) const BOUNDS: [f64; 14] = [
        1.0, 2.0, 4.0, 6.0, 8.0, 10.0, 15.0, 20.0, 30.0, 50.0, 70.0, 100.0, 150.0, 250.0,
    ];

    /// A sample whose svMain ticks fell into the intervals given.
    pub(crate) fn with_frames(mut sample: Sample, intervals: &[(usize, u64)]) -> Sample {
        let mut ticks = vec![0; BOUNDS.len() + 1];
        for &(index, count) in intervals {
            ticks[index] = count;
        }
        sample.frames = Some(Frames {
            bounds: BOUNDS.to_vec(),
            ticks,
        });
        sample
    }

    pub(crate) fn tick_rows(conn: &Connection, table: &str) -> Vec<(i64, i64, i64)> {
        let mut statement = conn
            .prepare(&format!(
                "SELECT ts, bucket, ticks FROM {table} ORDER BY ts, bucket"
            ))
            .unwrap();
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    #[test]
    fn a_slot_s_frames_keep_the_intervals_that_had_ticks_and_replace_them_when_written_again() {
        let mut conn = memory();
        store(
            &mut conn,
            &[
                with_frames(sample(100, ServerState::Online, &[]), &[(1, 98), (9, 2)]),
                sample(105, ServerState::Online, &[]),
            ],
        );
        assert_eq!(tick_rows(&conn, "tick_5s"), [(100, 1, 98), (100, 9, 2)]);

        store(
            &mut conn,
            &[with_frames(sample(100, ServerState::Online, &[]), &[(2, 100)])],
        );

        assert_eq!(tick_rows(&conn, "tick_5s"), [(100, 2, 100)]);
        let bounds: (i64, String) = conn
            .query_row("SELECT count(*), max(bounds) FROM tick_bounds", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert_eq!(
            bounds,
            (
                1,
                "[1.0,2.0,4.0,6.0,8.0,10.0,15.0,20.0,30.0,50.0,70.0,100.0,150.0,250.0]".to_owned()
            )
        );
    }

    #[test]
    fn a_slot_written_again_is_replaced() {
        let mut conn = memory();
        store(
            &mut conn,
            &[sample(100, ServerState::Online, &[(Column::Players, 3.0)])],
        );
        store(&mut conn, &[sample(100, ServerState::Stopped, &[])]);

        let (count, players): (i64, Option<i64>) = conn
            .query_row("SELECT count(*), max(players) FROM sample_5s", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert_eq!((count, players), (1, None));
    }
}
