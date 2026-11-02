//! The switches of Observer's optional features, as the owner set them. A feature without a row
//! keeps its default; a row of a feature this build does not know is left alone.

use jiff::Timestamp;
use observer_domain::features::Feature;
use rusqlite::{Connection, params};

use crate::database::StoreError;
use crate::time::Utc;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::memory;

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
