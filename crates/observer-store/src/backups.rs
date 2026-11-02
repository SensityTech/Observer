//! The records of the game database's backups: the settings, the destinations, each run and each
//! copy of its file. Secrets are read and written only by the functions named for them; no other
//! query here touches their tables. Which copies go and when is decided in `observer-app`.

use jiff::Timestamp;
use observer_domain::backups::{
    BackupCopy, BackupDestination, BackupError, BackupOutcome, BackupRun, BackupSettings, BackupTrigger,
    BeforeRestarts, CopyState, DatabaseEngine, DeleteReason, DestinationKind, DestinationTarget,
    ManualSource, Retention, Secret, SourceKind, StoredBackupSettings, StoredCopy,
};
use rusqlite::types::{FromSqlError, Type};
use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::database::StoreError;
use crate::time::Utc;

const RUN_COLUMNS: &str = "id, trigger, actor, started_at, finished_at, outcome, engine, database_name, host, \
    client, file_name, sql_bytes, file_bytes, file_sha256, server_started, loose_tables, loose_table_count, \
    error, error_detail";

const COPY_COLUMNS: &str = "c.id, c.run_id, c.destination_id, d.name, d.kind, c.state, c.location, c.bytes, \
    c.started_at, c.stored_at, c.deleted_at, c.delete_reason, c.error, c.error_detail";

const DESTINATION_COLUMNS: &str = "d.id, d.name, d.enabled, d.target, d.public_key, d.keep_last, d.keep_days, \
    d.version, d.created_at, d.updated_at, \
    EXISTS (SELECT 1 FROM backup_destination_secrets AS s WHERE s.destination_id = d.id), \
    (SELECT count(*) FROM backup_copies AS c WHERE c.destination_id = d.id AND c.state = 'stored'), \
    (SELECT coalesce(sum(c.bytes), 0) FROM backup_copies AS c WHERE c.destination_id = d.id AND c.state = 'stored')";

/// A server's settings with their version, `None` until they are first saved.
pub fn settings(conn: &Connection, server_id: &str) -> Result<Option<StoredBackupSettings>, StoreError> {
    Ok(conn
        .query_row(
            "SELECT before_restarts, times, restart_wait_seconds, engine, source, host, port, user_name,
                    database_name, client_folder, version
             FROM backup_settings WHERE server_id = ?1",
            [server_id],
            |row| {
                let times: String = row.get(1)?;
                let manual = match row.get::<_, Option<String>>(5)? {
                    Some(host) => Some(ManualSource {
                        host,
                        port: row.get(6)?,
                        user: row.get(7)?,
                        database: row.get(8)?,
                    }),
                    None => None,
                };
                Ok(StoredBackupSettings {
                    settings: BackupSettings {
                        before_restarts: named(row, 0, BeforeRestarts::from_name)?,
                        times: serde_json::from_str(&times).map_err(|error| json_error(1, error))?,
                        restart_wait_seconds: row.get(2)?,
                        engine: named(row, 3, DatabaseEngine::from_name)?,
                        source: named(row, 4, SourceKind::from_name)?,
                        manual,
                        client_folder: row.get(9)?,
                    },
                    version: row.get(10)?,
                })
            },
        )
        .optional()?)
}

/// Saves a server's settings over the version they were read at, `None` for settings never
/// saved. Returns the new version, or `None` when someone saved them since.
pub fn save_settings(
    conn: &Connection,
    server_id: &str,
    settings: &BackupSettings,
    version: Option<i64>,
    now: Timestamp,
) -> Result<Option<i64>, StoreError> {
    let times = serde_json::to_string(&settings.times).map_err(conversion)?;
    let manual = settings.manual.as_ref();
    let (host, port, user, database) = (
        manual.map(|manual| manual.host.as_str()),
        manual.map(|manual| manual.port),
        manual.map(|manual| manual.user.as_str()),
        manual.map(|manual| manual.database.as_str()),
    );
    let fields = (
        settings.before_restarts.as_str(),
        settings.restart_wait_seconds,
        settings.engine.as_str(),
        settings.source.as_str(),
    );
    let saved = match version {
        None => conn.query_row(
            "INSERT INTO backup_settings (server_id, before_restarts, times, restart_wait_seconds, engine, source,
                 host, port, user_name, database_name, client_folder, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT (server_id) DO NOTHING
             RETURNING version",
            params![
                server_id,
                fields.0,
                times,
                fields.1,
                fields.2,
                fields.3,
                host,
                port,
                user,
                database,
                settings.client_folder,
                Utc(now)
            ],
            |row| row.get(0),
        ),
        Some(version) => conn.query_row(
            "UPDATE backup_settings
             SET before_restarts = ?2, times = ?3, restart_wait_seconds = ?4, engine = ?5, source = ?6, host = ?7,
                 port = ?8, user_name = ?9, database_name = ?10, client_folder = ?11, updated_at = ?12,
                 version = version + 1
             WHERE server_id = ?1 AND version = ?13
             RETURNING version",
            params![
                server_id,
                fields.0,
                times,
                fields.1,
                fields.2,
                fields.3,
                host,
                port,
                user,
                database,
                settings.client_folder,
                Utc(now),
                version
            ],
            |row| row.get(0),
        ),
    };
    Ok(saved.optional()?)
}

