//! What Observer sees of the default server from outside: the processes its tree starts, where
//! they connect, and its crashes and hangs with what their dumps say. One task follows the
//! supervisor, keeps the records, the alert list and the live feed; the panel reads them, and the
//! owner downloads or deletes a crash's files.

mod connections;
mod crashes;
mod processes;

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use jiff::{SignedDuration, Timestamp};
use observer_domain::audit::{ActorKind, AuditAction, Outcome};
use observer_domain::diagnostics::{
    Alert, AlertCrash, AlertException, AlertKind, AlertList, AnalysisState, Crash, CrashFile, CrashFileKind,
    CrashFolder, CrashKind, Destination, DiagnosticsEvent, DiagnosticsSnapshot, Listener, ProcessKind,
    ThreadRun, TreeProcess,
};
use observer_domain::error::{ApiError, ErrorCode};
use observer_fxserver::crashes::analysis::{self, Analysis, AnalysisError};
use observer_fxserver::crashes::{DumpLimits, SnapshotFailure};
use observer_fxserver::supervisor::{Event, EventKind, ProcessChange, TreeEvent};
use observer_platform::ProcessInfo;
use observer_store::Database;
use observer_store::diagnostics::{self as store, NewCrash, NewProcess};
use serde_json::{Value, json};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{broadcast, watch};
use tokio::task::{JoinHandle, JoinSet};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use self::connections::Destinations;
use self::crashes::Tracker;
use self::processes::{FXSERVER, Run, alert_note, audit_details, file_name, masked_command_line};
use crate::audit::Record;
use crate::{Caller, Clock, Error, Servers, Supervision};

/// Who the console names for the diagnostics' notes.
const OBSERVER: &str = "observer";
const SOCKETS_EVERY: Duration = Duration::from_secs(2);
const UPKEEP_EVERY: Duration = Duration::from_hours(1);
const ALERT_WINDOW: SignedDuration = SignedDuration::from_hours(24);
const MAX_ALERTS: usize = 10;
const FEED: usize = 64;
const SNAPSHOT_PROCESSES: u32 = 200;
const SNAPSHOT_CRASHES: u32 = 50;
/// The largest file the panel may download: full dumps stay on the host.
pub const MAX_DOWNLOAD: u64 = 64 * 1024 * 1024;
/// Tree changes of a run whose start has not come yet are held, up to this many.
const WAITING: usize = 1024;
const PROCESSES_KEPT: (SignedDuration, u32) = (SignedDuration::from_hours(30 * 24), 5_000);
const DESTINATIONS_KEPT: (SignedDuration, u32) = (SignedDuration::from_hours(7 * 24), 2_000);
const CRASHES_KEPT: SignedDuration = SignedDuration::from_hours(180 * 24);

/// What [`Diagnostics::start`] works with.
pub struct DiagnosticsSetup {
    pub db: Database,
    pub clock: Clock,
    pub supervision: Supervision,
    /// Observer's data folder, whose `crashes/` holds the dumps.
    pub data_dir: PathBuf,
}

/// The diagnostics of the default server. Clones share them.
#[derive(Clone)]
pub struct Diagnostics {
    inner: Arc<Inner>,
}

struct Inner {
    db: Database,
    clock: Clock,
    data_dir: PathBuf,
    limits: DumpLimits,
    alerts: watch::Receiver<AlertList>,
    feed: broadcast::Sender<DiagnosticsEvent>,
    connections: Mutex<(Vec<Destination>, Vec<Listener>)>,
}

/// The diagnostics' task; [`DiagnosticsTask::close`] ends it.
pub struct DiagnosticsTask {
    task: JoinHandle<()>,
    stop: CancellationToken,
}

impl DiagnosticsTask {
    pub async fn close(self) {
        self.stop.cancel();
        if let Err(error) = self.task.await {
            warn!(%error, "the diagnostics task failed");
        }
    }
}

impl Diagnostics {
    /// Reads what the history holds, then follows the supervisor. Starting it before FXServer
    /// boots lets it see the first run's tree.
    pub async fn start(setup: DiagnosticsSetup) -> Result<(Self, DiagnosticsTask), Error> {
        let (diagnostics, driver, buses) = Self::prepare(setup).await?;
        let stop = CancellationToken::new();
        let task = tokio::spawn(driver.run(buses, stop.clone()));
        Ok((diagnostics, DiagnosticsTask { task, stop }))
    }

