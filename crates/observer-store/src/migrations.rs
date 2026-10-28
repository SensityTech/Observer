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
    M::up(include_str!("../migrations/0008_diagnostics.sql")),
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
}
