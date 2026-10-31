//! What changed on the server between its starts. At each spawn of FXServer a job records what the
//! run starts from: the listing at once, the hashing of what changed once the server is online. One
//! task follows the supervisor for the spawns, the resources the bridge reports started, and the
//! starts and stops while a run is up; another keeps the history within its bounds.

mod compare;
mod job;
mod plan;
mod settings;
mod views;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use jiff::{SignedDuration, Timestamp};
use observer_domain::audit::AuditAction;
use observer_domain::changes::{
    ChangesLive, ChangesSettings, LiveChangeKind, MAX_TEXT_LIMIT, Purged, StartCause, valid_glob,
};
use observer_domain::error::{ApiError, ErrorCode};
use observer_domain::launch::LaunchSettings;
use observer_fxserver::changes::{Keys, Limits, ScanSettings, mask};
use observer_fxserver::command;
use observer_fxserver::supervisor::{Cause, Event, EventKind, ResourceEvent, State, Status};
use observer_store::changes::live::{self, LiveChange};
use observer_store::changes::{index, resources, snapshots, texts, upkeep};
use observer_store::{ChangesDatabase, Database, StoreError, launch};
use serde_json::json;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{Mutex as AsyncMutex, broadcast, watch};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use self::job::{Ended, Spawn};
pub use self::views::ComparedTo;
use crate::audit::Record;
use crate::{Caller, Clock, Error, Servers, Supervision};
pub use observer_fxserver::changes::diff::DiffOptions;

/// The one server Observer runs.
pub(crate) const SERVER: &str = Servers::DEFAULT;
const UPKEEP_EVERY: Duration = Duration::from_hours(1);
/// A console command matches a live change this long before it at most.
const COMMAND_WINDOW: SignedDuration = SignedDuration::from_secs(2);
/// Live changes a run records at most; past them a crash loop of `ensure` adds nothing.
const MAX_LIVE_CHANGES: u32 = 2000;
const MAX_NAME: usize = 64;
/// Text bodies masked again in one go of the upkeep pass.
const REMASK_BATCH: u32 = 200;
/// Runs whose snapshot a late report may still name.
const RUNS_KEPT: u32 = 8;
/// A check of the disk waits this long after the previous one.
const CHECK_EVERY: Duration = Duration::from_secs(30);
const MAX_GLOBS: usize = 200;
const MAX_EXTENSIONS: usize = 200;
/// When the last full verification ended, in the state database's settings.
const VERIFIED_AT: &str = "changes.verified_at";
/// The live state is read again at most this often while a job writes.
const LIVE_EVERY: Duration = Duration::from_millis(500);

/// What [`Changes::start`] works with.
pub struct ChangesSetup {
    /// The state database: settings, the keys and the audit log.
    pub db: Database,
    pub store: ChangesDatabase,
    pub clock: Clock,
    pub supervision: Supervision,
    /// Observer's data folder, never read even when it lies inside the server folder.
    pub data_dir: PathBuf,
    /// Where the builds are installed, never read either.
    pub artifacts: PathBuf,
}

/// The file changes of the default server. Clones share them.
#[derive(Clone)]
pub struct Changes {
    inner: Arc<Inner>,
}

struct Inner {
    shared: Arc<Shared>,
    supervision: Supervision,
    settings: Mutex<ChangesSettings>,
    /// The job running, if any: one at a time.
    job: AsyncMutex<Option<Running>>,
    /// When the latest check of the disk began.
    last_check: Mutex<Option<Instant>>,
    live: watch::Receiver<ChangesLive>,
}

/// What a job works with, shared by the service and its jobs.
pub(crate) struct Shared {
    pub db: Database,
    pub store: ChangesDatabase,
    pub clock: Clock,
    pub keys: Keys,
    pub data_dir: PathBuf,
    pub artifacts: PathBuf,
    /// The snapshot each recent run started, by generation.
    runs: Mutex<HashMap<u32, i64>>,
    feed: broadcast::Sender<i64>,
}