    /// The service and its driver, subscribed to the supervisor but not running yet.
    async fn prepare(setup: DiagnosticsSetup) -> Result<(Self, Driver, Buses), Error> {
        let supervisor = setup.supervision.supervisor().clone();
        let buses = Buses {
            processes: supervisor.processes(),
            events: supervisor.events(),
        };
        let now = setup.clock.now();
        let seen = setup
            .db
            .read(move |conn| store::connections_since(conn, now - DESTINATIONS_KEPT.0, DESTINATIONS_KEPT.1))
            .await?;
        let destinations = Destinations::restored(seen);
        let first = build_alerts(&setup.db, now).await?;
        let (alerts_tx, alerts) = watch::channel(first);
        let inner = Arc::new(Inner {
            db: setup.db,
            clock: setup.clock,
            data_dir: setup.data_dir,
            limits: DumpLimits::default(),
            alerts,
            feed: broadcast::channel(FEED).0,
            connections: Mutex::new((destinations.listed(now), Vec::new())),
        });
        let driver = Driver {
            inner: Arc::clone(&inner),
            supervision: setup.supervision,
            alerts: alerts_tx,
            runs: HashMap::new(),
            run_starts: HashMap::new(),
            waiting: Vec::new(),
            tracker: Tracker::default(),
            destinations,
            builds: HashMap::new(),
            analyses: JoinSet::new(),
        };
        Ok((Self { inner }, driver, buses))
    }

    /// The alert list as it changes.
    pub fn alerts(&self) -> watch::Receiver<AlertList> {
        self.inner.alerts.clone()
    }

    /// The topic's snapshot, and its events from then on: subscribed to first, so that nothing
    /// falls between the two.
    pub async fn follow(
        &self,
    ) -> Result<(DiagnosticsSnapshot, broadcast::Receiver<DiagnosticsEvent>), Error> {
        let events = self.inner.feed.subscribe();
        let now = self.inner.clock.now();
        let (processes, crashes) = self
            .inner
            .db
            .read(move |conn| {
                Ok::<_, Error>((
                    store::processes(conn, SNAPSHOT_PROCESSES)?,
                    store::crashes(conn, SNAPSHOT_CRASHES, now)?,
                ))
            })
            .await?;
        let (destinations, listeners) = self
            .inner
            .connections
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let crash_folder = self.inner.crash_folder().await;
        let snapshot = DiagnosticsSnapshot {
            processes,
            destinations,
            listeners,
            crashes,
            crash_folder,
        };
        Ok((snapshot, events))
    }

