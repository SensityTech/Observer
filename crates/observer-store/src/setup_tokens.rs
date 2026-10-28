use jiff::Timestamp;
use rusqlite::{Connection, params};

use crate::database::StoreError;
use crate::time::Utc;

/// Stores the hash of a setup token; the token itself only ever appears in the setup link.
pub fn insert(
    conn: &Connection,
    token_hash: &[u8],
    created_at: Timestamp,
    expires_at: Timestamp,
) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO setup_tokens (token_hash, created_at, expires_at) VALUES (?1, ?2, ?3)",
        params![token_hash, Utc(created_at), Utc(expires_at)],
    )?;
    Ok(())
}

pub fn is_valid(conn: &Connection, token_hash: &[u8], now: Timestamp) -> Result<bool, StoreError> {
    Ok(conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM setup_tokens WHERE token_hash = ?1 AND expires_at > ?2)",
        params![token_hash, Utc(now)],
        |row| row.get(0),
    )?)
}

/// Deletes the token if it is still valid, and says whether it was.
pub fn consume(conn: &Connection, token_hash: &[u8], now: Timestamp) -> Result<bool, StoreError> {
    Ok(conn.execute(
        "DELETE FROM setup_tokens WHERE token_hash = ?1 AND expires_at > ?2",
        params![token_hash, Utc(now)],
    )? == 1)
}

pub fn delete_all(conn: &Connection) -> Result<(), StoreError> {
    conn.execute("DELETE FROM setup_tokens", [])?;
    Ok(())
}

pub fn delete_expired(conn: &Connection, now: Timestamp) -> Result<usize, StoreError> {
    Ok(conn.execute("DELETE FROM setup_tokens WHERE expires_at <= ?1", [Utc(now)])?)
}
