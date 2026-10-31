//! FXServer's runs, from its start to its end, which the charts mark.

use observer_domain::metrics::{MarkerKind, RunMarker};
use rusqlite::{Connection, Transaction, params};

use crate::database::StoreError;

/// A run as the Observer that started it knows it: generations start again at 1 with each one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunKey {
    /// When that Observer's metrics started, in Unix milliseconds.
    pub observer_started: i64,
    pub generation: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartCause {
    Requested,
    Restart,
    Recovery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndCause {
    Stopped,
    Crashed,
    Hung,
    /// Observer stopped without seeing the end: it was killed, or the host went down.
    Lost,
}

impl StartCause {
    fn name(self) -> &'static str {
        match self {
            Self::Requested => "requested",
            Self::Restart => "restart",
            Self::Recovery => "recovery",
        }
    }
}

impl EndCause {
    fn name(self) -> &'static str {
        match self {
            Self::Stopped => "stopped",
            Self::Crashed => "crashed",
            Self::Hung => "hung",
            Self::Lost => "lost",
        }
    }
}

pub fn start(
    tx: &Transaction<'_>,
    run: RunKey,
    started_ms: i64,
    cause: StartCause,
) -> Result<(), StoreError> {
    tx.execute(
        "INSERT OR IGNORE INTO server_run (observer_started, generation, started_ms, start_cause) \
         VALUES (?1, ?2, ?3, ?4)",
        params![run.observer_started, run.generation, started_ms, cause.name()],
    )?;
    Ok(())
}

pub fn ready(tx: &Transaction<'_>, run: RunKey, ready_ms: i64) -> Result<(), StoreError> {
    tx.execute(
        "UPDATE server_run SET ready_ms = ?3 WHERE observer_started = ?1 AND generation = ?2 AND ready_ms IS NULL",
        params![run.observer_started, run.generation, ready_ms],
    )?;
    Ok(())
}

pub fn end(tx: &Transaction<'_>, run: RunKey, ended_ms: i64, cause: EndCause) -> Result<(), StoreError> {
    tx.execute(
        "UPDATE server_run SET ended_ms = ?3, end_cause = ?4 WHERE observer_started = ?1 AND generation = ?2",
        params![run.observer_started, run.generation, ended_ms, cause.name()],
    )?;
    Ok(())
}

/// A crash the supervisor then put down to a hang.
pub fn hung(tx: &Transaction<'_>, run: RunKey) -> Result<(), StoreError> {
    tx.execute(
        "UPDATE server_run SET end_cause = 'hung' WHERE observer_started = ?1 AND generation = ?2 \
         AND end_cause = 'crashed'",
        params![run.observer_started, run.generation],
    )?;
    Ok(())
}

/// When FXServer's process started: the state database's crashes know the run by it.
pub fn spawned(tx: &Transaction<'_>, run: RunKey, spawned_ms: i64) -> Result<(), StoreError> {
    tx.execute(
        "UPDATE server_run SET spawned_ms = ?3 WHERE observer_started = ?1 AND generation = ?2",
        params![run.observer_started, run.generation, spawned_ms],
    )?;
    Ok(())
}

pub fn build(tx: &Transaction<'_>, run: RunKey, build: u32) -> Result<(), StoreError> {
    tx.execute(
        "UPDATE server_run SET build = ?3 WHERE observer_started = ?1 AND generation = ?2",
        params![run.observer_started, run.generation, build],
    )?;
    Ok(())
}

/// Ends the runs an earlier Observer left open, at their last sample, and returns how many.
pub fn close_lost(tx: &Transaction<'_>) -> Result<usize, StoreError> {
    Ok(tx.execute(
        "UPDATE server_run SET end_cause = 'lost', ended_ms = max(started_ms, \
         coalesce((SELECT max(ts) * 1000 FROM sample_5s), started_ms)) WHERE ended_ms IS NULL",
        [],
    )?)
}

struct Run {
    key: RunKey,
    started_ms: i64,
    ready_ms: Option<i64>,
    ended_ms: Option<i64>,
    start_cause: String,
    end_cause: Option<String>,
}