    /// Opens one of a crash's files, by the name its record lists, and audits the download. A
    /// name the record does not list, or a file gone, is `crash_file_not_found`; a file over
    /// 64 MiB stays on the host.
    pub async fn open_file(
        &self,
        caller: &Caller,
        id: i64,
        name: &str,
    ) -> Result<(CrashFile, tokio::fs::File), Error> {
        let now = self.inner.clock.now();
        let crash = self
            .inner
            .db
            .read(move |conn| store::crash(conn, id, now))
            .await?
            .ok_or(ErrorCode::CrashNotFound)?;
        let file = crash
            .files
            .into_iter()
            .find(|file| file.name == name && file.kept)
            .ok_or(ErrorCode::CrashFileNotFound)?;
        let path = self.inner.crashes().join(&file.name);
        let opened = match tokio::fs::File::open(&path).await {
            Ok(opened) => opened,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(ErrorCode::CrashFileNotFound.into());
            }
            Err(error) => return Err(error.into()),
        };
        let bytes = opened.metadata().await?.len();
        if bytes > MAX_DOWNLOAD {
            return Err(ApiError::new(ErrorCode::CrashFileTooLarge)
                .with("maxBytes", MAX_DOWNLOAD)
                .into());
        }
        let record = Record::by(caller, AuditAction::CrashFileDownloaded)
            .target(Servers::DEFAULT)
            .details(json!({ "crash": id, "name": file.name, "bytes": bytes }));
        self.inner.audit(record).await?;
        Ok((CrashFile { bytes, ..file }, opened))
    }

    /// Deletes a crash's files from the host and audits it; the record and its analysis stay.
    pub async fn delete_files(&self, caller: &Caller, id: i64) -> Result<Crash, Error> {
        let now = self.inner.clock.now();
        let crash = self
            .inner
            .db
            .read(move |conn| store::crash(conn, id, now))
            .await?
            .ok_or(ErrorCode::CrashNotFound)?;
        let folder = self.inner.crashes();
        let kept: Vec<CrashFile> = crash.files.iter().filter(|file| file.kept).cloned().collect();
        let paths: Vec<PathBuf> = kept.iter().map(|file| folder.join(&file.name)).collect();
        tokio::task::spawn_blocking(move || {
            for path in paths {
                match std::fs::remove_file(&path) {
                    Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
                    _ => {}
                }
            }
            Ok(())
        })
        .await
        .map_err(io::Error::other)??;
        let files: Vec<CrashFile> = crash
            .files
            .iter()
            .map(|file| CrashFile {
                kept: false,
                ..file.clone()
            })
            .collect();
        let names: Vec<&str> = kept.iter().map(|file| file.name.as_str()).collect();
        let bytes: u64 = kept.iter().map(|file| file.bytes).sum();
        let record = Record::by(caller, AuditAction::CrashFilesDeleted)
            .target(Servers::DEFAULT)
            .details(json!({ "crash": id, "names": names, "bytes": bytes }));
        let updated = self
            .inner
            .db
            .write(move |tx| {
                store::set_files(tx, id, &files, now)?;
                record.write(tx, now)?;
                store::crash(tx, id, now)
            })
            .await?
            .ok_or(ErrorCode::CrashNotFound)?;
        self.inner.publish(DiagnosticsEvent::Crash {
            crash: Box::new(updated.clone()),
        });
        let folder = self.inner.crash_folder().await;
        self.inner.publish(DiagnosticsEvent::CrashFolder { folder });
        Ok(updated)
    }
}

impl Inner {
    fn crashes(&self) -> PathBuf {
        self.data_dir.join("crashes")
    }

    fn publish(&self, event: DiagnosticsEvent) {
        let _ = self.feed.send(event);
    }

    async fn audit(&self, record: Record) -> Result<(), Error> {
        let now = self.clock.now();
        self.db
            .write(move |tx| record.write(tx, now).map_err(Error::from))
            .await
    }

    async fn crash_folder(&self) -> CrashFolder {
        let (folder, data_dir, limits) = (self.crashes(), self.data_dir.clone(), self.limits);
        tokio::task::spawn_blocking(move || crash_folder(&folder, &data_dir, limits))
            .await
            .unwrap_or(CrashFolder {
                files: 0,
                bytes: 0,
                max_files: 0,
                max_bytes: 0,
                free: None,
            })
    }
}

/// The crash folder against the dump policy, and the free space of its volume.
fn crash_folder(folder: &Path, data_dir: &Path, limits: DumpLimits) -> CrashFolder {
    let (mut files, mut bytes) = (0u32, 0u64);
    if let Ok(entries) = std::fs::read_dir(folder) {
        for entry in entries.filter_map(Result::ok) {
            let partial = entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "partial");
            if let Ok(metadata) = entry.metadata()
                && metadata.is_file()
                && !partial
            {
                files += 1;
                bytes += metadata.len();
            }
        }
    }
    CrashFolder {
        files,
        bytes,
        max_files: u32::try_from(limits.files).unwrap_or(u32::MAX),
        max_bytes: limits.bytes,
        free: data_dir
            .ancestors()
            .find_map(|dir| observer_platform::available_space(dir).ok()),
    }
}

struct Buses {
    processes: broadcast::Receiver<TreeEvent>,
    events: broadcast::Receiver<Event>,
}

struct Driver {
    inner: Arc<Inner>,
    supervision: Supervision,
    alerts: watch::Sender<AlertList>,
    runs: HashMap<u32, Run>,
    /// When each run started, kept past its end for the records of its crash.
    run_starts: HashMap<u32, Timestamp>,
    waiting: Vec<TreeEvent>,
    tracker: Tracker,
    destinations: Destinations,
    /// The build each run reported.
    builds: HashMap<u32, u32>,
    analyses: JoinSet<(i64, Result<Analysis, AnalysisError>)>,
}

