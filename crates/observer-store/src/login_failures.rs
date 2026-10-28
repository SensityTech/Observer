use jiff::Timestamp;
use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::database::StoreError;
use crate::time::Utc;

/// The failed logins of one username from one client network. The services decide what the
/// counters mean; this only stores them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginFailureRow {
    pub username: String,
    pub network: String,
    /// Failures counted toward the next lock, since `window_started_at`.
    pub failures: u32,
    pub window_started_at: Timestamp,
    pub locked_until: Option<Timestamp>,
    /// Locks in a row, which lengthen the next one.
    pub lockouts: u32,
    /// Failures not written to the audit log yet, since `unaudited_since`.
    pub unaudited: u32,
    pub unaudited_since: Option<Timestamp>,
    pub last_failure_at: Timestamp,
}

const COLUMNS: &str = "username, network, failures, window_started_at, locked_until, lockouts, \
    unaudited, unaudited_since, last_failure_at";

pub fn get(conn: &Connection, username: &str, network: &str) -> Result<Option<LoginFailureRow>, StoreError> {
    let sql = format!("SELECT {COLUMNS} FROM login_failures WHERE username = ?1 AND network = ?2");
    Ok(conn.query_row(&sql, [username, network], read).optional()?)
}

pub fn save(conn: &Connection, row: &LoginFailureRow) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO login_failures (username, network, failures, window_started_at, locked_until,
             lockouts, unaudited, unaudited_since, last_failure_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT (username, network) DO UPDATE SET
             failures = excluded.failures,
             window_started_at = excluded.window_started_at,
             locked_until = excluded.locked_until,
             lockouts = excluded.lockouts,
             unaudited = excluded.unaudited,
             unaudited_since = excluded.unaudited_since,
             last_failure_at = excluded.last_failure_at",
        params![
            row.username,
            row.network,
            row.failures,
            Utc(row.window_started_at),
            row.locked_until.map(Utc),
            row.lockouts,
            row.unaudited,
            row.unaudited_since.map(Utc),
            Utc(row.last_failure_at),
        ],
    )?;
    Ok(())
}

pub fn delete(conn: &Connection, username: &str, network: &str) -> Result<(), StoreError> {
    conn.execute(
        "DELETE FROM login_failures WHERE username = ?1 AND network = ?2",
        [username, network],
    )?;
    Ok(())
}

/// Every network's failures for one username.
pub fn for_username(conn: &Connection, username: &str) -> Result<Vec<LoginFailureRow>, StoreError> {
    let sql = format!("SELECT {COLUMNS} FROM login_failures WHERE username = ?1 ORDER BY network");
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map([username], read)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn delete_for_username(conn: &Connection, username: &str) -> Result<usize, StoreError> {
    Ok(conn.execute("DELETE FROM login_failures WHERE username = ?1", [username])?)
}

/// The rows whose unaudited failures started at `since_before` or earlier.
pub fn unaudited_since_before(
    conn: &Connection,
    since_before: Timestamp,
) -> Result<Vec<LoginFailureRow>, StoreError> {
    let sql = format!(
        "SELECT {COLUMNS} FROM login_failures WHERE unaudited > 0 AND unaudited_since <= ?1 \
         ORDER BY unaudited_since"
    );
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map([Utc(since_before)], read)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Deletes the rows with nothing left to audit, no lock in force at `now` and no failure since
/// `quiet_since`.
pub fn delete_quiet(conn: &Connection, quiet_since: Timestamp, now: Timestamp) -> Result<usize, StoreError> {
    Ok(conn.execute(
        "DELETE FROM login_failures
         WHERE unaudited = 0 AND last_failure_at <= ?1 AND (locked_until IS NULL OR locked_until <= ?2)",
        params![Utc(quiet_since), Utc(now)],
    )?)
}

fn read(row: &Row<'_>) -> rusqlite::Result<LoginFailureRow> {
    Ok(LoginFailureRow {
        username: row.get(0)?,
        network: row.get(1)?,
        failures: row.get(2)?,
        window_started_at: row.get::<_, Utc>(3)?.0,
        locked_until: row.get::<_, Option<Utc>>(4)?.map(|at| at.0),
        lockouts: row.get(5)?,
        unaudited: row.get(6)?,
        unaudited_since: row.get::<_, Option<Utc>>(7)?.map(|at| at.0),
        last_failure_at: row.get::<_, Utc>(8)?.0,
    })
}
