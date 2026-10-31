//! Players' sessions, one row from the join to the drop. The players service is the only writer.

use jiff::Timestamp;
use observer_domain::drops::{DropCategory, DropKind};
use observer_domain::players::PlayerDrop;
use rusqlite::types::Type;
use rusqlite::{Connection, Row, Transaction, params};

use super::runs::RunKey;
use crate::database::StoreError;

/// The id the next session takes. The players service numbers its sessions itself, so that it can
/// refer to one before it is written.
pub fn next_id(conn: &Connection) -> Result<i64, StoreError> {
    Ok(
        conn.query_row("SELECT coalesce(max(id), 0) + 1 FROM player_session", [], |row| {
            row.get(0)
        })?,
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSession {
    pub id: i64,
    pub run: RunKey,
    pub net_id: u32,
    pub joined_ms: Option<i64>,
    pub seen_ms: i64,
}

/// Writes an open session. `false` while its run is not written yet.
pub fn insert(tx: &Transaction<'_>, session: &NewSession) -> Result<bool, StoreError> {
    let inserted = tx.execute(
        "INSERT INTO player_session (id, run, net_id, joined_ms, seen_ms)
         SELECT ?1, id, ?4, ?5, ?6 FROM server_run WHERE observer_started = ?2 AND generation = ?3",
        params![
            session.id,
            session.run.observer_started,
            session.run.generation,
            session.net_id,
            session.joined_ms,
            session.seen_ms,
        ],
    )?;
    Ok(inserted == 1)
}

/// Gives a session its player's pseudonym and the key of its crediting record.
pub fn attach(
    tx: &Transaction<'_>,
    id: i64,
    pseudonym: [u8; 16],
    credit: [u8; 16],
) -> Result<(), StoreError> {
    tx.execute(
        "UPDATE player_session SET player = ?2, credit = ?3 WHERE id = ?1",
        params![id, pseudonym, credit],
    )?;
    Ok(())
}

/// How a session ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Closure {
    pub id: i64,
    pub ended_ms: i64,
    pub length_ms: Option<i64>,
    pub category: DropCategory,
    pub kind: Option<DropKind>,
    pub native: Option<i64>,
    pub resource: Option<String>,
    /// Redacted, at most 512 bytes.
    pub crash_text: Option<String>,
    pub reason_text: Option<String>,
    /// Whether the player was still loading, when FXServer could tell.
    pub loading: Option<bool>,
    pub synthesized: bool,
}

/// Closes an open session.
pub fn close(tx: &Transaction<'_>, closure: &Closure) -> Result<(), StoreError> {
    let resource = resource_id(tx, closure.resource.as_deref())?;
    tx.execute(
        "UPDATE player_session SET ended_ms = ?2, length_ms = ?3, category = ?4, kind = ?5, native_category = ?6,
             resource = ?7, crash_text = ?8, reason_text = ?9, phase = ?10, synthesized = ?11
         WHERE id = ?1 AND ended_ms IS NULL",
        params![
            closure.id,
            closure.ended_ms,
            closure.length_ms.map(|length| length.max(0)),
            closure.category.as_str(),
            closure.kind.map(DropKind::as_str),
            closure.native,
            resource,
            closure.crash_text,
            closure.reason_text,
            closure.loading.map(phase),
            closure.synthesized,
        ],
    )?;
    Ok(())
}

/// Gives a session Observer closed as missed the drop FXServer reported for it afterwards: its
/// end and length stay, since its play time was credited with them.
pub fn reclassify(tx: &Transaction<'_>, closure: &Closure) -> Result<(), StoreError> {
    let resource = resource_id(tx, closure.resource.as_deref())?;
    tx.execute(
        "UPDATE player_session SET category = ?2, kind = ?3, native_category = ?4, resource = ?5, crash_text = ?6,
             reason_text = ?7, phase = ?8, synthesized = 0
         WHERE id = ?1 AND ended_ms IS NOT NULL",
        params![
            closure.id,
            closure.category.as_str(),
            closure.kind.map(DropKind::as_str),
            closure.native,
            resource,
            closure.crash_text,
            closure.reason_text,
            closure.loading.map(phase),
        ],
    )?;
    Ok(())
}

fn phase(loading: bool) -> &'static str {
    if loading { "loading" } else { "inGame" }
}

fn resource_id(tx: &Transaction<'_>, name: Option<&str>) -> Result<Option<i64>, StoreError> {
    let Some(name) = name else {
        return Ok(None);
    };
    Ok(Some(tx.query_row(
        "INSERT INTO resource_name (name) VALUES (?1)
         ON CONFLICT (name) DO UPDATE SET name = excluded.name RETURNING id",
        [name],
        |row| row.get(0),
    )?))
}

