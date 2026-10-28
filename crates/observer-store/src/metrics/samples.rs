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
