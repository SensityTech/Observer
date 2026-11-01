//! What the diagnostics keep: the processes FXServer's tree started, its crashes and hangs, and
//! where it connected.

use jiff::{SignedDuration, Timestamp};
use observer_domain::diagnostics::{
    AnalysisState, Crash, CrashAnalysis, CrashFile, CrashGroup, CrashKind, ParentProcess, ProcessKind,
    Protocol, TreeProcess,
};
use observer_domain::server::HangCause;
use rusqlite::types::{FromSqlError, Type};
use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::database::StoreError;
use crate::time::Utc;

/// Crashes alike are grouped over this long.
const GROUP_WINDOW: SignedDuration = SignedDuration::from_hours(7 * 24);

/// A start of a process of the tree, as recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewProcess {
    pub run_started: Timestamp,
    pub generation: u32,
    pub pid: u32,
    pub parent_pid: Option<u32>,
    pub parent_image: Option<String>,
    pub image: Option<String>,
    pub command_line: Option<String>,
    pub truncated: bool,
    pub kind: ProcessKind,
    pub at: Timestamp,
}

const PROCESS_COLUMNS: &str =
    "id, run_started, generation, pid, parent_pid, parent_image, image, command_line,
    truncated, kind, started_at, last_started_at, ended_at, count";

/// Records a start: a new row, or one more start of the same program with the same command line
/// in the same run, which then counts as running again.
pub fn record_process(conn: &Connection, process: &NewProcess) -> Result<TreeProcess, StoreError> {
    Ok(conn.query_row(
        &format!(
            "INSERT INTO tree_process (run_started, generation, pid, parent_pid, parent_image, image,
                 command_line, truncated, kind, started_at, last_started_at, count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10, 1)
             ON CONFLICT (run_started, generation, coalesce(image, ''), coalesce(command_line, ''))
             DO UPDATE SET count = count + 1, last_started_at = excluded.last_started_at,
                 pid = excluded.pid, parent_pid = excluded.parent_pid,
                 parent_image = excluded.parent_image, ended_at = NULL
             RETURNING {PROCESS_COLUMNS}"
        ),
        params![
            Utc(process.run_started),
            process.generation,
            process.pid,
            process.parent_pid,
            process.parent_image,
            process.image,
            process.command_line,
            process.truncated,
            process.kind.as_str(),
            Utc(process.at),
        ],
        read_process,
    )?)
}

/// Marks the program of row `id` ended, unless it already is. The caller knows when the last of
/// its instances ended: the row holds the latest one's pid only.
pub fn process_ended(conn: &Connection, id: i64, at: Timestamp) -> Result<Option<TreeProcess>, StoreError> {
    Ok(conn
        .query_row(
            &format!(
                "UPDATE tree_process SET ended_at = ?2 WHERE id = ?1 AND ended_at IS NULL
                 RETURNING {PROCESS_COLUMNS}"
            ),
            params![id, Utc(at)],
            read_process,
        )
        .optional()?)
}

