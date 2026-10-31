//! Snapshots: one per start of FXServer that was scanned, and the latest check of the disk, with
//! what holds for one of their files only.

use jiff::Timestamp;
use observer_domain::changes::{FileFlag, PartialReason, SnapshotKind, SnapshotState, StartCause};
use observer_domain::server_data::Game;
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde_json::Value;

use super::{json, name, parse, sql, unsigned};
use crate::database::StoreError;
use crate::time::Utc;

/// A snapshot as its job starts it.
#[derive(Debug, Clone, PartialEq)]
pub struct NewSnapshot {
    pub server: String,
    pub kind: SnapshotKind,
    pub run_started: Option<Timestamp>,
    pub generation: Option<u32>,
    pub cause: Option<StartCause>,
    pub by: Option<String>,
    pub build: Option<u32>,
    pub build_sha256: Option<String>,
    pub game: Option<Game>,
    pub game_build: Option<u32>,
    pub launch: Value,
    pub cfg: Value,
    pub baseline: bool,
    pub taken_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub id: i64,
    pub server: String,
    pub kind: SnapshotKind,
    pub run_started: Option<Timestamp>,
    pub generation: Option<u32>,
    pub cause: Option<StartCause>,
    pub by: Option<String>,
    pub build: Option<u32>,
    pub build_sha256: Option<String>,
    pub game: Option<Game>,
    pub game_build: Option<u32>,
    pub launch: Value,
    pub cfg: Value,
    pub started: Option<Vec<String>>,
    pub same_as: Option<i64>,
    pub baseline: bool,
    pub state: SnapshotState,
    pub partial_reason: Option<PartialReason>,
    pub failure: Option<String>,
    pub files: Option<u64>,
    pub bytes: Option<u64>,
    pub taken_at: Timestamp,
    pub listed_at: Option<Timestamp>,
    pub hashed_at: Option<Timestamp>,
}

impl Snapshot {
    /// The snapshot whose rows list this one's resources.
    pub fn holder(&self) -> i64 {
        self.same_as.unwrap_or(self.id)
    }

    /// Whether its job is over.
    pub fn settled(&self) -> bool {
        !matches!(self.state, SnapshotState::Listing | SnapshotState::Hashing)
    }
}

const COLUMNS: &str = "id, server_id, kind, run_started, generation, start_cause, started_by, build, \
    build_sha256, game, game_build, launch, cfg, started, same_as, baseline, state, partial_reason, \
    failure, files, bytes, taken_at, listed_at, hashed_at";

