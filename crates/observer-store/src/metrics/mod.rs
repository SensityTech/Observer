//! The statistics' own database, `metrics.sqlite3`: samples every five seconds, their minute and
//! hour aggregates, and FXServer's runs. It is not Observer's state: a file that cannot be read
//! is set aside and a new one started, and it is never backed up.

mod columns;
pub mod drops;
pub mod growth;
pub mod rollups;
pub mod runs;
pub mod samples;
pub mod series;
pub mod sessions;
pub mod threads;

use std::path::PathBuf;

use rusqlite::{Connection, Transaction};
use rusqlite_migration::M;
#[cfg(test)]
use rusqlite_migration::Migrations;

use crate::database::StoreError;
use crate::side::{Beside, Shape};

pub use columns::{Affinity, Column, Kind, Values};

const MIGRATIONS: &[M<'static>] = &[
    M::up(include_str!("../../migrations/metrics/0001_samples.sql")),
    M::up(include_str!("../../migrations/metrics/0002_depth.sql")),
    M::up(include_str!("../../migrations/metrics/0003_sessions.sql")),
    M::up(include_str!("../../migrations/metrics/0004_drops.sql")),
    M::up(include_str!("../../migrations/metrics/0005_ticks.sql")),
];

const SHAPE: Shape = Shape {
    name: "metrics",
    migrations: MIGRATIONS,
    foreign_keys: false,
};

/// The metrics database. One connection writes, as for the state database; reads have their own
/// beside it, which the write-ahead log allows, so that a burst of them never holds up a sample.
#[derive(Clone)]
pub struct MetricsDatabase {
    inner: Beside,
}

