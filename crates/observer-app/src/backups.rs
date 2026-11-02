//! Backups of the game server's database: the owner's settings and destinations, one run at a
//! time, the one before a restart, the timed ones, the retention, the downloads. The dump and the
//! copies are `observer-backups`'; this service decides when, records every run and copy, and
//! audits every change. Nothing here ever hands a password or a key back out.

mod job;

use std::collections::HashSet;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};
use observer_backups::Failure;
use observer_backups::destination::{self, Reader};
use observer_backups::destination::{local, sftp};
use observer_backups::dump::{self, Client};
use observer_domain::audit::{ActorKind, AuditAction, Outcome};
use observer_domain::backups::{
    BackupDestination, BackupError, BackupPhase, BackupProgress, BackupRun, BackupRunsPage, BackupSettings,
    BackupSettingsChange, BeforeRestarts, ClientView, CopyState, DatabaseConnection, DeleteReason,
    DestinationChange, DestinationDraft, DestinationField, DestinationIssue, DestinationTarget,
    DestinationTestRequest, DestinationTestResult, FieldProblem, MAX_RESTART_WAIT_SECONDS, Secret, SftpAuth,
    SourceKind, SourceTestRequest, SourceTestResult, SourceView, parse_connection_string,
};
use observer_domain::error::{ApiError, ErrorCode};
use observer_domain::schedule::ScheduleStatus;
use observer_fxserver::cfg::ConnectionString;
use observer_fxserver::pre_launch::{self, PreLaunch, PreLaunchFuture, PreLaunchRequest};
use observer_fxserver::supervisor::Cause;
use observer_store::backups::{self as store, DestinationRow, NewRun, Saved};
use observer_store::{Database, launch};
use serde_json::{Map, Value, json};
use tokio::sync::{Mutex, Notify, watch};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::{debug, info, warn};

use self::job::Job;
use crate::audit::Record;
use crate::schedule::fixed_occurrences;
use crate::{Caller, Clock, Error, Servers, ToolsHook};

const RECENT_RUNS: u32 = 5;
const DEFAULT_PAGE: u32 = 50;
const MAX_PAGE: u32 = 200;
/// A fixed time found later than this, after a host slept through it, is given up.
const LATE: SignedDuration = SignedDuration::from_mins(15);
/// How long the tool found is trusted before it is looked for again.
const CLIENT_KEPT: Duration = Duration::from_secs(30);
/// How often a restart waiting for its backup's dump looks at the stop.
const WAIT_STEP: Duration = Duration::from_millis(100);
/// The tools' own time before a spawn, as before the backups shared the phase.
pub const TOOLS_LIMIT: Duration = pre_launch::DEFAULT_LIMIT;
/// The phase before a spawn: the longest a restart may wait for its backup, and a minute more.
pub const PRE_LAUNCH_LIMIT: Duration = Duration::from_secs(MAX_RESTART_WAIT_SECONDS as u64 + 60);

/// What [`Backups::start`] works with.
pub struct BackupsSetup {
    pub db: Database,
    pub clock: Clock,
    /// Observer's data folder: runs work in its `database-backups/.work`.
    pub data_dir: PathBuf,
    /// The restart schedule, whose zone the times are in; the host's zone without it.
    pub schedule: Option<watch::Receiver<ScheduleStatus>>,
}

#[derive(Debug, Clone, Copy)]
pub struct BackupsTimings {
    /// The longest the timer sleeps between two looks at the clock, which absorbs a clock that
    /// jumps and a host that slept.
    pub longest_sleep: Duration,
}

impl Default for BackupsTimings {
    fn default() -> Self {
        Self {
            longest_sleep: Duration::from_secs(30),
        }
    }
}

/// The backups of the default server. Clones share the run under way.
#[derive(Clone)]
pub struct Backups {
    inner: Arc<Inner>,
}

struct Inner {
    db: Database,
    clock: Clock,
    work: PathBuf,
    schedule: Option<watch::Receiver<ScheduleStatus>>,
    running: Mutex<Option<Running>>,
    tasks: TaskTracker,
    stop: CancellationToken,
    /// Rings when the settings change, so that the timer looks at the times again.
    changed: Notify,
    client: std::sync::Mutex<Option<FoundClient>>,
}

struct FoundClient {
    settings: (observer_domain::backups::DatabaseEngine, Option<String>),
    at: std::time::Instant,
    found: Result<Client, Failure>,
}

/// The run under way.
#[derive(Clone)]
struct Running {
    run: i64,
    cancel: CancellationToken,
    progress: Arc<Progress>,
    dumped: watch::Receiver<Option<Dumped>>,
    server_started: Arc<AtomicBool>,
}

/// How a run's dump ended, which a restart waits for.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Dumped {
    Done { database: String, file_bytes: u64 },
    Failed(BackupError),
}

/// What a run is doing, as the page shows it while it runs.
#[derive(Debug, Default)]
struct Progress {
    phase: std::sync::Mutex<Option<BackupPhase>>,
    sql: Arc<AtomicU64>,
    copied: Arc<AtomicU64>,
    destination: std::sync::Mutex<Option<i64>>,
}

impl Progress {
    fn phase(&self, phase: BackupPhase) {
        if let Ok(mut current) = self.phase.lock() {
            *current = Some(phase);
        }
    }

    fn copying(&self, destination: i64) {
        self.copied.store(0, Ordering::Relaxed);
        if let Ok(mut current) = self.destination.lock() {
            *current = Some(destination);
        }
    }
}

/// Who asked for a run, which its record and its audit entries name.
#[derive(Debug, Clone)]
enum Asker {
    User(Caller),
    /// A restart, by whoever made it: `observer` for the schedule's own.
    Restart {
        by: String,
    },
    Schedule,
}