/// The manual source's password.
pub fn password(conn: &Connection, server_id: &str) -> Result<Option<Secret>, StoreError> {
    Ok(conn
        .query_row(
            "SELECT password FROM backup_source_secrets WHERE server_id = ?1",
            [server_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .map(Secret::new))
}

pub fn has_password(conn: &Connection, server_id: &str) -> Result<bool, StoreError> {
    Ok(conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM backup_source_secrets WHERE server_id = ?1)",
        [server_id],
        |row| row.get(0),
    )?)
}

/// Stores the manual source's password, after the settings it belongs to.
pub fn set_password(conn: &Connection, server_id: &str, password: &Secret) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO backup_source_secrets (server_id, password) VALUES (?1, ?2)
         ON CONFLICT (server_id) DO UPDATE SET password = excluded.password",
        params![server_id, password.expose()],
    )?;
    Ok(())
}

/// The destinations of a server, by name, with what they hold now.
pub fn destinations(conn: &Connection, server_id: &str) -> Result<Vec<BackupDestination>, StoreError> {
    let mut statement = conn.prepare(&format!(
        "SELECT {DESTINATION_COLUMNS} FROM backup_destinations AS d WHERE d.server_id = ?1 ORDER BY d.name, d.id"
    ))?;
    let rows = statement.query_map([server_id], read_destination)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn destination(
    conn: &Connection,
    server_id: &str,
    id: i64,
) -> Result<Option<BackupDestination>, StoreError> {
    Ok(conn
        .query_row(
            &format!(
                "SELECT {DESTINATION_COLUMNS} FROM backup_destinations AS d WHERE d.server_id = ?1 AND d.id = ?2"
            ),
            params![server_id, id],
            read_destination,
        )
        .optional()?)
}

/// The SFTP password or private key, or the S3 secret key, of a destination.
pub fn destination_secret(conn: &Connection, id: i64) -> Result<Option<Secret>, StoreError> {
    Ok(conn
        .query_row(
            "SELECT secret FROM backup_destination_secrets WHERE destination_id = ?1",
            [id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .map(Secret::new))
}

pub fn set_destination_secret(conn: &Connection, id: i64, secret: &Secret) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO backup_destination_secrets (destination_id, secret) VALUES (?1, ?2)
         ON CONFLICT (destination_id) DO UPDATE SET secret = excluded.secret",
        params![id, secret.expose()],
    )?;
    Ok(())
}

pub fn remove_destination_secret(conn: &Connection, id: i64) -> Result<(), StoreError> {
    conn.execute(
        "DELETE FROM backup_destination_secrets WHERE destination_id = ?1",
        [id],
    )?;
    Ok(())
}

/// A destination as it is written, its secret aside.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DestinationRow<'a> {
    pub server_id: &'a str,
    pub name: &'a str,
    pub enabled: bool,
    pub target: &'a DestinationTarget,
    pub public_key: Option<&'a str>,
    pub retention: Retention,
    pub at: Timestamp,
}

/// What became of a change to a destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Saved {
    Done,
    /// Someone changed or deleted it since it was read.
    Conflict,
    NameTaken,
}

/// Records a new destination and returns its id, or `None` when the server has one by that name.
pub fn insert_destination(conn: &Connection, row: &DestinationRow<'_>) -> Result<Option<i64>, StoreError> {
    let target = serde_json::to_string(row.target).map_err(conversion)?;
    let inserted = conn.execute(
        "INSERT INTO backup_destinations (server_id, name, enabled, kind, target, public_key, keep_last, keep_days,
             created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)
         ON CONFLICT (server_id, name) DO NOTHING",
        params![
            row.server_id,
            row.name,
            row.enabled,
            row.target.kind().as_str(),
            target,
            row.public_key,
            row.retention.keep_last,
            row.retention.keep_days,
            Utc(row.at),
        ],
    )?;
    Ok((inserted == 1).then(|| conn.last_insert_rowid()))
}

