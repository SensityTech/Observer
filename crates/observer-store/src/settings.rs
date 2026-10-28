use rusqlite::{Connection, OptionalExtension};

use crate::database::StoreError;

/// A setting's stored value, or `None` when it was never changed from its default.
pub fn get(conn: &Connection, key: &str) -> Result<Option<String>, StoreError> {
    Ok(conn
        .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
            row.get(0)
        })
        .optional()?)
}