impl Driver {
    async fn run(mut self, buses: Buses, stop: CancellationToken) {
        let Buses {
            mut processes,
            mut events,
        } = buses;
        let mut status = self.supervision.supervisor().status();
        let mut sockets = tokio::time::interval(SOCKETS_EVERY);
        sockets.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut upkeep = tokio::time::interval(UPKEEP_EVERY);
        loop {
            let expiry = self.alert_expiry();
            tokio::select! {
                () = stop.cancelled() => return,
                tree = processes.recv() => match tree {
                    Ok(tree) => self.tree(tree).await,
                    Err(RecvError::Lagged(missed)) => {
                        warn!(missed, "process starts were not recorded: the diagnostics fell behind");
                        self.supervision.note(OBSERVER, &format!("{missed} process starts were not recorded"));
                    }
                    Err(RecvError::Closed) => return,
                },
                event = events.recv() => match event {
                    Ok(event) => self.event(event).await,
                    Err(RecvError::Lagged(missed)) => warn!(missed, "the diagnostics missed supervisor events"),
                    Err(RecvError::Closed) => return,
                },
                changed = status.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    let status = status.borrow_and_update();
                    if let Some(build) = status.build {
                        self.builds.insert(status.generation, build);
                    }
                }
                Some(analysed) = self.analyses.join_next() => {
                    if let Ok((id, result)) = analysed {
                        self.analysed(id, result).await;
                    }
                }
                _ = sockets.tick() => self.look().await,
                _ = upkeep.tick() => self.upkeep().await,
                () = tokio::time::sleep(expiry) => self.refresh_alerts().await,
            }
        }
    }

    fn now(&self) -> Timestamp {
        self.inner.clock.now()
    }

    async fn tree(&mut self, tree: TreeEvent) {
        if !self.runs.contains_key(&tree.generation) {
            // The run's start is on the other bus and may come a moment later.
            if self.waiting.len() < WAITING && !self.run_starts.contains_key(&tree.generation) {
                self.waiting.push(tree);
            }
            return;
        }
        match tree.change {
            ProcessChange::Started { process, kind } => self.started(tree.generation, process, kind).await,
            ProcessChange::Ended { pid } => self.ended(tree.generation, pid).await,
        }
    }

    async fn started(&mut self, generation: u32, process: ProcessInfo, kind: ProcessKind) {
        let now = self.now();
        let Some(run) = self.runs.get_mut(&generation) else {
            return;
        };
        let image = process
            .image
            .as_ref()
            .map(|image| image.to_string_lossy().into_owned());
        let (command_line, truncated) = match &process.command_line {
            Some(line) => {
                let (masked, truncated) = masked_command_line(line);
                (Some(masked), truncated)
            }
            None => (None, false),
        };
        let parent = run.parent_name(process.parent);
        let parent_image = run.parent_image(process.parent);
        let admitted = run.admits(image.as_deref(), command_line.as_deref());
        if !admitted && run.overflow == 1 {
            warn!(
                generation,
                "FXServer's tree started more than 200 programs; further ones are only counted"
            );
        }
        let details = audit_details(
            &process,
            kind,
            command_line.as_deref(),
            parent.as_deref(),
            generation,
        );
        let program = image.clone().unwrap_or_else(|| process.pid.to_string());
        let entry = kind
            .is_unexpected()
            .then(|| run.audit_entry(&program, details, now))
            .flatten();
        let new = NewProcess {
            run_started: run.started,
            generation,
            pid: process.pid,
            parent_pid: process.parent,
            parent_image,
            image: image.clone(),
            command_line,
            truncated,
            kind,
            at: now,
        };
        let audited = entry.is_some();
        let written = self
            .inner
            .db
            .write(move |tx| {
                let row = admitted.then(|| store::record_process(tx, &new)).transpose()?;
                if let Some(details) = entry {
                    tree_audit(details).write(tx, now)?;
                }
                Ok::<_, Error>(row)
            })
            .await;
        let row = match written {
            Ok(row) => row,
            Err(error) => {
                warn!(%error, "cannot record a process FXServer's tree started");
                None
            }
        };
        if let Some(run) = self.runs.get_mut(&generation) {
            run.running
                .insert(process.pid, (row.as_ref().map(|row| row.id), image));
        }
        if let Some(row) = row {
            self.inner.publish(DiagnosticsEvent::Process {
                process: Box::new(row),
            });
        }
        if kind == ProcessKind::Shell {
            if audited {
                let name = file_name(&program);
                let note = alert_note(name, process.pid, parent.as_deref(), process.parent);
                warn!(%name, pid = process.pid, "FXServer's tree started a shell");
                self.supervision.note(OBSERVER, &note);
            }
            self.refresh_alerts().await;
        }
    }

    async fn ended(&mut self, generation: u32, pid: u32) {
        let Some((Some(id), _)) = self
            .runs
            .get_mut(&generation)
            .and_then(|run| run.running.remove(&pid))
        else {
            return;
        };
        let now = self.now();
        match self
            .inner
            .db
            .write(move |tx| store::process_ended(tx, id, pid, now))
            .await
        {
            Ok(Some(row)) => self.inner.publish(DiagnosticsEvent::Process {
                process: Box::new(row),
            }),
            Ok(None) => {}
            Err(error) => warn!(%error, "cannot record the end of a process of FXServer's tree"),
        }
    }

    async fn event(&mut self, event: Event) {
        match &event.kind {
            EventKind::Spawned { pid, .. } => {
                let started = Timestamp::from_millisecond(event.at_ms).unwrap_or_else(|_| self.now());
                self.runs.insert(event.generation, Run::new(started, Some(*pid)));
                self.run_starts.insert(event.generation, started);
                // Only the runs a record may still name are kept.
                self.run_starts
                    .retain(|generation, _| *generation + 8 >= event.generation);
                self.builds
                    .retain(|generation, _| *generation + 8 >= event.generation);
                let waiting = std::mem::take(&mut self.waiting);
                for tree in waiting {
                    if tree.generation == event.generation {
                        self.tree(tree).await;
                    }
                }
            }
            EventKind::Exited { .. } => self.run_ended(event.generation).await,
            EventKind::HangSnapshot {
                path,
                bytes,
                took_ms,
                failure,
                ..
            } => {
                let record = match (path, failure) {
                    (Some(path), None) => Record::operator(ActorKind::System, AuditAction::HangSnapshotTaken).details(json!({
                        "file": path.file_name().map(|name| name.to_string_lossy().into_owned()),
                        "bytes": bytes,
                        "tookMs": took_ms,
                        "kind": if cfg!(windows) { "dump" } else { "report" },
                    })),
                    (_, failure) => Record::operator(ActorKind::System, AuditAction::HangSnapshotTaken)
                        .outcome(Outcome::Failure)
                        .details(json!({ "reason": failure.unwrap_or(SnapshotFailure::Failed), "tookMs": took_ms })),
                };
                if let Err(error) = self.inner.audit(record.target(Servers::DEFAULT)).await {
                    warn!(%error, "cannot audit the hang snapshot");
                }
            }
            _ => {}
        }
        for outcome in self.tracker.event(&event) {
            self.outcome(outcome).await;
        }
    }

    /// The run is over: what was not audited yet is, and its processes end with it.
    async fn run_ended(&mut self, generation: u32) {
        let Some(mut run) = self.runs.remove(&generation) else {
            return;
        };
        if run.overflow > 0 {
            info!(
                generation,
                starts = run.overflow,
                "process starts past the 200 programs of the run were only counted"
            );
        }
        let now = self.now();
        let leftovers = run.leftovers();
        let started = run.started;
        let ended = self
            .inner
            .db
            .write(move |tx| {
                for details in leftovers {
                    tree_audit(details).write(tx, now)?;
                }
                Ok::<_, Error>(store::end_run(tx, started, generation, now)?)
            })
            .await;
        match ended {
            Ok(rows) => {
                for row in rows {
                    self.inner.publish(DiagnosticsEvent::Process {
                        process: Box::new(row),
                    });
                }
            }
            Err(error) => warn!(%error, "cannot record the end of a run's processes"),
        }
    }

    async fn outcome(&mut self, outcome: crashes::Outcome) {
        let now = self.now();
        let result = match outcome {
            crashes::Outcome::Removed(paths) => self.removed(paths).await,
            crashes::Outcome::Found(paths) => {
                let files = crashes::files(&paths);
                self.record(NewCrash {
                    at: now,
                    kind: CrashKind::Found,
                    run_started: None,
                    generation: None,
                    build: None,
                    exit: None,
                    exit_code: None,
                    signal: None,
                    hang_cause: None,
                    signature: None,
                    analysis_state: AnalysisState::NoDump,
                    watchdog_stack: None,
                    files,
                })
                .await
            }
            crashes::Outcome::Crash {
                kind,
                generation,
                exit,
                hang_cause,
                watchdog_stack,
                files,
            } => {
                let files = crashes::files(&files);
                let (analysis_state, signature) = if crashes::readable(&files).is_some() {
                    (AnalysisState::Pending, None)
                } else {
                    crashes::without_dump(&exit)
                };
                self.record(NewCrash {
                    at: now,
                    kind,
                    run_started: self.run_starts.get(&generation).copied(),
                    generation: Some(generation),
                    build: self.builds.get(&generation).copied(),
                    exit: Some(exit.description.clone()),
                    exit_code: exit.code.map(|code| format!("0x{code:08X}")),
                    signal: exit.signal,
                    hang_cause,
                    signature,
                    analysis_state,
                    watchdog_stack,
                    files,
                })
                .await
            }
        };
        if let Err(error) = result {
            warn!(%error, "cannot record a crash");
        }
    }

    async fn record(&mut self, mut crash: NewCrash) -> Result<(), Error> {
        let readable = crashes::readable(&crash.files).cloned();
        if readable.is_some() {
            crash.analysis_state = AnalysisState::Pending;
        }
        let now = self.now();
        let found = crash.kind == CrashKind::Found;
        let recorded = self
            .inner
            .db
            .write(move |tx| {
                let id = store::insert_crash(tx, &crash)?;
                store::crash(tx, id, now)
            })
            .await?;
        if let Some(recorded) = recorded {
            if let Some(file) = readable {
                self.analyse(recorded.id, &file);
            }
            self.inner.publish(DiagnosticsEvent::Crash {
                crash: Box::new(recorded),
            });
        }
        let folder = self.inner.crash_folder().await;
        self.inner.publish(DiagnosticsEvent::CrashFolder { folder });
        if !found {
            self.refresh_alerts().await;
        }
        Ok(())
    }

    fn analyse(&mut self, id: i64, file: &CrashFile) {
        let path = self.inner.crashes().join(&file.name);
        let report = file.kind == CrashFileKind::HangReport;
        self.analyses.spawn(async move {
            let read = tokio::task::spawn_blocking(move || {
                if report {
                    analysis::analyse_report(&path)
                } else {
                    analysis::analyse(&path)
                }
            })
            .await
            // A panic while reading a dump is a dump that cannot be read.
            .unwrap_or(Err(AnalysisError::Unreadable));
            (id, read)
        });
    }

    async fn analysed(&mut self, id: i64, result: Result<Analysis, AnalysisError>) {
        let now = self.now();
        let (state, signature, analysis) = match result {
            Ok(Analysis { analysis, signature }) => (AnalysisState::Done, Some(signature), Some(analysis)),
            Err(error) => {
                warn!(%error, crash = id, "a crash dump could not be read");
                (AnalysisState::Unreadable, None, None)
            }
        };
        let updated = self
            .inner
            .db
            .write(move |tx| {
                store::set_analysis(tx, id, state, signature.as_deref(), analysis.as_ref(), now)?;
                store::crash(tx, id, now)
            })
            .await;
        match updated {
            Ok(Some(crash)) => {
                let alert = crash.kind != CrashKind::Found;
                self.inner.publish(DiagnosticsEvent::Crash {
                    crash: Box::new(crash),
                });
                if alert {
                    self.refresh_alerts().await;
                }
            }
            Ok(None) => {}
            Err(error) => warn!(%error, "cannot store a crash analysis"),
        }
    }

    /// Files the dump policy removed stay listed in their records, as no longer kept.
    async fn removed(&mut self, paths: Vec<PathBuf>) -> Result<(), Error> {
        let names: Vec<String> = paths
            .iter()
            .filter_map(|path| Some(path.file_name()?.to_string_lossy().into_owned()))
            .collect();
        let now = self.now();
        let updated = self
            .inner
            .db
            .write(move |tx| {
                let mut updated: Vec<i64> = Vec::new();
                for name in &names {
                    for crash in store::crashes_holding(tx, name, now)? {
                        let files: Vec<CrashFile> = crash
                            .files
                            .iter()
                            .map(|file| CrashFile {
                                kept: file.kept && file.name != *name,
                                ..file.clone()
                            })
                            .collect();
                        store::set_files(tx, crash.id, &files, now)?;
                        updated.push(crash.id);
                    }
                }
                updated.dedup();
                updated
                    .into_iter()
                    .filter_map(|id| store::crash(tx, id, now).transpose())
                    .collect::<Result<Vec<Crash>, _>>()
            })
            .await?;
        for crash in updated {
            self.inner.publish(DiagnosticsEvent::Crash {
                crash: Box::new(crash),
            });
        }
        Ok(())
    }

    /// One look at the sockets of the running tree.
    async fn look(&mut self) {
        let status = self.supervision.supervisor().status().borrow().clone();
        let game = self
            .supervision
            .supervisor()
            .readings()
            .borrow()
            .run
            .as_ref()
            .map(|run| run.probe.port());
        let mut names: HashMap<u32, String> = HashMap::new();
        if let Some(main) = status.pid {
            names.insert(main, FXSERVER.to_owned());
            if let Some(run) = self.runs.get(&status.generation) {
                for (pid, (_, image)) in &run.running {
                    let name = image
                        .as_deref()
                        .map_or_else(|| pid.to_string(), |image| file_name(image).to_owned());
                    names.insert(*pid, name);
                }
            }
        }
        let sockets = if names.is_empty() {
            Vec::new()
        } else {
            // The main process first: on Linux its network namespace holds the tables.
            let pids: Vec<u32> = status
                .pid
                .into_iter()
                .chain(names.keys().copied().filter(|pid| Some(*pid) != status.pid))
                .collect();
            match tokio::task::spawn_blocking(move || observer_platform::sockets(&pids)).await {
                Ok(Ok(sockets)) => sockets,
                Ok(Err(error)) => {
                    warn!(%error, "cannot read the sockets of FXServer's tree");
                    return;
                }
                Err(_) => return,
            }
        };
        let now = self.now();
        let look = connections::look(&sockets, &names, game);
        let (writes, changed) = self.destinations.update(look, now);
        if !writes.is_empty() {
            let saved = self
                .inner
                .db
                .write(move |tx| {
                    for seen in &writes {
                        store::save_connection(tx, seen)?;
                    }
                    Ok::<_, Error>(())
                })
                .await;
            if let Err(error) = saved {
                warn!(%error, "cannot record FXServer's connections");
            }
        }
        if changed {
            let destinations = self.destinations.listed(now);
            let listeners = self.destinations.listeners();
            *self
                .inner
                .connections
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = (destinations.clone(), listeners.clone());
            self.inner.publish(DiagnosticsEvent::Connections {
                destinations,
                listeners,
            });
        }
    }

    /// The retention: processes 30 days and 5,000 rows, destinations 7 days and 2,000, crashes
    /// 180 days. A crash's files follow the dump policy, not this.
    async fn upkeep(&mut self) {
        let now = self.now();
        let purged = self
            .inner
            .db
            .write(move |tx| {
                Ok::<_, Error>((
                    store::purge_processes(tx, now - PROCESSES_KEPT.0, PROCESSES_KEPT.1)?,
                    store::purge_connections(tx, now - DESTINATIONS_KEPT.0, DESTINATIONS_KEPT.1)?,
                    store::purge_crashes(tx, now - CRASHES_KEPT)?,
                ))
            })
            .await;
        match purged {
            Ok((0, 0, 0)) => {}
            Ok((processes, destinations, crashes)) => {
                info!(processes, destinations, crashes, "old diagnostics deleted");
            }
            Err(error) => warn!(%error, "the diagnostics retention could not be applied"),
        }
        self.destinations.forget_before(now - DESTINATIONS_KEPT.0);
    }

    async fn refresh_alerts(&mut self) {
        match build_alerts(&self.inner.db, self.now()).await {
            Ok(list) => {
                self.alerts.send_if_modified(|current| {
                    let changed = *current != list;
                    *current = list;
                    changed
                });
            }
            Err(error) => warn!(%error, "cannot list the alerts"),
        }
    }

    /// How long until the oldest alert leaves the window.
    fn alert_expiry(&self) -> Duration {
        let now = self.now();
        self.alerts
            .borrow()
            .alerts
            .iter()
            .map(|alert| alert.last_at + ALERT_WINDOW)
            .min()
            .map_or(UPKEEP_EVERY, |leaves| {
                Duration::try_from(now.duration_until(leaves))
                    .unwrap_or(Duration::ZERO)
                    .min(UPKEEP_EVERY)
                    + Duration::from_millis(1)
            })
    }
}