impl Shared {
    /// Tells followers that a snapshot changed: its state, or its counts.
    pub fn changed(&self, id: i64) {
        let _ = self.feed.send(id);
    }

    pub fn scan_settings(&self, settings: &ChangesSettings) -> ScanSettings {
        ScanSettings {
            extensions: settings.extensions.clone(),
            excluded: settings.excluded.clone(),
            hidden: settings.hidden.clone(),
            never_read: vec![self.data_dir.clone(), self.artifacts.clone()],
            limits: Limits {
                max_text: settings.max_text,
                ..Limits::default()
            },
        }
    }

    /// The run of `generation` takes the snapshot `id`.
    pub fn started(&self, generation: u32, id: i64) {
        let mut runs = self.runs.lock().unwrap_or_else(PoisonError::into_inner);
        runs.insert(generation, id);
        runs.retain(|known, _| *known + RUNS_KEPT >= generation);
    }

    fn run(&self, generation: u32) -> Option<i64> {
        self.runs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&generation)
            .copied()
    }
}

/// A job and how to stop it.
struct Running {
    cancel: CancellationToken,
    task: JoinHandle<()>,
}

impl Running {
    async fn stop(self) {
        self.cancel.cancel();
        if let Err(error) = self.task.await {
            warn!(%error, "a file changes job failed");
        }
    }
}

/// The task that follows the supervisor; [`ChangesTask::close`] ends it and the job it runs.
pub struct ChangesTask {
    task: JoinHandle<()>,
    stop: CancellationToken,
}

impl ChangesTask {
    pub async fn close(self) {
        self.stop.cancel();
        if let Err(error) = self.task.await {
            warn!(%error, "the file changes task failed");
        }
    }
}

impl Changes {
    /// Reads the settings and the keys, ends what an earlier Observer left running, and follows
    /// the supervisor. Starting it before FXServer boots lets it see the first run.
    pub async fn start(setup: ChangesSetup) -> Result<(Self, ChangesTask), Error> {
        let (keys, current) = setup
            .db
            .write(|tx| -> Result<_, Error> { Ok((settings::keys(tx)?, settings::load(tx)?)) })
            .await?;
        let now = setup.clock.now();
        let interrupted = setup
            .store
            .write(move |tx| snapshots::interrupted(tx, now))
            .await?;
        if interrupted > 0 {
            info!(
                snapshots = interrupted,
                "snapshots an earlier Observer left unfinished are partial"
            );
        }
        let supervisor = setup.supervision.supervisor().clone();
        let events = supervisor.events();
        let status = supervisor.status();
        let shared = Arc::new(Shared {
            db: setup.db,
            store: setup.store,
            clock: setup.clock,
            keys,
            data_dir: setup.data_dir,
            artifacts: setup.artifacts,
            runs: Mutex::new(HashMap::new()),
            feed: broadcast::channel(64).0,
        });
        let (live_tx, live) = watch::channel(ChangesLive::default());
        let changes = Self {
            inner: Arc::new(Inner {
                shared,
                supervision: setup.supervision,
                settings: Mutex::new(current),
                job: AsyncMutex::new(None),
                last_check: Mutex::new(None),
                live,
            }),
        };
        changes.refresh(&live_tx).await;
        let stop = CancellationToken::new();
        let driver = Driver {
            changes: changes.clone(),
            causes: HashMap::new(),
            recorded: HashMap::new(),
        };
        let task = tokio::spawn(driver.run(events, status, live_tx, stop.clone()));
        Ok((changes, ChangesTask { task, stop }))
    }

    /// The latest start and check as they change, for the live topic.
    pub fn live(&self) -> watch::Receiver<ChangesLive> {
        self.inner.live.clone()
    }