impl Asker {
    fn trigger(&self) -> observer_domain::backups::BackupTrigger {
        use observer_domain::backups::BackupTrigger;
        match self {
            Self::User(_) => BackupTrigger::Manual,
            Self::Restart { .. } => BackupTrigger::Restart,
            Self::Schedule => BackupTrigger::Schedule,
        }
    }

    fn actor(&self) -> Option<String> {
        match self {
            Self::User(caller) => Some(caller.username.clone()),
            Self::Restart { by } if by != "observer" => Some(by.clone()),
            Self::Restart { .. } | Self::Schedule => None,
        }
    }

    fn record(&self, action: AuditAction) -> Record {
        match self {
            Self::User(caller) => Record::by(caller, action),
            Self::Restart { .. } | Self::Schedule => Record::operator(ActorKind::System, action),
        }
    }
}

/// The backups' timer; [`BackupsTask::close`] ends it, and the run under way.
pub struct BackupsTask {
    timer: JoinHandle<()>,
    inner: Arc<Inner>,
}

impl BackupsTask {
    /// Cancels the run under way and the timer, and waits for both.
    pub async fn close(self) {
        self.inner.stop.cancel();
        if let Some(running) = self.inner.running.lock().await.as_ref() {
            running.cancel.cancel();
        }
        if let Err(error) = self.timer.await {
            warn!(%error, "the backups' timer failed");
        }
        self.inner.tasks.close();
        self.inner.tasks.wait().await;
    }
}

/// A copy on its way out.
pub struct Download {
    pub file_name: String,
    pub bytes: Option<u64>,
    pub reader: Reader,
}

impl fmt::Debug for Download {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Download")
            .field("file_name", &self.file_name)
            .field("bytes", &self.bytes)
            .finish_non_exhaustive()
    }
}

impl Backups {
    /// Ends the runs an earlier Observer left unfinished, empties the work folder, then follows the
    /// clock for the timed runs. `tune` adjusts the timings, for tests.
    pub async fn start(
        setup: BackupsSetup,
        tune: impl FnOnce(&mut BackupsTimings),
    ) -> Result<(Self, BackupsTask), Error> {
        let mut timings = BackupsTimings::default();
        tune(&mut timings);
        let work = setup.data_dir.join("database-backups").join(".work");
        let inner = Arc::new(Inner {
            db: setup.db,
            clock: setup.clock,
            work,
            schedule: setup.schedule,
            running: Mutex::new(None),
            tasks: TaskTracker::new(),
            stop: CancellationToken::new(),
            changed: Notify::new(),
            client: std::sync::Mutex::new(None),
        });
        let backups = Self { inner };
        backups.recover().await?;
        let timer = tokio::spawn(backups.clone().timer(timings));
        let task = BackupsTask {
            timer,
            inner: Arc::clone(&backups.inner),
        };
        Ok((backups, task))
    }

