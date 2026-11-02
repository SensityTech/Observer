//! The server tools' records: each tool's settings, its runs, and the journal of the files it changed
//! in a server folder. A file's record stays open while the file holds a tool's change; each write is
//! recorded before its temporary file exists and again once its rename landed, so that a crash never
//! leaves a change unrecorded. When to heal, what to undo and what a run may write are decided in
//! `observer-app` and `observer-tools`.

use jiff::Timestamp;
use observer_domain::tools::{
    ArchetypeReport, Differences, EscrowPolicy, ToolChange, ToolChangeState, ToolCloseReason, ToolFileFormat,
    ToolKind, ToolMode, ToolRemoval, ToolRun, ToolRunMode, ToolRunOutcome, ToolRunSummary, ToolSettings,
    ToolTrigger, sha256_from_hex, sha256_hex,
};
use rusqlite::types::{FromSqlError, Type};
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};
use tracing::warn;

use crate::database::StoreError;
use crate::files;
use crate::time::Utc;

/// Runs kept per server and tool, besides those a change names.
pub const RUNS_KEPT: u32 = 50;

const RUN_COLUMNS: &str =
    "id, tool, trigger, actor, mode, started_at, finished_at, outcome, summary, plan_sha256";

const CHANGE_COLUMNS: &str = "id, server_id, tool, folder_path, folder_id, path, resource, format, \
    original_sha256, patched_sha256, removals, state, pending_temporary, pending_sha256, pending_removals, \
    pending_run, close_reason, opened_run, changed_run, closed_run, opened_at, changed_at, closed_at";

/// Whether a change names the run of the row `tool_runs.id`: it opened, wrote, closed or is writing
/// the change, or took out one of its entries.
const NAMED: &str = "EXISTS (SELECT 1 FROM tool_changes AS c
    WHERE tool_runs.id IN (c.opened_run, c.changed_run, c.closed_run, c.pending_run)
        OR EXISTS (SELECT 1 FROM json_each(c.removals)
                   WHERE json_extract(value, '$.run') = tool_runs.id)
        OR EXISTS (SELECT 1 FROM json_each(c.pending_removals)
                   WHERE json_extract(value, '$.run') = tool_runs.id))";

/// A tool's settings and their version: the defaults and no version until they are first saved.
pub fn settings(
    conn: &Connection,
    server_id: &str,
    tool: ToolKind,
) -> Result<(ToolSettings, Option<i64>), StoreError> {
    let stored = conn
        .query_row(
            "SELECT at_start, differences, escrowed, excluded, version FROM tool_settings
             WHERE server_id = ?1 AND tool = ?2",
            params![server_id, tool.as_str()],
            |row| Ok((read_settings(row)?, row.get::<_, i64>(4)?)),
        )
        .optional()?;
    Ok(stored.map_or_else(
        || (ToolSettings::default(), None),
        |(settings, version)| (settings, Some(version)),
    ))
}

/// Saves a tool's settings over the version they were read at, `None` for settings never saved.
/// Returns the new version, or `None` when someone saved them since.
pub fn save_settings(
    conn: &Connection,
    server_id: &str,
    tool: ToolKind,
    settings: &ToolSettings,
    version: Option<i64>,
    now: Timestamp,
) -> Result<Option<i64>, StoreError> {
    let excluded = serde_json::to_string(&settings.excluded).map_err(conversion)?;
    let (at_start, differences, escrowed) = (
        settings.at_start.as_str(),
        settings.differences.as_str(),
        settings.escrowed.as_str(),
    );
    let saved = match version {
        None => conn.query_row(
            "INSERT INTO tool_settings (server_id, tool, at_start, differences, escrowed, excluded,
                 updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT (server_id, tool) DO NOTHING
             RETURNING version",
            params![
                server_id,
                tool.as_str(),
                at_start,
                differences,
                escrowed,
                excluded,
                Utc(now)
            ],
            |row| row.get(0),
        ),
        Some(version) => conn.query_row(
            "UPDATE tool_settings
             SET at_start = ?3, differences = ?4, escrowed = ?5, excluded = ?6, version = version + 1,
                 updated_at = ?7
             WHERE server_id = ?1 AND tool = ?2 AND version = ?8
             RETURNING version",
            params![
                server_id,
                tool.as_str(),
                at_start,
                differences,
                escrowed,
                excluded,
                Utc(now),
                version
            ],
            |row| row.get(0),
        ),
    };
    Ok(saved.optional()?)
}

/// A run about to start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRun<'a> {
    pub server_id: &'a str,
    pub tool: ToolKind,
    pub trigger: ToolTrigger,
    /// The account that asked; `None` when Observer runs it by itself.
    pub actor: Option<&'a str>,
    pub mode: ToolRunMode,
    pub started_at: Timestamp,
}

/// How a run ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinishedRun<'a> {
    pub at: Timestamp,
    pub outcome: ToolRunOutcome,
    pub summary: &'a ToolRunSummary,
    /// The SHA-256 of the plan the report holds.
    pub plan: Option<[u8; 32]>,
    pub report: Option<&'a ArchetypeReport>,
}