pub fn insert(conn: &Connection, snapshot: &NewSnapshot) -> Result<i64, StoreError> {
    conn.execute(
        "INSERT INTO snapshot (server_id, kind, run_started, generation, start_cause, started_by, build,
             build_sha256, game, game_build, launch, cfg, baseline, state, taken_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, 'listing', ?14)",
        params![
            snapshot.server,
            name(&snapshot.kind),
            snapshot.run_started.map(Utc),
            snapshot.generation,
            snapshot.cause.as_ref().map(name),
            snapshot.by,
            snapshot.build,
            snapshot.build_sha256,
            snapshot.game.as_ref().map(name),
            snapshot.game_build,
            snapshot.launch.to_string(),
            snapshot.cfg.to_string(),
            snapshot.baseline,
            Utc(snapshot.taken_at),
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// The listing is written: the counts, and the state the hashing pass starts from.
pub fn listed(
    conn: &Connection,
    id: i64,
    files: u64,
    bytes: u64,
    state: SnapshotState,
    at: Timestamp,
) -> Result<(), StoreError> {
    conn.execute(
        "UPDATE snapshot SET files = ?2, bytes = ?3, state = ?4, listed_at = ?5 WHERE id = ?1",
        params![id, sql(files), sql(bytes), name(&state), Utc(at)],
    )?;
    Ok(())
}

/// The job ended: `Complete`, or `Partial` with its reason.
pub fn finish(
    conn: &Connection,
    id: i64,
    state: SnapshotState,
    reason: Option<PartialReason>,
    at: Timestamp,
) -> Result<(), StoreError> {
    conn.execute(
        "UPDATE snapshot SET state = ?2, partial_reason = ?3, hashed_at = ?4 WHERE id = ?1",
        params![id, name(&state), reason.as_ref().map(name), Utc(at)],
    )?;
    Ok(())
}

pub fn fail(conn: &Connection, id: i64, failure: &str, at: Timestamp) -> Result<(), StoreError> {
    conn.execute(
        "UPDATE snapshot SET state = 'failed', failure = ?2, hashed_at = ?3 WHERE id = ?1",
        params![id, failure, Utc(at)],
    )?;
    Ok(())
}

/// What the listing read of the cfg exec chain and the game it sets.
pub fn set_chain(
    conn: &Connection,
    id: i64,
    cfg: &Value,
    game: Option<Game>,
    game_build: Option<u32>,
) -> Result<(), StoreError> {
    conn.execute(
        "UPDATE snapshot SET cfg = ?2, game = ?3, game_build = coalesce(game_build, ?4) WHERE id = ?1",
        params![id, cfg.to_string(), game.as_ref().map(name), game_build],
    )?;
    Ok(())
}

/// The game build `/info.json` reported once the server answered, which wins over the cfg's.
pub fn set_game_build(conn: &Connection, id: i64, game_build: u32) -> Result<(), StoreError> {
    conn.execute(
        "UPDATE snapshot SET game_build = ?2 WHERE id = ?1",
        params![id, game_build],
    )?;
    Ok(())
}

/// The build the server reported, for a build folder Observer did not install.
pub fn set_build(conn: &Connection, id: i64, build: u32) -> Result<(), StoreError> {
    conn.execute(
        "UPDATE snapshot SET build = ?2 WHERE id = ?1 AND build IS NULL",
        params![id, build],
    )?;
    Ok(())
}

pub fn set_started(conn: &Connection, id: i64, started: &[String]) -> Result<(), StoreError> {
    conn.execute(
        "UPDATE snapshot SET started = ?2 WHERE id = ?1 AND started IS NULL",
        params![id, serde_json::to_string(started).unwrap_or_default()],
    )?;
    Ok(())
}

/// Its resource rows all equal those `holder` holds: they go, and it points there.
pub fn share_rows(conn: &Connection, id: i64, holder: i64) -> Result<(), StoreError> {
    conn.execute("DELETE FROM snapshot_resource WHERE snapshot_id = ?1", [id])?;
    conn.execute(
        "UPDATE snapshot SET same_as = ?2 WHERE id = ?1",
        params![id, holder],
    )?;
    Ok(())
}

pub fn get(conn: &Connection, id: i64) -> Result<Option<Snapshot>, StoreError> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM snapshot WHERE id = ?1"),
            [id],
            read,
        )
        .optional()?)
}

/// Starts of `server`, newest first, before the start `before` when given.
pub fn starts(
    conn: &Connection,
    server: &str,
    before: Option<i64>,
    limit: u32,
) -> Result<Vec<Snapshot>, StoreError> {
    let mut statement = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM snapshot
         WHERE server_id = ?1 AND kind = 'start' AND (?2 IS NULL OR id < ?2)
         ORDER BY id DESC LIMIT ?3"
    ))?;
    let rows = statement.query_map(params![server, before, limit], read)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// The newest complete start before `before`, the one a new start's rows are compared with.
pub fn previous_complete(
    conn: &Connection,
    server: &str,
    before: i64,
) -> Result<Option<Snapshot>, StoreError> {
    Ok(conn
        .query_row(
            &format!(
                "SELECT {COLUMNS} FROM snapshot
                 WHERE server_id = ?1 AND kind = 'start' AND state = 'complete' AND id < ?2
                 ORDER BY id DESC LIMIT 1"
            ),
            params![server, before],
            read,
        )
        .optional()?)
}