/// Saves a destination over the version it was read at.
pub fn update_destination(
    conn: &Connection,
    id: i64,
    version: i64,
    row: &DestinationRow<'_>,
) -> Result<Saved, StoreError> {
    let taken: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM backup_destinations WHERE server_id = ?1 AND name = ?2 AND id <> ?3)",
        params![row.server_id, row.name, id],
        |row| row.get(0),
    )?;
    if taken {
        return Ok(Saved::NameTaken);
    }
    let target = serde_json::to_string(row.target).map_err(conversion)?;
    let updated = conn.execute(
        "UPDATE backup_destinations
         SET name = ?4, enabled = ?5, kind = ?6, target = ?7, public_key = ?8, keep_last = ?9, keep_days = ?10,
             updated_at = ?11, version = version + 1
         WHERE id = ?1 AND server_id = ?2 AND version = ?3",
        params![
            id,
            row.server_id,
            version,
            row.name,
            row.enabled,
            row.target.kind().as_str(),
            target,
            row.public_key,
            row.retention.keep_last,
            row.retention.keep_days,
            Utc(row.at),
        ],
    )?;
    Ok(if updated == 1 {
        Saved::Done
    } else {
        Saved::Conflict
    })
}

/// Deletes a destination, its secret and the records of its copies; `false` when it is unknown.
pub fn delete_destination(conn: &Connection, server_id: &str, id: i64) -> Result<bool, StoreError> {
    Ok(conn.execute(
        "DELETE FROM backup_destinations WHERE id = ?1 AND server_id = ?2",
        params![id, server_id],
    )? == 1)
}

/// A run about to start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRun<'a> {
    pub server_id: &'a str,
    pub trigger: BackupTrigger,
    pub actor: Option<&'a str>,
    pub engine: DatabaseEngine,
    pub started_at: Timestamp,
}

pub fn insert_run(conn: &Connection, run: &NewRun<'_>) -> Result<i64, StoreError> {
    conn.execute(
        "INSERT INTO backup_runs (server_id, trigger, actor, engine, started_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            run.server_id,
            run.trigger.as_str(),
            run.actor,
            run.engine.as_str(),
            Utc(run.started_at)
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// What a run dumps, once its source and tool are known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunSource<'a> {
    pub database: &'a str,
    pub host: &'a str,
    pub client: &'a str,
    pub file_name: &'a str,
}

pub fn set_run_source(conn: &Connection, id: i64, source: &RunSource<'_>) -> Result<(), StoreError> {
    conn.execute(
        "UPDATE backup_runs SET database_name = ?2, host = ?3, client = ?4, file_name = ?5
         WHERE id = ?1 AND finished_at IS NULL",
        params![id, source.database, source.host, source.client, source.file_name],
    )?;
    Ok(())
}

/// How a run ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinishedRun<'a> {
    pub at: Timestamp,
    pub outcome: BackupOutcome,
    pub sql_bytes: Option<u64>,
    pub file_bytes: Option<u64>,
    pub sql_sha256: Option<&'a str>,
    pub file_sha256: Option<&'a str>,
    pub server_started: bool,
    pub loose_tables: &'a [String],
    pub loose_table_count: u32,
    pub error: Option<BackupError>,
    pub error_detail: Option<&'a str>,
}

/// Records how a run ended; `false` when it is unknown or ended already.
pub fn finish_run(conn: &Connection, id: i64, finished: &FinishedRun<'_>) -> Result<bool, StoreError> {
    let loose = serde_json::to_string(finished.loose_tables).map_err(conversion)?;
    Ok(conn.execute(
        "UPDATE backup_runs
         SET finished_at = ?2, outcome = ?3, sql_bytes = ?4, file_bytes = ?5, sql_sha256 = ?6, file_sha256 = ?7,
             server_started = ?8, loose_tables = ?9, loose_table_count = ?10, error = ?11, error_detail = ?12
         WHERE id = ?1 AND finished_at IS NULL",
        params![
            id,
            Utc(finished.at),
            finished.outcome.as_str(),
            finished.sql_bytes.map(sql_size),
            finished.file_bytes.map(sql_size),
            finished.sql_sha256,
            finished.file_sha256,
            finished.server_started,
            loose,
            finished.loose_table_count,
            finished.error.map(BackupError::as_str),
            finished.error_detail,
        ],
    )? == 1)
}