/// Records a run as it starts, and returns its id.
pub fn insert_run(conn: &Connection, run: &NewRun<'_>) -> Result<i64, StoreError> {
    conn.execute(
        "INSERT INTO tool_runs (server_id, tool, trigger, actor, mode, started_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            run.server_id,
            run.tool.as_str(),
            run.trigger.as_str(),
            run.actor,
            run.mode.as_str(),
            Utc(run.started_at),
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Records how a run ended, then applies the retention to the runs of its server and tool. `false`
/// when the run is unknown or ended already.
pub fn finish_run(tx: &Transaction<'_>, id: i64, finished: &FinishedRun<'_>) -> Result<bool, StoreError> {
    let summary = serde_json::to_string(finished.summary).map_err(conversion)?;
    let report = finished
        .report
        .map(serde_json::to_string)
        .transpose()
        .map_err(conversion)?;
    let ended: Option<(String, String)> = tx
        .query_row(
            "UPDATE tool_runs SET finished_at = ?2, outcome = ?3, summary = ?4, plan_sha256 = ?5, report = ?6
             WHERE id = ?1 AND finished_at IS NULL
             RETURNING server_id, tool",
            params![
                id,
                Utc(finished.at),
                finished.outcome.as_str(),
                summary,
                finished.plan.as_ref().map(sha256_hex),
                report,
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((server_id, tool)) = ended else {
        return Ok(false);
    };
    prune(tx, &server_id, &tool, RUNS_KEPT)?;
    Ok(true)
}

/// Ends as failed the runs started before `started_before` that never ended, and says how many: the
/// Observer that ran them stopped before they could.
pub fn end_unfinished_runs(
    conn: &Connection,
    started_before: Timestamp,
    at: Timestamp,
) -> Result<usize, StoreError> {
    Ok(conn.execute(
        "UPDATE tool_runs SET finished_at = ?2, outcome = 'failed'
         WHERE finished_at IS NULL AND started_at < ?1",
        params![Utc(started_before), Utc(at)],
    )?)
}

/// The latest runs of a tool, the newest first, without their reports.
pub fn runs(
    conn: &Connection,
    server_id: &str,
    tool: ToolKind,
    limit: u32,
) -> Result<Vec<ToolRun>, StoreError> {
    let mut statement = conn.prepare(&format!(
        "SELECT {RUN_COLUMNS}, NULL FROM tool_runs WHERE server_id = ?1 AND tool = ?2
         ORDER BY id DESC LIMIT ?3"
    ))?;
    let rows = statement.query_map(params![server_id, tool.as_str(), limit], read_run)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// A run, with its report while it is kept.
pub fn run(conn: &Connection, id: i64) -> Result<Option<ToolRun>, StoreError> {
    Ok(conn
        .query_row(
            &format!("SELECT {RUN_COLUMNS}, report FROM tool_runs WHERE id = ?1"),
            [id],
            read_run,
        )
        .optional()?)
}

/// The latest run of a tool that still holds a report, with it.
pub fn last_report(
    conn: &Connection,
    server_id: &str,
    tool: ToolKind,
) -> Result<Option<ToolRun>, StoreError> {
    Ok(conn
        .query_row(
            &format!(
                "SELECT {RUN_COLUMNS}, report FROM tool_runs
                 WHERE server_id = ?1 AND tool = ?2 AND report IS NOT NULL ORDER BY id DESC LIMIT 1"
            ),
            params![server_id, tool.as_str()],
            read_run,
        )
        .optional()?)
}

/// Deletes a tool's runs past the latest `keep`, except those a change names, and clears the report
/// of every run but the latest that holds one and those a change names. Returns the runs deleted.
pub fn prune_runs(
    tx: &Transaction<'_>,
    server_id: &str,
    tool: ToolKind,
    keep: u32,
) -> Result<usize, StoreError> {
    prune(tx, server_id, tool.as_str(), keep)
}

fn prune(conn: &Connection, server_id: &str, tool: &str, keep: u32) -> Result<usize, StoreError> {
    let deleted = conn.execute(
        &format!(
            "DELETE FROM tool_runs
             WHERE server_id = ?1 AND tool = ?2
                 AND id NOT IN (SELECT id FROM tool_runs WHERE server_id = ?1 AND tool = ?2
                                ORDER BY id DESC LIMIT ?3)
                 AND NOT {NAMED}"
        ),
        params![server_id, tool, keep],
    )?;
    conn.execute(
        &format!(
            "UPDATE tool_runs SET report = NULL
             WHERE server_id = ?1 AND tool = ?2 AND report IS NOT NULL
                 AND id < (SELECT max(id) FROM tool_runs
                           WHERE server_id = ?1 AND tool = ?2 AND report IS NOT NULL)
                 AND NOT {NAMED}"
        ),
        params![server_id, tool],
    )?;
    Ok(deleted)
}

/// A record of the journal, whole.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolChangeRow {
    pub id: i64,
    pub server_id: String,
    pub tool: ToolKind,
    /// The server folder's final path when the record opened.
    pub folder_path: String,
    /// The server folder's identity, which heal and undo go by.
    pub folder_id: String,
    /// Relative to the server folder, with `/`.
    pub path: String,
    pub resource: String,
    pub format: ToolFileFormat,
    /// The file before Observer touched it: its backup's name.
    pub original_sha256: [u8; 32],
    /// What the file holds while the change is applied; `None` until the first write landed.
    pub patched_sha256: Option<[u8; 32]>,
    /// Every entry out of the file since the latest write that landed, positions in the original.
    pub removals: Vec<ToolRemoval>,
    pub state: ToolChangeState,
    /// The write under way, while the record is pending.
    pub pending: Option<PendingWrite>,
    pub close_reason: Option<ToolCloseReason>,
    pub opened_run: i64,
    /// The run of the latest write that landed.
    pub changed_run: Option<i64>,
    pub closed_run: Option<i64>,
    pub opened_at: Timestamp,
    pub changed_at: Option<Timestamp>,
    pub closed_at: Option<Timestamp>,
}

impl ToolChangeRow {
    /// The record as the Tools page lists it.
    pub fn view(&self) -> ToolChange {
        ToolChange {
            id: self.id,
            folder: self.folder_path.clone(),
            path: self.path.clone(),
            resource: self.resource.clone(),
            format: self.format,
            removals: self.removals.clone(),
            state: self.state,
            close_reason: self.close_reason,
            backup: sha256_hex(&self.original_sha256),
            opened_at: self.opened_at,
            changed_at: self.changed_at,
            closed_at: self.closed_at,
        }
    }
}

/// A write under way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingWrite {
    /// The temporary file's name, in the changed file's folder.
    pub temporary: String,
    /// What the file holds once the rename lands.
    pub sha256: [u8; 32],
    /// Every entry out of the file then.
    pub removals: Vec<ToolRemoval>,
    pub run: i64,
}

/// A write about to happen, recorded before its temporary file is created.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BeginWrite<'a> {
    pub server_id: &'a str,
    pub tool: ToolKind,
    pub run: i64,
    /// The server folder's final path, under which the upload of the temporary file is recorded.
    pub folder_path: &'a str,
    pub folder_id: &'a str,
    /// Relative to the server folder, with `/`.
    pub path: &'a str,
    pub resource: &'a str,
    pub format: ToolFileFormat,
    /// The file before Observer touched it; for an open record, the one it holds.
    pub original_sha256: [u8; 32],
    /// The temporary file's name, in the changed file's folder.
    pub temporary: &'a str,
    /// What the file holds once the rename lands.
    pub sha256: [u8; 32],
    /// Every entry out of the file then, positions in the original.
    pub removals: &'a [ToolRemoval],
    /// The file's open record, which must be applied; `None` for a file that has none.
    pub existing: Option<i64>,
    pub at: Timestamp,
}

/// The open and the recently closed records, as the Tools page lists them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChangeList {
    pub open: Vec<ToolChangeRow>,
    pub closed: Vec<ToolChangeRow>,
}

/// Records a write before its temporary file exists: a new record, or the file's applied one moved
/// to pending, and the temporary file as an upload so that a start removes a leftover. Returns the
/// record's id, or `None` when the journal is not as the caller read it: the file has an open record
/// it did not name, or the record it named is not applied, not this file's or of another original.
pub fn begin(tx: &Transaction<'_>, write: &BeginWrite<'_>) -> Result<Option<i64>, StoreError> {
    let removals = serde_json::to_string(write.removals).map_err(conversion)?;
    let id = match write.existing {
        Some(existing) => tx
            .query_row(
                "UPDATE tool_changes
                 SET state = 'pending', pending_temporary = ?7, pending_sha256 = ?8, pending_removals = ?9,
                     pending_run = ?10
                 WHERE id = ?1 AND state = 'applied' AND server_id = ?2 AND tool = ?3 AND folder_id = ?4
                     AND path = ?5 AND original_sha256 = ?6
                 RETURNING id",
                params![
                    existing,
                    write.server_id,
                    write.tool.as_str(),
                    write.folder_id,
                    write.path,
                    sha256_hex(&write.original_sha256),
                    write.temporary,
                    sha256_hex(&write.sha256),
                    removals,
                    write.run,
                ],
                |row| row.get(0),
            )
            .optional()?,
        None => tx
            .query_row(
                "INSERT INTO tool_changes (server_id, tool, folder_path, folder_id, path, resource, format,
                     original_sha256, state, pending_temporary, pending_sha256, pending_removals, pending_run,
                     opened_run, opened_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'pending', ?9, ?10, ?11, ?12, ?12, ?13)
                 ON CONFLICT DO NOTHING
                 RETURNING id",
                params![
                    write.server_id,
                    write.tool.as_str(),
                    write.folder_path,
                    write.folder_id,
                    write.path,
                    write.resource,
                    write.format.as_str(),
                    sha256_hex(&write.original_sha256),
                    write.temporary,
                    sha256_hex(&write.sha256),
                    removals,
                    write.run,
                    Utc(write.at),
                ],
                |row| row.get(0),
            )
            .optional()?,
    };
    if id.is_some() {
        files::record_upload(
            tx,
            write.server_id,
            write.folder_path,
            parent(write.path),
            write.temporary,
            write.at,
        )?;
    }
    Ok(id)
}

