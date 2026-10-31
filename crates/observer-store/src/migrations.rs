use std::path::Path;

use rusqlite::Connection;
use rusqlite_migration::{M, Migrations, SchemaVersion};
use tracing::info;

use crate::backup::{self, BackupKind};
use crate::database::StoreError;

/// Applied in order and never edited once released: a schema change is a new file.
const MIGRATIONS: &[M<'static>] = &[
    M::up(include_str!("../migrations/0001_instance.sql")),
    M::up(include_str!("../migrations/0002_accounts.sql")),
    M::up(include_str!("../migrations/0003_launch.sql")),
    M::up(include_str!("../migrations/0004_launch_version.sql")),
    M::up(include_str!("../migrations/0005_schedule.sql")),
    M::up(include_str!("../migrations/0006_artifacts.sql")),
    M::up(include_str!("../migrations/0007_diagnostics.sql")),
    M::up(include_str!("../migrations/0008_file_access.sql")),
];

/// The schema version this build migrates to.
pub(crate) fn latest() -> usize {
    MIGRATIONS.len()
}

/// Brings the schema to the latest version, after a backup if an existing database changes.
pub(crate) fn apply(conn: &mut Connection, backups: &Path) -> Result<(), StoreError> {
    let migrations = Migrations::from_slice(MIGRATIONS);
    let known = latest();

    match migrations.current_version(conn)? {
        SchemaVersion::Outside(found) => {
            return Err(StoreError::TooNew {
                found: found.get(),
                known,
            });
        }
        SchemaVersion::Inside(current) if current.get() < known => {
            let backup = backup::create(conn, backups, BackupKind::PreMigration)?;
            info!(
                from = current.get(),
                to = known,
                backup = %backup.display(),
                "migrating the state database"
            );
        }
        SchemaVersion::NoneSet | SchemaVersion::Inside(_) => {}
    }

    migrations.to_latest(conn)?;
    Ok(())
}

/// Refuses a database whose schema is not exactly this build's, without changing it.
pub(crate) fn require_latest(conn: &Connection) -> Result<(), StoreError> {
    let known = latest();
    match Migrations::from_slice(MIGRATIONS).current_version(conn)? {
        SchemaVersion::Inside(current) if current.get() == known => Ok(()),
        SchemaVersion::Outside(found) => Err(StoreError::TooNew {
            found: found.get(),
            known,
        }),
        SchemaVersion::Inside(found) => Err(StoreError::NotMigrated {
            found: found.get(),
            known,
        }),
        SchemaVersion::NoneSet => Err(StoreError::NotMigrated { found: 0, known }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_are_valid() {
        Migrations::from_slice(MIGRATIONS).validate().unwrap();
    }

    #[test]
    fn a_0005_database_moves_its_build_folder_to_the_artifact_state() {
        let mut conn = Connection::open_in_memory().unwrap();
        let migrations = Migrations::from_slice(MIGRATIONS);
        migrations.to_version(&mut conn, 5).unwrap();
        conn.execute(
            "INSERT INTO launch_settings (server_id, build_path, data_path, cfg_file, onesync, auto_start,
                 txadmin_events, extra_arguments, version, updated_at)
             VALUES ('default', ?1, ?2, 'server.cfg', 'on',
                 1, 0, '[]', 7, '2026-10-28T09:00:00.000Z')",
            [r"C:\FXServer\build", r"C:\FXServer\server-data"],
        )
        .unwrap();

        migrations.to_latest(&mut conn).unwrap();

        let stored = crate::launch::get(&conn, "default").unwrap().unwrap();
        assert_eq!(stored.version, 7);
        assert_eq!(stored.settings.data_path, r"C:\FXServer\server-data");
        assert_eq!(stored.legacy_build_path.as_deref(), Some(r"C:\FXServer\build"));
        let state = crate::artifacts::state(&conn, "default").unwrap();
        assert_eq!(state.legacy_build_path.as_deref(), Some(r"C:\FXServer\build"));
        assert_eq!(state.selected, None);
        let columns: i64 = conn
            .query_row(
                "SELECT count(*) FROM pragma_table_info('launch_settings') WHERE name = 'build_path'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(columns, 0);
    }
    #[test]
    fn the_audit_log_rebuild_keeps_every_row_its_sequence_and_its_triggers() {
        let mut conn = Connection::open_in_memory().unwrap();
        let migrations = Migrations::from_slice(MIGRATIONS);
        migrations.to_version(&mut conn, 7).unwrap();
        for (id, actor) in [(1, "user"), (2, "cli"), (3, "anonymous"), (4, "system")] {
            conn.execute(
                "INSERT INTO audit_log (id, at, actor_kind, actor_id, actor_name, ip, user_agent, action, target,
                     outcome, details, expires_at)
                 VALUES (?1, '2026-10-30T09:00:00.000Z', ?2, 7, 'owner', '192.0.2.4', 'Firefox', 'login', NULL,
                     'success', '{\"a\":1}', '2027-10-30T09:00:00.000Z')",
                rusqlite::params![id, actor],
            )
            .unwrap();
        }
        // The newest entry gone, as if purged: its id must never come back.
        conn.execute_batch("DROP TRIGGER audit_log_kept_until_purged; DELETE FROM audit_log WHERE id = 4;")
            .unwrap();
        let rows = |conn: &Connection| -> Vec<String> {
            let mut statement = conn
                .prepare("SELECT id, at, actor_kind, actor_id, actor_name, ip, user_agent, action, target, outcome, details, expires_at FROM audit_log ORDER BY id")
                .unwrap();
            statement
                .query_map([], |row| {
                    Ok((0..12)
                        .map(|i| format!("{:?}", row.get_ref(i).unwrap()))
                        .collect::<Vec<_>>()
                        .join("|"))
                })
                .unwrap()
                .map(Result::unwrap)
                .collect()
        };
        let before = rows(&conn);

        migrations.to_latest(&mut conn).unwrap();

        assert_eq!(rows(&conn), before);
        conn.execute(
            "INSERT INTO audit_log (at, actor_kind, actor_id, actor_name, action, outcome, expires_at)
             VALUES ('2026-10-31T09:00:00.000Z', 'service', 3, '+deploy-main', 'file_written', 'success',
                 '2027-10-31T09:00:00.000Z')",
            [],
        )
        .unwrap();
        let newest: i64 = conn
            .query_row("SELECT max(id) FROM audit_log", [], |row| row.get(0))
            .unwrap();
        assert_eq!(newest, 5);
        let update = conn.execute("UPDATE audit_log SET action = 'x' WHERE id = 1", []);
        assert!(update.unwrap_err().to_string().contains("append-only"));
        let delete = conn.execute("DELETE FROM audit_log WHERE id = 1", []);
        assert!(delete.unwrap_err().to_string().contains("kept until"));
        let indexes: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type = 'index' AND tbl_name = 'audit_log' AND name LIKE 'audit_log_%'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(indexes, 3);
    }
}