/// A run an earlier Observer left unfinished.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Interrupted {
    pub id: i64,
    pub trigger: BackupTrigger,
    pub actor: Option<String>,
    pub database: Option<String>,
}

/// Ends the runs that never ended as interrupted, and their copies under way as failed, and
/// returns them: the Observer that ran them stopped before they could.
pub fn end_unfinished_runs(conn: &Connection, at: Timestamp) -> Result<Vec<Interrupted>, StoreError> {
    let interrupted = BackupError::Interrupted.as_str();
    conn.execute(
        "UPDATE backup_copies SET state = 'failed', error = ?1
         WHERE state = 'uploading' AND run_id IN (SELECT id FROM backup_runs WHERE finished_at IS NULL)",
        [interrupted],
    )?;
    let mut statement = conn.prepare(
        "UPDATE backup_runs SET finished_at = ?1, outcome = 'interrupted', error = ?2
         WHERE finished_at IS NULL
         RETURNING id, trigger, actor, database_name",
    )?;
    let rows = statement.query_map(params![Utc(at), interrupted], |row| {
        Ok(Interrupted {
            id: row.get(0)?,
            trigger: named(row, 1, BackupTrigger::from_name)?,
            actor: row.get(2)?,
            database: row.get(3)?,
        })
    })?;
    let mut ended: Vec<Interrupted> = rows.collect::<Result<_, _>>()?;
    ended.sort_by_key(|run| run.id);
    Ok(ended)
}

/// A server's runs before the run `before`, the newest first, with their copies.
pub fn runs(
    conn: &Connection,
    server_id: &str,
    before: Option<i64>,
    limit: u32,
) -> Result<Vec<BackupRun>, StoreError> {
    let mut statement = conn.prepare(&format!(
        "SELECT {RUN_COLUMNS} FROM backup_runs WHERE server_id = ?1 AND id < ?2 ORDER BY id DESC LIMIT ?3"
    ))?;
    let rows = statement.query_map(params![server_id, before.unwrap_or(i64::MAX), limit], read_run)?;
    let mut runs: Vec<BackupRun> = rows.collect::<Result<_, _>>()?;
    let Some(oldest) = runs.last().map(|run| run.id) else {
        return Ok(runs);
    };
    let newest = runs[0].id;
    let mut statement = conn.prepare(&format!(
        "SELECT {COPY_COLUMNS} FROM backup_copies AS c JOIN backup_destinations AS d ON d.id = c.destination_id
         WHERE c.run_id BETWEEN ?1 AND ?2 ORDER BY c.id"
    ))?;
    for copy in statement.query_map(params![oldest, newest], read_copy)? {
        let (run_id, copy) = copy?;
        if let Some(run) = runs.iter_mut().find(|run| run.id == run_id) {
            run.copies.push(copy);
        }
    }
    Ok(runs)
}

pub fn run(conn: &Connection, server_id: &str, id: i64) -> Result<Option<BackupRun>, StoreError> {
    let Some(mut run) = conn
        .query_row(
            &format!("SELECT {RUN_COLUMNS} FROM backup_runs WHERE server_id = ?1 AND id = ?2"),
            params![server_id, id],
            read_run,
        )
        .optional()?
    else {
        return Ok(None);
    };
    let mut statement = conn.prepare(&format!(
        "SELECT {COPY_COLUMNS} FROM backup_copies AS c JOIN backup_destinations AS d ON d.id = c.destination_id
         WHERE c.run_id = ?1 ORDER BY c.id"
    ))?;
    for copy in statement.query_map([id], read_copy)? {
        run.copies.push(copy?.1);
    }
    Ok(Some(run))
}