    async fn recover(&self) -> Result<(), Error> {
        let now = self.inner.clock.now();
        self.inner
            .db
            .write(move |tx| -> Result<(), Error> {
                for run in store::end_unfinished_runs(tx, now)? {
                    let mut details = json!({
                        "trigger": run.trigger.as_str(),
                        "outcome": "interrupted",
                        "error": BackupError::Interrupted.as_str(),
                    });
                    if let Some(database) = &run.database {
                        details["database"] = json!(database);
                    }
                    if let Some(actor) = &run.actor {
                        details["actor"] = json!(actor);
                    }
                    Record::operator(ActorKind::System, AuditAction::BackupFinished)
                        .target(run.id.to_string())
                        .outcome(Outcome::Failure)
                        .details(details)
                        .write(tx, now)?;
                }
                Ok(())
            })
            .await?;
        let work = self.inner.work.clone();
        blocking(move || {
            match std::fs::remove_dir_all(&work) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error),
                _ => {}
            }
            std::fs::create_dir_all(&work)
        })
        .await??;
        Ok(())
    }

    async fn stored_settings(&self) -> Result<(BackupSettings, Option<i64>), Error> {
        let stored = self
            .inner
            .db
            .read(|conn| store::settings(conn, Servers::DEFAULT))
            .await?;
        Ok(stored.map_or_else(
            || (BackupSettings::default(), None),
            |stored| (stored.settings, Some(stored.version)),
        ))
    }

    /// The Backups page: the settings, where the next run would read from and with what, the
    /// destinations, the run under way and the latest runs.
    pub async fn view(&self) -> Result<observer_domain::backups::BackupsView, Error> {
        let (settings, version) = self.stored_settings().await?;
        let (has_password, destinations, recent) = self
            .inner
            .db
            .read(|conn| -> Result<_, Error> {
                Ok((
                    store::has_password(conn, Servers::DEFAULT)?,
                    store::destinations(conn, Servers::DEFAULT)?,
                    store::runs(conn, Servers::DEFAULT, None, RECENT_RUNS)?,
                ))
            })
            .await?;
        let (zone_name, zone) = self.zone();
        let next_at = next_time(&settings, &zone, self.inner.clock.now());
        let source = self.source_view(&settings).await?;
        let client = client_view(self.client(&settings).await);
        let running = self.inner.running.lock().await.as_ref().map(|running| {
            let progress = &running.progress;
            BackupProgress {
                run: running.run,
                phase: progress
                    .phase
                    .lock()
                    .ok()
                    .and_then(|phase| *phase)
                    .unwrap_or(BackupPhase::Preparing),
                sql_bytes: progress.sql.load(Ordering::Relaxed),
                destination: progress
                    .destination
                    .lock()
                    .ok()
                    .and_then(|destination| *destination),
                copied_bytes: progress.copied.load(Ordering::Relaxed),
            }
        });
        Ok(observer_domain::backups::BackupsView {
            settings,
            version,
            has_password,
            zone: zone_name,
            next_at,
            source,
            client,
            destinations,
            running,
            recent,
        })
    }

    /// The zone the times are in: the restart schedule's, or the host's.
    fn zone(&self) -> (String, TimeZone) {
        let named = self
            .inner
            .schedule
            .as_ref()
            .map(|schedule| schedule.borrow().zone.clone())
            .and_then(|name| TimeZone::get(&name).ok().map(|zone| (name, zone)));
        named.unwrap_or_else(|| {
            let host = TimeZone::system();
            let name = host.iana_name().unwrap_or("UTC").to_owned();
            (name, host)
        })
    }

    /// Stores settings that pass every check, audited with what changed. `version` must be the
    /// one the change was made from, `None` before the first save.
    pub async fn change_settings(
        &self,
        caller: &Caller,
        change: BackupSettingsChange,
    ) -> Result<observer_domain::backups::BackupsView, Error> {
        let (before, _) = self.stored_settings().await?;
        let stored_password = self
            .inner
            .db
            .read(|conn| store::has_password(conn, Servers::DEFAULT))
            .await?;
        let password = change.password.filter(|password| !password.expose().is_empty());
        let issues = change.settings.issues(stored_password || password.is_some());
        if !issues.is_empty() {
            return Err(ApiError::new(ErrorCode::InvalidBackupSettings)
                .with("issues", serde_json::to_value(issues).unwrap_or_default())
                .into());
        }
        let settings = change.settings.normalized();
        let mut details = changes(&before, &settings);
        if password.is_some() {
            details["passwordChanged"] = json!(true);
        }
        let now = self.inner.clock.now();
        let record = Record::by(caller, AuditAction::BackupSettingsChanged)
            .target(Servers::DEFAULT)
            .details(details);
        let version = change.version;
        self.inner
            .db
            .write(move |tx| -> Result<(), Error> {
                store::save_settings(tx, Servers::DEFAULT, &settings, version, now)?
                    .ok_or(ErrorCode::Conflict)?;
                if let Some(password) = &password {
                    store::set_password(tx, Servers::DEFAULT, password)?;
                }
                record.write(tx, now)?;
                Ok(())
            })
            .await?;
        self.inner.changed.notify_one();
        self.view().await
    }

    /// The checks a destination's draft must pass besides its own: a host key that reads, and a
    /// folder outside the server's.
    async fn destination_issues(&self, draft: &DestinationDraft, has_secret: bool) -> Result<(), Error> {
        let mut issues = draft.issues(has_secret);
        match &draft.target {
            DestinationTarget::Local { folder } => {
                let folder = PathBuf::from(folder);
                let (server, work) = (self.server_folder().await?, self.inner.work.clone());
                let (in_server, in_work) = blocking(move || {
                    (
                        server.is_some_and(|server| local::inside(&folder, &server)),
                        local::inside(&folder, &work),
                    )
                })
                .await?;
                if in_server {
                    issues.push(DestinationIssue {
                        field: DestinationField::Folder,
                        problem: FieldProblem::InsideServerFolder,
                    });
                } else if in_work {
                    // Observer empties its work folder at every start.
                    issues.push(DestinationIssue {
                        field: DestinationField::Folder,
                        problem: FieldProblem::Invalid,
                    });
                }
            }
            DestinationTarget::Sftp {
                host_key: Some(key), ..
            } if sftp::parse_host_key(key).is_none() => issues.push(DestinationIssue {
                field: DestinationField::HostKey,
                problem: FieldProblem::Invalid,
            }),
            _ => {}
        }
        if issues.is_empty() {
            Ok(())
        } else {
            Err(ApiError::new(ErrorCode::InvalidBackupDestination)
                .with("issues", serde_json::to_value(issues).unwrap_or_default())
                .into())
        }
    }

    /// The server's folder, from the launch settings, when there are some.
    async fn server_folder(&self) -> Result<Option<PathBuf>, Error> {
        let launch = self
            .inner
            .db
            .read(|conn| launch::get(conn, Servers::DEFAULT))
            .await?;
        Ok(launch.map(|launch| PathBuf::from(launch.settings.data_path)))
    }

    /// Records a new destination. An SFTP destination that logs in with a key gets a new one of
    /// its own, whose public half the result carries.
    pub async fn create_destination(
        &self,
        caller: &Caller,
        draft: DestinationDraft,
    ) -> Result<BackupDestination, Error> {
        self.destination_issues(&draft, false).await?;
        let (secret, public_key) = match &draft.target {
            DestinationTarget::Sftp {
                auth: SftpAuth::Key, ..
            } => {
                let (private, public) = sftp::new_key().map_err(|failure| failure_error(&failure))?;
                (Some(private), Some(public))
            }
            _ => (draft.secret.clone(), None),
        };
        let now = self.inner.clock.now();
        let mut details = destination_details(&draft.name, &draft.target, draft.enabled, draft.retention);
        details["secretSet"] = json!(draft.secret.is_some());
        let caller = caller.clone();
        let id = self
            .inner
            .db
            .write(move |tx| -> Result<i64, Error> {
                let row = DestinationRow {
                    server_id: Servers::DEFAULT,
                    name: &draft.name,
                    enabled: draft.enabled,
                    target: &draft.target,
                    public_key: public_key.as_deref(),
                    retention: draft.retention,
                    at: now,
                };
                let id = store::insert_destination(tx, &row)?.ok_or(ErrorCode::NameTaken)?;
                if let Some(secret) = &secret {
                    store::set_destination_secret(tx, id, secret)?;
                }
                Record::by(&caller, AuditAction::BackupDestinationCreated)
                    .target(id.to_string())
                    .details(details)
                    .write(tx, now)?;
                Ok(id)
            })
            .await?;
        self.destination(id).await
    }

    async fn destination(&self, id: i64) -> Result<BackupDestination, Error> {
        self.inner
            .db
            .read(move |conn| store::destination(conn, Servers::DEFAULT, id))
            .await?
            .ok_or_else(|| ErrorCode::NotFound.into())
    }

    /// Saves a destination over the version it was read at. Its secret stays unless the change
    /// brings one, or changes how it logs in; a switch to key login makes a new key.
    pub async fn change_destination(
        &self,
        caller: &Caller,
        id: i64,
        change: DestinationChange,
    ) -> Result<BackupDestination, Error> {
        let before = self.destination(id).await?;
        let draft = change.destination;
        let same_login = same_login(&before.target, &draft.target);
        self.destination_issues(&draft, before.has_secret && same_login)
            .await?;
        let (secret, public_key) = match (&draft.target, same_login) {
            (
                DestinationTarget::Sftp {
                    auth: SftpAuth::Key, ..
                },
                false,
            ) => {
                let (private, public) = sftp::new_key().map_err(|failure| failure_error(&failure))?;
                (Some(private), Some(public))
            }
            (
                DestinationTarget::Sftp {
                    auth: SftpAuth::Key, ..
                },
                true,
            ) => (None, before.public_key.clone()),
            _ => (draft.secret.clone(), None),
        };
        let forget_secret = !same_login && secret.is_none();
        let mut details = destination_changes(&before, &draft);
        details["secretChanged"] = json!(secret.is_some() || forget_secret);
        let now = self.inner.clock.now();
        let caller = caller.clone();
        let version = change.version;
        self.inner
            .db
            .write(move |tx| -> Result<(), Error> {
                let row = DestinationRow {
                    server_id: Servers::DEFAULT,
                    name: &draft.name,
                    enabled: draft.enabled,
                    target: &draft.target,
                    public_key: public_key.as_deref(),
                    retention: draft.retention,
                    at: now,
                };
                match store::update_destination(tx, id, version, &row)? {
                    Saved::Done => {}
                    Saved::Conflict => return Err(ErrorCode::Conflict.into()),
                    Saved::NameTaken => return Err(ErrorCode::NameTaken.into()),
                }
                if let Some(secret) = &secret {
                    store::set_destination_secret(tx, id, secret)?;
                } else if forget_secret {
                    store::remove_destination_secret(tx, id)?;
                }
                Record::by(&caller, AuditAction::BackupDestinationChanged)
                    .target(id.to_string())
                    .details(details)
                    .write(tx, now)?;
                Ok(())
            })
            .await?;
        self.destination(id).await
    }

    /// Deletes a destination and forgets its copies; the files stay where they are.
    pub async fn delete_destination(&self, caller: &Caller, id: i64) -> Result<(), Error> {
        let before = self.destination(id).await?;
        let now = self.inner.clock.now();
        let record = Record::by(caller, AuditAction::BackupDestinationDeleted)
            .target(id.to_string())
            .details(json!({
                "name": before.name,
                "kind": before.target.kind().as_str(),
                "stored": before.stored,
            }));
        self.inner
            .db
            .write(move |tx| -> Result<(), Error> {
                if !store::delete_destination(tx, Servers::DEFAULT, id)? {
                    return Err(ErrorCode::NotFound.into());
                }
                record.write(tx, now)?;
                Ok(())
            })
            .await
    }

    /// Tries a destination as drafted, with the secret of the stored one it names when the draft
    /// brings none and logs in the same way.
    pub async fn test_destination(
        &self,
        request: DestinationTestRequest,
    ) -> Result<DestinationTestResult, Error> {
        let draft = request.destination;
        let stored = match request.id {
            Some(id) => {
                let before = self.destination(id).await?;
                let secret = self
                    .inner
                    .db
                    .read(move |conn| store::destination_secret(conn, id))
                    .await?;
                same_login(&before.target, &draft.target)
                    .then_some(secret)
                    .flatten()
            }
            None => None,
        };
        let has_secret = stored.is_some() || draft.secret.is_some();
        let issues = draft.issues(has_secret);
        if !issues.is_empty() {
            return Err(ApiError::new(ErrorCode::InvalidBackupDestination)
                .with("issues", serde_json::to_value(issues).unwrap_or_default())
                .into());
        }
        if let DestinationTarget::Local { folder } = &draft.target
            && let Some(server) = self.server_folder().await?
        {
            let folder = PathBuf::from(folder);
            if blocking(move || local::inside(&folder, &server)).await? {
                return Ok(DestinationTestResult {
                    error: Some(BackupError::InsideServerFolder),
                    error_detail: None,
                    host_key: None,
                });
            }
        }
        let secret = draft.secret.clone().or(stored);
        if matches!(
            draft.target,
            DestinationTarget::Sftp {
                auth: SftpAuth::Key,
                ..
            }
        ) && secret.is_none()
        {
            return Ok(DestinationTestResult {
                error: Some(BackupError::Authentication),
                error_detail: Some("save the destination first: its key is made then".to_owned()),
                host_key: None,
            });
        }
        Ok(destination::test(&draft.target, secret.as_ref()).await)
    }

    /// Connects to the database as the stored settings, or a draft, say, and reads what it holds.
    pub async fn test_source(&self, request: SourceTestRequest) -> Result<SourceTestResult, Error> {
        let settings = match request.settings {
            Some(settings) => settings,
            None => self.stored_settings().await?.0,
        };
        let password = match request.password {
            Some(password) => Some(password),
            None => {
                self.inner
                    .db
                    .read(|conn| store::password(conn, Servers::DEFAULT))
                    .await?
            }
        };
        let connection = self.connection(&settings, password, None).await?;
        let source = source_view(&settings, connection.as_ref());
        let found = self.client(&settings).await;
        let client = client_view(found.clone());
        let mut result = SourceTestResult {
            source,
            client,
            server_version: None,
            tables: None,
            bytes: None,
            loose_tables: None,
            error: None,
            error_detail: None,
        };
        let tested = match (connection, found) {
            (Ok(connection), Ok(client)) => {
                let name = format!("test-{}", self.inner.clock.now().as_millisecond());
                dump::test(&client, &connection, &self.inner.work, &name).await
            }
            (Err(failure), _) | (_, Err(failure)) => Err(failure),
        };
        match tested {
            Ok(facts) => {
                result.server_version = Some(facts.server_version);
                result.tables = Some(facts.tables);
                result.bytes = Some(facts.bytes);
                result.loose_tables = Some(facts.loose_tables);
            }
            Err(failure) => {
                result.error = Some(failure.error);
                result.error_detail = failure.detail;
            }
        }
        Ok(result)
    }

    /// The connection the settings give: the cfg's string, from the scan of the spawn when there
    /// is one or read again, or the one typed in with its password.
    async fn connection(
        &self,
        settings: &BackupSettings,
        password: Option<Secret>,
        scanned: Option<ConnectionString>,
    ) -> Result<Result<DatabaseConnection, Failure>, Error> {
        if settings.source == SourceKind::Manual {
            let Some(manual) = &settings.manual else {
                return Ok(Err(Failure::new(BackupError::SourceUnreadable)));
            };
            let Some(password) = password else {
                return Ok(Err(Failure::with(
                    BackupError::SourceUnreadable,
                    "no password is stored",
                )));
            };
            return Ok(Ok(DatabaseConnection {
                host: manual.host.clone(),
                port: manual.port,
                socket: None,
                user: manual.user.clone(),
                password,
                database: manual.database.clone(),
            }));
        }
        let text = if let Some(text) = scanned {
            Some(text)
        } else {
            let Some(launch) = self
                .inner
                .db
                .read(|conn| launch::get(conn, Servers::DEFAULT))
                .await?
            else {
                return Ok(Err(Failure::new(BackupError::LaunchNotConfigured)));
            };
            let (folder, cfg) = (PathBuf::from(launch.settings.data_path), launch.settings.cfg_file);
            let scan = blocking(move || observer_tools::resources::scan_cfg(&folder, &cfg, None)).await?;
            match scan {
                Ok(scan) => scan.database,
                Err(error) => {
                    return Ok(Err(Failure::with(BackupError::SourceMissing, error.to_string())));
                }
            }
        };
        let Some(text) = text else {
            return Ok(Err(Failure::new(BackupError::SourceMissing)));
        };
        Ok(parse_connection_string(text.expose())
            .map_err(|problem| Failure::with(BackupError::SourceUnreadable, problem.to_string())))
    }

    async fn source_view(&self, settings: &BackupSettings) -> Result<SourceView, Error> {
        // The password is not needed to say where the source is: a placeholder stands in.
        let placeholder = Some(Secret::new(""));
        let connection = self.connection(settings, placeholder, None).await?;
        Ok(source_view(settings, connection.as_ref()))
    }

    /// The dump tool the settings lead to, looked for again once the one found is old.
    async fn client(&self, settings: &BackupSettings) -> Result<Client, Failure> {
        let key = (settings.engine, settings.client_folder.clone());
        if let Ok(cached) = self.inner.client.lock()
            && let Some(found) = cached.as_ref()
            && found.settings == key
            && found.at.elapsed() < CLIENT_KEPT
        {
            return found.found.clone();
        }
        let found = dump::find(settings.engine, settings.client_folder.as_deref().map(Path::new)).await;
        if let Ok(mut cached) = self.inner.client.lock() {
            *cached = Some(FoundClient {
                settings: key,
                at: std::time::Instant::now(),
                found: found.clone(),
            });
        }
        found
    }

    /// Starts a backup now.
    pub async fn run_now(&self, caller: &Caller) -> Result<BackupRun, Error> {
        let running = self.begin(Asker::User(caller.clone()), None).await?;
        self.run(running.run).await
    }

    /// Cancels the run under way: the dump is killed, the uploads stopped, nothing kept.
    pub async fn cancel(&self, caller: &Caller) -> Result<(), Error> {
        let running = self.inner.running.lock().await.clone();
        let Some(running) = running else {
            return Err(ErrorCode::NotFound.into());
        };
        running.cancel.cancel();
        let now = self.inner.clock.now();
        let record = Record::by(caller, AuditAction::BackupCancelled).target(running.run.to_string());
        self.inner
            .db
            .write(move |tx| record.write(tx, now).map_err(Error::from))
            .await
    }

    pub async fn runs(&self, before: Option<i64>, limit: Option<u32>) -> Result<BackupRunsPage, Error> {
        let limit = limit.unwrap_or(DEFAULT_PAGE).clamp(1, MAX_PAGE);
        let runs = self
            .inner
            .db
            .read(move |conn| store::runs(conn, Servers::DEFAULT, before, limit))
            .await?;
        let next_before = (runs.len() == limit as usize)
            .then(|| runs.last().map(|run| run.id))
            .flatten();
        Ok(BackupRunsPage { runs, next_before })
    }

    pub async fn run(&self, id: i64) -> Result<BackupRun, Error> {
        self.inner
            .db
            .read(move |conn| store::run(conn, Servers::DEFAULT, id))
            .await?
            .ok_or_else(|| ErrorCode::NotFound.into())
    }

    /// Deletes every copy of a run, wherever it is; a copy that cannot be deleted stays listed.
    pub async fn delete_run(&self, caller: &Caller, id: i64) -> Result<BackupRun, Error> {
        if self
            .inner
            .running
            .lock()
            .await
            .as_ref()
            .is_some_and(|running| running.run == id)
        {
            return Err(ErrorCode::BackupRunning.into());
        }
        self.run(id).await?;
        let copies = self
            .inner
            .db
            .read(move |conn| store::run_copies(conn, id))
            .await?;
        let mut deleted = Vec::new();
        let mut failed = 0u32;
        for copy in copies {
            let (destination, secret) = self.destination_with_secret(copy.destination_id).await?;
            match destination::delete(&destination.target, secret.as_ref(), &copy.location).await {
                Ok(()) => deleted.push(copy.id),
                Err(failure) => {
                    warn!(copy = copy.id, error = %failure, "a backup's copy could not be deleted");
                    failed += 1;
                }
            }
        }
        let now = self.inner.clock.now();
        let record = Record::by(caller, AuditAction::BackupDeleted)
            .target(id.to_string())
            .outcome(if failed == 0 {
                Outcome::Success
            } else {
                Outcome::Failure
            })
            .details(json!({ "deleted": deleted.len(), "failed": failed }));
        self.inner
            .db
            .write(move |tx| -> Result<(), Error> {
                for copy in deleted {
                    store::copy_deleted(tx, copy, DeleteReason::Manual, now)?;
                }
                record.write(tx, now)?;
                Ok(())
            })
            .await?;
        self.run(id).await
    }

    async fn destination_with_secret(&self, id: i64) -> Result<(BackupDestination, Option<Secret>), Error> {
        let destination = self.destination(id).await?;
        let secret = self
            .inner
            .db
            .read(move |conn| store::destination_secret(conn, id))
            .await?;
        Ok((destination, secret))
    }

    /// Opens a stored copy for whoever downloads it, audited before the first byte.
    pub async fn download(&self, caller: &Caller, copy: i64) -> Result<Download, Error> {
        let row = self
            .inner
            .db
            .read(move |conn| store::copy(conn, Servers::DEFAULT, copy))
            .await?
            .ok_or(ErrorCode::NotFound)?;
        if row.state != CopyState::Stored {
            return Err(ErrorCode::BackupCopyUnavailable.into());
        }
        let (destination, secret) = self.destination_with_secret(row.destination_id).await?;
        let (reader, bytes) = destination::open(&destination.target, secret.as_ref(), &row.location)
            .await
            .map_err(|failure| {
                Error::from(
                    ApiError::new(ErrorCode::BackupCopyUnavailable).with("error", failure.error.as_str()),
                )
            })?;
        let now = self.inner.clock.now();
        let record = Record::by(caller, AuditAction::BackupDownloaded)
            .target(row.run.to_string())
            .details(json!({ "copy": row.id, "destination": destination.name }));
        self.inner
            .db
            .write(move |tx| record.write(tx, now).map_err(Error::from))
            .await?;
        Ok(Download {
            file_name: row
                .file_name
                .unwrap_or_else(|| format!("backup-{}.sql.gz", row.run)),
            bytes: bytes.or(row.bytes),
            reader,
        })
    }

    /// Starts a run for `asker`, unless one is under way or no destination is enabled. `scanned`
    /// is the cfg's connection string as the spawn about to happen read it.
    async fn begin(&self, asker: Asker, scanned: Option<ConnectionString>) -> Result<Running, Error> {
        let mut slot = self.inner.running.lock().await;
        if slot.is_some() {
            return Err(ErrorCode::BackupRunning.into());
        }
        if self.inner.stop.is_cancelled() {
            return Err(ErrorCode::ShuttingDown.into());
        }
        let (settings, _) = self.stored_settings().await?;
        let (password, destinations) = self
            .inner
            .db
            .read(|conn| -> Result<_, Error> {
                let destinations = store::destinations(conn, Servers::DEFAULT)?
                    .into_iter()
                    .filter(|destination| destination.enabled)
                    .map(|destination| {
                        let secret = store::destination_secret(conn, destination.id)?;
                        Ok((destination, secret))
                    })
                    .collect::<Result<Vec<_>, Error>>()?;
                Ok((store::password(conn, Servers::DEFAULT)?, destinations))
            })
            .await?;
        if destinations.is_empty() {
            return Err(ErrorCode::BackupNotConfigured.into());
        }
        let mut destinations = destinations;
        job::order(&mut destinations);
        // Before the run is recorded, so that nothing but the run itself can leave it unfinished.
        let connection = self.connection(&settings, password, scanned).await?;
        let client = self.client(&settings).await;
        let server_folder = self.server_folder().await?;
        let started_at = self.inner.clock.now();
        let (trigger, engine, actor) = (asker.trigger(), settings.engine, asker.actor());
        let run = self
            .inner
            .db
            .write(move |tx| {
                store::insert_run(
                    tx,
                    &NewRun {
                        server_id: Servers::DEFAULT,
                        trigger,
                        actor: actor.as_deref(),
                        engine,
                        started_at,
                    },
                )
            })
            .await?;
        let (dumped_tx, dumped) = watch::channel(None);
        let running = Running {
            run,
            cancel: self.inner.stop.child_token(),
            progress: Arc::default(),
            dumped,
            server_started: Arc::default(),
        };
        *slot = Some(running.clone());
        drop(slot);
        let job = Job {
            db: self.inner.db.clone(),
            clock: self.inner.clock.clone(),
            work: self.inner.work.clone(),
            run,
            started_at,
            asker,
            server_folder,
            connection,
            client,
            destinations,
            progress: Arc::clone(&running.progress),
            cancel: running.cancel.clone(),
            dumped: dumped_tx,
            server_started: Arc::clone(&running.server_started),
        };
        let backups = self.clone();
        self.inner.tasks.spawn(async move {
            if let Err(error) = job.run().await {
                warn!(run, %error, "a backup could not be recorded to its end");
            }
            *backups.inner.running.lock().await = None;
        });
        Ok(running)
    }

    /// The backup before a restart, when the settings ask for one: started, or the one under way
    /// joined, then waited for until its dump ends, the wait runs out or the start is stopped.
    /// Returns the lines for the console.
    pub async fn before_restart(
        &self,
        cause: &Cause,
        scanned: Option<ConnectionString>,
        stop: Arc<AtomicBool>,
    ) -> Vec<String> {
        let settings = match self.stored_settings().await {
            Ok((settings, _)) => settings,
            Err(error) => {
                warn!(%error, "the backup settings could not be read before a start");
                return Vec::new();
            }
        };
        let by = match (cause, settings.before_restarts) {
            (Cause::Scheduled { by }, BeforeRestarts::Scheduled | BeforeRestarts::AllRestarts)
            | (Cause::Restart { by }, BeforeRestarts::AllRestarts) => by.clone(),
            _ => return Vec::new(),
        };
        let running = match self.begin(Asker::Restart { by }, scanned).await {
            Ok(running) => running,
            Err(Error::Refused(refusal)) if refusal.code == ErrorCode::BackupRunning => {
                let Some(running) = self.inner.running.lock().await.clone() else {
                    return Vec::new();
                };
                if running.dumped.borrow().is_some() {
                    return vec![format!(
                        "A backup dumped before this restart (run {}) stands for it: no second one is made.",
                        running.run
                    )];
                }
                running
            }
            Err(Error::Refused(refusal)) if refusal.code == ErrorCode::BackupNotConfigured => {
                return Vec::new();
            }
            Err(error) => {
                warn!(%error, "the backup before a start could not begin");
                return vec![format!("The backup before this start could not begin: {error}")];
            }
        };
        let wait = Duration::from_secs(u64::from(settings.restart_wait_seconds));
        let deadline = tokio::time::Instant::now() + wait;
        let mut dumped = running.dumped.clone();
        loop {
            if let Some(dumped) = dumped.borrow_and_update().clone() {
                return vec![match dumped {
                    Dumped::Done { database, file_bytes } => format!(
                        "Backed up {database} before the start ({file_bytes} bytes); its copies go out while FXServer runs."
                    ),
                    Dumped::Failed(error) => format!(
                        "The backup before this start failed ({}): see Backups.",
                        error.as_str()
                    ),
                }];
            }
            if stop.load(Ordering::Relaxed) {
                return Vec::new();
            }
            if tokio::time::Instant::now() >= deadline {
                running.server_started.store(true, Ordering::Relaxed);
                info!(
                    run = running.run,
                    "FXServer starts before its backup's dump ended"
                );
                return vec![format!(
                    "The backup's dump goes on while FXServer starts, past the {}s it waits: tables with \
                     transactions stay as they were before the start.",
                    settings.restart_wait_seconds
                )];
            }
            tokio::select! {
                _ = dumped.changed() => {}
                () = tokio::time::sleep(WAIT_STEP) => {}
            }
        }
    }

    /// Starts the runs the settings' times call for, one fixed time after the other.
    async fn timer(self, timings: BackupsTimings) {
        let mut settled = self.inner.clock.now();
        loop {
            let next = match self.stored_settings().await {
                Ok((settings, _)) => next_time(&settings, &self.zone().1, settled),
                Err(error) => {
                    warn!(%error, "the backup settings could not be read");
                    None
                }
            };
            let now = self.inner.clock.now();
            let sleep = next.map_or(timings.longest_sleep, |at| {
                Duration::try_from(now.duration_until(at))
                    .unwrap_or(Duration::ZERO)
                    .min(timings.longest_sleep)
            });
            tokio::select! {
                () = self.inner.stop.cancelled() => return,
                () = self.inner.changed.notified() => continue,
                () = tokio::time::sleep(sleep) => {}
            }
            let now = self.inner.clock.now();
            let Some(at) = next.filter(|at| *at <= now) else {
                continue;
            };
            settled = now;
            if now.duration_since(at) > LATE {
                warn!(%at, "a backup's time passed while Observer was held up; it is skipped");
                continue;
            }
            match self.begin(Asker::Schedule, None).await {
                Ok(running) => debug!(run = running.run, "timed backup started"),
                Err(Error::Refused(refusal)) if refusal.code == ErrorCode::BackupRunning => {
                    info!(%at, "a timed backup is skipped: another one is under way");
                }
                Err(Error::Refused(refusal)) if refusal.code == ErrorCode::BackupNotConfigured => {
                    debug!(%at, "a timed backup is skipped: no destination is enabled");
                }
                Err(error) => warn!(%error, "a timed backup could not start"),
            }
        }
    }
}

