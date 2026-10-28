use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use rusqlite::config::DbConfig;
use rusqlite::{Connection, OpenFlags, Transaction, TransactionBehavior};

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
    #[error(
        "the database has schema {found} and this build needs {known}; start Observer once to          migrate it"
    )]
    NotMigrated { found: usize, known: usize },
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Migration(#[from] rusqlite_migration::Error),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("no database at {}", .0.display())]
    Missing(PathBuf),
    #[error("the database connection is closed")]
    Closed,
}

/// What `Database::inspect` found in a database file, without changing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Inspection {
    pub schema: usize,
    pub latest: usize,
}

/// The state database. One connection owns the file: every statement runs on its thread, in
/// order, so writes never contend. Clones share that connection.
#[derive(Clone)]
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

        let conn = blocking(move || open_blocking(&path, &dir)).await?;

        Ok(Self {
            conn: conn.into(),
            backups,
        })
    }

    /// Opens an existing database without migrating it, while Observer itself may be running: the
    /// command line uses it to change accounts under a live instance.
    pub async fn open_existing(
        path: impl Into<PathBuf>,
        backups: impl Into<PathBuf>,
    ) -> Result<Self, StoreError> {
        let path = path.into();
        let backups = backups.into();

        let conn = blocking(move || {
            if !path.exists() {
                return Err(StoreError::Missing(path));
            }
            let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
            let conn = Connection::open_with_flags(&path, flags)?;
            configure(&conn)?;
            check_integrity(&conn)?;
            migrations::require_latest(&conn)?;
            Ok(conn)
        })
        .await?;

        Ok(Self {
            conn: conn.into(),
            backups,
        })
    }

    /// Checks an existing database's integrity and schema version, read-only and without migrating.
    pub async fn inspect(path: impl Into<PathBuf>) -> Result<Inspection, StoreError> {
        let path = path.into();
        blocking(move || {
            let conn = open_read_only(&path)?;
            check_integrity(&conn)?;
            let schema: u32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
            Ok(Inspection {
                schema: schema as usize,
                latest: migrations::latest(),
            })
        })
        .await
    }

    /// Backs up a database that is not open, read-only and without migrating it first.
    pub async fn backup_file(
        path: impl Into<PathBuf>,
        backups: impl Into<PathBuf>,
        kind: BackupKind,
    ) -> Result<PathBuf, StoreError> {
        let path = path.into();
        let backups = backups.into();
        blocking(move || {
            let conn = open_read_only(&path)?;
            backup::create(&conn, &backups, kind)
        })
        .await
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

    /// Runs `function` in one transaction, committed when it returns `Ok`. The transaction takes
    /// the write lock up front, so it never fails halfway for want of it.
    pub async fn write<R, E, F>(&self, function: F) -> Result<R, E>
    where
        R: Send + 'static,
        E: From<StoreError> + Send + 'static,
        F: FnOnce(&Transaction<'_>) -> Result<R, E> + Send + 'static,
    {
        self.call(move |conn| {
            let transaction = conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(StoreError::from)?;
            let value = function(&transaction)?;
            transaction.commit().map_err(StoreError::from)?;
            Ok(value)
        })
        .await
    }

    /// Runs `function` on the connection, for reads.
    pub async fn read<R, E, F>(&self, function: F) -> Result<R, E>
    where
        R: Send + 'static,
        E: From<StoreError> + Send + 'static,
        F: FnOnce(&Connection) -> Result<R, E> + Send + 'static,
    {
        self.call(move |conn| function(conn)).await
    }

    /// Closes the connection, which folds the WAL back into the database file.
    pub async fn close(self) -> Result<(), StoreError> {
        self.conn.close().await.map_err(|_| StoreError::Closed)
    }

    async fn call<R, E, F>(&self, function: F) -> Result<R, E>
    where
        R: Send + 'static,
        E: From<StoreError> + Send + 'static,
        F: FnOnce(&mut Connection) -> Result<R, E> + Send + 'static,
    {
        self.conn.call(function).await.map_err(|error| match error {
            tokio_rusqlite::Error::Error(error) => error,
            _ => StoreError::Closed.into(),
        })
    }
}

async fn blocking<R, F>(function: F) -> Result<R, StoreError>
where
    R: Send + 'static,
    F: FnOnce() -> Result<R, StoreError> + Send + 'static,
{
    tokio::task::spawn_blocking(function)
        .await
        .map_err(|_| StoreError::Closed)?
}

fn open_read_only(path: &Path) -> Result<Connection, StoreError> {
    if !path.exists() {
        return Err(StoreError::Missing(path.to_path_buf()));
    }
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    Ok(Connection::open_with_flags(path, flags)?)
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

/// A migrated in-memory database, for the repositories' tests.
#[cfg(test)]
pub(crate) fn memory() -> Connection {
    let mut conn = Connection::open_in_memory().unwrap();
    configure(&conn).unwrap();
    migrations::apply(&mut conn, Path::new("unused")).unwrap();
    conn
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

    #[tokio::test]
    async fn inspect_reports_the_schema_without_migrating() {
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
            .pragma_update(None, "user_version", 0)
            .unwrap();

        let inspection = Database::inspect(&path).await.unwrap();

        assert_eq!(inspection.schema, 0);
        assert_eq!(inspection.latest, migrations::latest());
        let schema: u32 = Connection::open(&path)
            .unwrap()
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(schema, 0);
    }

    #[tokio::test]
    async fn inspecting_a_missing_database_says_so_instead_of_creating_one() {
        let dir = tempfile::tempdir().unwrap();
        let (path, _) = paths(dir.path());

        let error = Database::inspect(&path).await.err().unwrap();

        assert!(matches!(error, StoreError::Missing(_)));
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn an_existing_database_opens_without_migrating_only_at_this_builds_schema() {
        let dir = tempfile::tempdir().unwrap();
        let (path, backups) = paths(dir.path());
        assert!(matches!(
            Database::open_existing(&path, &backups).await.err().unwrap(),
            StoreError::Missing(_)
        ));
        let db = Database::open(&path, &backups).await.unwrap();

        let second = Database::open_existing(&path, &backups).await.unwrap();
        assert_eq!(
            second.instance_id().await.unwrap(),
            db.instance_id().await.unwrap()
        );
        second.close().await.unwrap();
        db.close().await.unwrap();

        Connection::open(&path)
            .unwrap()
            .pragma_update(None, "user_version", 1)
            .unwrap();
        let error = Database::open_existing(&path, &backups).await.err().unwrap();
        assert!(matches!(error, StoreError::NotMigrated { found: 1, .. }));
    }

    #[tokio::test]
    async fn a_failed_write_rolls_back_everything_it_did() {
        let dir = tempfile::tempdir().unwrap();
        let (path, backups) = paths(dir.path());
        let db = Database::open(&path, &backups).await.unwrap();

        let result: Result<(), StoreError> = db
            .write(|tx| {
                tx.execute("INSERT INTO settings (key, value) VALUES ('probe', '1')", [])?;
                Err(StoreError::Closed)
            })
            .await;

        assert!(result.is_err());
        let value = db.read(|conn| crate::settings::get(conn, "probe")).await.unwrap();
        assert_eq!(value, None);
    }

    #[tokio::test]
    async fn a_closed_database_can_be_backed_up_read_only() {
        let dir = tempfile::tempdir().unwrap();
        let (path, backups) = paths(dir.path());
        Database::open(&path, &backups)
            .await
            .unwrap()
            .close()
            .await
            .unwrap();

        let backup = Database::backup_file(&path, &backups, BackupKind::Manual)
            .await
            .unwrap();

        assert!(backup.exists());
        assert!(backup.to_string_lossy().ends_with("-manual.sqlite3"));
    }
}
