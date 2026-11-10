//! The launch's cfg as it was before each save from the panel, the latest few of each file.

use jiff::Timestamp;
use observer_domain::cfg::CfgVersion;
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::database::StoreError;
use crate::time::Utc;

/// The file as it was, about to be replaced.
#[derive(Debug, Clone, Copy)]
pub struct Replaced<'a> {
    pub server: &'a str,
    /// Relative to the server folder, with `/`.
    pub file: &'a str,
    pub content: &'a [u8],
    pub lines: u32,
    pub by: &'a str,
    pub at: Timestamp,
}

/// Keeps `replaced`, and drops the oldest versions of its file past the `kept` latest.
pub fn keep(tx: &Transaction<'_>, replaced: Replaced<'_>, kept: u32) -> Result<i64, StoreError> {
    tx.execute(
        "INSERT INTO cfg_version (server_id, file, replaced_at, replaced_by, lines, content)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            replaced.server,
            replaced.file,
            Utc(replaced.at),
            replaced.by,
            replaced.lines,
            replaced.content
        ],
    )?;
    let id = tx.last_insert_rowid();
    tx.execute(
        "DELETE FROM cfg_version WHERE server_id = ?1 AND file = ?2 AND id NOT IN (
             SELECT id FROM cfg_version WHERE server_id = ?1 AND file = ?2 ORDER BY id DESC LIMIT ?3)",
        params![replaced.server, replaced.file, kept],
    )?;
    Ok(id)
}

/// The versions kept of `file`, the latest first.
pub fn list(conn: &Connection, server: &str, file: &str) -> Result<Vec<CfgVersion>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT id, replaced_at, replaced_by, length(content), lines FROM cfg_version
         WHERE server_id = ?1 AND file = ?2 ORDER BY id DESC",
    )?;
    let rows = statement.query_map(params![server, file], |row| {
        let replaced_at: Utc = row.get(1)?;
        Ok(CfgVersion {
            id: row.get(0)?,
            replaced_at: replaced_at.0,
            replaced_by: row.get(2)?,
            bytes: row.get(3)?,
            lines: row.get(4)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn count(conn: &Connection, server: &str, file: &str) -> Result<u32, StoreError> {
    Ok(conn.query_row(
        "SELECT count(*) FROM cfg_version WHERE server_id = ?1 AND file = ?2",
        params![server, file],
        |row| row.get(0),
    )?)
}

/// The bytes of the version `id` of `file`, if it is still kept.
pub fn content(conn: &Connection, server: &str, file: &str, id: i64) -> Result<Option<Vec<u8>>, StoreError> {
    Ok(conn
        .query_row(
            "SELECT content FROM cfg_version WHERE server_id = ?1 AND file = ?2 AND id = ?3",
            params![server, file, id],
            |row| row.get(0),
        )
        .optional()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::memory;

    fn replaced<'a>(file: &'a str, content: &'a [u8], by: &'a str, at: Timestamp) -> Replaced<'a> {
        Replaced {
            server: "default",
            file,
            content,
            lines: 2,
            by,
            at,
        }
    }

    #[test]
    fn the_latest_versions_of_each_file_are_kept_and_the_oldest_dropped() {
        let mut conn = memory();
        let at: Timestamp = "2026-11-09T21:35:00.250Z".parse().unwrap();
        let tx = conn.transaction().unwrap();
        let first = keep(&tx, replaced("server.cfg", b"one\n", "ada", at), 2).unwrap();
        keep(&tx, replaced("resources.cfg", b"other\n", "ada", at), 2).unwrap();
        let second = keep(&tx, replaced("server.cfg", b"two\n", "grace", at), 2).unwrap();
        let third = keep(&tx, replaced("server.cfg", b"three!\n", "ada", at), 2).unwrap();
        tx.commit().unwrap();

        let kept = list(&conn, "default", "server.cfg").unwrap();
        assert_eq!(
            kept,
            [
                CfgVersion {
                    id: third,
                    replaced_at: at,
                    replaced_by: "ada".to_owned(),
                    bytes: 7,
                    lines: 2,
                },
                CfgVersion {
                    id: second,
                    replaced_at: at,
                    replaced_by: "grace".to_owned(),
                    bytes: 4,
                    lines: 2,
                },
            ]
        );
        assert_eq!(count(&conn, "default", "server.cfg").unwrap(), 2);
        assert_eq!(count(&conn, "default", "resources.cfg").unwrap(), 1);
        assert_eq!(content(&conn, "default", "server.cfg", first).unwrap(), None);
        assert_eq!(
            content(&conn, "default", "server.cfg", second).unwrap(),
            Some(b"two\n".to_vec())
        );
        assert_eq!(content(&conn, "default", "resources.cfg", second).unwrap(), None);
        assert_eq!(content(&conn, "other", "server.cfg", second).unwrap(), None);
    }
}