/// The write's rename landed: the record holds what the write made, and its upload is forgotten.
/// `false` when the record is not pending.
pub fn applied(tx: &Transaction<'_>, id: i64, at: Timestamp) -> Result<bool, StoreError> {
    if !forget_pending_upload(tx, id)? {
        return Ok(false);
    }
    tx.execute(
        "UPDATE tool_changes
         SET state = 'applied', patched_sha256 = pending_sha256, removals = pending_removals,
             changed_run = pending_run, changed_at = ?2,
             pending_temporary = NULL, pending_sha256 = NULL, pending_removals = NULL, pending_run = NULL
         WHERE id = ?1",
        params![id, Utc(at)],
    )?;
    Ok(true)
}

/// The write did not happen: the record goes back to what it was before it, or is closed as
/// abandoned if the write was its first, and its upload is forgotten. `false` when the record is
/// not pending.
pub fn abandon(tx: &Transaction<'_>, id: i64, at: Timestamp) -> Result<bool, StoreError> {
    if !forget_pending_upload(tx, id)? {
        return Ok(false);
    }
    tx.execute(
        "UPDATE tool_changes
         SET state = CASE WHEN patched_sha256 IS NULL THEN 'closed' ELSE 'applied' END,
             close_reason = CASE WHEN patched_sha256 IS NULL THEN 'abandoned' END,
             closed_run = CASE WHEN patched_sha256 IS NULL THEN pending_run END,
             closed_at = CASE WHEN patched_sha256 IS NULL THEN ?2 END,
             pending_temporary = NULL, pending_sha256 = NULL, pending_removals = NULL, pending_run = NULL
         WHERE id = ?1",
        params![id, Utc(at)],
    )?;
    Ok(true)
}

/// Closes an open record, pending or applied, for `reason`; a pending write's upload is forgotten.
/// `false` when the record is not open.
pub fn close(
    tx: &Transaction<'_>,
    id: i64,
    run: i64,
    reason: ToolCloseReason,
    at: Timestamp,
) -> Result<bool, StoreError> {
    let open: Option<bool> = tx
        .query_row(
            "SELECT state = 'pending' FROM tool_changes WHERE id = ?1 AND state IN ('pending', 'applied')",
            [id],
            |row| row.get(0),
        )
        .optional()?;
    match open {
        None => return Ok(false),
        Some(true) => {
            forget_pending_upload(tx, id)?;
        }
        Some(false) => {}
    }
    tx.execute(
        "UPDATE tool_changes
         SET state = 'closed', close_reason = ?2, closed_run = ?3, closed_at = ?4,
             pending_temporary = NULL, pending_sha256 = NULL, pending_removals = NULL, pending_run = NULL
         WHERE id = ?1",
        params![id, reason.as_str(), run, Utc(at)],
    )?;
    Ok(true)
}

/// Forgets the upload of a pending record's temporary file; `false` when the record is not pending.
fn forget_pending_upload(tx: &Transaction<'_>, id: i64) -> Result<bool, StoreError> {
    let pending: Option<(String, String, String)> = tx
        .query_row(
            "SELECT folder_path, path, pending_temporary FROM tool_changes
             WHERE id = ?1 AND state = 'pending'",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let Some((root, path, temporary)) = pending else {
        return Ok(false);
    };
    tx.execute(
        "DELETE FROM file_uploads WHERE root = ?1 AND folder = ?2 AND name = ?3",
        params![root, parent(&path), temporary],
    )?;
    Ok(true)
}

/// The folder of a path relative to the server folder, as the uploads record it.
fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(folder, _)| folder)
}

/// A record by its id, open or closed.
pub fn change(conn: &Connection, id: i64) -> Result<Option<ToolChangeRow>, StoreError> {
    Ok(conn
        .query_row(
            &format!("SELECT {CHANGE_COLUMNS} FROM tool_changes WHERE id = ?1"),
            [id],
            read_change,
        )
        .optional()?)
}

/// The open records of a server folder, found by its identity whatever its path: pending and
/// applied ones, by path.
pub fn open_changes(
    conn: &Connection,
    server_id: &str,
    tool: ToolKind,
    folder_id: &str,
) -> Result<Vec<ToolChangeRow>, StoreError> {
    let mut statement = conn.prepare(&format!(
        "SELECT {CHANGE_COLUMNS} FROM tool_changes
         WHERE server_id = ?1 AND tool = ?2 AND folder_id = ?3 AND state IN ('pending', 'applied')
         ORDER BY path, id"
    ))?;
    let rows = statement.query_map(params![server_id, tool.as_str(), folder_id], read_change)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Whether a file of any server folder still holds a change of the tool.
pub fn any_open(conn: &Connection, server_id: &str, tool: ToolKind) -> Result<bool, StoreError> {
    Ok(conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM tool_changes
         WHERE server_id = ?1 AND tool = ?2 AND state IN ('pending', 'applied'))",
        params![server_id, tool.as_str()],
        |row| row.get(0),
    )?)
}

/// What the Tools page lists: the open records of every server folder, by path, and the records
/// closed at `closed_since` or later, the latest first, at most `closed_limit` of them.
pub fn changes(
    conn: &Connection,
    server_id: &str,
    tool: ToolKind,
    closed_since: Timestamp,
    closed_limit: u32,
) -> Result<ChangeList, StoreError> {
    let mut open = conn.prepare(&format!(
        "SELECT {CHANGE_COLUMNS} FROM tool_changes
         WHERE server_id = ?1 AND tool = ?2 AND state IN ('pending', 'applied')
         ORDER BY path, folder_id, id"
    ))?;
    let open = open
        .query_map(params![server_id, tool.as_str()], read_change)?
        .collect::<Result<_, _>>()?;
    let mut closed = conn.prepare(&format!(
        "SELECT {CHANGE_COLUMNS} FROM tool_changes
         WHERE server_id = ?1 AND tool = ?2 AND state = 'closed' AND closed_at >= ?3
         ORDER BY closed_at DESC, id DESC LIMIT ?4"
    ))?;
    let closed = closed
        .query_map(
            params![server_id, tool.as_str(), Utc(closed_since), closed_limit],
            read_change,
        )?
        .collect::<Result<_, _>>()?;
    Ok(ChangeList { open, closed })
}

