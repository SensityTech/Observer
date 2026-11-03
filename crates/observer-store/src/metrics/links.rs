//! The links between a crash wave and a change that someone said are not related.

use std::collections::HashSet;

use rusqlite::{Connection, Transaction, params};

use crate::database::StoreError;

pub fn dismissed(conn: &Connection) -> Result<HashSet<String>, StoreError> {
    let mut statement = conn.prepare_cached("SELECT link FROM link_dismissed")?;
    let rows = statement.query_map([], |row| row.get(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Records that `by` said the link is not related, at `at_ms`; again replaces who and when.
pub fn dismiss(tx: &Transaction<'_>, link: &str, by: &str, at_ms: i64) -> Result<(), StoreError> {
    tx.prepare_cached("INSERT OR REPLACE INTO link_dismissed (link, by, at_ms) VALUES (?1, ?2, ?3)")?
        .execute(params![link, by, at_ms])?;
    Ok(())
}

/// Takes the link back; whether it was dismissed.
pub fn restore(tx: &Transaction<'_>, link: &str) -> Result<bool, StoreError> {
    Ok(tx
        .prepare_cached("DELETE FROM link_dismissed WHERE link = ?1")?
        .execute([link])?
        > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::memory;

    #[test]
    fn a_link_said_not_related_is_kept_until_it_is_taken_back() {
        let mut conn = memory();
        let tx = conn.transaction().unwrap();
        dismiss(&tx, "start:4>wave:9:120", "alice", 1_000).unwrap();
        dismiss(&tx, "start:4>wave:9:120", "bob", 2_000).unwrap();
        tx.commit().unwrap();

        assert_eq!(
            dismissed(&conn).unwrap(),
            HashSet::from(["start:4>wave:9:120".to_owned()])
        );
        let by: String = conn
            .query_row("SELECT by FROM link_dismissed", [], |row| row.get(0))
            .unwrap();
        assert_eq!(by, "bob");

        let tx = conn.transaction().unwrap();
        assert!(restore(&tx, "start:4>wave:9:120").unwrap());
        assert!(!restore(&tx, "start:4>wave:9:120").unwrap());
        tx.commit().unwrap();
        assert!(dismissed(&conn).unwrap().is_empty());
    }
}