    /// Reads the live state again and sends it when it moved.
    async fn refresh(&self, live: &watch::Sender<ChangesLive>) {
        match self.latest().await {
            Ok(latest) => {
                live.send_if_modified(|current| {
                    let moved = *current != latest;
                    *current = latest;
                    moved
                });
            }
            Err(error) => warn!(%error, "cannot read the latest snapshot"),
        }
    }

    pub fn settings(&self) -> ChangesSettings {
        self.inner
            .settings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Lists and hashes the server folder now, to compare with the running start: what the next
    /// restart applies. One at a time, 30 seconds apart, and never while a start's job runs.
    pub async fn check(&self) -> Result<(), Error> {
        let mut job = self.inner.job.lock().await;
        if job.as_ref().is_some_and(|running| !running.task.is_finished()) {
            return Err(ErrorCode::CheckRunning.into());
        }
        {
            let last = self
                .inner
                .last_check
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if let Some(left) = last.and_then(|at| CHECK_EVERY.checked_sub(at.elapsed())) {
                let retry = left.as_secs().max(1);
                return Err(ApiError::new(ErrorCode::CheckTooSoon)
                    .with("retryAfter", retry)
                    .into());
            }
        }
        let launch = self
            .inner
            .shared
            .db
            .read(|conn| launch::get(conn, SERVER))
            .await?
            .ok_or(ErrorCode::LaunchNotConfigured)?
            .settings;
        *self
            .inner
            .last_check
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(Instant::now());
        let (shared, settings) = (Arc::clone(&self.inner.shared), self.settings());
        let cancel = CancellationToken::new();
        let task = tokio::spawn({
            let cancel = cancel.clone();
            async move {
                match job::check(shared, settings, launch, cancel).await {
                    Ok(Ended::Done(id)) => info!(snapshot = id, "the disk is checked"),
                    Ok(Ended::Cancelled(_) | Ended::Failed) => {}
                    Err(error) => warn!(%error, "the check of the disk failed"),
                }
            }
        });
        *job = Some(Running { cancel, task });
        Ok(())
    }

    /// Raises `observer:startChanges` for the start `id`, in the run it started. Best effort, as
    /// every event: a run that ended or a console that is not read gets nothing.
    async fn announce(&self, id: i64) {
        match self.announcement(id).await {
            Ok(event) => {
                if let Err(error) = self.inner.supervision.supervisor().announce(event).await {
                    info!(%error, snapshot = id, "observer:startChanges was not raised");
                }
            }
            Err(error) => warn!(%error, snapshot = id, "cannot tell resources what changed"),
        }
    }

    /// Reads every file again at the capped rate, whatever the index says: what a copy that kept
    /// both size and time hid shows at the next start.
    pub async fn verify(&self, caller: &Caller) -> Result<(), Error> {
        let mut job = self.inner.job.lock().await;
        if job.as_ref().is_some_and(|running| !running.task.is_finished()) {
            return Err(ErrorCode::CheckRunning.into());
        }
        let launch = self
            .inner
            .shared
            .db
            .read(|conn| launch::get(conn, SERVER))
            .await?
            .ok_or(ErrorCode::LaunchNotConfigured)?
            .settings;
        let record = Record::by(caller, AuditAction::ChangeVerifyRequested).target(SERVER);
        let now = self.inner.shared.clock.now();
        self.inner
            .shared
            .db
            .write(move |tx| record.write(tx, now).map_err(Error::from))
            .await?;
        let (shared, settings) = (Arc::clone(&self.inner.shared), self.settings());
        let cancel = CancellationToken::new();
        let task = tokio::spawn(run_verification(shared, settings, launch, cancel.clone()));
        *job = Some(Running { cancel, task });
        Ok(())
    }

    /// Clears every kept version of the file at `path`, in any start or check, with no copy left
    /// in the database's free pages, its log or the copies set aside. A version shared with
    /// another path goes too: it is stored once.
    pub async fn purge(&self, caller: &Caller, path: String) -> Result<Purged, Error> {
        let store = &self.inner.shared.store;
        let wanted = path.clone();
        let digests = store.read(move |conn| upkeep::digests_of(conn, &wanted)).await?;
        let (versions, bytes) = store
            .maintain(move |conn| -> Result<(usize, u64), Error> {
                conn.pragma_update(None, "secure_delete", true)
                    .map_err(StoreError::from)?;
                let transaction = conn.transaction().map_err(StoreError::from)?;
                let cleared = upkeep::clear_bodies(&transaction, &digests)?;
                transaction.commit().map_err(StoreError::from)?;
                upkeep::compact(conn)?;
                conn.pragma_update(None, "secure_delete", false)
                    .map_err(StoreError::from)?;
                Ok(cleared)
            })
            .await?;
        for copy in store.set_asides() {
            if let Err(error) = tokio::fs::remove_file(&copy).await {
                warn!(%error, copy = %copy.display(), "a set-aside copy of the changes database could not be deleted");
            }
        }
        let purged = Purged {
            versions: u32::try_from(versions).unwrap_or(u32::MAX),
            bytes,
        };
        let record = Record::by(caller, AuditAction::ChangeContentsPurged)
            .target(path)
            .details(json!({ "versions": purged.versions, "bytes": purged.bytes }));
        let now = self.inner.shared.clock.now();
        self.inner
            .shared
            .db
            .write(move |tx| record.write(tx, now).map_err(Error::from))
            .await?;
        Ok(purged)
    }

    /// Stores the owner's settings, audited with what changed. New extensions forget the index, so
    /// that the next start reads every file again for what it is now.
    pub async fn change_settings(
        &self,
        caller: &Caller,
        changed: ChangesSettings,
    ) -> Result<ChangesSettings, Error> {
        let changed = checked(changed)?;
        let before = self.settings();
        if before == changed {
            return Ok(changed);
        }
        let record = Record::by(caller, AuditAction::ChangeSettingsChanged)
            .target(SERVER)
            .details(json!({ "before": before, "after": changed }));
        let reindex = before.extensions != changed.extensions;
        let saved = changed.clone();
        let now = self.inner.shared.clock.now();
        self.inner
            .shared
            .db
            .write(move |tx| -> Result<(), Error> {
                settings::save(tx, &saved)?;
                record.write(tx, now)?;
                Ok(())
            })
            .await?;
        *self.inner.settings.lock().unwrap_or_else(PoisonError::into_inner) = changed.clone();
        if reindex {
            let forgotten = self.inner.shared.store.write(|tx| index::clear(tx)).await?;
            info!(
                files = forgotten,
                "the extensions changed: the next start reads every file again"
            );
        }
        Ok(changed)
    }

    /// Starts the snapshot of a spawn, after the job of an earlier one has stopped.
    async fn spawned(&self, spawn: Spawn) {
        let settings = self.settings();
        let mut job = self.inner.job.lock().await;
        if let Some(running) = job.take() {
            running.stop().await;
        }
        if !settings.enabled {
            return;
        }
        let shared = Arc::clone(&self.inner.shared);
        let status = self.inner.supervision.supervisor().status();
        let cancel = CancellationToken::new();
        let generation = spawn.generation;
        let launch = spawn.launch.clone();
        let changes = self.clone();
        let task = tokio::spawn({
            let cancel = cancel.clone();
            async move {
                let started = job::start(
                    Arc::clone(&shared),
                    settings.clone(),
                    spawn,
                    status,
                    cancel.clone(),
                )
                .await;
                if let Ok(Ended::Done(id)) = &started
                    && settings.start_event
                {
                    changes.announce(*id).await;
                }
                if matches!(started, Ok(Ended::Done(_))) && verification_due(&shared, &settings).await {
                    run_verification(shared, settings, launch, cancel).await;
                }
                match started {
                    Ok(Ended::Done(id)) => info!(snapshot = id, generation, "the start's snapshot is taken"),
                    Ok(Ended::Cancelled(id)) => {
                        info!(snapshot = id, generation, "the start's snapshot was cut short");
                    }
                    Ok(Ended::Failed) => {}
                    Err(error) => warn!(%error, generation, "the start's snapshot failed"),
                }
            }
        });
        *job = Some(Running { cancel, task });
    }

    /// Keeps the history within retention and the budget, and masks again the texts older rules
    /// masked. Skipped while a job runs: what it has stored is not referenced yet.
    async fn upkeep(&self) -> Result<(), Error> {
        let job = self.inner.job.lock().await;
        if job.as_ref().is_some_and(|running| !running.task.is_finished()) {
            return Ok(());
        }
        let settings = self.settings();
        let store = &self.inner.shared.store;
        let cutoff = self.inner.shared.clock.now()
            - SignedDuration::from_hours(24 * i64::from(settings.retention_days));
        let (expired, (trees, texts)) = store
            .write(move |tx| -> Result<_, Error> {
                let expired = upkeep::expire(tx, SERVER, cutoff)?;
                Ok((expired, upkeep::unreferenced(tx)?))
            })
            .await?;
        let protected: Vec<i64> = store
            .read(|conn| snapshots::starts(conn, SERVER, None, 2))
            .await?
            .iter()
            .map(|snapshot| snapshot.id)
            .collect();
        let budget = settings.budget;
        let (dropped, freed) = store
            .write(move |tx| upkeep::enforce_budget(tx, budget, &protected))
            .await?;
        if dropped > 0 {
            info!(
                texts = dropped,
                bytes = freed,
                "texts past the budget lost their contents"
            );
        }
        self.remask().await?;
        store.maintain(upkeep::compact).await?;
        if expired + trees + texts > 0 {
            info!(
                starts = expired,
                trees, texts, "the file changes history was trimmed"
            );
        }
        Ok(())
    }

    /// Masks again, with the current rules, the texts older rules masked: masking only removes
    /// text, so the stored bodies are enough.
    async fn remask(&self) -> Result<(), Error> {
        let store = &self.inner.shared.store;
        let tag_key = self.inner.shared.keys.tag;
        loop {
            let older = store
                .read(|conn| texts::masked_before(conn, mask::VERSION, REMASK_BATCH))
                .await?;
            if older.is_empty() {
                return Ok(());
            }
            let mut again = Vec::with_capacity(older.len());
            for (digest, text) in older {
                let path = store
                    .read({
                        let digest = digest.clone();
                        move |conn| texts::path_of(conn, &digest)
                    })
                    .await?
                    .unwrap_or_default();
                let Some(body) = text.text else {
                    continue;
                };
                let masked = mask::remask(
                    &mask::Masked {
                        text: body,
                        redactions: mask::Redaction::decode(&text.redactions),
                    },
                    &path,
                    path.to_ascii_lowercase().ends_with(".cfg"),
                    &tag_key,
                );
                again.push((digest, masked));
            }
            store
                .write(move |tx| -> Result<(), Error> {
                    for (digest, masked) in &again {
                        texts::remasked(
                            tx,
                            digest,
                            &masked.text,
                            mask::VERSION,
                            &mask::Redaction::encode(&masked.redactions),
                        )?;
                    }
                    Ok(())
                })
                .await?;
        }
    }

    /// The bridge reported the resources started: the first report of a run is its snapshot's,
    /// and the starts and stops it tells are live changes.
    async fn resources(
        &self,
        generation: u32,
        at: Timestamp,
        started: Vec<String>,
        events: Vec<ResourceEvent>,
    ) -> Result<(), Error> {
        let Some(id) = self.inner.shared.run(generation) else {
            return Ok(());
        };
        let changes = live_changes(&events, at);
        let commands = if changes.is_empty() {
            Vec::new()
        } else {
            let from = at - COMMAND_WINDOW - SignedDuration::from_millis(500);
            self.inner
                .shared
                .db
                .read(move |conn| {
                    observer_store::audit::of_actions(conn, &[AuditAction::ConsoleCommandSent], from, at, 100)
                })
                .await?
        };
        let attributed: Vec<LiveChange> = changes
            .into_iter()
            .map(|mut change| {
                if let Some((by, command)) = command_for(&change.name, &commands) {
                    change.by = Some(by);
                    change.command = Some(command);
                }
                change
            })
            .collect();
        self.inner
            .shared
            .store
            .write(move |tx| -> Result<(), Error> {
                snapshots::set_started(tx, id, &started)?;
                let recorded = live::list(tx, id, MAX_LIVE_CHANGES)?.len();
                let room = (MAX_LIVE_CHANGES as usize).saturating_sub(recorded);
                for change in attributed.iter().take(room) {
                    live::insert(tx, id, change)?;
                    if change.change != LiveChangeKind::Stopped
                        && let Some(snapshot) = snapshots::get(tx, id)?
                    {
                        for row in resources::rows(tx, snapshot.holder())? {
                            if row.name == change.name {
                                snapshots::reloaded(tx, id, &row.path)?;
                            }
                        }
                    }
                }
                Ok(())
            })
            .await?;
        self.inner.shared.changed(id);
        Ok(())
    }

    /// What the run reported of itself once it answered: its build when the build folder did not
    /// say, and the game build it enforces.
    async fn reported(&self, status: &Status) -> Result<(), Error> {
        let Some(id) = self.inner.shared.run(status.generation) else {
            return Ok(());
        };
        let (build, game_build) = (status.build, status.game_build);
        self.inner
            .shared
            .store
            .write(move |tx| -> Result<(), Error> {
                if let Some(build) = build {
                    snapshots::set_build(tx, id, build)?;
                }
                if let Some(game_build) = game_build {
                    snapshots::set_game_build(tx, id, game_build)?;
                }
                Ok(())
            })
            .await
    }
}

/// Whether the last full verification is older than the settings ask for.
async fn verification_due(shared: &Shared, settings: &ChangesSettings) -> bool {
    if settings.verify_days == 0 {
        return false;
    }
    let last = shared
        .db
        .read(|conn| observer_store::settings::get(conn, VERIFIED_AT))
        .await
        .ok()
        .flatten()
        .and_then(|at| at.parse::<Timestamp>().ok());
    let every = SignedDuration::from_hours(24 * i64::from(settings.verify_days));
    last.is_none_or(|last| shared.clock.now() - every >= last)
}

async fn run_verification(
    shared: Arc<Shared>,
    settings: ChangesSettings,
    launch: LaunchSettings,
    cancel: CancellationToken,
) {
    match job::verify(Arc::clone(&shared), settings, launch, cancel).await {
        Ok(true) => {
            let now = shared.clock.now().to_string();
            let saved = shared
                .db
                .write(move |tx| observer_store::settings::set(tx, VERIFIED_AT, &now))
                .await;
            if let Err(error) = saved {
                warn!(%error, "cannot record the end of the full verification");
            }
            info!("every file of the server was read again");
        }
        Ok(false) => {}
        Err(error) => warn!(%error, "the full verification failed"),
    }
}

/// The settings as they are stored: extensions in lower case without their dot, each once, and
/// globs that read the same on both systems. Anything else is refused.
fn checked(mut settings: ChangesSettings) -> Result<ChangesSettings, Error> {
    let invalid = |field: &str| ApiError::new(ErrorCode::InvalidRequest).with("field", field);
    for (field, globs) in [
        ("excluded", &mut settings.excluded),
        ("hidden", &mut settings.hidden),
    ] {
        if globs.len() > MAX_GLOBS {
            return Err(invalid(field).into());
        }
        for (index, glob) in globs.iter_mut().enumerate() {
            *glob = glob.trim().to_owned();
            if !valid_glob(glob) {
                return Err(ApiError::new(ErrorCode::InvalidGlob)
                    .with("field", field)
                    .with("index", index)
                    .into());
            }
        }
    }
    let mut extensions: Vec<String> = Vec::with_capacity(settings.extensions.len());
    for extension in &settings.extensions {
        let extension = extension.trim().trim_start_matches('.').to_ascii_lowercase();
        let valid = !extension.is_empty()
            && extension.len() <= 16
            && extension
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-');
        if !valid {
            return Err(invalid("extensions").into());
        }
        if !extensions.contains(&extension) {
            extensions.push(extension);
        }
    }
    if extensions.len() > MAX_EXTENSIONS {
        return Err(invalid("extensions").into());
    }
    settings.extensions = extensions;
    let ranges = [
        ("maxText", (1024..=MAX_TEXT_LIMIT).contains(&settings.max_text)),
        ("budget", settings.budget <= 1_000_000_000_000),
        ("retentionDays", (1..=3650).contains(&settings.retention_days)),
        (
            "hashRate",
            (1_000_000..=10_000_000_000).contains(&settings.hash_rate),
        ),
        ("verifyDays", settings.verify_days <= 365),
        ("driftMinutes", settings.drift_minutes <= 1440),
    ];
    if let Some((field, _)) = ranges.into_iter().find(|(_, valid)| !valid) {
        return Err(invalid(field).into());
    }
    Ok(settings)
}

/// The starts and stops of one report as live changes: a stop then a start of one name is a
/// restart, which leaves the list of started resources as it was.
fn live_changes(events: &[ResourceEvent], at: Timestamp) -> Vec<LiveChange> {
    let mut changes: Vec<LiveChange> = Vec::new();
    for event in events {
        let name: String = event.name.chars().take(MAX_NAME).collect();
        let last = changes.last_mut().filter(|last| last.name == name);
        match (last, event.started) {
            (Some(last), true) if last.change == LiveChangeKind::Stopped => {
                last.change = LiveChangeKind::Restarted;
            }
            (_, started) => changes.push(LiveChange {
                at,
                name,
                change: if started {
                    LiveChangeKind::Started
                } else {
                    LiveChangeKind::Stopped
                },
                by: None,
                command: None,
            }),
        }
    }
    changes
}

/// Who typed a command in Observer's console that starts, stops or restarts `name`, and the
/// command as the audit log shows it, masked.
fn command_for(name: &str, commands: &[observer_domain::audit::AuditEntry]) -> Option<(String, String)> {
    commands.iter().rev().find_map(|entry| {
        let shown = entry.details.get("command")?.as_str()?;
        let matches = command::split(shown).into_iter().any(|single| {
            let words = command::words(single);
            matches!(
                words.as_slice(),
                [verb, resource, ..]
                    if ["ensure", "start", "stop", "restart"].iter().any(|known| verb.eq_ignore_ascii_case(known))
                        && resource.eq_ignore_ascii_case(name)
            )
        });
        matches.then(|| (entry.actor_name.clone().unwrap_or_default(), shown.to_owned()))
    })
}

/// Follows the supervisor for the spawns and what the bridge reports.
struct Driver {
    changes: Changes,
    /// Why each recent run started, from its `Starting` state change.
    causes: HashMap<u32, (Option<StartCause>, Option<String>)>,
    /// What each run has reported of itself, so that it is written once.
    recorded: HashMap<u32, (Option<u32>, Option<u32>)>,
}

impl Driver {
    async fn run(
        mut self,
        mut events: broadcast::Receiver<Event>,
        mut status: watch::Receiver<Status>,
        live: watch::Sender<ChangesLive>,
        stop: CancellationToken,
    ) {
        let mut upkeep = tokio::time::interval(UPKEEP_EVERY);
        upkeep.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        upkeep.reset();
        let mut changed = self.changes.inner.shared.feed.subscribe();
        let mut refresh: Option<tokio::time::Instant> = None;
        loop {
            let due = refresh.unwrap_or_else(|| tokio::time::Instant::now() + UPKEEP_EVERY);
            tokio::select! {
                () = stop.cancelled() => break,
                () = tokio::time::sleep_until(due), if refresh.is_some() => {
                    refresh = None;
                    self.changes.refresh(&live).await;
                }
                moved = changed.recv() => {
                    if matches!(moved, Err(RecvError::Closed)) {
                        break;
                    }
                    refresh.get_or_insert_with(|| tokio::time::Instant::now() + LIVE_EVERY);
                }
                event = events.recv() => match event {
                    Ok(event) => self.event(event).await,
                    Err(RecvError::Lagged(missed)) => warn!(missed, "file changes missed supervisor events"),
                    Err(RecvError::Closed) => break,
                },
                changed = status.changed() => {
                    if changed.is_err() {
                        break;
                    }
                    let now = status.borrow_and_update().clone();
                    self.status(&now).await;
                }
                _ = upkeep.tick() => {
                    if let Err(error) = self.changes.upkeep().await {
                        warn!(%error, "the file changes upkeep failed");
                    }
                }
            }
        }
        if let Some(running) = self.changes.inner.job.lock().await.take() {
            running.stop().await;
        }
    }