/// The next of the settings' times after `after`.
fn next_time(settings: &BackupSettings, zone: &TimeZone, after: Timestamp) -> Option<Timestamp> {
    fixed_occurrences(&settings.times, zone, after, 1)
        .first()
        .map(|(at, _)| *at)
}

/// Whether two targets log in the same way, so that a stored secret still fits.
fn same_login(before: &DestinationTarget, after: &DestinationTarget) -> bool {
    match (before, after) {
        (DestinationTarget::Sftp { auth: a, .. }, DestinationTarget::Sftp { auth: b, .. }) => a == b,
        (before, after) => before.kind() == after.kind(),
    }
}

fn source_view(settings: &BackupSettings, connection: Result<&DatabaseConnection, &Failure>) -> SourceView {
    match connection {
        Ok(connection) => SourceView {
            kind: settings.source,
            host: connection.socket.is_none().then(|| connection.host.clone()),
            port: connection.socket.is_none().then_some(connection.port),
            socket: connection.socket.clone(),
            user: Some(connection.user.clone()),
            database: Some(connection.database.clone()),
            problem: None,
        },
        Err(failure) => SourceView {
            kind: settings.source,
            host: None,
            port: None,
            socket: None,
            user: None,
            database: None,
            problem: Some(failure.error),
        },
    }
}

