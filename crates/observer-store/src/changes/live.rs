//! Live changes: resources started, stopped or restarted while a run was up.

use jiff::Timestamp;
use observer_domain::changes::LiveChangeKind;
use rusqlite::{Connection, params};

use super::{name, parse};
use crate::database::StoreError;
use crate::time::Utc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveChange {
    pub at: Timestamp,
    pub name: String,
    pub change: LiveChangeKind,
    /// Who typed the matching command in Observer's console, and the command, masked.
    pub by: Option<String>,
    pub command: Option<String>,
}

pub fn insert(conn: &Connection, snapshot: i64, change: &LiveChange) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO live_change (snapshot_id, at, name, change, by, command) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            snapshot,
            Utc(change.at),
            change.name,
            name(&change.change),
            change.by,
            change.command
        ],
    )?;
    Ok(())
}

/// The live changes of a snapshot's run, in order, at most `limit`.
pub fn list(conn: &Connection, snapshot: i64, limit: u32) -> Result<Vec<LiveChange>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT at, name, change, by, command FROM live_change WHERE snapshot_id = ?1 ORDER BY at, id LIMIT ?2",
    )?;
    let rows = statement.query_map(params![snapshot, limit], |row| {
        Ok(LiveChange {
            at: row.get::<_, Utc>(0)?.0,
            name: row.get(1)?,
            change: parse(2, &row.get::<_, String>(2)?)?,
            by: row.get(3)?,
            command: row.get(4)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::changes::{memory, snapshots};

    #[test]
    fn live_changes_read_back_in_order() {
        let conn = memory();
        let id = snapshots::insert(&conn, &snapshots::sample("default", 0)).unwrap();
        let change = |second: i64, name: &str| LiveChange {
            at: Timestamp::from_second(1_793_000_000 + second).unwrap(),
            name: name.to_owned(),
            change: LiveChangeKind::Restarted,
            by: Some("bob".to_owned()),
            command: Some(format!("ensure {name}")),
        };
        insert(&conn, id, &change(5, "b")).unwrap();
        insert(&conn, id, &change(1, "a")).unwrap();

        let names: Vec<String> = list(&conn, id, 10).unwrap().into_iter().map(|c| c.name).collect();

        assert_eq!(names, ["a", "b"]);
    }
}