/// The originals the journal still needs, by SHA-256, across servers and tools: those of the open
/// records and of the records closed at `closed_since` or later. Any other backup can go.
pub fn referenced_backups(conn: &Connection, closed_since: Timestamp) -> Result<Vec<[u8; 32]>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT DISTINCT original_sha256 FROM tool_changes
         WHERE state <> 'closed' OR closed_at >= ?1
         ORDER BY original_sha256",
    )?;
    let rows = statement.query_map([Utc(closed_since)], |row| digest(row, 0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Deletes the records closed before `closed_before`, and says how many: their backups are gone by
/// then, and the runs they named fall back under the runs' retention.
pub fn forget_closed_changes(conn: &Connection, closed_before: Timestamp) -> Result<usize, StoreError> {
    Ok(conn.execute(
        "DELETE FROM tool_changes WHERE state = 'closed' AND closed_at < ?1",
        [Utc(closed_before)],
    )?)
}

fn conversion(error: serde_json::Error) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(error))
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

fn digest(row: &Row<'_>, index: usize) -> rusqlite::Result<[u8; 32]> {
    let hex: String = row.get(index)?;
    sha256_from_hex(&hex).ok_or_else(|| malformed(index, "not a SHA-256".to_owned()))
}

fn maybe_digest(row: &Row<'_>, index: usize) -> rusqlite::Result<Option<[u8; 32]>> {
    match row.get::<_, Option<String>>(index)? {
        Some(hex) => sha256_from_hex(&hex)
            .map(Some)
            .ok_or_else(|| malformed(index, "not a SHA-256".to_owned())),
        None => Ok(None),
    }
}

fn removals(row: &Row<'_>, index: usize) -> rusqlite::Result<Vec<ToolRemoval>> {
    let text: String = row.get(index)?;
    serde_json::from_str(&text)
        .map_err(|error| rusqlite::Error::FromSqlConversionFailure(index, Type::Text, Box::new(error)))
}

fn at(row: &Row<'_>, index: usize) -> rusqlite::Result<Option<Timestamp>> {
    Ok(row.get::<_, Option<Utc>>(index)?.map(|at| at.0))
}

fn read_settings(row: &Row<'_>) -> rusqlite::Result<ToolSettings> {
    let excluded: String = row.get(3)?;
    Ok(ToolSettings {
        at_start: named(row, 0, ToolMode::from_name)?,
        differences: named(row, 1, Differences::from_name)?,
        escrowed: named(row, 2, EscrowPolicy::from_name)?,
        excluded: serde_json::from_str(&excluded)
            .map_err(|error| rusqlite::Error::FromSqlConversionFailure(3, Type::Text, Box::new(error)))?,
    })
}

fn read_run(row: &Row<'_>) -> rusqlite::Result<ToolRun> {
    let id = row.get(0)?;
    let summary = match row.get::<_, Option<String>>(8)? {
        Some(text) => Some(
            serde_json::from_str(&text)
                .map_err(|error| rusqlite::Error::FromSqlConversionFailure(8, Type::Text, Box::new(error)))?,
        ),
        None => None,
    };
    // A report an older build wrote in a shape this one no longer reads is left out rather than
    // failing the history: the next scan makes a new one.
    let report = row.get::<_, Option<String>>(10)?.and_then(|text| {
        match serde_json::from_str::<ArchetypeReport>(&text) {
            Ok(report) => Some(report),
            Err(error) => {
                warn!(run = id, %error, "a tool run's report could not be read");
                None
            }
        }
    });
    Ok(ToolRun {
        id,
        tool: named(row, 1, ToolKind::from_name)?,
        trigger: named(row, 2, ToolTrigger::from_name)?,
        actor: row.get(3)?,
        mode: named(row, 4, ToolRunMode::from_name)?,
        started_at: row.get::<_, Utc>(5)?.0,
        finished_at: at(row, 6)?,
        outcome: maybe_named(row, 7, ToolRunOutcome::from_name)?,
        summary,
        plan: maybe_digest(row, 9)?.as_ref().map(sha256_hex),
        report,
    })
}

fn read_change(row: &Row<'_>) -> rusqlite::Result<ToolChangeRow> {
    let pending = match row.get::<_, Option<String>>(12)? {
        Some(temporary) => Some(PendingWrite {
            temporary,
            sha256: digest(row, 13)?,
            removals: removals(row, 14)?,
            run: row.get(15)?,
        }),
        None => None,
    };
    Ok(ToolChangeRow {
        id: row.get(0)?,
        server_id: row.get(1)?,
        tool: named(row, 2, ToolKind::from_name)?,
        folder_path: row.get(3)?,
        folder_id: row.get(4)?,
        path: row.get(5)?,
        resource: row.get(6)?,
        format: named(row, 7, ToolFileFormat::from_name)?,
        original_sha256: digest(row, 8)?,
        patched_sha256: maybe_digest(row, 9)?,
        removals: removals(row, 10)?,
        state: named(row, 11, ToolChangeState::from_name)?,
        pending,
        close_reason: maybe_named(row, 16, ToolCloseReason::from_name)?,
        opened_run: row.get(17)?,
        changed_run: row.get(18)?,
        closed_run: row.get(19)?,
        opened_at: row.get::<_, Utc>(20)?.0,
        changed_at: at(row, 21)?,
        closed_at: at(row, 22)?,
    })
}

#[cfg(test)]
mod tests {
    use observer_domain::tools::{ArchetypeCounts, ToolKeeper, ToolTimings};

    use super::*;
    use crate::database::memory;

    const SERVER: &str = "default";
    const TOOL: ToolKind = ToolKind::DuplicateArchetypes;
    const FOLDER: &str = r"C:\FXServer\server-data";
    const FOLDER_ID: &str = "00000000c0ffee00-0000000000000000000000000000002a";
    const PATH: &str = "resources/[maps]/gifts/stream/bz_prop_gift.ytyp";

    fn at(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    fn start(conn: &Connection, mode: ToolRunMode) -> i64 {
        insert_run(
            conn,
            &NewRun {
                server_id: SERVER,
                tool: TOOL,
                trigger: ToolTrigger::Panel,
                actor: Some("owner"),
                mode,
                started_at: at("2026-10-31T09:00:00Z"),
            },
        )
        .unwrap()
    }

    fn finish(conn: &mut Connection, id: i64, report: Option<&ArchetypeReport>) -> bool {
        within(conn, |tx| {
            finish_run(
                tx,
                id,
                &FinishedRun {
                    at: at("2026-10-31T09:00:05Z"),
                    outcome: ToolRunOutcome::Done,
                    summary: &ToolRunSummary::default(),
                    plan: None,
                    report,
                },
            )
            .unwrap()
        })
    }

    fn report() -> ArchetypeReport {
        ArchetypeReport {
            folder: FOLDER.to_owned(),
            counts: ArchetypeCounts {
                groups: 2,
                ..ArchetypeCounts::default()
            },
            groups: Vec::new(),
            groups_total: 2,
            shadowed: Vec::new(),
            shadowed_total: 0,
            unreadable: Vec::new(),
            unreadable_total: 0,
            notes: Vec::new(),
            notes_total: 0,
            timings: ToolTimings::default(),
        }
    }

    fn removal(index: u32, run: i64) -> ToolRemoval {
        ToolRemoval {
            index,
            hash: 0x1000 + index,
            name: Some(format!("prop_{index}")),
            keeper: ToolKeeper {
                path: "resources/[maps]/gifts/stream/bz_prop_gift2.ytyp".to_owned(),
                format: ToolFileFormat::Ytyp,
                index: 0,
                sha256: sha256_hex(&[9; 32]),
            },
            run,
        }
    }

    fn write<'a>(
        run: i64,
        removals: &'a [ToolRemoval],
        existing: Option<i64>,
        temporary: &'a str,
    ) -> BeginWrite<'a> {
        BeginWrite {
            server_id: SERVER,
            tool: TOOL,
            run,
            folder_path: FOLDER,
            folder_id: FOLDER_ID,
            path: PATH,
            resource: "gifts",
            format: ToolFileFormat::Ytyp,
            original_sha256: [0xa1; 32],
            temporary,
            sha256: [0xb2; 32],
            removals,
            existing,
            at: at("2026-10-31T09:01:00Z"),
        }
    }

    fn within<R>(conn: &mut Connection, work: impl FnOnce(&Transaction<'_>) -> R) -> R {
        let tx = conn.transaction().unwrap();
        let result = work(&tx);
        tx.commit().unwrap();
        result
    }

    /// A record whose first write landed, as a fix leaves it.
    fn applied_record(conn: &mut Connection, write: &BeginWrite<'_>) -> i64 {
        within(conn, |tx| {
            let id = begin(tx, write).unwrap().unwrap();
            assert!(applied(tx, id, at("2026-10-31T09:01:01Z")).unwrap());
            id
        })
    }

    fn uploads(conn: &Connection) -> Vec<(String, String, String)> {
        files::uploads(conn)
            .unwrap()
            .into_iter()
            .map(|upload| (upload.root, upload.folder, upload.name))
            .collect()
    }

    #[test]
    fn settings_read_as_the_defaults_until_saved_and_save_only_over_the_version_read() {
        let conn = memory();
        let now = at("2026-10-31T09:00:00Z");
        let chosen = ToolSettings {
            at_start: ToolMode::Fix,
            differences: Differences::KeepLatest,
            escrowed: EscrowPolicy::Allow,
            excluded: vec!["maps".to_owned(), "[vehicles]".to_owned()],
        };
        assert_eq!(
            settings(&conn, SERVER, TOOL).unwrap(),
            (ToolSettings::default(), None)
        );

        assert_eq!(
            save_settings(&conn, SERVER, TOOL, &chosen, None, now).unwrap(),
            Some(1)
        );
        let stale = save_settings(&conn, SERVER, TOOL, &ToolSettings::default(), None, now).unwrap();
        assert_eq!(stale, None);
        assert_eq!(settings(&conn, SERVER, TOOL).unwrap(), (chosen.clone(), Some(1)));

        let saved = save_settings(&conn, SERVER, TOOL, &ToolSettings::default(), Some(1), now).unwrap();
        assert_eq!(saved, Some(2));
        assert_eq!(
            save_settings(&conn, SERVER, TOOL, &chosen, Some(1), now).unwrap(),
            None
        );
        assert_eq!(
            settings(&conn, SERVER, TOOL).unwrap(),
            (ToolSettings::default(), Some(2))
        );
        assert_eq!(
            settings(&conn, "other", TOOL).unwrap(),
            (ToolSettings::default(), None)
        );
        assert_eq!(
            save_settings(&conn, "other", TOOL, &chosen, Some(1), now).unwrap(),
            None
        );
    }

    #[test]
    fn settings_and_runs_refuse_names_and_values_the_domain_never_writes() {
        let conn = memory();
        let setting = |at_start: &str, differences: &str, excluded: &str| {
            conn.execute(
                "INSERT INTO tool_settings (server_id, tool, at_start, differences, escrowed, excluded,
                     updated_at)
                 VALUES ('default', 'duplicateArchetypes', ?1, ?2, 'skip', ?3, '2026-10-31T09:00:00.000Z')",
                params![at_start, differences, excluded],
            )
        };
        assert!(setting("always", "identical", "[]").is_err());
        assert!(setting("off", "keep_latest", "[]").is_err());
        assert!(setting("off", "identical", "{}").is_err());
        assert!(setting("off", "identical", "maps").is_err());
        setting("fix", "keepLatest", r#"["maps"]"#).unwrap();

        let run = |trigger: &str, mode: &str, outcome: Option<&str>, summary: Option<&str>| {
            conn.execute(
                "INSERT INTO tool_runs (server_id, tool, trigger, mode, started_at, finished_at, outcome,
                     summary)
                 VALUES ('default', 'duplicateArchetypes', ?1, ?2, '2026-10-31T09:00:00.000Z',
                     CASE WHEN ?3 IS NULL THEN NULL ELSE '2026-10-31T09:00:05.000Z' END, ?3, ?4)",
                params![trigger, mode, outcome, summary],
            )
        };
        assert!(run("schedule", "report", None, None).is_err());
        assert!(run("start", "scan", None, None).is_err());
        assert!(run("start", "fix", Some("timed_out"), None).is_err());
        assert!(run("start", "fix", None, Some("{}")).is_err());
        assert!(run("start", "fix", Some("done"), Some("[]")).is_err());
        run("cli", "undo", Some("timedOut"), Some("{}")).unwrap();
        let unended = conn.execute(
            "INSERT INTO tool_runs (server_id, tool, trigger, mode, started_at, outcome)
             VALUES ('default', 'duplicateArchetypes', 'start', 'heal', '2026-10-31T09:00:00.000Z', 'done')",
            [],
        );
        assert!(unended.is_err());
        let plan = conn.execute(
            "INSERT INTO tool_runs (server_id, tool, trigger, mode, started_at, finished_at, outcome,
                 plan_sha256)
             VALUES ('default', 'duplicateArchetypes', 'panel', 'report', '2026-10-31T09:00:00.000Z',
                 '2026-10-31T09:00:01.000Z', 'done', ?1)",
            [sha256_hex(&[0xab; 32]).to_uppercase()],
        );
        assert!(plan.is_err());
    }

    #[test]
    fn a_run_ends_once_and_lists_the_newest_first_without_its_report() {
        let mut conn = memory();
        let first = start(&conn, ToolRunMode::Report);
        let second = start(&conn, ToolRunMode::Heal);
        let summary = ToolRunSummary {
            groups: 3,
            fixable: 1,
            left: 2,
            elapsed_ms: 640,
            ..ToolRunSummary::default()
        };
        let ended = FinishedRun {
            at: at("2026-10-31T09:00:05Z"),
            outcome: ToolRunOutcome::Done,
            summary: &summary,
            plan: Some([7; 32]),
            report: Some(&report()),
        };

        assert!(within(&mut conn, |tx| finish_run(tx, first, &ended).unwrap()));
        assert!(!within(&mut conn, |tx| finish_run(tx, first, &ended).unwrap()));
        assert!(!within(&mut conn, |tx| finish_run(tx, 999, &ended).unwrap()));

        let listed = runs(&conn, SERVER, TOOL, 10).unwrap();
        assert_eq!(
            listed.iter().map(|run| run.id).collect::<Vec<_>>(),
            [second, first]
        );
        assert_eq!((listed[0].outcome, listed[0].summary.as_ref()), (None, None));
        assert_eq!(listed[1].report, None);
        assert_eq!(listed[1].summary.as_ref(), Some(&summary));
        let stored = run(&conn, first).unwrap().unwrap();
        assert_eq!(
            stored,
            ToolRun {
                id: first,
                tool: TOOL,
                trigger: ToolTrigger::Panel,
                actor: Some("owner".to_owned()),
                mode: ToolRunMode::Report,
                started_at: at("2026-10-31T09:00:00Z"),
                finished_at: Some(at("2026-10-31T09:00:05Z")),
                outcome: Some(ToolRunOutcome::Done),
                summary: Some(summary),
                plan: Some(sha256_hex(&[7; 32])),
                report: Some(report()),
            }
        );
        assert_eq!(last_report(&conn, SERVER, TOOL).unwrap(), Some(stored));
        assert_eq!(run(&conn, 999).unwrap(), None);
        assert_eq!(runs(&conn, "other", TOOL, 10).unwrap(), Vec::new());
    }

    #[test]
    fn runs_left_unended_by_an_earlier_observer_end_as_failed() {
        let conn = memory();
        let earlier = start(&conn, ToolRunMode::Fix);
        let current = insert_run(
            &conn,
            &NewRun {
                server_id: SERVER,
                tool: TOOL,
                trigger: ToolTrigger::Cli,
                actor: None,
                mode: ToolRunMode::Report,
                started_at: at("2026-10-31T10:00:00Z"),
            },
        )
        .unwrap();

        let ended = end_unfinished_runs(&conn, at("2026-10-31T09:30:00Z"), at("2026-10-31T09:30:00Z"));

        assert_eq!(ended.unwrap(), 1);
        let earlier = run(&conn, earlier).unwrap().unwrap();
        assert_eq!(earlier.outcome, Some(ToolRunOutcome::Failed));
        assert_eq!(earlier.finished_at, Some(at("2026-10-31T09:30:00Z")));
        assert_eq!(earlier.summary, None);
        assert_eq!(run(&conn, current).unwrap().unwrap().outcome, None);
    }

    #[test]
    fn the_retention_keeps_the_latest_runs_and_every_run_a_change_names() {
        let mut conn = memory();
        let ids: Vec<i64> = (0..60).map(|_| start(&conn, ToolRunMode::Fix)).collect();
        let elsewhere = insert_run(
            &conn,
            &NewRun {
                server_id: "other",
                tool: TOOL,
                trigger: ToolTrigger::Start,
                actor: None,
                mode: ToolRunMode::Heal,
                started_at: at("2026-10-31T08:00:00Z"),
            },
        )
        .unwrap();
        // ids[0] wrote a change whose entry ids[1] planned; ids[2] is writing another, with an
        // entry of ids[3].
        let landed = [removal(0, ids[1])];
        applied_record(&mut conn, &write(ids[0], &landed, None, ".observer-upload-a"));
        let pending = [removal(4, ids[3])];
        within(&mut conn, |tx| {
            begin(
                tx,
                &BeginWrite {
                    path: "resources/[maps]/gifts/stream/other.ytyp",
                    ..write(ids[2], &pending, None, ".observer-upload-b")
                },
            )
            .unwrap()
            .unwrap()
        });

        let deleted = within(&mut conn, |tx| prune_runs(tx, SERVER, TOOL, RUNS_KEPT).unwrap());

        assert_eq!(deleted, 6);
        let mut kept: Vec<i64> = runs(&conn, SERVER, TOOL, 100)
            .unwrap()
            .iter()
            .map(|run| run.id)
            .collect();
        kept.sort_unstable();
        assert_eq!(kept, [&ids[..4], &ids[10..]].concat());
        assert!(run(&conn, elsewhere).unwrap().is_some());
        let named = conn.execute("DELETE FROM tool_runs WHERE id = ?1", [ids[0]]);
        assert!(named.unwrap_err().to_string().contains("FOREIGN KEY"));
    }

    #[test]
    fn reports_stay_on_the_latest_report_and_on_runs_that_changed_files() {
        let mut conn = memory();
        let fixed = start(&conn, ToolRunMode::Fix);
        let removals = [removal(0, fixed)];
        applied_record(&mut conn, &write(fixed, &removals, None, ".observer-upload-a"));
        assert!(finish(&mut conn, fixed, Some(&report())));
        let scanned = start(&conn, ToolRunMode::Report);
        assert!(finish(&mut conn, scanned, Some(&report())));
        let healed = start(&conn, ToolRunMode::Heal);
        assert!(finish(&mut conn, healed, None));

        let kept = |conn: &Connection, id| run(conn, id).unwrap().unwrap().report.is_some();
        assert!(kept(&conn, fixed));
        assert!(kept(&conn, scanned));
        assert_eq!(last_report(&conn, SERVER, TOOL).unwrap().unwrap().id, scanned);

        let rescanned = start(&conn, ToolRunMode::Report);
        assert!(finish(&mut conn, rescanned, Some(&report())));

        assert!(kept(&conn, fixed));
        assert!(!kept(&conn, scanned));
        assert!(run(&conn, scanned).unwrap().unwrap().summary.is_some());
        assert!(kept(&conn, rescanned));
        assert_eq!(last_report(&conn, SERVER, TOOL).unwrap().unwrap().id, rescanned);
    }

    #[test]
    fn a_first_write_is_recorded_pending_with_its_upload_and_lands_applied() {
        let mut conn = memory();
        let run = start(&conn, ToolRunMode::Fix);
        let removals = [removal(0, run), removal(3, run)];

        let id = within(&mut conn, |tx| {
            begin(tx, &write(run, &removals, None, ".observer-upload-aaaa")).unwrap()
        })
        .unwrap();

        let pending = change(&conn, id).unwrap().unwrap();
        assert_eq!(pending.state, ToolChangeState::Pending);
        assert_eq!(pending.original_sha256, [0xa1; 32]);
        assert_eq!((pending.patched_sha256, pending.removals.len()), (None, 0));
        assert_eq!(
            pending.pending,
            Some(PendingWrite {
                temporary: ".observer-upload-aaaa".to_owned(),
                sha256: [0xb2; 32],
                removals: removals.to_vec(),
                run,
            })
        );
        assert_eq!(
            (pending.opened_run, pending.changed_run, pending.changed_at),
            (run, None, None)
        );
        assert_eq!(
            uploads(&conn),
            [(
                FOLDER.to_owned(),
                "resources/[maps]/gifts/stream".to_owned(),
                ".observer-upload-aaaa".to_owned()
            )]
        );
        assert_eq!(open_changes(&conn, SERVER, TOOL, FOLDER_ID).unwrap(), [pending]);

        assert!(within(&mut conn, |tx| applied(
            tx,
            id,
            at("2026-10-31T09:01:02Z")
        )
        .unwrap()));

        let landed = change(&conn, id).unwrap().unwrap();
        assert_eq!(landed.state, ToolChangeState::Applied);
        assert_eq!(landed.patched_sha256, Some([0xb2; 32]));
        assert_eq!(landed.removals, removals);
        assert_eq!(landed.pending, None);
        assert_eq!(landed.changed_run, Some(run));
        assert_eq!(landed.changed_at, Some(at("2026-10-31T09:01:02Z")));
        assert_eq!(uploads(&conn), Vec::new());
        assert!(!within(&mut conn, |tx| applied(
            tx,
            id,
            at("2026-10-31T09:01:03Z")
        )
        .unwrap()));
    }

    #[test]
    fn a_file_has_one_open_record_per_server_folder() {
        let mut conn = memory();
        let run = start(&conn, ToolRunMode::Fix);
        let removals = [removal(0, run)];
        let first = applied_record(&mut conn, &write(run, &removals, None, ".observer-upload-a"));

        let again = within(&mut conn, |tx| {
            begin(tx, &write(run, &removals, None, ".observer-upload-b")).unwrap()
        });
        assert_eq!(again, None);
        assert_eq!(uploads(&conn), Vec::new());
        let copy = conn.execute(
            "INSERT INTO tool_changes (server_id, tool, folder_path, folder_id, path, resource, format,
                 original_sha256, state, pending_temporary, pending_sha256, pending_removals, pending_run,
                 opened_run, opened_at)
             SELECT server_id, tool, folder_path, folder_id, path, resource, format, original_sha256,
                 'pending', 'x', original_sha256, '[]', opened_run, opened_run, opened_at
             FROM tool_changes WHERE id = ?1",
            [first],
        );
        assert!(copy.unwrap_err().to_string().contains("UNIQUE"));
        let other_folder = within(&mut conn, |tx| {
            begin(
                tx,
                &BeginWrite {
                    folder_id: "00000000c0ffee00-00000000000000000000000000000099",
                    ..write(run, &removals, None, ".observer-upload-c")
                },
            )
            .unwrap()
        });
        assert!(other_folder.is_some());

        assert!(within(&mut conn, |tx| close(
            tx,
            first,
            run,
            ToolCloseReason::Restored,
            at("2026-10-31T09:02:00Z")
        )
        .unwrap()));
        let reopened = within(&mut conn, |tx| {
            begin(tx, &write(run, &removals, None, ".observer-upload-d")).unwrap()
        });
        assert!(reopened.is_some_and(|id| id != first));
    }

    #[test]
    fn abandoning_a_write_puts_an_applied_record_back_exactly_as_it_was() {
        let mut conn = memory();
        let first = start(&conn, ToolRunMode::Fix);
        let before_removals = [removal(0, first)];
        let id = applied_record(
            &mut conn,
            &write(first, &before_removals, None, ".observer-upload-a"),
        );
        let before = change(&conn, id).unwrap().unwrap();
        let second = start(&conn, ToolRunMode::Fix);
        let after_removals = [removal(0, first), removal(5, second)];

        let moved = within(&mut conn, |tx| {
            begin(
                tx,
                &BeginWrite {
                    sha256: [0xc3; 32],
                    at: at("2026-10-31T10:00:00Z"),
                    ..write(second, &after_removals, Some(id), ".observer-upload-b")
                },
            )
            .unwrap()
        });
        assert_eq!(moved, Some(id));
        let pending = change(&conn, id).unwrap().unwrap();
        assert_eq!(pending.state, ToolChangeState::Pending);
        assert_eq!(pending.patched_sha256, before.patched_sha256);
        assert_eq!(pending.removals, before.removals);
        assert_eq!(pending.pending.as_ref().map(|write| write.run), Some(second));
        assert_eq!(uploads(&conn).len(), 1);

        assert!(within(&mut conn, |tx| abandon(
            tx,
            id,
            at("2026-10-31T10:00:01Z")
        )
        .unwrap()));

        assert_eq!(change(&conn, id).unwrap().unwrap(), before);
        assert_eq!(uploads(&conn), Vec::new());
        assert!(!within(&mut conn, |tx| abandon(
            tx,
            id,
            at("2026-10-31T10:00:02Z")
        )
        .unwrap()));
    }

    #[test]
    fn abandoning_a_first_write_closes_its_record() {
        let mut conn = memory();
        let run = start(&conn, ToolRunMode::Fix);
        let removals = [removal(0, run)];
        let id = within(&mut conn, |tx| {
            begin(tx, &write(run, &removals, None, ".observer-upload-a")).unwrap()
        })
        .unwrap();

        assert!(within(&mut conn, |tx| abandon(
            tx,
            id,
            at("2026-10-31T09:01:01Z")
        )
        .unwrap()));

        let closed = change(&conn, id).unwrap().unwrap();
        assert_eq!(closed.state, ToolChangeState::Closed);
        assert_eq!(closed.close_reason, Some(ToolCloseReason::Abandoned));
        assert_eq!(closed.closed_run, Some(run));
        assert_eq!(closed.closed_at, Some(at("2026-10-31T09:01:01Z")));
        assert_eq!((closed.patched_sha256, closed.pending), (None, None));
        assert_eq!(uploads(&conn), Vec::new());
        assert_eq!(open_changes(&conn, SERVER, TOOL, FOLDER_ID).unwrap(), Vec::new());
    }

    #[test]
    fn a_write_over_a_record_that_is_not_applied_or_not_this_file_s_is_refused() {
        let mut conn = memory();
        let run = start(&conn, ToolRunMode::Fix);
        let removals = [removal(0, run)];
        let id = applied_record(&mut conn, &write(run, &removals, None, ".observer-upload-a"));
        let mut attempt = |write: &BeginWrite<'_>| within(&mut conn, |tx| begin(tx, write).unwrap());

        let other_original = BeginWrite {
            original_sha256: [0xee; 32],
            ..write(run, &removals, Some(id), ".observer-upload-b")
        };
        assert_eq!(attempt(&other_original), None);
        let other_file = BeginWrite {
            path: "resources/[maps]/gifts/stream/other.ytyp",
            ..write(run, &removals, Some(id), ".observer-upload-c")
        };
        assert_eq!(attempt(&other_file), None);
        assert_eq!(
            attempt(&write(run, &removals, Some(999), ".observer-upload-d")),
            None
        );
        assert_eq!(
            attempt(&write(run, &removals, Some(id), ".observer-upload-e")),
            Some(id)
        );
        assert_eq!(
            attempt(&write(run, &removals, Some(id), ".observer-upload-f")),
            None
        );
        assert_eq!(uploads(&conn).len(), 1);

        assert!(within(&mut conn, |tx| close(
            tx,
            id,
            run,
            ToolCloseReason::Stale,
            at("2026-10-31T09:03:00Z")
        )
        .unwrap()));
        let closed = within(&mut conn, |tx| {
            begin(tx, &write(run, &removals, Some(id), ".observer-upload-g")).unwrap()
        });
        assert_eq!(closed, None);
        assert_eq!(uploads(&conn), Vec::new());
    }

    #[test]
    fn closing_takes_a_record_out_of_the_open_ones_with_its_reason_its_run_and_its_upload() {
        let mut conn = memory();
        let fixed = start(&conn, ToolRunMode::Fix);
        let removals = [removal(0, fixed)];
        let applied_id = applied_record(&mut conn, &write(fixed, &removals, None, ".observer-upload-a"));
        let pending_id = within(&mut conn, |tx| {
            begin(
                tx,
                &BeginWrite {
                    path: "resources/[maps]/gifts/stream/other.ytyp",
                    ..write(fixed, &removals, None, ".observer-upload-b")
                },
            )
            .unwrap()
        })
        .unwrap();
        let healed = start(&conn, ToolRunMode::Heal);
        let now = at("2026-10-31T11:00:00Z");

        assert!(within(&mut conn, |tx| close(
            tx,
            applied_id,
            healed,
            ToolCloseReason::Healed,
            now
        )
        .unwrap()));
        assert!(within(&mut conn, |tx| close(
            tx,
            pending_id,
            healed,
            ToolCloseReason::Restored,
            now
        )
        .unwrap()));

        let closed = change(&conn, applied_id).unwrap().unwrap();
        assert_eq!(closed.state, ToolChangeState::Closed);
        assert_eq!(closed.close_reason, Some(ToolCloseReason::Healed));
        assert_eq!((closed.closed_run, closed.closed_at), (Some(healed), Some(now)));
        assert_eq!(closed.removals, removals);
        assert_eq!(closed.patched_sha256, Some([0xb2; 32]));
        assert_eq!(change(&conn, pending_id).unwrap().unwrap().pending, None);
        assert_eq!(uploads(&conn), Vec::new());
        assert_eq!(open_changes(&conn, SERVER, TOOL, FOLDER_ID).unwrap(), Vec::new());
        assert!(!within(&mut conn, |tx| close(
            tx,
            applied_id,
            healed,
            ToolCloseReason::Undone,
            now
        )
        .unwrap()));
    }

    #[test]
    fn the_panel_lists_open_records_of_every_folder_and_those_closed_recently() {
        let mut conn = memory();
        let run = start(&conn, ToolRunMode::Fix);
        let removals = [removal(2, run)];
        let here = applied_record(&mut conn, &write(run, &removals, None, ".observer-upload-a"));
        let moved = applied_record(
            &mut conn,
            &BeginWrite {
                folder_path: r"D:\old\server-data",
                folder_id: "00000000c0ffee00-00000000000000000000000000000099",
                path: "resources/[maps]/gifts/stream/a.ytyp",
                original_sha256: [0xa2; 32],
                ..write(run, &removals, None, ".observer-upload-b")
            },
        );
        let mut closed_at = |path: &'static str, original: [u8; 32], when: &str| {
            let id = applied_record(
                &mut conn,
                &BeginWrite {
                    path,
                    original_sha256: original,
                    ..write(run, &removals, None, ".observer-upload-c")
                },
            );
            within(&mut conn, |tx| {
                close(tx, id, run, ToolCloseReason::Undone, at(when)).unwrap()
            });
            id
        };
        let old = closed_at(
            "resources/[maps]/gifts/stream/old.ytyp",
            [0xa3; 32],
            "2026-09-01T00:00:00Z",
        );
        let recent = closed_at(
            "resources/[maps]/gifts/stream/new.ytyp",
            [0xa4; 32],
            "2026-10-30T00:00:00Z",
        );
        let since = at("2026-10-01T00:00:00Z");

        let listed = changes(&conn, SERVER, TOOL, since, 10).unwrap();

        let ids = |rows: &[ToolChangeRow]| rows.iter().map(|row| row.id).collect::<Vec<_>>();
        assert_eq!(ids(&listed.open), [moved, here]);
        assert_eq!(ids(&listed.closed), [recent]);
        assert_eq!(changes(&conn, SERVER, TOOL, since, 0).unwrap().closed, Vec::new());
        assert_eq!(
            listed.open[1].view(),
            ToolChange {
                id: here,
                folder: FOLDER.to_owned(),
                path: PATH.to_owned(),
                resource: "gifts".to_owned(),
                format: ToolFileFormat::Ytyp,
                removals: removals.to_vec(),
                state: ToolChangeState::Applied,
                close_reason: None,
                backup: sha256_hex(&[0xa1; 32]),
                opened_at: at("2026-10-31T09:01:00Z"),
                changed_at: Some(at("2026-10-31T09:01:01Z")),
                closed_at: None,
            }
        );
        assert_eq!(
            referenced_backups(&conn, since).unwrap(),
            [[0xa1; 32], [0xa2; 32], [0xa4; 32]]
        );

        assert_eq!(forget_closed_changes(&conn, since).unwrap(), 1);
        assert_eq!(change(&conn, old).unwrap(), None);
        assert!(change(&conn, recent).unwrap().is_some());
    }

    #[test]
    fn the_journal_refuses_rows_that_say_two_things_at_once() {
        let mut conn = memory();
        let run = start(&conn, ToolRunMode::Fix);
        let removals = [removal(0, run)];
        let pending = within(&mut conn, |tx| {
            begin(tx, &write(run, &removals, None, ".observer-upload-a")).unwrap()
        })
        .unwrap();
        let refused = |change: &str| {
            conn.execute(
                &format!("UPDATE tool_changes SET {change} WHERE id = ?1"),
                [pending],
            )
            .is_err()
        };

        assert!(refused("state = 'open'"));
        assert!(refused("format = 'model_info'"));
        assert!(refused("original_sha256 = 'a1'"));
        assert!(refused("original_sha256 = upper(original_sha256)"));
        assert!(refused("removals = '{}'"));
        assert!(refused("pending_temporary = NULL"));
        assert!(refused("pending_run = NULL"));
        assert!(refused("state = 'applied'"));
        assert!(refused("close_reason = 'healed'"));
        assert!(refused(
            "state = 'closed', close_reason = 'gone', closed_run = opened_run, closed_at = opened_at, \
             pending_temporary = NULL, pending_sha256 = NULL, pending_removals = NULL, pending_run = NULL"
        ));
        assert!(refused(
            "state = 'closed', close_reason = 'healed', closed_run = opened_run, \
             pending_temporary = NULL, pending_sha256 = NULL, pending_removals = NULL, pending_run = NULL"
        ));
        assert!(refused("changed_run = opened_run"));
        assert!(refused("opened_run = 999"));
        assert!(!refused(
            "state = 'closed', close_reason = 'abandoned', closed_run = opened_run, closed_at = opened_at, \
             pending_temporary = NULL, pending_sha256 = NULL, pending_removals = NULL, pending_run = NULL"
        ));
    }
}
