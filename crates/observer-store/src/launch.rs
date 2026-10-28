use jiff::Timestamp;
use observer_domain::launch::{LaunchSettings, OneSync, StoredLaunch};
use rusqlite::types::{FromSqlError, Type};
use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::database::StoreError;
use crate::time::Utc;

/// A server's launch settings, or `None` until the owner first saves them.
pub fn get(conn: &Connection, server_id: &str) -> Result<Option<StoredLaunch>, StoreError> {
    Ok(conn
        .query_row(
            "SELECT build_path, data_path, cfg_file, onesync, auto_start, txadmin_events, extra_arguments,
                 version
             FROM launch_settings WHERE server_id = ?1",
            [server_id],
            read,
        )
        .optional()?)
}

/// Stores the settings under their version, which the caller has already moved on.
pub fn save(
    conn: &Connection,
    server_id: &str,
    stored: &StoredLaunch,
    now: Timestamp,
) -> Result<(), StoreError> {
    let settings = &stored.settings;
    let arguments = serde_json::to_string(&settings.extra_arguments)
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    conn.execute(
        "INSERT INTO launch_settings (server_id, build_path, data_path, cfg_file, onesync, auto_start,
             txadmin_events, extra_arguments, version, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT (server_id) DO UPDATE SET build_path = excluded.build_path,
             data_path = excluded.data_path, cfg_file = excluded.cfg_file, onesync = excluded.onesync,
             auto_start = excluded.auto_start, txadmin_events = excluded.txadmin_events,
             extra_arguments = excluded.extra_arguments, version = excluded.version,
             updated_at = excluded.updated_at",
        params![
            server_id,
            settings.build_path,
            settings.data_path,
            settings.cfg_file,
            settings.onesync.as_str(),
            settings.auto_start,
            settings.txadmin_events,
            arguments,
            stored.version,
            Utc(now),
        ],
    )?;
    Ok(())
}

fn read(row: &Row<'_>) -> rusqlite::Result<StoredLaunch> {
    let onesync: String = row.get(3)?;
    let arguments: String = row.get(6)?;
    let settings = LaunchSettings {
        build_path: row.get(0)?,
        data_path: row.get(1)?,
        cfg_file: row.get(2)?,
        onesync: OneSync::from_name(&onesync).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                3,
                Type::Text,
                Box::new(FromSqlError::Other("unknown OneSync mode".into())),
            )
        })?,
        auto_start: row.get(4)?,
        txadmin_events: row.get(5)?,
        extra_arguments: serde_json::from_str(&arguments)
            .map_err(|error| rusqlite::Error::FromSqlConversionFailure(6, Type::Text, Box::new(error)))?,
    };
    Ok(StoredLaunch {
        settings,
        version: row.get(7)?,
    })
}

#[cfg(test)]
mod tests {
    use observer_domain::launch::ExtraArgument;

    use super::*;
    use crate::database::memory;

    fn settings() -> LaunchSettings {
        LaunchSettings {
            build_path: r"C:\FXServer\build".to_owned(),
            data_path: r"C:\FXServer\server-data".to_owned(),
            cfg_file: "server.cfg".to_owned(),
            onesync: OneSync::Legacy,
            auto_start: true,
            txadmin_events: false,
            extra_arguments: vec![
                ExtraArgument {
                    name: "sv_enforceGameBuild".to_owned(),
                    value: "3258".to_owned(),
                },
                ExtraArgument {
                    name: "sv_hostname".to_owned(),
                    value: "Observer lab".to_owned(),
                },
            ],
        }
    }

    fn stored(settings: LaunchSettings, version: i64) -> StoredLaunch {
        StoredLaunch { settings, version }
    }

    #[test]
    fn settings_are_kept_per_server_and_replaced_on_save() {
        let conn = memory();
        let now: Timestamp = "2026-10-28T09:00:00Z".parse().unwrap();
        assert_eq!(get(&conn, "default").unwrap(), None);

        save(&conn, "default", &stored(settings(), 1), now).unwrap();
        let changed = LaunchSettings {
            onesync: OneSync::Off,
            extra_arguments: Vec::new(),
            ..settings()
        };
        save(&conn, "other", &stored(changed.clone(), 1), now).unwrap();

        assert_eq!(get(&conn, "default").unwrap(), Some(stored(settings(), 1)));
        save(&conn, "default", &stored(changed.clone(), 2), now).unwrap();
        assert_eq!(get(&conn, "default").unwrap(), Some(stored(changed, 2)));
        let rows: i64 = conn
            .query_row("SELECT count(*) FROM launch_settings", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, 2);
    }

    #[test]
    fn the_schema_refuses_a_mode_or_an_argument_list_it_does_not_know() {
        let conn = memory();
        let insert = |onesync: &str, arguments: &str| {
            conn.execute(
                "INSERT INTO launch_settings (server_id, build_path, data_path, cfg_file, onesync,
                     auto_start, txadmin_events, extra_arguments, updated_at)
                 VALUES ('default', '/srv/build', '/srv/data', 'server.cfg', ?1, 0, 0, ?2,
                     '2026-10-28T09:00:00.000Z')",
                [onesync, arguments],
            )
        };

        assert!(insert("sometimes", "[]").is_err());
        assert!(insert("on", "{\"name\": \"x\"}").is_err());
        assert!(insert("on", "[oops").is_err());
        insert("on", "[]").unwrap();
    }
}