/// The start a start is compared with when nothing else is asked: the newest listed one before
/// it, one whose files were all read if there is any. A failed start, or one cut short before its
/// listing ended, has no rows to compare.
pub fn reference(conn: &Connection, server: &str, before: i64) -> Result<Option<Snapshot>, StoreError> {
    Ok(conn
        .query_row(
            &format!(
                "SELECT {COLUMNS} FROM snapshot
                 WHERE server_id = ?1 AND kind = 'start' AND id < ?2
                   AND state <> 'failed' AND listed_at IS NOT NULL
                 ORDER BY state = 'complete' OR (state = 'partial' AND partial_reason <> 'cancelled') DESC,
                          id DESC
                 LIMIT 1"
            ),
            params![server, before],
            read,
        )
        .optional()?)
}

/// The start that ran when a check was taken at `at`: the newest listed one taken before it.
pub fn ran_at(conn: &Connection, server: &str, at: Timestamp) -> Result<Option<Snapshot>, StoreError> {
    Ok(conn
        .query_row(
            &format!(
                "SELECT {COLUMNS} FROM snapshot
                 WHERE server_id = ?1 AND kind = 'start' AND taken_at <= ?2
                   AND state <> 'failed' AND listed_at IS NOT NULL
                 ORDER BY id DESC LIMIT 1"
            ),
            params![server, Utc(at)],
            read,
        )
        .optional()?)
}

/// The latest check of the disk, kept until the next one replaces it.
pub fn latest_check(conn: &Connection, server: &str) -> Result<Option<Snapshot>, StoreError> {
    Ok(conn
        .query_row(
            &format!(
                "SELECT {COLUMNS} FROM snapshot WHERE server_id = ?1 AND kind = 'check'
                 ORDER BY id DESC LIMIT 1"
            ),
            [server],
            read,
        )
        .optional()?)
}

/// Whether any start of `server` was listed: the first one is a baseline.
pub fn has_starts(conn: &Connection, server: &str) -> Result<bool, StoreError> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM snapshot
             WHERE server_id = ?1 AND kind = 'start' AND state <> 'failed' AND listed_at IS NOT NULL
             LIMIT 1",
            [server],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// Snapshots whose job an earlier Observer left running end as partial.
pub fn interrupted(conn: &Connection, at: Timestamp) -> Result<usize, StoreError> {
    Ok(conn.execute(
        "UPDATE snapshot SET state = 'partial', partial_reason = 'cancelled', hashed_at = ?1
         WHERE state IN ('listing', 'hashing')",
        [Utc(at)],
    )?)
}

pub fn flag(conn: &Connection, id: i64, path: &str, flag: FileFlag) -> Result<(), StoreError> {
    conn.execute(
        "INSERT OR IGNORE INTO snapshot_flag (snapshot_id, path, flag) VALUES (?1, ?2, ?3)",
        params![id, path, name(&flag)],
    )?;
    Ok(())
}

/// The files of the resource at `resource` modified after the start were reloaded by a restart of
/// the resource while the server ran.
pub fn reloaded(conn: &Connection, id: i64, resource: &str) -> Result<usize, StoreError> {
    Ok(conn.execute(
        "INSERT OR IGNORE INTO snapshot_flag (snapshot_id, path, flag)
         SELECT snapshot_id, path, 'reloadedLive' FROM snapshot_flag
         WHERE snapshot_id = ?1 AND flag = 'afterStart' AND substr(path, 1, length(?2) + 1) = ?2 || '/'",
        params![id, resource],
    )?)
}