impl MetricsDatabase {
    /// Opens or creates the database and migrates it. A corrupt file, or one a newer Observer
    /// wrote, is renamed `metrics.sqlite3.set-aside-<time>` with its journal files, and a new one
    /// takes its place.
    pub async fn open(path: impl Into<PathBuf>) -> Result<Self, StoreError> {
        Ok(Self {
            inner: Beside::open(path.into(), SHAPE).await?,
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

    /// Runs `function` on the read-only connection, which sees what was committed last.
    pub async fn read<R, E, F>(&self, function: F) -> Result<R, E>
    where
        R: Send + 'static,
        E: From<StoreError> + Send + 'static,
        F: FnOnce(&Connection) -> Result<R, E> + Send + 'static,
    {
        self.inner.read(function).await
    }

    /// Writes the log back into the file and truncates it, so that what was overwritten leaves
    /// both.
    pub async fn checkpoint(&self) -> Result<(), StoreError> {
        self.inner
            .maintain(|conn| crate::database::checkpoint(conn))
            .await
    }

    pub async fn close(self) -> Result<(), StoreError> {
        self.inner.close().await
    }
}

/// A migrated in-memory metrics database, for the tests.
#[cfg(test)]
pub(crate) fn memory() -> Connection {
    let mut conn = Connection::open_in_memory().unwrap();
    conn.pragma_update(None, "auto_vacuum", "INCREMENTAL").unwrap();
    Migrations::from_slice(MIGRATIONS).to_latest(&mut conn).unwrap();
    conn
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::Duration;

    use super::*;

    #[test]
    fn migrations_are_valid() {
        Migrations::from_slice(MIGRATIONS).validate().unwrap();
    }

    fn table_columns(conn: &Connection, table: &str) -> Vec<(String, String)> {
        let mut statement = conn
            .prepare(&format!(
                "SELECT name, type FROM pragma_table_info('{table}') ORDER BY cid"
            ))
            .unwrap();
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    fn sql_type(affinity: Affinity) -> &'static str {
        match affinity {
            Affinity::Integer => "INTEGER",
            Affinity::Real => "REAL",
        }
    }

    #[test]
    fn the_column_list_matches_the_three_tables() {
        let conn = memory();
        let pair = |name: String, kind: &str| (name, kind.to_owned());

        let mut raw = vec![pair("ts".into(), "INTEGER"), pair("state".into(), "TEXT")];
        raw.extend(Column::ALL.map(|column| pair(column.name().into(), sql_type(column.affinity()))));
        assert_eq!(table_columns(&conn, "sample_5s"), raw);

        let mut aggregates = vec![
            pair("ts".into(), "INTEGER"),
            pair("samples".into(), "INTEGER"),
            pair("online".into(), "INTEGER"),
        ];
        for column in Column::ALL {
            let name = column.name();
            match column.kind() {
                Kind::Gauge => aggregates.extend([
                    pair(format!("{name}_avg"), "REAL"),
                    pair(format!("{name}_min"), "REAL"),
                    pair(format!("{name}_max"), "REAL"),
                    pair(format!("{name}_n"), "INTEGER"),
                ]),
                Kind::Count => aggregates.extend([
                    pair(format!("{name}_sum"), "INTEGER"),
                    pair(format!("{name}_n"), "INTEGER"),
                ]),
            }
        }
        assert_eq!(table_columns(&conn, "sample_1m"), aggregates);
        assert_eq!(table_columns(&conn, "sample_1h"), aggregates);
    }

    #[test]
    fn the_depth_migration_keeps_every_row_and_leaves_the_new_measures_unmeasured() {
        let mut conn = Connection::open_in_memory().unwrap();
        Migrations::from_slice(&MIGRATIONS[..1])
            .to_latest(&mut conn)
            .unwrap();
        conn.execute(
            "INSERT INTO sample_5s (ts, state, players, fxs_cpu) VALUES (1761642000, 'online', 12, 41.5)",
            [],
        )
        .unwrap();
        for table in ["sample_1m", "sample_1h"] {
            let counts: Vec<String> = table_columns(&conn, table)
                .into_iter()
                .map(|(name, _)| name)
                .filter(|name| name.ends_with("_n") || name == "samples" || name == "online")
                .collect();
            conn.execute(
                &format!(
                    "INSERT INTO {table} (ts, players_avg, {}) VALUES (1761642000, 12.0, {})",
                    counts.join(", "),
                    vec!["1"; counts.len()].join(", ")
                ),
                [],
            )
            .unwrap();
        }

        Migrations::from_slice(MIGRATIONS).to_latest(&mut conn).unwrap();

        let sample: (String, i64, f64, Option<f64>, Option<f64>) = conn
            .query_row(
                "SELECT state, players, fxs_cpu, fxs_cpu_kernel, host_psi_io FROM sample_5s",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .unwrap();
        assert_eq!(sample, ("online".to_owned(), 12, 41.5, None, None));
        for table in ["sample_1m", "sample_1h"] {
            let aggregate: (i64, f64, i64, Option<f64>, i64, i64) = conn
                .query_row(
                    &format!(
                        "SELECT ts, players_avg, players_n, fxs_cpu_kernel_avg, fxs_cpu_kernel_n, \
                         host_disk_ops_n FROM {table}"
                    ),
                    [],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                            row.get(5)?,
                        ))
                    },
                )
                .unwrap();
            assert_eq!(aggregate, (1_761_642_000, 12.0, 1, None, 0, 0), "{table}");
        }
        let refused = conn.execute(
            "INSERT INTO sample_5s (ts, state) VALUES (1761642005, 'paused')",
            [],
        );
        assert!(refused.is_err(), "the state is still checked");
        let strict: i64 = conn
            .query_row(
                "SELECT count(*) FROM pragma_table_list WHERE name LIKE 'sample_%' AND strict",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(strict, 3);
    }

    #[tokio::test]
    async fn a_new_database_is_in_wal_mode_with_incremental_vacuum() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("metrics.sqlite3");

        MetricsDatabase::open(&path).await.unwrap().close().await.unwrap();

        let conn = Connection::open(&path).unwrap();
        let journal: String = conn
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .unwrap();
        let vacuum: i64 = conn
            .pragma_query_value(None, "auto_vacuum", |row| row.get(0))
            .unwrap();
        assert_eq!(journal, "wal");
        assert_eq!(vacuum, 2, "incremental");
    }

    #[tokio::test]
    async fn a_read_does_not_wait_for_a_write_under_way() {
        let dir = tempfile::tempdir().unwrap();
        let db = MetricsDatabase::open(dir.path().join("metrics.sqlite3"))
            .await
            .unwrap();
        let (release, held) = std::sync::mpsc::channel::<()>();
        let (begun, writing) = tokio::sync::oneshot::channel();
        let write = tokio::spawn({
            let db = db.clone();
            async move {
                db.write(move |tx| -> Result<(), StoreError> {
                    tx.execute(
                        "INSERT INTO sample_5s (ts, state) VALUES (1761642000, 'online')",
                        [],
                    )?;
                    let _ = begun.send(());
                    let _ = held.recv();
                    Ok(())
                })
                .await
            }
        });
        writing.await.unwrap();

        let read = tokio::time::timeout(
            Duration::from_secs(2),
            db.read(|conn| -> Result<i64, StoreError> {
                Ok(conn.query_row("SELECT count(*) FROM sample_5s", [], |row| row.get(0))?)
            }),
        )
        .await;
        release.send(()).unwrap();
        write.await.unwrap().unwrap();

        assert_eq!(
            read.expect("the read waited for the write").unwrap(),
            0,
            "the write is not committed yet"
        );
        db.close().await.unwrap();
    }

    fn set_aside_files(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".set-aside-"))
            .collect();
        names.sort();
        names
    }

    #[tokio::test]
    async fn a_file_that_is_not_a_database_is_set_aside_and_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("metrics.sqlite3");
        std::fs::write(&path, vec![0x42; 8192]).unwrap();

        let db = MetricsDatabase::open(&path).await.unwrap();

        let rows: i64 = db
            .read(|conn| {
                Ok::<_, StoreError>(conn.query_row("SELECT count(*) FROM sample_5s", [], |row| row.get(0))?)
            })
            .await
            .unwrap();
        assert_eq!(rows, 0);
        let aside = set_aside_files(dir.path());
        assert_eq!(aside.len(), 1, "{aside:?}");
        assert!(aside[0].starts_with("metrics.sqlite3.set-aside-"));
        assert_eq!(
            std::fs::read(dir.path().join(&aside[0])).unwrap(),
            vec![0x42; 8192]
        );
    }

    #[tokio::test]
    async fn a_database_from_a_newer_observer_is_set_aside() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("metrics.sqlite3");
        MetricsDatabase::open(&path).await.unwrap().close().await.unwrap();
        Connection::open(&path)
            .unwrap()
            .pragma_update(None, "user_version", 99)
            .unwrap();

        MetricsDatabase::open(&path).await.unwrap().close().await.unwrap();

        let aside = set_aside_files(dir.path());
        let database = aside
            .iter()
            .find(|name| !name.ends_with("-wal") && !name.ends_with("-shm"))
            .expect("the newer database is kept aside");
        let version = |path: &Path| -> u32 {
            Connection::open(path)
                .unwrap()
                .pragma_query_value(None, "user_version", |row| row.get(0))
                .unwrap()
        };
        assert_eq!(version(&dir.path().join(database)), 99);
        assert_eq!(version(&path) as usize, MIGRATIONS.len());
    }
}