fn client_view(found: Result<Client, Failure>) -> ClientView {
    match found {
        Ok(client) => ClientView {
            path: Some(client.dump.to_string_lossy().into_owned()),
            flavour: Some(client.flavour),
            version: Some(client.version),
            problem: None,
        },
        Err(failure) => ClientView {
            path: None,
            flavour: None,
            version: None,
            problem: Some(failure.error),
        },
    }
}

fn failure_error(failure: &Failure) -> Error {
    Error::Io(std::io::Error::other(failure.to_string()))
}

/// Each setting that differs, with its value before and after.
fn changes(before: &BackupSettings, after: &BackupSettings) -> Value {
    let fields = |settings: &BackupSettings| match serde_json::to_value(settings) {
        Ok(Value::Object(fields)) => fields,
        _ => Map::new(),
    };
    let old = fields(before);
    Value::Object(
        fields(after)
            .into_iter()
            .filter_map(|(name, value)| {
                let previous = old.get(&name).cloned().unwrap_or(Value::Null);
                (previous != value).then(|| (name, json!({ "before": previous, "after": value })))
            })
            .collect(),
    )
}

fn destination_details(
    name: &str,
    target: &DestinationTarget,
    enabled: bool,
    retention: observer_domain::backups::Retention,
) -> Value {
    json!({
        "name": name,
        "kind": target.kind().as_str(),
        "target": target,
        "enabled": enabled,
        "retention": retention,
    })
}