/// A session an earlier Observer left open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeftOpen {
    pub id: i64,
    pub credit: Option<[u8; 16]>,
    pub seen_ms: i64,
    /// Its run's end, once the run was closed as lost.
    pub run_ended_ms: Option<i64>,
}

pub fn left_open(conn: &Connection) -> Result<Vec<LeftOpen>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT s.id, s.credit, s.seen_ms, r.ended_ms FROM player_session s JOIN server_run r ON r.id = s.run
         WHERE s.ended_ms IS NULL",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(LeftOpen {
            id: row.get(0)?,
            credit: row.get(1)?,
            seen_ms: row.get(2)?,
            run_ended_ms: row.get(3)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// When a session an earlier Observer left open ends: at its run's end, never before it was seen
/// and never after `now`, which a clock set back could put the run's last sample past.
pub fn lost_end(session: &LeftOpen, now: i64) -> i64 {
    session
        .run_ended_ms
        .unwrap_or(session.seen_ms)
        .min(now)
        .max(session.seen_ms)
}

/// The player's last sessions that ended, newest first.
pub fn player_drops(
    conn: &Connection,
    pseudonym: [u8; 16],
    limit: u32,
) -> Result<Vec<PlayerDrop>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT s.ended_ms, s.category, s.kind, r.name, s.length_ms FROM player_session s
         LEFT JOIN resource_name r ON r.id = s.resource
         WHERE s.player = ?1 AND s.ended_ms IS NOT NULL ORDER BY s.ended_ms DESC, s.id DESC LIMIT ?2",
    )?;
    let rows = statement.query_map(params![pseudonym, limit], player_drop)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn player_drop(row: &Row<'_>) -> rusqlite::Result<PlayerDrop> {
    let ended_ms: i64 = row.get(0)?;
    let category: String = row.get(1)?;
    let kind: Option<String> = row.get(2)?;
    let invalid =
        |index, what: String| rusqlite::Error::FromSqlConversionFailure(index, Type::Text, what.into());
    Ok(PlayerDrop {
        ended_at: Timestamp::from_millisecond(ended_ms).map_err(|error| invalid(0, error.to_string()))?,
        category: DropCategory::from_name(&category).ok_or_else(|| invalid(1, category.clone()))?,
        kind: kind.as_deref().and_then(DropKind::from_name),
        resource: row.get(3)?,
        length_ms: row.get(4)?,
    })
}

/// Crashes per 100 hours of time connected over the sessions that ended from `since_ms`.
pub fn crash_rate(conn: &Connection, since_ms: i64) -> Result<Option<f64>, StoreError> {
    let (crashes, played): (i64, Option<i64>) = conn.query_row(
        "SELECT count(*) FILTER (WHERE category = 'crash'), sum(length_ms) FROM player_session
         WHERE ended_ms >= ?1 AND category NOT IN ('serverStop', 'serverCrash')",
        [since_ms],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok(played
        .filter(|&ms| ms > 0)
        .map(|ms| per_hundred_hours(crashes, ms)))
}

#[expect(
    clippy::cast_precision_loss,
    reason = "counts and milliseconds far below 2^52"
)]
pub fn per_hundred_hours(count: i64, ms: i64) -> f64 {
    count as f64 * 360_000_000.0 / ms as f64
}

/// Deletes the sessions that ended before `before_ms`.
pub fn expire(tx: &Transaction<'_>, before_ms: i64) -> Result<usize, StoreError> {
    Ok(tx.execute(
        "DELETE FROM player_session WHERE ended_ms IS NOT NULL AND ended_ms < ?1",
        [before_ms],
    )?)
}