/// Marks every process of a run still running as ended with it.
pub fn end_run(
    conn: &Connection,
    run_started: Timestamp,
    generation: u32,
    at: Timestamp,
) -> Result<Vec<TreeProcess>, StoreError> {
    let mut statement = conn.prepare(&format!(
        "UPDATE tree_process SET ended_at = ?3
         WHERE run_started = ?1 AND generation = ?2 AND ended_at IS NULL
         RETURNING {PROCESS_COLUMNS}"
    ))?;
    let rows = statement.query_map(params![Utc(run_started), generation, Utc(at)], read_process)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Ends every process the records still have running, and says how many. At a start nothing
/// follows them any more: the tree of the Observer before died with it, through its job on
/// Windows, and on Linux through the sweep before the next start.
pub fn end_processes_left_running(conn: &Connection, at: Timestamp) -> Result<usize, StoreError> {
    Ok(conn.execute(
        "UPDATE tree_process SET ended_at = ?1 WHERE ended_at IS NULL",
        [Utc(at)],
    )?)
}

/// The latest starts first.
pub fn processes(conn: &Connection, limit: u32) -> Result<Vec<TreeProcess>, StoreError> {
    let mut statement = conn.prepare(&format!(
        "SELECT {PROCESS_COLUMNS} FROM tree_process ORDER BY last_started_at DESC, id DESC LIMIT ?1"
    ))?;
    let rows = statement.query_map([limit], read_process)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// The shells started since `since`, the latest first.
pub fn shells_since(conn: &Connection, since: Timestamp) -> Result<Vec<TreeProcess>, StoreError> {
    let mut statement = conn.prepare(&format!(
        "SELECT {PROCESS_COLUMNS} FROM tree_process WHERE kind = 'shell' AND last_started_at >= ?1
         ORDER BY last_started_at DESC, id DESC"
    ))?;
    let rows = statement.query_map([Utc(since)], read_process)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Removes the processes last started before `before`, then all but the newest `keep`.
pub fn purge_processes(conn: &Connection, before: Timestamp, keep: u32) -> Result<usize, StoreError> {
    let old = conn.execute(
        "DELETE FROM tree_process WHERE last_started_at < ?1",
        [Utc(before)],
    )?;
    let over = conn.execute(
        "DELETE FROM tree_process WHERE id NOT IN
             (SELECT id FROM tree_process ORDER BY last_started_at DESC, id DESC LIMIT ?1)",
        [keep],
    )?;
    Ok(old + over)
}

fn read_process(row: &Row<'_>) -> rusqlite::Result<TreeProcess> {
    let kind: String = row.get(9)?;
    let image: Option<String> = row.get(6)?;
    let parent_image: Option<String> = row.get(5)?;
    let parent_pid: Option<u32> = row.get(4)?;
    Ok(TreeProcess {
        id: row.get(0)?,
        run_started: row.get::<_, Utc>(1)?.0,
        generation: row.get(2)?,
        pid: row.get(3)?,
        parent: parent_pid.map(|pid| ParentProcess {
            pid,
            name: parent_image.as_deref().map(file_name),
        }),
        name: image.as_deref().map(file_name),
        image,
        command_line: row.get(7)?,
        truncated: row.get(8)?,
        kind: ProcessKind::from_name(&kind).ok_or_else(|| unknown(9, "process kind"))?,
        started_at: row.get::<_, Utc>(10)?.0,
        last_started_at: row.get::<_, Utc>(11)?.0,
        ended_at: row.get::<_, Option<Utc>>(12)?.map(|at| at.0),
        count: row.get(13)?,
    })
}

/// The name after the last separator of either system: images are read on one, shown on both.
fn file_name(path: &str) -> String {
    path.rsplit(['\\', '/']).next().unwrap_or(path).to_owned()
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewCrash {
    pub at: Timestamp,
    pub kind: CrashKind,
    pub run_started: Option<Timestamp>,
    pub generation: Option<u32>,
    pub build: Option<u32>,
    pub exit: Option<String>,
    pub exit_code: Option<String>,
    pub signal: Option<i32>,
    pub hang_cause: Option<HangCause>,
    pub signature: Option<String>,
    pub analysis_state: AnalysisState,
    pub watchdog_stack: Option<String>,
    pub files: Vec<CrashFile>,
}

const CRASH_COLUMNS: &str =
    "id, at, kind, run_started, generation, build, exit, exit_code, signal, hang_cause,
    signature, analysis_state, analysis, watchdog_stack, files, updated_at";

pub fn insert_crash(conn: &Connection, crash: &NewCrash) -> Result<i64, StoreError> {
    conn.execute(
        "INSERT INTO crash (at, kind, run_started, generation, build, exit, exit_code, signal, hang_cause,
             signature, analysis_state, watchdog_stack, files, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?1)",
        params![
            Utc(crash.at),
            crash.kind.as_str(),
            crash.run_started.map(Utc),
            crash.generation,
            crash.build,
            crash.exit,
            crash.exit_code,
            crash.signal,
            crash.hang_cause.map(hang_cause_name),
            crash.signature,
            crash.analysis_state.as_str(),
            crash.watchdog_stack,
            serde_json::to_string(&crash.files).map_err(conversion)?,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Stores what the analysis of a crash's dump found.
pub fn set_analysis(
    conn: &Connection,
    id: i64,
    state: AnalysisState,
    signature: Option<&str>,
    analysis: Option<&CrashAnalysis>,
    now: Timestamp,
) -> Result<(), StoreError> {
    let analysis = analysis
        .map(serde_json::to_string)
        .transpose()
        .map_err(conversion)?;
    conn.execute(
        "UPDATE crash SET analysis_state = ?2, signature = coalesce(?3, signature), analysis = ?4,
             updated_at = ?5
         WHERE id = ?1",
        params![id, state.as_str(), signature, analysis, Utc(now)],
    )?;
    Ok(())
}

pub fn set_files(conn: &Connection, id: i64, files: &[CrashFile], now: Timestamp) -> Result<(), StoreError> {
    conn.execute(
        "UPDATE crash SET files = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, serde_json::to_string(files).map_err(conversion)?, Utc(now)],
    )?;
    Ok(())
}

/// A crash with its group as of `now`.
pub fn crash(conn: &Connection, id: i64, now: Timestamp) -> Result<Option<Crash>, StoreError> {
    let crash = conn
        .query_row(
            &format!("SELECT {CRASH_COLUMNS} FROM crash WHERE id = ?1"),
            [id],
            read_crash,
        )
        .optional()?;
    crash.map(|crash| with_group(conn, crash, now)).transpose()
}

/// The newest crashes first.
pub fn crashes(conn: &Connection, limit: u32, now: Timestamp) -> Result<Vec<Crash>, StoreError> {
    let mut statement = conn.prepare(&format!(
        "SELECT {CRASH_COLUMNS} FROM crash ORDER BY at DESC, id DESC LIMIT ?1"
    ))?;
    let crashes = statement
        .query_map([limit], read_crash)?
        .collect::<Result<Vec<_>, _>>()?;
    crashes
        .into_iter()
        .map(|crash| with_group(conn, crash, now))
        .collect()
}

/// The crashes and hangs since `since`, newest first.
pub fn crashes_since(conn: &Connection, since: Timestamp, now: Timestamp) -> Result<Vec<Crash>, StoreError> {
    let mut statement = conn.prepare(&format!(
        "SELECT {CRASH_COLUMNS} FROM crash WHERE kind != 'found' AND at >= ?1 ORDER BY at DESC, id DESC"
    ))?;
    let crashes = statement
        .query_map([Utc(since)], read_crash)?
        .collect::<Result<Vec<_>, _>>()?;
    crashes
        .into_iter()
        .map(|crash| with_group(conn, crash, now))
        .collect()
}

/// The crashes whose analysis has not been stored yet.
pub fn crashes_pending(conn: &Connection, now: Timestamp) -> Result<Vec<Crash>, StoreError> {
    let mut statement = conn.prepare(&format!(
        "SELECT {CRASH_COLUMNS} FROM crash WHERE analysis_state = 'pending' ORDER BY id"
    ))?;
    let crashes = statement
        .query_map([], read_crash)?
        .collect::<Result<Vec<_>, _>>()?;
    crashes
        .into_iter()
        .map(|crash| with_group(conn, crash, now))
        .collect()
}

/// The crashes one of whose files is called `name`.
pub fn crashes_holding(conn: &Connection, name: &str, now: Timestamp) -> Result<Vec<Crash>, StoreError> {
    let mut statement = conn.prepare(&format!(
        "SELECT {CRASH_COLUMNS} FROM crash
         WHERE EXISTS (SELECT 1 FROM json_each(crash.files) WHERE json_extract(value, '$.name') = ?1)"
    ))?;
    let crashes = statement
        .query_map([name], read_crash)?
        .collect::<Result<Vec<_>, _>>()?;
    crashes
        .into_iter()
        .map(|crash| with_group(conn, crash, now))
        .collect()
}

/// The runs that ended in a crash or a hang since `since`, by start and generation.
pub fn crashed_runs(conn: &Connection, since: Timestamp) -> Result<Vec<(Timestamp, u32)>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT run_started, generation FROM crash
         WHERE kind IN ('crash', 'hang') AND at >= ?1 AND run_started IS NOT NULL AND generation IS NOT NULL",
    )?;
    let rows = statement.query_map([Utc(since)], |row| Ok((row.get::<_, Utc>(0)?.0, row.get(1)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn purge_crashes(conn: &Connection, before: Timestamp) -> Result<usize, StoreError> {
    Ok(conn.execute("DELETE FROM crash WHERE at < ?1", [Utc(before)])?)
}

fn with_group(conn: &Connection, mut crash: Crash, now: Timestamp) -> Result<Crash, StoreError> {
    if let (Some(signature), true) = (&crash.signature, crash.kind != CrashKind::Found) {
        let (count, first): (u32, Option<Utc>) = conn.query_row(
            "SELECT count(*), min(at) FROM crash WHERE signature = ?1 AND kind != 'found' AND at >= ?2",
            params![signature, Utc(now - GROUP_WINDOW)],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        crash.group = first.map(|first| CrashGroup {
            count,
            first_at: first.0,
        });
    }
    Ok(crash)
}

fn read_crash(row: &Row<'_>) -> rusqlite::Result<Crash> {
    let kind: String = row.get(2)?;
    let hang_cause: Option<String> = row.get(9)?;
    let state: String = row.get(11)?;
    let analysis: Option<String> = row.get(12)?;
    let files: String = row.get(14)?;
    let json_error =
        |index| move |error| rusqlite::Error::FromSqlConversionFailure(index, Type::Text, Box::new(error));
    Ok(Crash {
        id: row.get(0)?,
        at: row.get::<_, Utc>(1)?.0,
        kind: CrashKind::from_name(&kind).ok_or_else(|| unknown(2, "crash kind"))?,
        run_started: row.get::<_, Option<Utc>>(3)?.map(|at| at.0),
        generation: row.get(4)?,
        build: row.get(5)?,
        exit: row.get(6)?,
        exit_code: row.get(7)?,
        signal: row.get(8)?,
        hang_cause: hang_cause
            .map(|cause| hang_cause_of(&cause).ok_or_else(|| unknown(9, "hang cause")))
            .transpose()?,
        signature: row.get(10)?,
        analysis_state: AnalysisState::from_name(&state).ok_or_else(|| unknown(11, "analysis state"))?,
        analysis: analysis
            .map(|analysis| serde_json::from_str(&analysis).map_err(json_error(12)))
            .transpose()?,
        watchdog_stack: row.get(13)?,
        files: serde_json::from_str(&files).map_err(json_error(14))?,
        group: None,
        updated_at: row.get::<_, Utc>(15)?.0,
    })
}

fn hang_cause_name(cause: HangCause) -> &'static str {
    match cause {
        HangCause::NoHeartbeat => "noHeartbeat",
        HangCause::NoHttp => "noHttp",
    }
}

fn hang_cause_of(name: &str) -> Option<HangCause> {
    match name {
        "noHeartbeat" => Some(HangCause::NoHeartbeat),
        "noHttp" => Some(HangCause::NoHttp),
        _ => None,
    }
}

/// A destination as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeenConnection {
    pub protocol: Protocol,
    pub address: String,
    pub port: u16,
    pub process: String,
    pub first_seen: Timestamp,
    pub last_seen: Timestamp,
    pub peak: u32,
}

/// Stores a destination: its first sighting stays, the last and the peak move up.
pub fn save_connection(conn: &Connection, seen: &SeenConnection) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO connection_seen (protocol, address, port, process, first_seen, last_seen, peak)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT (protocol, address, port, process) DO UPDATE SET
             first_seen = min(first_seen, excluded.first_seen),
             last_seen = max(last_seen, excluded.last_seen),
             peak = max(peak, excluded.peak)",
        params![
            seen.protocol.as_str(),
            seen.address,
            seen.port,
            seen.process,
            Utc(seen.first_seen),
            Utc(seen.last_seen),
            seen.peak,
        ],
    )?;
    Ok(())
}

/// The destinations seen since `since`, the most recent first.
pub fn connections_since(
    conn: &Connection,
    since: Timestamp,
    limit: u32,
) -> Result<Vec<SeenConnection>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT protocol, address, port, process, first_seen, last_seen, peak FROM connection_seen
         WHERE last_seen >= ?1 ORDER BY last_seen DESC LIMIT ?2",
    )?;
    let rows = statement.query_map(params![Utc(since), limit], |row| {
        let protocol: String = row.get(0)?;
        Ok(SeenConnection {
            protocol: Protocol::from_name(&protocol).ok_or_else(|| unknown(0, "protocol"))?,
            address: row.get(1)?,
            port: row.get(2)?,
            process: row.get(3)?,
            first_seen: row.get::<_, Utc>(4)?.0,
            last_seen: row.get::<_, Utc>(5)?.0,
            peak: row.get(6)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Removes the destinations last seen before `before`, then all but the `keep` seen last.
pub fn purge_connections(conn: &Connection, before: Timestamp, keep: u32) -> Result<usize, StoreError> {
    let old = conn.execute("DELETE FROM connection_seen WHERE last_seen < ?1", [Utc(before)])?;
    let over = conn.execute(
        "DELETE FROM connection_seen WHERE (protocol, address, port, process) NOT IN
             (SELECT protocol, address, port, process FROM connection_seen ORDER BY last_seen DESC LIMIT ?1)",
        [keep],
    )?;
    Ok(old + over)
}

fn unknown(index: usize, what: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        index,
        Type::Text,
        Box::new(FromSqlError::Other(format!("unknown {what}").into())),
    )
}

fn conversion(error: serde_json::Error) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(error))
}

#[cfg(test)]
mod tests {
    use observer_domain::diagnostics::{CrashException, CrashFileKind};

    use super::*;
    use crate::Database;

    const T0: &str = "2026-10-29T10:00:00Z";

    async fn database() -> (tempfile::TempDir, Database) {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path().join("observer.sqlite3"), dir.path().join("backups"))
            .await
            .unwrap();
        (dir, db)
    }

    fn at(offset_minutes: i64) -> Timestamp {
        T0.parse::<Timestamp>().unwrap() + SignedDuration::from_mins(offset_minutes)
    }

    fn start(pid: u32, minutes: i64) -> NewProcess {
        NewProcess {
            run_started: at(0),
            generation: 4,
            pid,
            parent_pid: Some(3312),
            parent_image: Some(r"C:\fx\FXServer.exe".to_owned()),
            image: Some(r"C:\Windows\System32\cmd.exe".to_owned()),
            command_line: Some("cmd /c echo diag".to_owned()),
            truncated: false,
            kind: ProcessKind::Shell,
            at: at(minutes),
        }
    }

    #[tokio::test]
    async fn a_program_started_again_in_the_same_run_counts_on_its_row() {
        let (_dir, db) = database().await;

        let (first, second, other_run) = db
            .write(|tx| {
                let first = record_process(tx, &start(4120, 1))?;
                process_ended(tx, first.id, at(2))?;
                let second = record_process(tx, &start(4188, 3))?;
                let other_run = record_process(
                    tx,
                    &NewProcess {
                        generation: 5,
                        ..start(5000, 4)
                    },
                )?;
                Ok::<_, StoreError>((first, second, other_run))
            })
            .await
            .unwrap();

        assert_eq!(first.id, second.id);
        assert_eq!(second.count, 2);
        assert_eq!(second.pid, 4188);
        assert_eq!(second.started_at, at(1));
        assert_eq!(second.last_started_at, at(3));
        assert_eq!(second.ended_at, None, "it runs again");
        assert_eq!(second.name.as_deref(), Some("cmd.exe"));
        assert_eq!(
            second.parent,
            Some(ParentProcess {
                pid: 3312,
                name: Some("FXServer.exe".to_owned())
            })
        );
        assert_ne!(other_run.id, first.id);
    }

    #[tokio::test]
    async fn a_process_that_ended_before_it_was_read_still_gets_a_row_and_the_run_s_end_closes_the_rest() {
        let (_dir, db) = database().await;

        let (unread, ended) = db
            .write(|tx| {
                let unread = record_process(
                    tx,
                    &NewProcess {
                        image: None,
                        command_line: None,
                        ..start(4300, 1)
                    },
                )?;
                record_process(tx, &start(4120, 1))?;
                let ended = end_run(tx, at(0), 4, at(9))?;
                Ok::<_, StoreError>((unread, ended))
            })
            .await
            .unwrap();

        assert_eq!(unread.image, None);
        assert_eq!(ended.len(), 2);
        assert!(ended.iter().all(|process| process.ended_at == Some(at(9))));
        let listed = db.read(|conn| processes(conn, 10)).await.unwrap();
        assert_eq!(listed.len(), 2);
    }

    #[tokio::test]
    async fn the_schema_refuses_a_command_line_past_4_kib() {
        let (_dir, db) = database().await;

        let refused = db
            .write(|tx| {
                record_process(
                    tx,
                    &NewProcess {
                        command_line: Some("x".repeat(4097)),
                        ..start(1, 1)
                    },
                )
            })
            .await;

        assert!(refused.is_err());
    }

    fn crash_at(minutes: i64, signature: &str) -> NewCrash {
        NewCrash {
            at: at(minutes),
            kind: CrashKind::Crash,
            run_started: Some(at(0)),
            generation: Some(4),
            build: Some(35245),
            exit: Some("exit code 0xFFFFFFFE".to_owned()),
            exit_code: Some("0xFFFFFFFE".to_owned()),
            signal: None,
            hang_cause: None,
            signature: Some(signature.to_owned()),
            analysis_state: AnalysisState::Pending,
            watchdog_stack: None,
            files: vec![CrashFile {
                name: format!("{minutes}.dmp"),
                bytes: 1_300_000,
                kind: CrashFileKind::Minidump,
                kept: true,
            }],
        }
    }

    #[tokio::test]
    async fn a_crash_round_trips_with_its_analysis_and_its_group_over_seven_days() {
        let (_dir, db) = database().await;
        let signature = "0xC0000005 citizen-scripting-lua.dll+0x1a2b3";
        let analysis = CrashAnalysis {
            exception: Some(CrashException {
                code: "0xC0000005".to_owned(),
                name: "EXCEPTION_ACCESS_VIOLATION_WRITE".to_owned(),
                address: Some("0x10".to_owned()),
            }),
            ..CrashAnalysis::default()
        };
        let now = at(60 * 24 * 8);

        let (crash, files) = db
            .write(move |tx| {
                insert_crash(tx, &crash_at(0, signature))?;
                insert_crash(tx, &crash_at(60 * 24 * 7, signature))?;
                let id = insert_crash(tx, &crash_at(60 * 24 * 8, signature))?;
                set_analysis(tx, id, AnalysisState::Done, Some(signature), Some(&analysis), now)?;
                let files = crashes_holding(tx, &format!("{}.dmp", 60 * 24 * 8), now)?;
                Ok::<_, StoreError>((crash(tx, id, now)?, files))
            })
            .await
            .unwrap();

        let crash = crash.unwrap();
        assert_eq!(crash.analysis_state, AnalysisState::Done);
        assert_eq!(crash.analysis.unwrap().exception.unwrap().code, "0xC0000005");
        assert_eq!(crash.build, Some(35245));
        assert_eq!(
            crash.group,
            Some(CrashGroup {
                count: 2,
                first_at: at(60 * 24 * 7)
            })
        );
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].id, crash.id);
    }

    #[tokio::test]
    async fn a_destination_keeps_its_first_sighting_and_its_highest_peak() {
        let (_dir, db) = database().await;
        let seen = |minutes, peak| SeenConnection {
            protocol: Protocol::Tcp,
            address: "127.0.0.1".to_owned(),
            port: 3306,
            process: "FXServer.exe".to_owned(),
            first_seen: at(minutes),
            last_seen: at(minutes),
            peak,
        };

        let listed = db
            .write(move |tx| {
                save_connection(tx, &seen(1, 4))?;
                save_connection(tx, &seen(5, 2))?;
                connections_since(tx, at(0), 10)
            })
            .await
            .unwrap();

        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].first_seen, at(1));
        assert_eq!(listed[0].last_seen, at(5));
        assert_eq!(listed[0].peak, 4);
    }

    #[tokio::test]
    async fn retention_drops_old_rows_and_keeps_only_the_newest() {
        let (_dir, db) = database().await;

        let (purged, left) = db
            .write(|tx| {
                for pid in 0..5 {
                    record_process(
                        tx,
                        &NewProcess {
                            command_line: Some(format!("cmd {pid}")),
                            ..start(pid, i64::from(pid))
                        },
                    )?;
                }
                let purged = purge_processes(tx, at(1), 2)?;
                Ok::<_, StoreError>((purged, processes(tx, 10)?))
            })
            .await
            .unwrap();

        assert_eq!(purged, 3);
        let pids: Vec<u32> = left.iter().map(|process| process.pid).collect();
        assert_eq!(pids, [4, 3]);
    }
}
