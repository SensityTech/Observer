//! The switches of Observer's optional features, as the owner set them. A feature without a row
//! keeps its default; a row of a feature this build does not know is left alone.

use std::net::SocketAddr;

use jiff::Timestamp;
use observer_domain::features::Feature;
use rusqlite::{Connection, params};

use crate::database::StoreError;
use crate::settings;
use crate::time::Utc;

/// Where SFTP listens, once the owner said so in the panel.
const SFTP_LISTEN: &str = "sftp.listen";

/// A switch the owner set, and when.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoredSwitch {
    pub feature: Feature,
    pub enabled: bool,
    pub changed_at: Timestamp,
}

pub fn switches(conn: &Connection) -> Result<Vec<StoredSwitch>, StoreError> {
    let mut statement = conn.prepare("SELECT key, enabled, changed_at FROM features ORDER BY key")?;
    let rows = statement.query_map([], |row| {
        let key: String = row.get(0)?;
        let changed_at: Utc = row.get(2)?;
        Ok((key, row.get::<_, bool>(1)?, changed_at.0))
    })?;
    let mut switches = Vec::new();
    for row in rows {
        let (key, enabled, changed_at) = row?;
        if let Some(feature) = Feature::from_name(&key) {
            switches.push(StoredSwitch {
                feature,
                enabled,
                changed_at,
            });
        }
    }
    Ok(switches)
}

pub fn set(conn: &Connection, feature: Feature, enabled: bool, now: Timestamp) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO features (key, enabled, changed_at) VALUES (?1, ?2, ?3)
         ON CONFLICT (key) DO UPDATE SET enabled = excluded.enabled, changed_at = excluded.changed_at",
        params![feature.as_str(), enabled, Utc(now)],
    )?;
    Ok(())
}

/// The address the owner gave SFTP. `None`: never given, or taken back.
pub fn sftp_listen(conn: &Connection) -> Result<Option<SocketAddr>, StoreError> {
    Ok(settings::get(conn, SFTP_LISTEN)?.and_then(|listen| listen.parse().ok()))
}

pub fn set_sftp_listen(conn: &Connection, listen: Option<SocketAddr>) -> Result<(), StoreError> {
    if let Some(listen) = listen {
        return settings::set(conn, SFTP_LISTEN, &listen.to_string());
    }
    conn.execute("DELETE FROM settings WHERE key = ?1", [SFTP_LISTEN])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::memory;

    #[test]
    fn the_address_of_sftp_is_kept_until_it_is_taken_back() {
        let conn = memory();
        assert_eq!(sftp_listen(&conn).unwrap(), None);

        let listen: SocketAddr = "0.0.0.0:40126".parse().unwrap();
        set_sftp_listen(&conn, Some(listen)).unwrap();
        assert_eq!(sftp_listen(&conn).unwrap(), Some(listen));
        let v6: SocketAddr = "[::1]:2022".parse().unwrap();
        set_sftp_listen(&conn, Some(v6)).unwrap();
        assert_eq!(sftp_listen(&conn).unwrap(), Some(v6));

        set_sftp_listen(&conn, None).unwrap();
        assert_eq!(sftp_listen(&conn).unwrap(), None);
    }

    #[test]
    fn a_switch_is_replaced_on_save_and_unknown_keys_are_skipped() {
        let conn = memory();
        assert_eq!(switches(&conn).unwrap(), []);

        let first: Timestamp = "2026-11-02T09:00:00Z".parse().unwrap();
        let later: Timestamp = "2026-11-02T10:30:00.250Z".parse().unwrap();
        set(&conn, Feature::Sftp, false, first).unwrap();
        set(&conn, Feature::Metrics, false, first).unwrap();
        set(&conn, Feature::Sftp, true, later).unwrap();
        conn.execute(
            "INSERT INTO features (key, enabled, changed_at) VALUES ('gameBackups', 0, ?1)",
            [Utc(first)],
        )
        .unwrap();

        assert_eq!(
            switches(&conn).unwrap(),
            [
                StoredSwitch {
                    feature: Feature::Metrics,
                    enabled: false,
                    changed_at: first,
                },
                StoredSwitch {
                    feature: Feature::Sftp,
                    enabled: true,
                    changed_at: later,
                },
            ]
        );
    }
}