/// Takes a forgotten player's pseudonym and texts off their sessions: they no longer belong
/// together, and say nothing the player wrote.
pub fn forget(tx: &Transaction<'_>, pseudonym: [u8; 16]) -> Result<usize, StoreError> {
    tx.pragma_update(None, "secure_delete", true)?;
    let blanked = tx.execute(
        "UPDATE player_session SET player = NULL, credit = NULL, crash_text = NULL, reason_text = NULL
         WHERE player = ?1",
        [pseudonym],
    )?;
    tx.pragma_update(None, "secure_delete", "FAST")?;
    Ok(blanked)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::memory;
    use crate::metrics::runs::{self, StartCause};

    const RUN: RunKey = RunKey {
        observer_started: 1_761_900_000_000,
        generation: 1,
    };

    fn with_run(conn: &mut Connection) {
        let tx = conn.transaction().unwrap();
        runs::start(&tx, RUN, RUN.observer_started, StartCause::Requested).unwrap();
        tx.commit().unwrap();
    }

    fn open(tx: &Transaction<'_>, id: i64, net_id: u32) -> bool {
        insert(
            tx,
            &NewSession {
                id,
                run: RUN,
                net_id,
                joined_ms: Some(RUN.observer_started + 1000),
                seen_ms: RUN.observer_started + 1000,
            },
        )
        .unwrap()
    }

    fn closure(id: i64, category: DropCategory, length_ms: i64) -> Closure {
        Closure {
            id,
            ended_ms: RUN.observer_started + 1000 + length_ms,
            length_ms: Some(length_ms),
            category,
            kind: None,
            native: Some(2),
            resource: None,
            crash_text: Some("ERR_GFX_D3D_INIT".to_owned()),
            reason_text: None,
            loading: Some(false),
            synthesized: false,
        }
    }

    #[test]
    fn a_session_waits_for_its_run() {
        let mut conn = memory();
        let tx = conn.transaction().unwrap();

        assert!(!open(&tx, 1, 7));
        tx.commit().unwrap();
        with_run(&mut conn);
        let tx = conn.transaction().unwrap();
        assert!(open(&tx, 1, 7));
        tx.commit().unwrap();
        assert_eq!(next_id(&conn).unwrap(), 2);
    }

    #[test]
    fn only_an_open_session_is_unique_per_net_id() {
        let mut conn = memory();
        with_run(&mut conn);
        let tx = conn.transaction().unwrap();

        assert!(open(&tx, 1, 7));
        close(&tx, &closure(1, DropCategory::Quit, 10)).unwrap();
        assert!(open(&tx, 2, 7));
        assert!(
            insert(
                &tx,
                &NewSession {
                    id: 3,
                    run: RUN,
                    net_id: 7,
                    joined_ms: None,
                    seen_ms: 0
                }
            )
            .is_err()
        );
    }

    #[test]
    fn a_crash_rate_counts_time_connected_of_the_players_own_sessions() {
        let mut conn = memory();
        with_run(&mut conn);
        let tx = conn.transaction().unwrap();
        for (id, category) in [
            (1, DropCategory::Crash),
            (2, DropCategory::Quit),
            (3, DropCategory::Crash),
            (4, DropCategory::ServerStop),
        ] {
            open(&tx, id, u32::try_from(id).unwrap());
            close(&tx, &closure(id, category, 3_600_000)).unwrap();
        }
        tx.commit().unwrap();

        let rate = crash_rate(&conn, 0).unwrap().unwrap();

        assert!((rate - 66.666).abs() < 0.01, "{rate}");
        assert_eq!(crash_rate(&conn, i64::MAX).unwrap(), None);
    }

    #[test]
    fn a_forgotten_players_sessions_lose_their_pseudonym_and_texts() {
        let mut conn = memory();
        with_run(&mut conn);
        let tx = conn.transaction().unwrap();
        open(&tx, 1, 7);
        attach(&tx, 1, [9; 16], [8; 16]).unwrap();
        close(&tx, &closure(1, DropCategory::Crash, 1000)).unwrap();

        assert_eq!(player_drops(&tx, [9; 16], 10).unwrap().len(), 1);
        assert_eq!(forget(&tx, [9; 16]).unwrap(), 1);

        assert_eq!(player_drops(&tx, [9; 16], 10).unwrap(), []);
        let text: Option<String> = tx
            .query_row("SELECT crash_text FROM player_session WHERE id = 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(text, None);
    }

    #[test]
    fn a_missed_session_takes_the_drop_reported_after_it_but_keeps_its_end() {
        let mut conn = memory();
        with_run(&mut conn);
        let tx = conn.transaction().unwrap();
        open(&tx, 1, 7);
        close(
            &tx,
            &Closure {
                kind: Some(DropKind::Missed),
                synthesized: true,
                crash_text: None,
                ..closure(1, DropCategory::Unknown, 5000)
            },
        )
        .unwrap();

        reclassify(
            &tx,
            &Closure {
                resource: Some("anticheat".to_owned()),
                kind: Some(DropKind::Resource),
                ..closure(1, DropCategory::Kick, 9000)
            },
        )
        .unwrap();

        let (category, length, synthesized, resource): (String, i64, bool, String) = tx
            .query_row(
                "SELECT s.category, s.length_ms, s.synthesized, r.name FROM player_session s
                 JOIN resource_name r ON r.id = s.resource",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(
            (category.as_str(), length, synthesized, resource.as_str()),
            ("kick", 5000, false, "anticheat")
        );
    }

    #[test]
    fn sessions_left_open_end_with_their_run() {
        let mut conn = memory();
        with_run(&mut conn);
        let tx = conn.transaction().unwrap();
        open(&tx, 1, 7);
        runs::close_lost(&tx).unwrap();

        let left = left_open(&tx).unwrap();

        assert_eq!(left.len(), 1);
        assert_eq!(lost_end(&left[0], i64::MAX), RUN.observer_started + 1000);
        assert_eq!(lost_end(&left[0], 0), RUN.observer_started + 1000);
    }
}
