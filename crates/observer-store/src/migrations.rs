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
}
