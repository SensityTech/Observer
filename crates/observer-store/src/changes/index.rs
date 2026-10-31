//! The file index: what the last scan saw of each file, so that a file whose size and times did
//! not move keeps its digest without being read. A cache: lost, the next scan reads everything.

use std::collections::{HashMap, HashSet};
use std::hash::BuildHasher;

use jiff::Timestamp;
use rusqlite::{Connection, params};

use super::{sql, unsigned};
use crate::database::StoreError;
use crate::time::Utc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexEntry {
    pub size: u64,
    pub modified_ns: i64,
    pub inode: Option<u64>,
    pub changed_ns: Option<i64>,
    /// The keyed digest; `None` for a file never read.
    pub digest: Option<String>,
    pub sha256: Option<String>,
    /// The kind's stored name.
    pub kind: String,
    /// Hashed less than two seconds after its last write: read again next time.
    pub racy: bool,
}

pub fn load(conn: &Connection) -> Result<HashMap<String, IndexEntry>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT path, size, mtime_ns, inode, ctime_ns, digest, sha256, kind, racy FROM file_index",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            IndexEntry {
                size: unsigned(row.get(1)?),
                modified_ns: row.get(2)?,
                inode: row.get::<_, Option<i64>>(3)?.map(unsigned),
                changed_ns: row.get(4)?,
                digest: row.get(5)?,
                sha256: row.get(6)?,
                kind: row.get(7)?,
                racy: row.get(8)?,
            },
        ))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn save(conn: &Connection, entries: &[(String, IndexEntry)], at: Timestamp) -> Result<(), StoreError> {
    let mut statement = conn.prepare(
        "INSERT OR REPLACE INTO file_index (path, size, mtime_ns, inode, ctime_ns, digest, sha256, kind, racy, hashed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
    )?;
    for (path, entry) in entries {
        statement.execute(params![
            path,
            sql(entry.size),
            entry.modified_ns,
            entry.inode.map(sql),
            entry.changed_ns,
            entry.digest,
            entry.sha256,
            entry.kind,
            entry.racy,
            Utc(at),
        ])?;
    }
    Ok(())
}

/// Forgets every path a complete listing did not see.
pub fn keep_only<S: BuildHasher>(conn: &Connection, seen: &HashSet<String, S>) -> Result<usize, StoreError> {
    let mut statement = conn.prepare("SELECT path FROM file_index")?;
    let gone: Vec<String> = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .filter_map(Result::ok)
        .filter(|path| !seen.contains(path))
        .collect();
    let mut delete = conn.prepare("DELETE FROM file_index WHERE path = ?1")?;
    for path in &gone {
        delete.execute([path])?;
    }
    Ok(gone.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::changes::memory;

    fn entry(size: u64) -> IndexEntry {
        IndexEntry {
            size,
            modified_ns: 1_000,
            inode: Some(7),
            changed_ns: Some(2_000),
            digest: Some("d".to_owned()),
            sha256: None,
            kind: "text".to_owned(),
            racy: true,
        }
    }

    #[test]
    fn entries_are_saved_replaced_and_pruned() {
        let conn = memory();
        save(
            &conn,
            &[("a".to_owned(), entry(1)), ("b".to_owned(), entry(2))],
            Timestamp::now(),
        )
        .unwrap();
        save(&conn, &[("a".to_owned(), entry(3))], Timestamp::now()).unwrap();

        assert_eq!(keep_only(&conn, &HashSet::from(["a".to_owned()])).unwrap(), 1);

        let index = load(&conn).unwrap();
        assert_eq!(index.len(), 1);
        assert_eq!(index["a"], entry(3));
    }
}