    async fn event(&mut self, event: Event) {
        let generation = event.generation;
        let at = Timestamp::from_millisecond(event.at_ms)
            .unwrap_or_else(|_| self.changes.inner.shared.clock.now());
        match event.kind {
            EventKind::State {
                to: State::Starting,
                cause,
                ..
            } => {
                self.causes.insert(generation, start_cause(&cause));
                self.causes.retain(|known, _| *known + RUNS_KEPT >= generation);
            }
            EventKind::Spawned { build, launch, .. } => {
                let (cause, by) = self.causes.get(&generation).cloned().unwrap_or((None, None));
                self.recorded.retain(|known, _| *known + RUNS_KEPT >= generation);
                let spawn = Spawn {
                    generation,
                    run_started: at,
                    cause,
                    by,
                    launch: *launch,
                    build,
                };
                self.changes.spawned(spawn).await;
            }
            EventKind::Resources { started, events, .. } => {
                if let Err(error) = self.changes.resources(generation, at, started, events).await {
                    warn!(%error, "cannot record the resources a run reported");
                }
            }
            _ => {}
        }
    }

    async fn status(&mut self, status: &Status) {
        let reported = (status.build, status.game_build);
        if reported == (None, None) || self.recorded.get(&status.generation) == Some(&reported) {
            return;
        }
        if self.changes.inner.shared.run(status.generation).is_none() {
            return;
        }
        self.recorded.insert(status.generation, reported);
        if let Err(error) = self.changes.reported(status).await {
            warn!(%error, "cannot record the build a run reported");
        }
    }
}

/// Why a run started, and who asked, from the cause of its `Starting` state.
fn start_cause(cause: &Cause) -> (Option<StartCause>, Option<String>) {
    match cause {
        Cause::Requested { by } => (Some(StartCause::Requested), Some(by.clone())),
        Cause::Restart { by } => (Some(StartCause::Restart), Some(by.clone())),
        Cause::Scheduled { by } => (Some(StartCause::Scheduled), Some(by.clone())),
        Cause::Recovery => (Some(StartCause::Recovery), None),
        _ => (None, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(name: &str, started: bool) -> ResourceEvent {
        ResourceEvent {
            name: name.to_owned(),
            started,
        }
    }

    #[test]
    fn a_stop_then_a_start_of_one_resource_is_a_restart() {
        let at = Timestamp::from_second(1_793_000_000).unwrap();

        let changes = live_changes(
            &[
                event("a", false),
                event("a", true),
                event("b", true),
                event("c", false),
            ],
            at,
        );

        let kinds: Vec<(&str, LiveChangeKind)> =
            changes.iter().map(|c| (c.name.as_str(), c.change)).collect();
        assert_eq!(
            kinds,
            [
                ("a", LiveChangeKind::Restarted),
                ("b", LiveChangeKind::Started),
                ("c", LiveChangeKind::Stopped)
            ]
        );
    }
}
