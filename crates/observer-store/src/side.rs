//! The databases kept beside Observer's state, the statistics and the file changes: a file that
//! cannot be read is set aside and a new one started, and none is ever backed up. One connection
//! writes; reads have their own beside it, which the write-ahead log allows.

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::config::DbConfig;
use rusqlite::{Connection, ErrorCode, OpenFlags, Transaction, TransactionBehavior};
use rusqlite_migration::{M, Migrations, SchemaVersion};
use tracing::{info, warn};

use crate::database::StoreError;

/// How a database beside the state is opened.
#[derive(Clone, Copy)]
pub(crate) struct Shape {
    /// `metrics` for `metrics.sqlite3`, in the log.
    pub name: &'static str,
    pub migrations: &'static [M<'static>],
    pub foreign_keys: bool,
}

#[derive(Clone)]
pub(crate) struct Beside {
    conn: tokio_rusqlite::Connection,
    reader: tokio_rusqlite::Connection,
}

impl Beside {
    /// Opens or creates the database and migrates it. A corrupt file, or one a newer Observer
    /// wrote, is renamed `<name>.set-aside-<time>` with its journal files, and a new one takes its
    /// place.
    pub async fn open(path: PathBuf, shape: Shape) -> Result<Self, StoreError> {
        let (conn, reader) = tokio::task::spawn_blocking(move || {
            let conn = match open_blocking(&path, shape) {
                Err(error) if unreadable(&error) => {
                    let aside = set_aside(&path)?;
                    warn!(%error, database = shape.name, set_aside = %aside.display(), "a database could not be read; starting a new one");
                    open_blocking(&path, shape)
                }
                opened => opened,
            }?;
            Ok::<_, StoreError>((conn, open_reader(&path, shape)?))
        })
        .await
        .map_err(|_| StoreError::Closed)??;
        Ok(Self {
            conn: conn.into(),
            reader: reader.into(),
        })
    }

    pub async fn write<R, E, F>(&self, function: F) -> Result<R, E>
    where
        R: Send + 'static,
        E: From<StoreError> + Send + 'static,
        F: FnOnce(&Transaction<'_>) -> Result<R, E> + Send + 'static,
    {
        call(&self.conn, move |conn| {
            let transaction = conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(StoreError::from)?;
            let value = function(&transaction)?;
            transaction.commit().map_err(StoreError::from)?;
            Ok(value)
        })
        .await
    }

    /// Runs `function` on the writing connection outside a transaction, for what cannot run inside
    /// one: a checkpoint, a vacuum, a pragma.
    pub async fn maintain<R, E, F>(&self, function: F) -> Result<R, E>
    where
        R: Send + 'static,
        E: From<StoreError> + Send + 'static,
        F: FnOnce(&mut Connection) -> Result<R, E> + Send + 'static,
    {
        call(&self.conn, function).await
    }

    pub async fn read<R, E, F>(&self, function: F) -> Result<R, E>
    where
        R: Send + 'static,
        E: From<StoreError> + Send + 'static,
        F: FnOnce(&Connection) -> Result<R, E> + Send + 'static,
    {
        call(&self.reader, move |conn| function(conn)).await
    }

    pub async fn close(self) -> Result<(), StoreError> {
        // The writer goes last: closing it checkpoints the log into the file.
        self.reader.close().await.map_err(|_| StoreError::Closed)?;
        self.conn.close().await.map_err(|_| StoreError::Closed)
    }
}

async fn call<R, E, F>(conn: &tokio_rusqlite::Connection, function: F) -> Result<R, E>
where
    R: Send + 'static,
    E: From<StoreError> + Send + 'static,
    F: FnOnce(&mut Connection) -> Result<R, E> + Send + 'static,
{
    conn.call(function).await.map_err(|error| match error {
        tokio_rusqlite::Error::Error(error) => error,
        _ => StoreError::Closed.into(),
    })
}

fn open_blocking(path: &Path, shape: Shape) -> Result<Connection, StoreError> {
    let flags =
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let mut conn = Connection::open_with_flags(path, flags)?;
    // Only takes effect before the first table exists, which is the point: the upkeep gives back
    // the pages retention frees.
    conn.pragma_update(None, "auto_vacuum", "INCREMENTAL")?;
    conn.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)?;
    conn.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get::<_, String>(0))?;
    // Losing the last writes to a power cut is acceptable; a sync on every one of them is not.
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "trusted_schema", false)?;
    conn.pragma_update(None, "journal_size_limit", 64 * 1024 * 1024)?;
    conn.pragma_update(None, "foreign_keys", shape.foreign_keys)?;
    // What is deleted is overwritten, not only unlinked, where that costs no I/O: a forgotten
    // player's texts, a purged file's contents.
    conn.pragma_update(None, "secure_delete", "FAST")?;
    conn.busy_timeout(Duration::from_secs(5))?;
    let verdict: String = conn.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    if verdict != "ok" {
        return Err(StoreError::Corrupt(verdict));
    }
    let migrations = Migrations::from_slice(shape.migrations);
    match migrations.current_version(&conn)? {
        SchemaVersion::Outside(found) => {
            return Err(StoreError::TooNew {
                found: found.get(),
                known: shape.migrations.len(),
            });
        }
        // A year of aggregates takes a second or two, before the panel answers.
        SchemaVersion::Inside(current) if current.get() < shape.migrations.len() => {
            info!(
                database = shape.name,
                from = current.get(),
                to = shape.migrations.len(),
                "migrating a database"
            );
        }
        SchemaVersion::NoneSet | SchemaVersion::Inside(_) => {}
    }
    migrations.to_latest(&mut conn)?;
    Ok(conn)
}

/// Opened once the writer has migrated the file.
fn open_reader(path: &Path, shape: Shape) -> Result<Connection, StoreError> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)?;
    conn.pragma_update(None, "trusted_schema", false)?;
    conn.pragma_update(None, "foreign_keys", shape.foreign_keys)?;
    conn.busy_timeout(Duration::from_secs(5))?;
    Ok(conn)
}

/// What makes the file useless to this build, as opposed to a folder it cannot write to.
fn unreadable(error: &StoreError) -> bool {
    match error {
        StoreError::Corrupt(_) | StoreError::TooNew { .. } => true,
        StoreError::Sqlite(error) => matches!(
            error.sqlite_error_code(),
            Some(ErrorCode::NotADatabase | ErrorCode::DatabaseCorrupt)
        ),
        _ => false,
    }
}

fn set_aside(path: &Path) -> Result<PathBuf, StoreError> {
    let stamp = jiff::Timestamp::now().strftime("%Y%m%d-%H%M%S").to_string();
    let aside = path.with_file_name(format!(
        "{}.set-aside-{stamp}",
        path.file_name().unwrap_or_default().to_string_lossy()
    ));
    std::fs::rename(path, &aside)?;
    for suffix in ["-wal", "-shm"] {
        let mut journal = path.as_os_str().to_owned();
        journal.push(suffix);
        let mut moved = aside.as_os_str().to_owned();
        moved.push(suffix);
        match std::fs::rename(&journal, &moved) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error.into()),
            _ => {}
        }
    }
    Ok(aside)
}

/// The copies of `path` set aside so far, with their journal files.
pub(crate) fn set_asides(path: &Path) -> Vec<PathBuf> {
    let Some(name) = path
        .file_name()
        .map(|name| format!("{}.set-aside-", name.to_string_lossy()))
    else {
        return Vec::new();
    };
    let folder = path.parent().unwrap_or(Path::new("."));
    std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(&name))
        .map(|entry| entry.path())
        .collect()
}
