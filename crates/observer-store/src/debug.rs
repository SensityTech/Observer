//! The hosts each resource called over HTTP, as the Debug feature saw them, and until when it
//! takes every new one as known.

use jiff::Timestamp;
use rusqlite::{Connection, params};

use crate::database::StoreError;
use crate::settings;
use crate::time::Utc;

const LEARNING_UNTIL: &str = "debug.learning_until";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredHost {
    pub resource: String,
    pub host: String,
    pub first_seen: Timestamp,
    pub known: bool,
}

pub fn hosts(conn: &Connection) -> Result<Vec<StoredHost>, StoreError> {
    let mut statement =
        conn.prepare("SELECT resource, host, first_seen, known FROM debug_hosts ORDER BY resource, host")?;
    let rows = statement.query_map([], |row| {
        let first_seen: Utc = row.get(2)?;
        Ok(StoredHost {
            resource: row.get(0)?,
            host: row.get(1)?,
            first_seen: first_seen.0,
            known: row.get(3)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Keeps a host seen for the first time. One already there keeps what it holds.
pub fn add(conn: &Connection, host: &StoredHost) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO debug_hosts (resource, host, first_seen, known) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (resource, host) DO NOTHING",
        params![host.resource, host.host, Utc(host.first_seen), host.known],
    )?;
    Ok(())
}

/// Marks a host as known. False when the resource never called it.
pub fn know(conn: &Connection, resource: &str, host: &str) -> Result<bool, StoreError> {
    let changed = conn.execute(
        "UPDATE debug_hosts SET known = 1 WHERE resource = ?1 AND host = ?2",
        params![resource, host],
    )?;
    Ok(changed > 0)
}

pub fn learning_until(conn: &Connection) -> Result<Option<Timestamp>, StoreError> {
    Ok(settings::get(conn, LEARNING_UNTIL)?.and_then(|until| until.parse().ok()))
}

pub fn set_learning_until(conn: &Connection, until: Timestamp) -> Result<(), StoreError> {
    settings::set(conn, LEARNING_UNTIL, &until.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::memory;

    #[test]
    fn a_host_keeps_its_first_day_and_is_known_once_said_so() {
        let conn = memory();
        let first: Timestamp = "2026-11-10T21:40:00Z".parse().unwrap();
        let later: Timestamp = "2026-11-11T08:00:00Z".parse().unwrap();
        let seen = |known| StoredHost {
            resource: "logs".to_owned(),
            host: "discord.com".to_owned(),
            first_seen: first,
            known,
        };

        add(&conn, &seen(false)).unwrap();
        add(
            &conn,
            &StoredHost {
                first_seen: later,
                ..seen(true)
            },
        )
        .unwrap();

        assert_eq!(hosts(&conn).unwrap(), [seen(false)], "the first sight is kept");
        assert!(know(&conn, "logs", "discord.com").unwrap());
        assert!(!know(&conn, "logs", "example.com").unwrap());
        assert_eq!(hosts(&conn).unwrap(), [seen(true)]);
    }

    #[test]
    fn the_day_of_learning_is_kept_once_set() {
        let conn = memory();
        assert_eq!(learning_until(&conn).unwrap(), None);

        let until: Timestamp = "2026-11-11T21:35:00Z".parse().unwrap();
        set_learning_until(&conn, until).unwrap();

        assert_eq!(learning_until(&conn).unwrap(), Some(until));
    }
}