/// The markers between `from` and `end` (Unix seconds). A crash and the automatic restart after
/// it are one marker, which says when the server was back online; a restart after a crash before
/// the range shows as a restart.
pub fn markers(conn: &Connection, from: i64, end: i64) -> Result<Vec<RunMarker>, StoreError> {
    let (from_ms, end_ms) = (from * 1000, end * 1000);
    let mut statement = conn.prepare_cached(
        "SELECT observer_started, generation, started_ms, ready_ms, ended_ms, start_cause, end_cause \
         FROM server_run WHERE (started_ms >= ?1 AND started_ms < ?2) OR (ended_ms >= ?1 AND ended_ms < ?2) \
         ORDER BY observer_started, generation",
    )?;
    let runs: Vec<Run> = statement
        .query_map(params![from_ms, end_ms], |row| {
            Ok(Run {
                key: RunKey {
                    observer_started: row.get(0)?,
                    generation: row.get(1)?,
                },
                started_ms: row.get(2)?,
                ready_ms: row.get(3)?,
                ended_ms: row.get(4)?,
                start_cause: row.get(5)?,
                end_cause: row.get(6)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    let within = |ms: i64| (from_ms..end_ms).contains(&ms);
    let crashed = |run: &Run| {
        matches!(run.end_cause.as_deref(), Some("crashed" | "hung")) && run.ended_ms.is_some_and(within)
    };
    let mut markers = Vec::new();
    for (index, run) in runs.iter().enumerate() {
        let previous = index
            .checked_sub(1)
            .and_then(|previous| beside(&runs, index, previous));
        if within(run.started_ms) {
            let kind = match run.start_cause.as_str() {
                "requested" => Some(MarkerKind::Start),
                "restart" => Some(MarkerKind::Restart),
                // Drawn with the crash it follows, unless that crash is outside the range.
                _ if previous.is_some_and(crashed) => None,
                _ => Some(MarkerKind::Restart),
            };
            if let Some(kind) = kind {
                markers.push(RunMarker {
                    at: run.started_ms / 1000,
                    kind,
                    back_online_at: None,
                });
            }
        }
        if crashed(run) {
            let back = beside(&runs, index, index + 1)
                .filter(|next| next.start_cause == "recovery")
                .and_then(|next| next.ready_ms);
            markers.push(RunMarker {
                at: run.ended_ms.unwrap_or_default() / 1000,
                kind: MarkerKind::Crash,
                back_online_at: back.map(|ms| ms / 1000),
            });
        }
    }
    markers.sort_by_key(|marker| marker.at);
    Ok(markers)
}

/// The run at `other` when it is the same Observer's as the one at `index`. The runs come in the
/// order of their generations, which a spawn that failed used up without starting a run, so the
/// run next to another is not always one generation away.
fn beside(runs: &[Run], index: usize, other: usize) -> Option<&Run> {
    runs.get(other)
        .filter(|other| other.key.observer_started == runs[index].key.observer_started)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::memory;

    const OBSERVER: i64 = 1_761_600_000_000;
    const T: i64 = 1_761_642_000;

    fn key(generation: u32) -> RunKey {
        RunKey {
            observer_started: OBSERVER,
            generation,
        }
    }

    fn with_tx(conn: &mut Connection, write: impl FnOnce(&Transaction<'_>)) {
        let tx = conn.transaction().unwrap();
        write(&tx);
        tx.commit().unwrap();
    }

    #[test]
    fn a_crash_and_its_recovery_are_one_marker_that_says_when_the_server_came_back() {
        let mut conn = memory();
        with_tx(&mut conn, |tx| {
            start(tx, key(1), T * 1000, StartCause::Requested).unwrap();
            ready(tx, key(1), (T + 30) * 1000).unwrap();
            end(tx, key(1), (T + 600) * 1000, EndCause::Crashed).unwrap();
            start(tx, key(2), (T + 605) * 1000, StartCause::Recovery).unwrap();
            ready(tx, key(2), (T + 720) * 1000).unwrap();
            end(tx, key(2), (T + 900) * 1000, EndCause::Stopped).unwrap();
            start(tx, key(3), (T + 1000) * 1000, StartCause::Restart).unwrap();
        });

        assert_eq!(
            markers(&conn, T - 60, T + 3600).unwrap(),
            [
                RunMarker {
                    at: T,
                    kind: MarkerKind::Start,
                    back_online_at: None
                },
                RunMarker {
                    at: T + 600,
                    kind: MarkerKind::Crash,
                    back_online_at: Some(T + 720)
                },
                RunMarker {
                    at: T + 1000,
                    kind: MarkerKind::Restart,
                    back_online_at: None
                },
            ]
        );
    }

    #[test]
    fn a_recovery_whose_first_spawn_failed_is_still_drawn_with_its_crash() {
        let mut conn = memory();
        // Generation 2 was the spawn that failed: it never started a run.
        with_tx(&mut conn, |tx| {
            start(tx, key(1), T * 1000, StartCause::Requested).unwrap();
            ready(tx, key(1), (T + 30) * 1000).unwrap();
            end(tx, key(1), (T + 600) * 1000, EndCause::Crashed).unwrap();
            start(tx, key(3), (T + 640) * 1000, StartCause::Recovery).unwrap();
            ready(tx, key(3), (T + 700) * 1000).unwrap();
        });

        assert_eq!(
            markers(&conn, T - 60, T + 3600).unwrap(),
            [
                RunMarker {
                    at: T,
                    kind: MarkerKind::Start,
                    back_online_at: None
                },
                RunMarker {
                    at: T + 600,
                    kind: MarkerKind::Crash,
                    back_online_at: Some(T + 700)
                },
            ]
        );
    }

    #[test]
    fn a_hang_is_a_crash_and_a_recovery_after_a_crash_before_the_range_is_a_restart() {
        let mut conn = memory();
        with_tx(&mut conn, |tx| {
            start(tx, key(1), T * 1000, StartCause::Requested).unwrap();
            end(tx, key(1), (T + 100) * 1000, EndCause::Crashed).unwrap();
            hung(tx, key(1)).unwrap();
            start(tx, key(2), (T + 110) * 1000, StartCause::Recovery).unwrap();
        });

        let hung_cause: String = conn
            .query_row(
                "SELECT end_cause FROM server_run WHERE generation = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(hung_cause, "hung");
        assert_eq!(
            markers(&conn, T + 105, T + 3600).unwrap(),
            [RunMarker {
                at: T + 110,
                kind: MarkerKind::Restart,
                back_online_at: None
            }]
        );
        assert_eq!(
            markers(&conn, T + 50, T + 3600).unwrap()[0].kind,
            MarkerKind::Crash
        );
    }

    #[test]
    fn runs_left_open_are_closed_as_lost_at_the_last_sample() {
        let mut conn = memory();
        with_tx(&mut conn, |tx| {
            start(tx, key(1), T * 1000, StartCause::Requested).unwrap();
            start(tx, key(2), (T + 50) * 1000, StartCause::Restart).unwrap();
            end(tx, key(1), (T + 40) * 1000, EndCause::Stopped).unwrap();
            build(tx, key(2), 35245).unwrap();
        });
        conn.execute(
            "INSERT INTO sample_5s (ts, state) VALUES (?1, 'online')",
            [T + 300],
        )
        .unwrap();

        with_tx(&mut conn, |tx| assert_eq!(close_lost(tx).unwrap(), 1));

        let lost: (i64, String) = conn
            .query_row(
                "SELECT ended_ms, end_cause FROM server_run WHERE generation = 2",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(lost, ((T + 300) * 1000, "lost".to_owned()));
        assert!(
            markers(&conn, T - 10, T + 3600)
                .unwrap()
                .iter()
                .all(|marker| marker.kind != MarkerKind::Crash),
            "a lost run is no crash"
        );
    }
}