/// Each field of a destination that differs, before and after, its secret aside.
fn destination_changes(before: &BackupDestination, after: &DestinationDraft) -> Value {
    let old = destination_details(&before.name, &before.target, before.enabled, before.retention);
    let new = destination_details(&after.name, &after.target, after.enabled, after.retention);
    let (Value::Object(old), Value::Object(new)) = (old, new) else {
        return json!({});
    };
    let names: HashSet<&String> = old.keys().chain(new.keys()).collect();
    let mut changed = Map::new();
    for name in names {
        let (was, is) = (old.get(name), new.get(name));
        if was != is {
            changed.insert(name.clone(), json!({ "before": was, "after": is }));
        }
    }
    changed.insert("name".to_owned(), json!(after.name));
    Value::Object(changed)
}

async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Result<T, Error> {
    Ok(tokio::task::spawn_blocking(work)
        .await
        .map_err(std::io::Error::other)?)
}

/// Settles the backup before each restart on the [`Backups`] it is bound to once they exist.
#[derive(Clone, Default)]
pub struct BackupsHook(Arc<OnceLock<Backups>>);

impl BackupsHook {
    /// Binds the backups; a second call changes nothing.
    pub fn bind(&self, backups: Backups) {
        let _ = self.0.set(backups);
    }
}

