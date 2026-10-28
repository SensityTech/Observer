use jiff::Timestamp;
use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::database::StoreError;
use crate::time::Utc;

/// A signed-in browser. Both of its secrets are stored as SHA-256 hashes only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRow {
    pub id: i64,
    pub user_id: i64,
    pub key_hash: Vec<u8>,
    pub auth_epoch: i64,
    pub created_at: Timestamp,
    pub last_seen_at: Timestamp,
    /// The absolute expiry; the idle one follows from `last_seen_at`.
    pub expires_at: Timestamp,
    pub reauthenticated_at: Option<Timestamp>,
    pub ip: Option<String>,
    pub user_agent: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NewSession<'a> {
    pub user_id: i64,
    pub token_hash: &'a [u8],
    pub key_hash: &'a [u8],
    pub auth_epoch: i64,
    pub created_at: Timestamp,
    pub expires_at: Timestamp,
    pub ip: Option<&'a str>,
    pub user_agent: Option<&'a str>,
}

const COLUMNS: &str = "id, user_id, key_hash, auth_epoch, created_at, last_seen_at, expires_at, \
    reauthenticated_at, ip, user_agent";

/// Opens a session that is freshly authenticated: signing in counts as entering the password.
pub fn insert(conn: &Connection, session: &NewSession<'_>) -> Result<i64, StoreError> {
    conn.execute(
        "INSERT INTO sessions (user_id, token_hash, key_hash, auth_epoch, created_at, last_seen_at,
             expires_at, reauthenticated_at, ip, user_agent)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6, ?5, ?7, ?8)",
        params![
            session.user_id,
            session.token_hash,
            session.key_hash,
            session.auth_epoch,
            Utc(session.created_at),
            Utc(session.expires_at),
            session.ip,
            session.user_agent,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn get(conn: &Connection, id: i64) -> Result<Option<SessionRow>, StoreError> {
    let sql = format!("SELECT {COLUMNS} FROM sessions WHERE id = ?1");
    Ok(conn.query_row(&sql, [id], read).optional()?)
}

pub fn find_by_token(conn: &Connection, token_hash: &[u8]) -> Result<Option<SessionRow>, StoreError> {
    let sql = format!("SELECT {COLUMNS} FROM sessions WHERE token_hash = ?1");
    Ok(conn.query_row(&sql, [token_hash], read).optional()?)
}

/// A user's sessions, the most recently used first.
pub fn list_for_user(conn: &Connection, user_id: i64) -> Result<Vec<SessionRow>, StoreError> {
    let sql =
        format!("SELECT {COLUMNS} FROM sessions WHERE user_id = ?1 ORDER BY last_seen_at DESC, id DESC");
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map([user_id], read)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Writes what changes while a session lives: its last use, its re-authentication and its epoch.
pub fn update(conn: &Connection, session: &SessionRow) -> Result<(), StoreError> {
    conn.execute(
        "UPDATE sessions SET auth_epoch = ?2, last_seen_at = ?3, reauthenticated_at = ?4 WHERE id = ?1",
        params![
            session.id,
            session.auth_epoch,
            Utc(session.last_seen_at),
            session.reauthenticated_at.map(Utc),
        ],
    )?;
    Ok(())
}

pub fn delete(conn: &Connection, id: i64) -> Result<bool, StoreError> {
    Ok(conn.execute("DELETE FROM sessions WHERE id = ?1", [id])? == 1)
}

/// Deletes every session of a user, except `keep` when given.
pub fn delete_for_user(conn: &Connection, user_id: i64, keep: Option<i64>) -> Result<usize, StoreError> {
    Ok(conn.execute(
        "DELETE FROM sessions WHERE user_id = ?1 AND id IS NOT ?2",
        params![user_id, keep],
    )?)
}

/// Deletes the sessions unused since `idle_before` or past their absolute expiry at `now`.
pub fn delete_expired(
    conn: &Connection,
    idle_before: Timestamp,
    now: Timestamp,
) -> Result<usize, StoreError> {
    Ok(conn.execute(
        "DELETE FROM sessions WHERE last_seen_at <= ?1 OR expires_at <= ?2",
        params![Utc(idle_before), Utc(now)],
    )?)
}

fn read(row: &Row<'_>) -> rusqlite::Result<SessionRow> {
    Ok(SessionRow {
        id: row.get(0)?,
        user_id: row.get(1)?,
        key_hash: row.get(2)?,
        auth_epoch: row.get(3)?,
        created_at: row.get::<_, Utc>(4)?.0,
        last_seen_at: row.get::<_, Utc>(5)?.0,
        expires_at: row.get::<_, Utc>(6)?.0,
        reauthenticated_at: row.get::<_, Option<Utc>>(7)?.map(|at| at.0),
        ip: row.get(8)?,
        user_agent: row.get(9)?,
    })
}