/// Records a copy as its upload begins, and returns its id.
pub fn insert_copy(
    conn: &Connection,
    run: i64,
    destination: i64,
    location: &str,
    at: Timestamp,
) -> Result<i64, StoreError> {
    conn.execute(
        "INSERT INTO backup_copies (run_id, destination_id, state, location, started_at)
         VALUES (?1, ?2, 'uploading', ?3, ?4)",
        params![run, destination, location, Utc(at)],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn copy_stored(conn: &Connection, id: i64, bytes: u64, at: Timestamp) -> Result<(), StoreError> {
    conn.execute(
        "UPDATE backup_copies SET state = 'stored', bytes = ?2, stored_at = ?3 WHERE id = ?1 AND state = 'uploading'",
        params![id, sql_size(bytes), Utc(at)],
    )?;
    Ok(())
}

pub fn copy_failed(
    conn: &Connection,
    id: i64,
    error: BackupError,
    detail: Option<&str>,
) -> Result<(), StoreError> {
    conn.execute(
        "UPDATE backup_copies SET state = 'failed', error = ?2, error_detail = ?3 WHERE id = ?1 AND state = 'uploading'",
        params![id, error.as_str(), detail],
    )?;
    Ok(())
}

pub fn copy_deleted(
    conn: &Connection,
    id: i64,
    reason: DeleteReason,
    at: Timestamp,
) -> Result<(), StoreError> {
    conn.execute(
        "UPDATE backup_copies SET state = 'deleted', deleted_at = ?2, delete_reason = ?3
         WHERE id = ?1 AND state = 'stored'",
        params![id, Utc(at), reason.as_str()],
    )?;
    Ok(())
}

/// A copy, with what reading or deleting it needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyRow {
    pub id: i64,
    pub run: i64,
    pub destination_id: i64,
    pub kind: DestinationKind,
    pub state: CopyState,
    pub location: String,
    pub bytes: Option<u64>,
    pub stored_at: Option<Timestamp>,
    pub file_name: Option<String>,
}

const COPY_ROW: &str = "SELECT c.id, c.run_id, c.destination_id, d.kind, c.state, c.location, c.bytes, c.stored_at, \
    r.file_name FROM backup_copies AS c JOIN backup_destinations AS d ON d.id = c.destination_id \
    JOIN backup_runs AS r ON r.id = c.run_id";

pub fn copy(conn: &Connection, server_id: &str, id: i64) -> Result<Option<CopyRow>, StoreError> {
    Ok(conn
        .query_row(
            &format!("{COPY_ROW} WHERE c.id = ?1 AND r.server_id = ?2"),
            params![id, server_id],
            read_copy_row,
        )
        .optional()?)
}

/// The copies a destination holds now, which its retention looks at.
pub fn stored_copies(conn: &Connection, destination: i64) -> Result<Vec<CopyRow>, StoreError> {
    let mut statement = conn.prepare(&format!(
        "{COPY_ROW} WHERE c.destination_id = ?1 AND c.state = 'stored' ORDER BY c.stored_at, c.id"
    ))?;
    let rows = statement.query_map([destination], read_copy_row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// The copies of a run stored anywhere.
pub fn run_copies(conn: &Connection, run: i64) -> Result<Vec<CopyRow>, StoreError> {
    let mut statement = conn.prepare(&format!(
        "{COPY_ROW} WHERE c.run_id = ?1 AND c.state = 'stored' ORDER BY c.id"
    ))?;
    let rows = statement.query_map([run], read_copy_row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

impl CopyRow {
    /// The copy as the retention sees it; `None` for a copy that is not stored.
    pub fn stored(&self) -> Option<StoredCopy> {
        Some(StoredCopy {
            id: self.id,
            stored_at: self.stored_at?,
        })
    }
}

/// Forgets the runs of a server that ended before `cutoff` and keep no stored copy, and says how
/// many.
pub fn forget_runs(conn: &Connection, server_id: &str, cutoff: Timestamp) -> Result<usize, StoreError> {
    Ok(conn.execute(
        "DELETE FROM backup_runs
         WHERE server_id = ?1 AND finished_at IS NOT NULL AND finished_at < ?2
             AND NOT EXISTS (SELECT 1 FROM backup_copies AS c WHERE c.run_id = backup_runs.id AND c.state = 'stored')",
        params![server_id, Utc(cutoff)],
    )?)
}

fn read_destination(row: &Row<'_>) -> rusqlite::Result<BackupDestination> {
    let target: String = row.get(3)?;
    Ok(BackupDestination {
        id: row.get(0)?,
        name: row.get(1)?,
        enabled: row.get(2)?,
        target: serde_json::from_str(&target).map_err(|error| json_error(3, error))?,
        public_key: row.get(4)?,
        retention: Retention {
            keep_last: row.get(5)?,
            keep_days: row.get(6)?,
        },
        version: row.get(7)?,
        created_at: row.get::<_, Utc>(8)?.0,
        updated_at: row.get::<_, Utc>(9)?.0,
        has_secret: row.get(10)?,
        stored: row.get(11)?,
        stored_bytes: size(row, 12)?,
    })
}

fn read_run(row: &Row<'_>) -> rusqlite::Result<BackupRun> {
    let loose: String = row.get(15)?;
    Ok(BackupRun {
        id: row.get(0)?,
        trigger: named(row, 1, BackupTrigger::from_name)?,
        actor: row.get(2)?,
        started_at: row.get::<_, Utc>(3)?.0,
        finished_at: row.get::<_, Option<Utc>>(4)?.map(|at| at.0),
        outcome: maybe_named(row, 5, BackupOutcome::from_name)?,
        engine: named(row, 6, DatabaseEngine::from_name)?,
        database: row.get(7)?,
        host: row.get(8)?,
        client: row.get(9)?,
        file_name: row.get(10)?,
        sql_bytes: maybe_size(row, 11)?,
        file_bytes: maybe_size(row, 12)?,
        file_sha256: row.get(13)?,
        server_started: row.get(14)?,
        loose_tables: serde_json::from_str(&loose).map_err(|error| json_error(15, error))?,
        loose_table_count: row.get(16)?,
        error: maybe_named(row, 17, BackupError::from_name)?,
        error_detail: row.get(18)?,
        copies: Vec::new(),
    })
}

fn read_copy(row: &Row<'_>) -> rusqlite::Result<(i64, BackupCopy)> {
    Ok((
        row.get(1)?,
        BackupCopy {
            id: row.get(0)?,
            destination_id: row.get(2)?,
            destination: row.get(3)?,
            kind: named(row, 4, DestinationKind::from_name)?,
            state: named(row, 5, CopyState::from_name)?,
            location: row.get(6)?,
            bytes: maybe_size(row, 7)?,
            started_at: row.get::<_, Utc>(8)?.0,
            stored_at: row.get::<_, Option<Utc>>(9)?.map(|at| at.0),
            deleted_at: row.get::<_, Option<Utc>>(10)?.map(|at| at.0),
            delete_reason: maybe_named(row, 11, DeleteReason::from_name)?,
            error: maybe_named(row, 12, BackupError::from_name)?,
            error_detail: row.get(13)?,
        },
    ))
}

fn read_copy_row(row: &Row<'_>) -> rusqlite::Result<CopyRow> {
    Ok(CopyRow {
        id: row.get(0)?,
        run: row.get(1)?,
        destination_id: row.get(2)?,
        kind: named(row, 3, DestinationKind::from_name)?,
        state: named(row, 4, CopyState::from_name)?,
        location: row.get(5)?,
        bytes: maybe_size(row, 6)?,
        stored_at: row.get::<_, Option<Utc>>(7)?.map(|at| at.0),
        file_name: row.get(8)?,
    })
}

/// SQLite integers are signed; no file reaches 2^63 bytes.
fn sql_size(bytes: u64) -> i64 {
    i64::try_from(bytes).unwrap_or(i64::MAX)
}

fn size(row: &Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let bytes: i64 = row.get(index)?;
    u64::try_from(bytes).map_err(|_| malformed(index, format!("negative size {bytes}")))
}

fn maybe_size(row: &Row<'_>, index: usize) -> rusqlite::Result<Option<u64>> {
    match row.get::<_, Option<i64>>(index)? {
        Some(bytes) => u64::try_from(bytes)
            .map(Some)
            .map_err(|_| malformed(index, format!("negative size {bytes}"))),
        None => Ok(None),
    }
}

fn conversion(error: serde_json::Error) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(error))
}

fn json_error(index: usize, error: serde_json::Error) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(index, Type::Text, Box::new(error))
}

fn malformed(index: usize, what: String) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(index, Type::Text, Box::new(FromSqlError::Other(what.into())))
}

fn named<T>(row: &Row<'_>, index: usize, parse: fn(&str) -> Option<T>) -> rusqlite::Result<T> {
    let name: String = row.get(index)?;
    parse(&name).ok_or_else(|| malformed(index, format!("unknown name {name:?}")))
}

fn maybe_named<T>(row: &Row<'_>, index: usize, parse: fn(&str) -> Option<T>) -> rusqlite::Result<Option<T>> {
    match row.get::<_, Option<String>>(index)? {
        Some(name) => parse(&name)
            .map(Some)
            .ok_or_else(|| malformed(index, format!("unknown name {name:?}"))),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests;
