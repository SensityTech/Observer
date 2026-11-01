//! The file changes' own database, `changes.sqlite3`: what each start of FXServer ran from, its
//! resources as trees of entries, the masked texts by keyed digest, and the index of what the last
//! scan saw. It is not Observer's state: a file that cannot be read is set aside and a new one
//! started, and it is never backed up, which would copy gigabytes of history every day.

pub mod index;
pub mod live;
pub mod resources;
pub mod snapshots;
pub mod texts;
pub mod trees;
pub mod upkeep;

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use flate2::Compression;
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use rusqlite::{Connection, Transaction};
use rusqlite_migration::M;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::database::StoreError;
use crate::side::{self, Beside, Shape};

const MIGRATIONS: &[M<'static>] = &[M::up(include_str!("../../migrations/changes/0001_changes.sql"))];

const SHAPE: Shape = Shape {
    name: "changes",
    migrations: MIGRATIONS,
    foreign_keys: true,
};

/// The most a stored blob inflates to: far past any listing or text this work stores.
const MAX_INFLATED: u64 = 512 * 1024 * 1024;

/// The file changes' database. Clones share its connections.
#[derive(Clone)]
pub struct ChangesDatabase {
    inner: Beside,
    path: PathBuf,
}

impl ChangesDatabase {
    /// Opens or creates the database and migrates it. A corrupt file, or one a newer Observer
    /// wrote, is renamed `changes.sqlite3.set-aside-<time>` with its journal files, and a new one
    /// takes its place.
    pub async fn open(path: impl Into<PathBuf>) -> Result<Self, StoreError> {
        let path = path.into();
        Ok(Self {
            inner: Beside::open(path.clone(), SHAPE).await?,
            path,
        })
    }

    /// Runs `function` in one transaction, committed when it returns `Ok`.
    pub async fn write<R, E, F>(&self, function: F) -> Result<R, E>
    where
        R: Send + 'static,
        E: From<StoreError> + Send + 'static,
        F: FnOnce(&Transaction<'_>) -> Result<R, E> + Send + 'static,
    {
        self.inner.write(function).await
    }

    /// Runs `function` on the writing connection outside a transaction: a purge's
    /// `secure_delete`, a checkpoint, a vacuum.
    pub async fn maintain<R, E, F>(&self, function: F) -> Result<R, E>
    where
        R: Send + 'static,
        E: From<StoreError> + Send + 'static,
        F: FnOnce(&mut Connection) -> Result<R, E> + Send + 'static,
    {
        self.inner.maintain(function).await
    }

    /// Runs `function` on the read-only connection, which sees what was committed last.
    pub async fn read<R, E, F>(&self, function: F) -> Result<R, E>
    where
        R: Send + 'static,
        E: From<StoreError> + Send + 'static,
        F: FnOnce(&Connection) -> Result<R, E> + Send + 'static,
    {
        self.inner.read(function).await
    }

    /// The copies of the file set aside so far, which hold the same texts.
    pub fn set_asides(&self) -> Vec<PathBuf> {
        side::set_asides(&self.path)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub async fn close(self) -> Result<(), StoreError> {
        self.inner.close().await
    }
}

/// Counts and sizes are stored as SQLite's signed integers; none comes near 2^63.
pub(crate) fn sql(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

pub(crate) fn unsigned(stored: i64) -> u64 {
    u64::try_from(stored).unwrap_or(0)
}

pub(crate) fn deflate(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
    // Writing to a vector cannot fail.
    let _ = encoder.write_all(bytes);
    encoder.finish().unwrap_or_default()
}

pub(crate) fn inflate(bytes: &[u8]) -> Result<Vec<u8>, StoreError> {
    let mut inflated = Vec::new();
    DeflateDecoder::new(bytes)
        .take(MAX_INFLATED)
        .read_to_end(&mut inflated)
        .map_err(|error| StoreError::Corrupt(format!("a stored blob does not inflate: {error}")))?;
    Ok(inflated)
}

/// The name an enum of the domain is stored by: its serialised form.
pub(crate) fn name<T: Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(name)) => name,
        _ => String::new(),
    }
}

pub(crate) fn parse<T: DeserializeOwned>(index: usize, name: &str) -> rusqlite::Result<T> {
    serde_json::from_value(serde_json::Value::String(name.to_owned())).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(index, rusqlite::types::Type::Text, Box::new(error))
    })
}

pub(crate) fn json<T: DeserializeOwned>(index: usize, text: Option<String>) -> rusqlite::Result<Option<T>> {
    text.map(|text| {
        serde_json::from_str(&text).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(index, rusqlite::types::Type::Text, Box::new(error))
        })
    })
    .transpose()
}

/// A migrated in-memory changes database, for the tests.
#[cfg(test)]
pub(crate) fn memory() -> Connection {
    use rusqlite_migration::Migrations;
    let mut conn = Connection::open_in_memory().unwrap();
    conn.pragma_update(None, "foreign_keys", true).unwrap();
    Migrations::from_slice(MIGRATIONS).to_latest(&mut conn).unwrap();
    conn
}

#[cfg(test)]
mod tests {
    use rusqlite_migration::Migrations;

    use super::*;

    #[test]
    fn migrations_are_valid() {
        Migrations::from_slice(MIGRATIONS).validate().unwrap();
    }

    #[test]
    fn blobs_inflate_to_what_was_deflated() {
        let text = "local a = 1\n".repeat(1000);

        let stored = deflate(text.as_bytes());

        assert!(stored.len() < text.len() / 10);
        assert_eq!(inflate(&stored).unwrap(), text.as_bytes());
    }

    #[tokio::test]
    async fn the_database_opens_with_foreign_keys_on_both_connections() {
        let dir = tempfile::tempdir().unwrap();
        let db = ChangesDatabase::open(dir.path().join("changes.sqlite3"))
            .await
            .unwrap();

        let writer: bool = db
            .maintain(|conn| -> Result<bool, StoreError> {
                Ok(conn.pragma_query_value(None, "foreign_keys", |row| row.get(0))?)
            })
            .await
            .unwrap();
        let reader: bool = db
            .read(|conn| -> Result<bool, StoreError> {
                Ok(conn.pragma_query_value(None, "foreign_keys", |row| row.get(0))?)
            })
            .await
            .unwrap();

        assert!(writer && reader);
        db.close().await.unwrap();
    }

    #[tokio::test]
    async fn a_corrupt_file_is_set_aside_and_listed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("changes.sqlite3");
        std::fs::write(&path, vec![0x42; 8192]).unwrap();

        let db = ChangesDatabase::open(&path).await.unwrap();

        assert_eq!(db.set_asides().len(), 1);
        db.close().await.unwrap();
    }
}