pub fn flags(conn: &Connection, id: i64) -> Result<Vec<(String, FileFlag)>, StoreError> {
    let mut statement =
        conn.prepare("SELECT path, flag FROM snapshot_flag WHERE snapshot_id = ?1 ORDER BY path, flag")?;
    let rows = statement.query_map([id], |row| {
        Ok((row.get(0)?, parse(1, &row.get::<_, String>(1)?)?))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn read(row: &Row<'_>) -> rusqlite::Result<Snapshot> {
    let optional = |index: usize, row: &Row<'_>| -> rusqlite::Result<Option<String>> { row.get(index) };
    Ok(Snapshot {
        id: row.get(0)?,
        server: row.get(1)?,
        kind: parse(2, &row.get::<_, String>(2)?)?,
        run_started: row.get::<_, Option<Utc>>(3)?.map(|at| at.0),
        generation: row.get(4)?,
        cause: optional(5, row)?.map(|cause| parse(5, &cause)).transpose()?,
        by: row.get(6)?,
        build: row.get(7)?,
        build_sha256: row.get(8)?,
        game: optional(9, row)?.map(|game| parse(9, &game)).transpose()?,
        game_build: row.get(10)?,
        launch: json(11, row.get(11)?)?.unwrap_or(Value::Null),
        cfg: json(12, row.get(12)?)?.unwrap_or(Value::Null),
        started: json(13, row.get(13)?)?,
        same_as: row.get(14)?,
        baseline: row.get(15)?,
        state: parse(16, &row.get::<_, String>(16)?)?,
        partial_reason: optional(17, row)?.map(|reason| parse(17, &reason)).transpose()?,
        failure: row.get(18)?,
        files: row.get::<_, Option<i64>>(19)?.map(unsigned),
        bytes: row.get::<_, Option<i64>>(20)?.map(unsigned),
        taken_at: row.get::<_, Utc>(21)?.0,
        listed_at: row.get::<_, Option<Utc>>(22)?.map(|at| at.0),
        hashed_at: row.get::<_, Option<Utc>>(23)?.map(|at| at.0),
    })
}

#[cfg(test)]
pub(crate) fn sample(server: &str, minutes: i64) -> NewSnapshot {
    NewSnapshot {
        server: server.to_owned(),
        kind: SnapshotKind::Start,
        run_started: Some(Timestamp::from_second(1_793_000_000 + minutes * 60).unwrap()),
        generation: Some(1),
        cause: Some(StartCause::Requested),
        by: Some("ada".to_owned()),
        build: Some(26100),
        build_sha256: None,
        game: Some(Game::Gta5),
        game_build: None,
        launch: serde_json::json!({ "onesync": "on" }),
        cfg: serde_json::json!({ "files": [] }),
        baseline: false,
        taken_at: Timestamp::from_second(1_793_000_000 + minutes * 60).unwrap(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::changes::memory;

    #[test]
    fn a_snapshot_reads_back_as_it_was_written() {
        let conn = memory();
        let id = insert(&conn, &sample("default", 0)).unwrap();
        let at = Timestamp::from_second(1_793_000_100).unwrap();
        listed(&conn, id, 12, 3400, SnapshotState::Hashing, at).unwrap();
        set_started(&conn, id, &["chat".to_owned()]).unwrap();
        set_game_build(&conn, id, 3095).unwrap();
        finish(&conn, id, SnapshotState::Partial, Some(PartialReason::Limit), at).unwrap();

        let snapshot = get(&conn, id).unwrap().unwrap();

        assert_eq!(snapshot.kind, SnapshotKind::Start);
        assert_eq!(snapshot.cause, Some(StartCause::Requested));
        assert_eq!(snapshot.game, Some(Game::Gta5));
        assert_eq!(snapshot.game_build, Some(3095));
        assert_eq!(snapshot.files, Some(12));
        assert_eq!(snapshot.started.as_deref(), Some(&["chat".to_owned()][..]));
        assert_eq!(snapshot.state, SnapshotState::Partial);
        assert_eq!(snapshot.partial_reason, Some(PartialReason::Limit));
        assert_eq!(snapshot.holder(), id);
    }

    #[test]
    fn starts_list_newest_first_and_the_previous_complete_one_is_found() {
        let conn = memory();
        let first = insert(&conn, &sample("default", 0)).unwrap();
        finish(&conn, first, SnapshotState::Complete, None, Timestamp::now()).unwrap();
        let second = insert(&conn, &sample("default", 1)).unwrap();
        finish(
            &conn,
            second,
            SnapshotState::Partial,
            Some(PartialReason::Cancelled),
            Timestamp::now(),
        )
        .unwrap();
        let third = insert(&conn, &sample("default", 2)).unwrap();

        let ids: Vec<i64> = starts(&conn, "default", None, 10)
            .unwrap()
            .iter()
            .map(|s| s.id)
            .collect();

        assert_eq!(ids, [third, second, first]);
        assert_eq!(starts(&conn, "default", Some(third), 1).unwrap()[0].id, second);
        assert_eq!(
            previous_complete(&conn, "default", third).unwrap().unwrap().id,
            first
        );
        assert_eq!(interrupted(&conn, Timestamp::now()).unwrap(), 1);
    }

    #[test]
    fn a_start_compares_with_the_newest_listed_one_whose_files_were_all_read() {
        let conn = memory();
        let at = Timestamp::from_second(1_793_000_000).unwrap();
        assert!(!has_starts(&conn, "default").unwrap());
        let read = insert(&conn, &sample("default", 0)).unwrap();
        listed(&conn, read, 1, 1, SnapshotState::Hashing, at).unwrap();
        finish(
            &conn,
            read,
            SnapshotState::Partial,
            Some(PartialReason::Limit),
            at,
        )
        .unwrap();
        let cut = insert(&conn, &sample("default", 1)).unwrap();
        listed(&conn, cut, 1, 1, SnapshotState::Hashing, at).unwrap();
        finish(
            &conn,
            cut,
            SnapshotState::Partial,
            Some(PartialReason::Cancelled),
            at,
        )
        .unwrap();
        let unlisted = insert(&conn, &sample("default", 2)).unwrap();
        finish(
            &conn,
            unlisted,
            SnapshotState::Partial,
            Some(PartialReason::Cancelled),
            at,
        )
        .unwrap();
        let failed = insert(&conn, &sample("default", 3)).unwrap();
        fail(&conn, failed, "no resources folder", at).unwrap();
        let latest = insert(&conn, &sample("default", 4)).unwrap();

        assert_eq!(reference(&conn, "default", latest).unwrap().unwrap().id, read);
        assert_eq!(reference(&conn, "default", read).unwrap(), None);
        let check_at = Timestamp::from_second(1_793_000_000 + 150).unwrap();
        assert_eq!(ran_at(&conn, "default", check_at).unwrap().unwrap().id, cut);
        assert!(has_starts(&conn, "default").unwrap());
    }

    #[test]
    fn a_start_that_failed_or_was_never_listed_leaves_the_next_a_baseline() {
        let conn = memory();
        let at = Timestamp::from_second(1_793_000_000).unwrap();
        let failed = insert(&conn, &sample("default", 0)).unwrap();
        fail(&conn, failed, "no resources folder", at).unwrap();
        let unlisted = insert(&conn, &sample("default", 1)).unwrap();
        finish(
            &conn,
            unlisted,
            SnapshotState::Partial,
            Some(PartialReason::Cancelled),
            at,
        )
        .unwrap();

        assert!(!has_starts(&conn, "default").unwrap());
    }

    #[test]
    fn rows_shared_with_an_earlier_snapshot_point_at_it() {
        let conn = memory();
        let holder = insert(&conn, &sample("default", 0)).unwrap();
        let later = insert(&conn, &sample("default", 1)).unwrap();

        share_rows(&conn, later, holder).unwrap();

        assert_eq!(get(&conn, later).unwrap().unwrap().holder(), holder);
        assert!(
            conn.execute("DELETE FROM snapshot WHERE id = ?1", [holder])
                .is_err()
        );
    }
}