impl fmt::Debug for BackupsHook {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BackupsHook")
            .field("bound", &self.0.get().is_some())
            .finish()
    }
}

/// The work before each spawn: the server tools within their own minute, and beside them the
/// backup a restart may call for, within its wait. Both read what the spawn uses.
#[derive(Debug, Clone, Default)]
pub struct BeforeSpawn {
    pub tools: ToolsHook,
    pub backups: BackupsHook,
}

impl PreLaunch for BeforeSpawn {
    fn run(&self, request: PreLaunchRequest) -> PreLaunchFuture<'_> {
        Box::pin(async move {
            let stop = Arc::clone(&request.cancel);
            let cause = request.cause.clone();
            let scanned = request.scan.database.clone();
            let tools_stop = Arc::new(AtomicBool::new(false));
            let mut tools_request = request;
            tools_request.cancel = Arc::clone(&tools_stop);
            let tools = async {
                let work = self.tools.run(tools_request);
                tokio::pin!(work);
                let limit = tokio::time::sleep(TOOLS_LIMIT);
                tokio::pin!(limit);
                loop {
                    tokio::select! {
                        result = &mut work => return result,
                        () = &mut limit, if !tools_stop.load(Ordering::Relaxed) => tools_stop.store(true, Ordering::Relaxed),
                        () = tokio::time::sleep(WAIT_STEP), if !tools_stop.load(Ordering::Relaxed) => {
                            if stop.load(Ordering::Relaxed) {
                                tools_stop.store(true, Ordering::Relaxed);
                            }
                        }
                    }
                }
            };
            let backup = async {
                match self.backups.0.get() {
                    Some(backups) => backups.before_restart(&cause, scanned, Arc::clone(&stop)).await,
                    None => Vec::new(),
                }
            };
            let (tools, backup) = tokio::join!(tools, backup);
            let mut lines = tools?;
            lines.extend(backup);
            Ok(lines)
        })
    }
}
