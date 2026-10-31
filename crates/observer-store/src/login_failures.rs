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

/// Which failures: the panel's sign-ins, or SFTP's, kept apart so that neither locks the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Failures(&'static str);

pub const PANEL: Failures = Failures("login_failures");
pub const FILES: Failures = Failures("file_login_failures");

impl Failures {
    pub fn get(
        self,
        conn: &Connection,
        username: &str,
        network: &str,
    ) -> Result<Option<LoginFailureRow>, StoreError> {
        let sql = format!(
            "SELECT {COLUMNS} FROM {} WHERE username = ?1 AND network = ?2",
            self.0
        );
        Ok(conn.query_row(&sql, [username, network], read).optional()?)
    }

    pub fn save(self, conn: &Connection, row: &LoginFailureRow) -> Result<(), StoreError> {
        let sql = format!(
            "INSERT INTO {} (username, network, failures, window_started_at, locked_until,
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
            self.0
        );
        conn.execute(
            &sql,
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

    pub fn delete(self, conn: &Connection, username: &str, network: &str) -> Result<(), StoreError> {
        let sql = format!("DELETE FROM {} WHERE username = ?1 AND network = ?2", self.0);
        conn.execute(&sql, [username, network])?;
        Ok(())
    }

    /// Every network's failures for one username.
    pub fn for_username(self, conn: &Connection, username: &str) -> Result<Vec<LoginFailureRow>, StoreError> {
        let sql = format!(
            "SELECT {COLUMNS} FROM {} WHERE username = ?1 ORDER BY network",
            self.0
        );
        let mut statement = conn.prepare(&sql)?;
        let rows = statement.query_map([username], read)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn delete_for_username(self, conn: &Connection, username: &str) -> Result<usize, StoreError> {
        let sql = format!("DELETE FROM {} WHERE username = ?1", self.0);
        Ok(conn.execute(&sql, [username])?)
    }

    /// The rows whose unaudited failures started at `since_before` or earlier.
    pub fn unaudited_since_before(
        self,
        conn: &Connection,
        since_before: Timestamp,
    ) -> Result<Vec<LoginFailureRow>, StoreError> {
        let sql = format!(
            "SELECT {COLUMNS} FROM {} WHERE unaudited > 0 AND unaudited_since <= ?1 ORDER BY unaudited_since",
            self.0
        );
        let mut statement = conn.prepare(&sql)?;
        let rows = statement.query_map([Utc(since_before)], read)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Deletes the rows with nothing left to audit, no lock in force at `now` and no failure since
    /// `quiet_since`.
    pub fn delete_quiet(
        self,
        conn: &Connection,
        quiet_since: Timestamp,
        now: Timestamp,
    ) -> Result<usize, StoreError> {
        let sql = format!(
            "DELETE FROM {}
             WHERE unaudited = 0 AND last_failure_at <= ?1 AND (locked_until IS NULL OR locked_until <= ?2)",
            self.0
        );
        Ok(conn.execute(&sql, params![Utc(quiet_since), Utc(now)])?)
    }
}

pub fn get(conn: &Connection, username: &str, network: &str) -> Result<Option<LoginFailureRow>, StoreError> {
    PANEL.get(conn, username, network)
}

pub fn save(conn: &Connection, row: &LoginFailureRow) -> Result<(), StoreError> {
    PANEL.save(conn, row)
}

pub fn delete(conn: &Connection, username: &str, network: &str) -> Result<(), StoreError> {
    PANEL.delete(conn, username, network)
}

/// Every network's failures for one username.
pub fn for_username(conn: &Connection, username: &str) -> Result<Vec<LoginFailureRow>, StoreError> {
    PANEL.for_username(conn, username)
}

pub fn delete_for_username(conn: &Connection, username: &str) -> Result<usize, StoreError> {
    PANEL.delete_for_username(conn, username)
}

/// The rows whose unaudited failures started at `since_before` or earlier.
pub fn unaudited_since_before(
    conn: &Connection,
    since_before: Timestamp,
) -> Result<Vec<LoginFailureRow>, StoreError> {
    PANEL.unaudited_since_before(conn, since_before)
}

/// Deletes the rows with nothing left to audit, no lock in force at `now` and no failure since
/// `quiet_since`.
pub fn delete_quiet(conn: &Connection, quiet_since: Timestamp, now: Timestamp) -> Result<usize, StoreError> {
    PANEL.delete_quiet(conn, quiet_since, now)
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
