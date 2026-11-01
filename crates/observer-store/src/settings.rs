use rusqlite::{Connection, OptionalExtension};

use crate::database::StoreError;

/// Stores a setting, replacing what was there.
pub fn set(conn: &Connection, key: &str, value: &str) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        [key, value],
    )?;
    Ok(())
}

/// Stores `value` unless the setting is there already, and returns what is stored either way, so
/// that two processes creating it at once end up with one value.
pub fn create(conn: &Connection, key: &str, value: &str) -> Result<String, StoreError> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO NOTHING",
        [key, value],
    )?;
    Ok(
        conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
            row.get(0)
        })?,
    )
}

/// A setting's stored value, or `None` when it was never changed from its default.
pub fn get(conn: &Connection, key: &str) -> Result<Option<String>, StoreError> {
    Ok(conn
        .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
            row.get(0)
        })
        .optional()?)
}