fn tree_audit(details: Value) -> Record {
    Record::operator(ActorKind::System, AuditAction::TreeProcessStarted)
        .target(Servers::DEFAULT)
        .details(details)
}

async fn build_alerts(db: &Database, now: Timestamp) -> Result<AlertList, Error> {
    let since = now - ALERT_WINDOW;
    let (shells, crashes) = db
        .read(move |conn| {
            Ok::<_, Error>((
                store::shells_since(conn, since)?,
                store::crashes_since(conn, since, now)?,
            ))
        })
        .await?;
    Ok(alerts(&shells, &crashes, since))
}

/// The alert list: shells and crashes of the window, newest first, at most 10. It names files,
/// never paths, command lines or addresses.
fn alerts(shells: &[TreeProcess], crashes: &[Crash], since: Timestamp) -> AlertList {
    let mut alerts: Vec<Alert> = shells
        .iter()
        .map(|process| Alert {
            key: format!("process:{}", process.id),
            kind: AlertKind::Shell,
            at: process.started_at,
            last_at: process.last_started_at,
            count: process.count,
            name: process.name.clone(),
            parent: process.parent.as_ref().and_then(|parent| parent.name.clone()),
            crash: None,
        })
        .chain(crashes.iter().map(crash_alert))
        .collect();
    alerts.sort_by_key(|alert| std::cmp::Reverse(alert.last_at));
    alerts.truncate(MAX_ALERTS);
    AlertList { alerts, since }
}

