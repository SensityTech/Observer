use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use rusqlite::config::DbConfig;
use rusqlite::{Connection, OpenFlags};

use crate::backup::{self, BackupKind};
use crate::migrations;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("the database failed its integrity check: {0}")]
    Corrupt(String),
    #[error(
        "the database was written by a newer Observer (schema {found}, this build knows {known}); \
         use that version or restore a backup"
    )]
    TooNew { found: usize, known: usize },
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Migration(#[from] rusqlite_migration::Error),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("the database connection is closed")]
    Closed,
}

/// The state database. One connection owns the file: every statement runs on its thread, in
/// order, so writes never contend.
pub struct Database {
    conn: tokio_rusqlite::Connection,
    backups: PathBuf,
}

impl Database {
    /// Opens or creates the database, checks its integrity and migrates it. An existing database
    /// is backed up before its schema changes.
    pub async fn open(path: impl Into<PathBuf>, backups: impl Into<PathBuf>) -> Result<Self, StoreError> {
        let path = path.into();
        let backups = backups.into();
        let dir = backups.clone();

        let conn = tokio::task::spawn_blocking(move || open_blocking(&path, &dir))
            .await
            .map_err(|_| StoreError::Closed)??;

        Ok(Self {
            conn: conn.into(),
            backups,
        })
    }

    /// The random id this installation got when its database was created.
    pub async fn instance_id(&self) -> Result<String, StoreError> {
        self.call(|conn| {
            Ok(
                conn.query_row("SELECT instance_id FROM instance WHERE id = 1", [], |row| {
                    row.get(0)
                })?,
            )
        })
        .await
    }

    pub async fn backup(&self, kind: BackupKind) -> Result<PathBuf, StoreError> {
        let dir = self.backups.clone();
        self.call(move |conn| backup::create(conn, &dir, kind)).await
    }

    pub async fn newest_backup(&self, kind: BackupKind) -> Result<Option<SystemTime>, StoreError> {
        let dir = self.backups.clone();
        self.call(move |_| Ok(backup::newest(&dir, kind)?)).await
    }

    /// Closes the connection, which folds the WAL back into the database file.
    pub async fn close(self) -> Result<(), StoreError> {
        self.conn.close().await.map_err(|_| StoreError::Closed)
    }

    async fn call<R, F>(&self, function: F) -> Result<R, StoreError>
    where
        R: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<R, StoreError> + Send + 'static,
    {
        self.conn.call(function).await.map_err(|error| match error {
            tokio_rusqlite::Error::Error(error) => error,
            _ => StoreError::Closed,
        })
    }
}

fn open_blocking(path: &Path, backups: &Path) -> Result<Connection, StoreError> {
    let flags =
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let mut conn = Connection::open_with_flags(path, flags)?;

    configure(&conn)?;
    check_integrity(&conn)?;
    migrations::apply(&mut conn, backups)?;
    Ok(conn)
}

fn configure(conn: &Connection) -> rusqlite::Result<()> {
    // Defensive mode stops SQL from corrupting the file on purpose (writable_schema and the like).
    conn.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)?;
    conn.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get::<_, String>(0))?;
    conn.pragma_update(None, "synchronous", "FULL")?;
    conn.pragma_update(None, "foreign_keys", true)?;
    conn.pragma_update(None, "trusted_schema", false)?;
    conn.pragma_update(None, "journal_size_limit", 64 * 1024 * 1024)?;
    conn.busy_timeout(Duration::from_secs(5))?;
    Ok(())
}

fn check_integrity(conn: &Connection) -> Result<(), StoreError> {
    let verdict: String = conn.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    if verdict == "ok" {
        Ok(())
    } else {
        Err(StoreError::Corrupt(verdict))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(dir: &Path) -> (PathBuf, PathBuf) {
        (dir.join("observer.sqlite3"), dir.join("backups"))
    }

    #[tokio::test]
    async fn a_new_database_gets_an_instance_id_that_survives_a_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let (path, backups) = paths(dir.path());

        let db = Database::open(&path, &backups).await.unwrap();
        let first = db.instance_id().await.unwrap();
        db.close().await.unwrap();

        let db = Database::open(&path, &backups).await.unwrap();
        assert_eq!(db.instance_id().await.unwrap(), first);
        assert_eq!(first.len(), 16);
    }

    #[tokio::test]
    async fn opening_a_new_database_writes_no_backup() {
        let dir = tempfile::tempdir().unwrap();
        let (path, backups) = paths(dir.path());

        let db = Database::open(&path, &backups).await.unwrap();

        assert_eq!(db.newest_backup(BackupKind::PreMigration).await.unwrap(), None);
    }

    #[tokio::test]
    async fn a_database_from_a_newer_build_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (path, backups) = paths(dir.path());
        Database::open(&path, &backups)
            .await
            .unwrap()
            .close()
            .await
            .unwrap();
        Connection::open(&path)
            .unwrap()
            .pragma_update(None, "user_version", 99)
            .unwrap();

        let error = Database::open(&path, &backups).await.err().unwrap();

        assert!(matches!(error, StoreError::TooNew { found: 99, .. }));
    }

    #[tokio::test]
    async fn a_backup_is_a_readable_copy_of_the_database() {
        let dir = tempfile::tempdir().unwrap();
        let (path, backups) = paths(dir.path());
        let db = Database::open(&path, &backups).await.unwrap();

        let backup = db.backup(BackupKind::Daily).await.unwrap();

        let copy = Connection::open(&backup).unwrap();
        let id: String = copy
            .query_row("SELECT instance_id FROM instance", [], |row| row.get(0))
            .unwrap();
        assert_eq!(id, db.instance_id().await.unwrap());
        assert!(db.newest_backup(BackupKind::Daily).await.unwrap().is_some());
    }
}