fn crash_alert(crash: &Crash) -> Alert {
    let analysis = crash.analysis.as_ref();
    let main = analysis.and_then(|analysis| analysis.threads.first());
    let exit = match (crash.analysis_state, &crash.exit_code, crash.signal) {
        (AnalysisState::NoDump, _, Some(signal)) => Some(
            observer_fxserver::exit::signal_name(signal)
                .map_or_else(|| format!("signal {signal}"), |(name, _)| name.to_owned()),
        ),
        (AnalysisState::NoDump, Some(code), None) => Some(code.clone()),
        _ => None,
    };
    Alert {
        key: format!("crash:{}", crash.id),
        kind: if crash.kind == CrashKind::Hang {
            AlertKind::Hang
        } else {
            AlertKind::Crash
        },
        at: crash.at,
        last_at: crash.at,
        count: 1,
        name: None,
        parent: None,
        crash: Some(AlertCrash {
            exception: analysis
                .and_then(|analysis| analysis.exception.as_ref())
                .map(|exception| AlertException {
                    code: exception.code.clone(),
                    name: exception.name.clone(),
                }),
            module: analysis
                .and_then(|analysis| analysis.frames.iter().find_map(|frame| frame.module.clone())),
            thread: analysis.and_then(|analysis| analysis.thread.as_ref()?.name.clone()),
            exit,
            busy: main.is_some_and(|main| {
                main.state == ThreadRun::Running && main.cpu.is_some_and(|cpu| cpu >= 90.0)
            }),
            waiting_in: main
                .filter(|main| main.state != ThreadRun::Running)
                .and_then(|main| main.syscall.clone()),
            snapshot: crash
                .files
                .iter()
                .any(|file| matches!(file.kind, CrashFileKind::HangDump | CrashFileKind::HangReport)),
            group_count: crash.group.map_or(1, |group| group.count),
        }),
    }
}

#[cfg(test)]
mod tests;
