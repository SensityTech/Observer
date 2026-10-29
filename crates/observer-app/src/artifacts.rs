//! FXServer's builds: the catalog of those Cfx.re publishes, the install and import jobs, which
//! build runs and which runs next, the probation of a new build and the return to the last healthy
//! one, and the cleanup of old builds. One task owns all of it, so that a selection and a deletion
//! never race; the handle asks it, and the `server.artifacts` topic follows its status.

mod catalog;
mod probation;
mod retention;
mod selection;
mod view;

use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use jiff::Timestamp;
use observer_artifacts::catalog::{Availability as Offered, Catalog, Support};
use observer_artifacts::{
    Error as InstallError, Health, Http, InstallRequest, Installed, Installer, Platform, Progress, Scanned,
};
use observer_domain::artifacts::{
    ApplyMode, ArtifactNotice, ArtifactSettings, ArtifactSettingsChange, ArtifactSettingsField,
    ArtifactSettingsIssue, ArtifactSettingsProblem, ArtifactsStatus, BUILDS, BuildPlatform, CatalogView,
    CleanupBuild, CleanupPreview, CleanupResult, DiskSpace, DownloadProgress, EntryProgress, HealthyBuild,
    Job, JobKind, JobState, JobStep, JobWarning, KeptBuild, MAX_BRANCHES, MAX_KEEP_BUILDS, NoticeKind,
    NoticeReason, PendingBuild, Protection, SelectBuild, SkipReason, SkippedBuild,
};
use observer_domain::audit::{ActorKind, AuditAction, Outcome};
use observer_domain::error::{ApiError, ErrorCode};
use observer_domain::launch::{Platform as PathRules, folder_problem};
use observer_domain::permissions::Permissions;
use observer_domain::schedule::ScheduleStatus;
use observer_domain::server::ServerState;
use observer_fxserver::supervisor::{Cause, Event, EventKind, State};
use observer_store::artifacts::{self as stored, ArtifactState};
use observer_store::{Database, Transaction};
use serde_json::{Map, Value, json};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use self::catalog::CatalogCache;
use self::probation::{End, Input, Probation, Verdict};
use self::retention::Candidate;
use self::selection::{Change, Context, Server};
use crate::audit::Record;
use crate::{Accounts, Caller, Clock, Error, Servers, Supervision};

const COMMANDS: usize = 16;
/// What the download rate is measured over.
const RATE_WINDOW: Duration = Duration::from_secs(5);
/// The longest the task sleeps while a probation counts down, so that a clock that jumped is
/// caught up with soon.
const PROBATION_TICK: Duration = Duration::from_secs(1);

/// What [`Artifacts::start`] works with.
pub struct ArtifactsSetup {
    pub db: Database,
    pub clock: Clock,
    /// Where builds are installed, one folder per build number.
    pub root: PathBuf,
    pub accounts: Accounts,
    pub servers: Servers,
    pub supervision: Supervision,
    pub schedule: watch::Receiver<ScheduleStatus>,
    /// Where builds are listed and downloaded from: Cfx.re, or a test's local server.
    pub catalog: Catalog,
    pub http: Http,
}

#[derive(Debug, Clone, Copy)]
pub struct ArtifactTimings {
    /// How long a new build must stay up once ready to pass its probation.
    pub stable_for: Duration,
    /// How long a listing of the catalog is answered from memory.
    pub catalog_fresh_for: Duration,
    /// How old a listing must be before a refresh fetches again.
    pub refresh_after: Duration,
    /// How often a job's progress is published.
    pub progress_every: Duration,
}

impl Default for ArtifactTimings {
    fn default() -> Self {
        Self {
            stable_for: Duration::from_mins(10),
            catalog_fresh_for: Duration::from_mins(10),
            refresh_after: Duration::from_secs(30),
            progress_every: Duration::from_millis(250),
        }
    }
}

/// The builds of the default server. Clones talk to the same task.
#[derive(Clone)]
pub struct Artifacts {
    commands: mpsc::Sender<Command>,
    status: watch::Receiver<ArtifactsStatus>,
    catalog: Arc<CatalogCache>,
    accounts: Accounts,
    clock: Clock,
}

/// The artifacts task; [`ArtifactsTask::close`] cancels a running job, waits for it, then ends.
pub struct ArtifactsTask {
    task: JoinHandle<()>,
    stop: CancellationToken,
}

impl ArtifactsTask {
    pub async fn close(self) {
        self.stop.cancel();
        if let Err(error) = self.task.await {
            warn!(%error, "the artifacts task failed");
        }
    }
}

type Reply<T> = oneshot::Sender<Result<T, Error>>;

enum Command {
    Install {
        caller: Caller,
        request: Box<InstallRequest>,
        allow_end_of_life: bool,
        reply: Reply<ArtifactsStatus>,
    },
    Import {
        caller: Caller,
        folder: PathBuf,
        build: u32,
        reply: Reply<ArtifactsStatus>,
    },
    CancelJob {
        caller: Caller,
        reply: Reply<ArtifactsStatus>,
    },
    Select {
        caller: Caller,
        request: SelectBuild,
        reply: Reply<ArtifactsStatus>,
    },
    CancelPending {
        caller: Caller,
        reply: Reply<ArtifactsStatus>,
    },
    Delete {
        caller: Caller,
        build: u32,
        reply: Reply<ArtifactsStatus>,
    },
    Preview {
        reply: Reply<CleanupPreview>,
    },
    Cleanup {
        caller: Caller,
        builds: Vec<u32>,
        reply: Reply<CleanupResult>,
    },
    Settings {
        caller: Caller,
        change: ArtifactSettingsChange,
        reply: Reply<ArtifactsStatus>,
    },
    /// Looks at the folder again: a build the command line installed shows up.
    Rescan,
}

impl Artifacts {
    /// Reads the stored state, scans the artifacts folder, adopts the build folder of the launch
    /// settings when it already lies there, then follows FXServer. `tune` adjusts the timings,
    /// for tests. A data folder FXServer cannot take builds from is refused here.
    pub async fn start(
        setup: ArtifactsSetup,
        tune: impl FnOnce(&mut ArtifactTimings),
    ) -> Result<(Self, ArtifactsTask), Error> {
        let mut timings = ArtifactTimings::default();
        tune(&mut timings);
        let root = std::path::absolute(&setup.root)?;
        let text = root.to_str().unwrap_or_default();
        if let Some(problem) = folder_problem(text, PathRules::host()) {
            let problem = serde_json::to_value(problem).unwrap_or_default();
            return Err(Error::Io(std::io::Error::other(format!(
                "FXServer cannot run builds from {}: {problem}; choose another data folder",
                root.display()
            ))));
        }
        // Made now, so that the space of its volume is known before the first install.
        tokio::fs::create_dir_all(&root).await?;
        let installer = Arc::new(Installer::new(setup.http, root.clone()));
        installer.sweep().await;
        let (state, settings) = setup
            .db
            .read(|conn| {
                Ok::<_, Error>((
                    stored::state(conn, Servers::DEFAULT)?,
                    stored::settings(conn, Servers::DEFAULT)?,
                ))
            })
            .await?;
        let (settings, version) = match settings {
            Some((settings, version)) => (settings, Some(version)),
            None => (ArtifactSettings::default(), None),
        };
        let catalog = Arc::new(CatalogCache::new(
            setup.catalog,
            setup.db.clone(),
            setup.clock.clone(),
            timings,
        ));
        let probation = Probation {
            open: state.probation,
            run: None,
            last_healthy: state.last_healthy.map(|healthy| healthy.build),
        };
        let mut driver = Driver {
            db: setup.db,
            clock: setup.clock.clone(),
            root,
            installer,
            servers: setup.servers,
            supervision: setup.supervision,
            schedule: setup.schedule,
            timings,
            catalog: Arc::clone(&catalog),
            state,
            settings,
            version,
            installed: Vec::new(),
            running: None,
            probation,
            job: None,
            next_job: 1,
            rollback_after: None,
            disk: None,
            status: watch::channel(placeholder()).0,
        };
        driver.rescan().await;
        driver.adopt().await?;
        let (status_tx, status) = watch::channel(driver.status());
        driver.status = status_tx;

        let (commands, commands_rx) = mpsc::channel(COMMANDS);
        let stop = CancellationToken::new();
        let task = tokio::spawn(driver.run(commands_rx, stop.clone()));
        let artifacts = Self {
            commands,
            status,
            catalog,
            accounts: setup.accounts,
            clock: setup.clock,
        };
        Ok((artifacts, ArtifactsTask { task, stop }))
    }

    /// The status as it changes.
    pub fn status(&self) -> watch::Receiver<ArtifactsStatus> {
        self.status.clone()
    }

    /// The catalog of the host's platform, with what is installed.
    pub async fn catalog(&self, refresh: bool) -> Result<CatalogView, Error> {
        let _ = self.commands.try_send(Command::Rescan);
        let branches = self.status.borrow().settings.branches.clone();
        let fetched = self.catalog.listing(&branches, refresh).await?;
        let installed: Vec<u32> = self
            .status
            .borrow()
            .installed
            .iter()
            .map(|build| build.build)
            .collect();
        Ok(view::catalog(
            &fetched.listing,
            fetched.taken_at,
            &installed,
            self.clock.now(),
        ))
    }

    /// Starts the download of a build the catalog lists. Everything that would refuse it is
    /// checked before the job starts.
    pub async fn install(
        &self,
        caller: &Caller,
        build: u32,
        allow_end_of_life: bool,
    ) -> Result<ArtifactsStatus, Error> {
        if job_running(&self.status.borrow()) {
            return Err(ErrorCode::JobRunning.into());
        }
        let branches = self.status.borrow().settings.branches.clone();
        let mut fetched = self.catalog.listing(&branches, false).await?;
        if fetched.listing.find(build).is_none() {
            fetched = self.catalog.listing(&branches, true).await?;
        }
        let entry = fetched.listing.find(build).ok_or(ErrorCode::BuildNotListed)?;
        let not_downloadable =
            |reason: &str| ApiError::new(ErrorCode::NotDownloadable).with("reason", reason);
        match entry.availability {
            Offered::Downloadable { .. } => {}
            Offered::NoCommit => return Err(not_downloadable("noCommit").into()),
            Offered::NoArchive => return Err(not_downloadable("noArchive").into()),
        }
        if self
            .status
            .borrow()
            .installed
            .iter()
            .any(|installed| installed.build == build)
        {
            return Err(ErrorCode::AlreadyInstalled.into());
        }
        if entry.support(self.clock.now()) == Support::EndOfLife && !allow_end_of_life {
            return Err(ErrorCode::EndOfLife.into());
        }
        let request = fetched
            .listing
            .install_request(build)
            .ok_or_else(|| not_downloadable("noCommit"))?;
        self.ask(|reply| Command::Install {
            caller: caller.clone(),
            request: Box::new(request),
            allow_end_of_life,
            reply,
        })
        .await
    }

    /// Starts the copy of a build folder of the owner's. The owner names the build number.
    pub async fn import(&self, caller: &Caller, folder: &str, build: u32) -> Result<ArtifactsStatus, Error> {
        let usable = BUILDS.contains(&build)
            && Path::new(folder).is_absolute()
            && folder_problem(folder, PathRules::host()).is_none();
        if !usable {
            return Err(ErrorCode::InvalidRequest.into());
        }
        self.ask(|reply| Command::Import {
            caller: caller.clone(),
            folder: PathBuf::from(folder),
            build,
            reply,
        })
        .await
    }

    /// Cancels the running job, until its build is being installed.
    pub async fn cancel_job(&self, caller: &Caller) -> Result<ArtifactsStatus, Error> {
        self.ask(|reply| Command::CancelJob {
            caller: caller.clone(),
            reply,
        })
        .await
    }

    /// Chooses the build FXServer runs, now, at its next start or at the next scheduled restart.
    /// `now` restarts FXServer, which also takes the right to control it, checked as `method
    /// path` before anything changes.
    pub async fn select(
        &self,
        caller: &Caller,
        request: SelectBuild,
        method: &str,
        path: &str,
    ) -> Result<ArtifactsStatus, Error> {
        if request.apply == ApplyMode::Now {
            self.accounts
                .authorize(caller, Permissions::SERVER_CONTROL, method, path)
                .await?;
        }
        self.ask(|reply| Command::Select {
            caller: caller.clone(),
            request,
            reply,
        })
        .await
    }

    pub async fn cancel_pending(&self, caller: &Caller) -> Result<ArtifactsStatus, Error> {
        self.ask(|reply| Command::CancelPending {
            caller: caller.clone(),
            reply,
        })
        .await
    }

    pub async fn delete(&self, caller: &Caller, build: u32) -> Result<ArtifactsStatus, Error> {
        self.ask(|reply| Command::Delete {
            caller: caller.clone(),
            build,
            reply,
        })
        .await
    }

    /// What a cleanup would remove now.
    pub async fn cleanup_preview(&self) -> Result<CleanupPreview, Error> {
        self.ask(|reply| Command::Preview { reply }).await
    }

    /// Removes the builds of the preview the user saw; a preview that changed since is a
    /// conflict.
    pub async fn cleanup(&self, caller: &Caller, builds: Vec<u32>) -> Result<CleanupResult, Error> {
        self.ask(|reply| Command::Cleanup {
            caller: caller.clone(),
            builds,
            reply,
        })
        .await
    }

    pub async fn change_settings(
        &self,
        caller: &Caller,
        change: ArtifactSettingsChange,
    ) -> Result<ArtifactsStatus, Error> {
        self.ask(|reply| Command::Settings {
            caller: caller.clone(),
            change,
            reply,
        })
        .await
    }

    async fn ask<T>(&self, command: impl FnOnce(Reply<T>) -> Command) -> Result<T, Error> {
        let (reply, answer) = oneshot::channel();
        self.commands
            .send(command(reply))
            .await
            .map_err(|_| ErrorCode::ShuttingDown)?;
        answer.await.map_err(|_| ErrorCode::ShuttingDown)?
    }
}

fn job_running(status: &ArtifactsStatus) -> bool {
    status
        .job
        .as_ref()
        .is_some_and(|job| job.state == JobState::Running)
}

/// What a job does, by kind.
enum Work {
    Install(Box<InstallRequest>),
    Import(PathBuf),
}

/// The progress a job reports, which the task reads at its own pace.
#[derive(Default)]
struct Live {
    step: Option<JobStep>,
    received: u64,
    total: u64,
    entries: Option<(Arc<AtomicU64>, Option<u64>)>,
}

/// The job running, or the last one.
struct Slot {
    job: Job,
    cancel: CancellationToken,
    live: Arc<Mutex<Live>>,
    task: Option<JoinHandle<()>>,
    /// The bytes received at each publication of the last few seconds.
    samples: VecDeque<(Instant, u64)>,
    starter: Caller,
}

struct Finished {
    id: u32,
    result: Result<Installed, InstallError>,
}

struct Driver {
    db: Database,
    clock: Clock,
    root: PathBuf,
    installer: Arc<Installer>,
    servers: Servers,
    supervision: Supervision,
    schedule: watch::Receiver<ScheduleStatus>,
    timings: ArtifactTimings,
    catalog: Arc<CatalogCache>,
    state: ArtifactState,
    settings: ArtifactSettings,
    version: Option<i64>,
    installed: Vec<Scanned>,
    /// The build of the current run, from its spawn until it ends.
    running: Option<u32>,
    probation: Probation,
    job: Option<Slot>,
    next_job: u32,
    /// The run that failed its probation and must end before the last healthy build starts.
    rollback_after: Option<u32>,
    disk: Option<DiskSpace>,
    status: watch::Sender<ArtifactsStatus>,
}

impl Driver {
    async fn run(mut self, mut commands: mpsc::Receiver<Command>, stop: CancellationToken) {
        let mut events = self.supervision.supervisor().events();
        let mut launch_changes = self.servers.launch_changes();
        launch_changes.mark_unchanged();
        let (finished_tx, mut finished) = mpsc::channel::<Finished>(4);
        loop {
            let job_running = self.job.as_ref().is_some_and(|slot| slot.task.is_some());
            let probation_due = probation::passes_at(&self.probation, self.timings.stable_for).map(|at| {
                Duration::try_from(self.clock.now().duration_until(at))
                    .unwrap_or(Duration::ZERO)
                    .min(PROBATION_TICK)
            });
            tokio::select! {
                () = stop.cancelled() => break,
                command = commands.recv() => match command {
                    Some(command) => self.command(command, &finished_tx).await,
                    None => break,
                },
                event = events.recv() => match event {
                    Ok(event) => self.event(event).await,
                    Err(RecvError::Lagged(missed)) => {
                        warn!(missed, "the artifacts task missed events of the supervisor");
                    }
                    Err(RecvError::Closed) => break,
                },
                Ok(()) = launch_changes.changed() => self.reload().await,
                Some(done) = finished.recv() => self.finished(done).await,
                () = tokio::time::sleep(self.timings.progress_every), if job_running => {
                    self.publish();
                }
                () = tokio::time::sleep(probation_due.unwrap_or_default()), if probation_due.is_some() => {
                    self.probation_step(Input::Tick, false).await;
                }
            }
        }
        if let Some(slot) = self.job.as_mut() {
            slot.cancel.cancel();
            if let Some(task) = slot.task.take()
                && let Err(error) = task.await
            {
                warn!(%error, "the install job failed");
            }
        }
    }

    async fn command(&mut self, command: Command, finished: &mpsc::Sender<Finished>) {
        match command {
            Command::Install {
                caller,
                request,
                allow_end_of_life,
                reply,
            } => {
                let started = self.install(&caller, request, allow_end_of_life, finished).await;
                let _ = reply.send(started);
            }
            Command::Import {
                caller,
                folder,
                build,
                reply,
            } => {
                let started = self.import(&caller, folder, build, finished).await;
                let _ = reply.send(started);
            }
            Command::CancelJob { caller, reply } => {
                let _ = reply.send(self.cancel_job(&caller).await);
            }
            Command::Select {
                caller,
                request,
                reply,
            } => {
                let _ = reply.send(self.select(&caller, request).await);
            }
            Command::CancelPending { caller, reply } => {
                let _ = reply.send(self.cancel_pending(&caller).await);
            }
            Command::Delete { caller, build, reply } => {
                let _ = reply.send(self.delete(&caller, build).await);
            }
            Command::Preview { reply } => {
                let _ = reply.send(Ok(self.preview()));
            }
            Command::Cleanup {
                caller,
                builds,
                reply,
            } => {
                let _ = reply.send(self.cleanup(&caller, builds).await);
            }
            Command::Settings {
                caller,
                change,
                reply,
            } => {
                let _ = reply.send(self.change_settings(&caller, change).await);
            }
            Command::Rescan => {
                self.rescan().await;
                self.publish();
            }
        }
    }

    async fn install(
        &mut self,
        caller: &Caller,
        mut request: Box<InstallRequest>,
        allow_end_of_life: bool,
        finished: &mpsc::Sender<Finished>,
    ) -> Result<ArtifactsStatus, Error> {
        let build = request.build;
        self.refuse_job(build)?;
        let platform = BuildPlatform::host();
        request.fingerprint = self
            .db
            .read(move |conn| stored::fingerprint(conn, platform, build))
            .await?;
        request.installed_by = Some(caller.username.clone());
        let mut details = json!({ "build": build, "kind": JobKind::Install });
        if allow_end_of_life {
            details["allowEndOfLife"] = json!(true);
        }
        self.audit(Record::by(caller, AuditAction::BuildInstallStarted).details(details))
            .await?;
        self.start_job(caller, JobKind::Install, build, Work::Install(request), finished);
        Ok(self.publish())
    }

    async fn import(
        &mut self,
        caller: &Caller,
        folder: PathBuf,
        build: u32,
        finished: &mpsc::Sender<Finished>,
    ) -> Result<ArtifactsStatus, Error> {
        self.refuse_job(build)?;
        // The folder is the owner's to see, as the launch settings' paths are: the log and the
        // manifest keep it, the audit log does not.
        let details = json!({ "build": build, "kind": JobKind::Import });
        self.audit(Record::by(caller, AuditAction::BuildInstallStarted).details(details))
            .await?;
        self.start_job(caller, JobKind::Import, build, Work::Import(folder), finished);
        Ok(self.publish())
    }

    fn refuse_job(&self, build: u32) -> Result<(), Error> {
        if self.job.as_ref().is_some_and(|slot| slot.task.is_some()) {
            return Err(ErrorCode::JobRunning.into());
        }
        if self.installed.iter().any(|scanned| scanned.build == build) {
            return Err(ErrorCode::AlreadyInstalled.into());
        }
        Ok(())
    }

    fn start_job(
        &mut self,
        caller: &Caller,
        kind: JobKind,
        build: u32,
        work: Work,
        finished: &mpsc::Sender<Finished>,
    ) {
        let id = self.next_job;
        self.next_job += 1;
        let cancel = CancellationToken::new();
        let live = Arc::new(Mutex::new(Live::default()));
        let mut warnings = Vec::new();
        if self.installer.long_path().is_some() {
            warnings.push(JobWarning::LongPath);
        }
        if self.disk.is_none() {
            warnings.push(JobWarning::FreeSpaceUnknown);
        }
        let task = {
            let (installer, cancel, live, finished) = (
                Arc::clone(&self.installer),
                cancel.clone(),
                Arc::clone(&live),
                finished.clone(),
            );
            let by = Some(caller.username.clone());
            tokio::spawn(async move {
                let report = |progress: Progress| record(&live, &progress);
                let result = match work {
                    Work::Install(request) => installer.install(&request, &cancel, report).await,
                    Work::Import(folder) => installer.import(&folder, build, by, &cancel, report).await,
                };
                let _ = finished.send(Finished { id, result }).await;
            })
        };
        info!(build, ?kind, by = %caller.username, "a build job started");
        self.job = Some(Slot {
            job: Job {
                id,
                kind,
                build,
                step: JobStep::PreCheck,
                state: JobState::Running,
                started_by: caller.username.clone(),
                started_at: self.clock.now(),
                finished_at: None,
                download: None,
                entries: None,
                cancel_requested: false,
                failure: None,
                warnings,
            },
            cancel,
            live,
            task: Some(task),
            samples: VecDeque::new(),
            starter: caller.clone(),
        });
    }

    async fn cancel_job(&mut self, caller: &Caller) -> Result<ArtifactsStatus, Error> {
        let Some(slot) = self.job.as_mut().filter(|slot| slot.task.is_some()) else {
            return Err(ErrorCode::NoJob.into());
        };
        // The step is read and the token cancelled under the lock the job reports through: the job
        // reports its commit, then reads the token, so a cancel either stops it or finds it
        // committing.
        let committing = {
            let live = slot.live.lock().unwrap_or_else(PoisonError::into_inner);
            let committing = live.step == Some(JobStep::Commit);
            if !committing {
                slot.cancel.cancel();
            }
            committing
        };
        refresh(slot);
        if committing {
            return Err(ErrorCode::JobCommitting.into());
        }
        if !slot.job.cancel_requested {
            slot.job.cancel_requested = true;
            let details = json!({ "build": slot.job.build, "kind": slot.job.kind, "step": slot.job.step });
            self.audit(Record::by(caller, AuditAction::BuildInstallCancelled).details(details))
                .await?;
        }
        Ok(self.publish())
    }

    async fn finished(&mut self, done: Finished) {
        let now = self.clock.now();
        let Some(slot) = self.job.as_mut().filter(|slot| slot.job.id == done.id) else {
            return;
        };
        if let Some(task) = slot.task.take() {
            let _ = task.await;
        }
        refresh(slot);
        slot.job.finished_at = Some(now);
        let (build, kind) = (slot.job.build, slot.job.kind);
        let starter = slot.starter.clone();
        let record = match &done.result {
            Ok(installed) => {
                slot.job.state = JobState::Installed;
                slot.job.step = JobStep::Commit;
                info!(build, ?kind, "a build was installed");
                let mut details = json!({ "build": build, "kind": kind, "bytes": installed.manifest.bytes });
                if let Some(sha256) = installed.manifest.origin.archive_sha256() {
                    details["archiveSha256"] = json!(sha256);
                }
                Record::by(&starter, AuditAction::BuildInstallFinished).details(details)
            }
            Err(error) => {
                warn!(build, ?kind, %error, "a build job ended without installing the build");
                let failure = view::failure(error);
                slot.job.state = if matches!(error, InstallError::Cancelled) {
                    JobState::Cancelled
                } else {
                    JobState::Failed
                };
                let reason = serde_json::to_value(&failure)
                    .ok()
                    .and_then(|value| value.get("code").cloned())
                    .unwrap_or_default();
                let details =
                    json!({ "build": build, "kind": kind, "step": slot.job.step, "reason": reason });
                slot.job.failure = Some(failure);
                Record::by(&starter, AuditAction::BuildInstallFinished)
                    .outcome(Outcome::Failure)
                    .details(details)
            }
        };
        if let Err(error) = self.audit(record).await {
            warn!(%error, "cannot audit the end of a build job");
        }
        if let Ok(installed) = &done.result {
            self.installed_now(build, installed).await;
        }
        self.publish();
    }

    /// Keeps the fingerprint of a new build and, when none is selected yet, selects it: a new
    /// Observer needs no other step.
    async fn installed_now(&mut self, build: u32, installed: &Installed) {
        let now = self.clock.now();
        let sha256 = installed.manifest.origin.archive_sha256().map(str::to_owned);
        let platform = BuildPlatform::host();
        let first = self.state.selected.is_none();
        let changed = self
            .change(move |tx, state| {
                if let Some(sha256) = &sha256 {
                    stored::remember_fingerprint(tx, platform, build, sha256, now)?;
                }
                if !first {
                    return Ok(false);
                }
                state.selected = Some(build);
                system(AuditAction::BuildSelected)
                    .details(json!({ "build": build, "first": true }))
                    .write(tx, now)?;
                Ok(true)
            })
            .await;
        match changed {
            Ok(true) => self.servers.launch_changed(),
            Ok(false) => {}
            Err(error) => warn!(%error, "cannot record the build just installed"),
        }
        self.rescan().await;
    }

    async fn select(&mut self, caller: &Caller, request: SelectBuild) -> Result<ArtifactsStatus, Error> {
        let build = request.build;
        let now = self.clock.now();
        let state = self.supervision.status().borrow().state;
        let server = match state {
            ServerState::Stopping => Server::Stopping,
            ServerState::Starting | ServerState::Online | ServerState::Degraded | ServerState::Hung => {
                Server::Running
            }
            ServerState::Stopped | ServerState::Crashed | ServerState::CrashLoop => Server::Down,
        };
        let context = Context {
            installed: self.health(build),
            selected: self.state.selected,
            pending: self.state.pending.as_ref().map(|pending| pending.build),
            running: self.running,
            highest_run: self.state.highest_run,
            end_of_life: self
                .catalog
                .latest()
                .is_some_and(|fetched| fetched.end_of_life(build, now)),
            server,
            restart_ahead: self.schedule.borrow().next.is_some(),
        };
        let change = selection::decide(request, &context)?;
        let caller_record = |action| Record::by(caller, action).target(Servers::DEFAULT);
        match change {
            Change::Nothing => return Ok(self.publish()),
            Change::Select { restart } => {
                let mut details =
                    json!({ "build": build, "from": self.state.selected, "apply": request.apply });
                if request.confirm_kvp_rollback && context.highest_run.is_some_and(|highest| build < highest)
                {
                    details["kvpConfirmed"] = json!(true);
                }
                if request.allow_end_of_life && context.end_of_life {
                    details["endOfLifeAllowed"] = json!(true);
                }
                let record = caller_record(AuditAction::BuildSelected).details(details);
                self.change(move |tx, state| {
                    state.selected = Some(build);
                    state.pending = None;
                    state.notice = None;
                    record.write(tx, now)?;
                    Ok(())
                })
                .await?;
                self.servers.launch_changed();
                if restart {
                    self.supervision.restart_server(caller).await?;
                }
            }
            Change::Pend => {
                let mut details = json!({ "build": build });
                if let Some(previous) = &self.state.pending {
                    details["previous"] = json!(previous.build);
                }
                let record = caller_record(AuditAction::PendingBuildSet).details(details);
                let by = caller.username.clone();
                self.change(move |tx, state| {
                    state.pending = Some(PendingBuild { build, by, at: now });
                    record.write(tx, now)?;
                    Ok(())
                })
                .await?;
            }
        }
        Ok(self.publish())
    }

    async fn cancel_pending(&mut self, caller: &Caller) -> Result<ArtifactsStatus, Error> {
        let Some(pending) = self.state.pending.clone() else {
            return Err(ErrorCode::NoPending.into());
        };
        let record = Record::by(caller, AuditAction::PendingBuildCancelled)
            .target(Servers::DEFAULT)
            .details(json!({ "build": pending.build }));
        let now = self.clock.now();
        self.change(move |tx, state| {
            state.pending = None;
            record.write(tx, now)?;
            Ok(())
        })
        .await?;
        Ok(self.publish())
    }

    async fn delete(&mut self, caller: &Caller, build: u32) -> Result<ArtifactsStatus, Error> {
        if !self.installed.iter().any(|scanned| scanned.build == build) {
            return Err(ErrorCode::BuildNotInstalled.into());
        }
        let reasons = self.protections().remove(&build).unwrap_or_default();
        if !reasons.is_empty() {
            return Err(ApiError::new(ErrorCode::BuildProtected)
                .with("reasons", serde_json::to_value(reasons).unwrap_or_default())
                .into());
        }
        match self.remove(build).await {
            Ok(bytes) => {
                let record = Record::by(caller, AuditAction::BuildDeleted)
                    .target(Servers::DEFAULT)
                    .details(json!({ "build": build, "bytes": bytes }));
                self.audit(record).await?;
                self.rescan().await;
                Ok(self.publish())
            }
            Err(Removal::InUse | Removal::Busy) => Err(ErrorCode::BuildInUse.into()),
            Err(Removal::Failed(error)) => Err(error),
        }
    }

    /// Deletes a build no process runs from. The process check comes first: on Linux a running
    /// build renames fine.
    async fn remove(&mut self, build: u32) -> Result<u64, Removal> {
        let folder = self.root.join(build.to_string());
        let users = tokio::task::spawn_blocking(move || observer_platform::processes_in(&folder))
            .await
            .map_err(|error| Removal::Failed(std::io::Error::other(error).into()))?
            .map_err(|error| Removal::Failed(error.into()))?;
        if !users.is_empty() {
            return Err(Removal::InUse);
        }
        match self.installer.delete(build).await {
            Ok(bytes) => Ok(bytes),
            Err(InstallError::Busy(_)) => Err(Removal::Busy),
            Err(error) => {
                warn!(build, %error, "cannot delete a build");
                Err(Removal::Failed(Error::Io(std::io::Error::other(
                    error.to_string(),
                ))))
            }
        }
    }

    fn preview(&self) -> CleanupPreview {
        let protected = self.protections();
        let candidates: Vec<Candidate> = self
            .installed
            .iter()
            .map(|scanned| Candidate {
                build: scanned.build,
                installed_at: scanned.manifest.as_ref().map(|manifest| manifest.installed_at),
                valid: scanned.health == Health::Valid,
            })
            .collect();
        let (remove, keep) = retention::plan(&candidates, &protected, usize::from(self.settings.keep_builds));
        let latest = self.catalog.latest();
        let remove: Vec<CleanupBuild> = remove
            .into_iter()
            .map(|build| CleanupBuild {
                build,
                bytes: self.bytes(build),
                reinstallable: latest
                    .as_ref()
                    .is_some_and(|fetched| fetched.reinstallable(build)),
            })
            .collect();
        CleanupPreview {
            freed: remove.iter().map(|build| build.bytes).sum(),
            remove,
            keep: keep
                .into_iter()
                .map(|(build, protected)| KeptBuild { build, protected })
                .collect(),
        }
    }

    async fn cleanup(&mut self, caller: &Caller, builds: Vec<u32>) -> Result<CleanupResult, Error> {
        let mut asked = builds;
        asked.sort_unstable();
        let mut planned: Vec<u32> = self.preview().remove.iter().map(|build| build.build).collect();
        planned.sort_unstable();
        if asked != planned {
            return Err(ErrorCode::Conflict.into());
        }
        let mut result = CleanupResult {
            deleted: Vec::new(),
            skipped: Vec::new(),
            freed: 0,
        };
        for build in planned.into_iter().rev() {
            if self
                .protections()
                .get(&build)
                .is_some_and(|reasons| !reasons.is_empty())
            {
                continue;
            }
            match self.remove(build).await {
                Ok(bytes) => {
                    let record = Record::by(caller, AuditAction::BuildDeleted)
                        .target(Servers::DEFAULT)
                        .details(json!({ "build": build, "bytes": bytes, "cleanup": true }));
                    self.audit(record).await?;
                    result.deleted.push(build);
                    result.freed += bytes;
                }
                Err(Removal::InUse) => result.skipped.push(SkippedBuild {
                    build,
                    reason: SkipReason::InUse,
                }),
                Err(Removal::Busy) => result.skipped.push(SkippedBuild {
                    build,
                    reason: SkipReason::Busy,
                }),
                Err(Removal::Failed(error)) => return Err(error),
            }
        }
        self.rescan().await;
        self.publish();
        Ok(result)
    }

    async fn change_settings(
        &mut self,
        caller: &Caller,
        change: ArtifactSettingsChange,
    ) -> Result<ArtifactsStatus, Error> {
        let issues = settings_issues(&change.settings);
        if !issues.is_empty() {
            return Err(ApiError::new(ErrorCode::InvalidArtifactSettings)
                .with("issues", serde_json::to_value(issues).unwrap_or_default())
                .into());
        }
        if change.version != self.version {
            return Err(ErrorCode::Conflict.into());
        }
        if change.settings == self.settings {
            return Ok(self.publish());
        }
        let version = self.version.map_or(1, |version| version + 1);
        let record = Record::by(caller, AuditAction::ArtifactSettingsChanged)
            .target(Servers::DEFAULT)
            .details(setting_changes(&self.settings, &change.settings));
        let (settings, now) = (change.settings.clone(), self.clock.now());
        self.db
            .write(move |tx| -> Result<(), Error> {
                stored::save_settings(tx, Servers::DEFAULT, &settings, version, now)?;
                record.write(tx, now)?;
                Ok(())
            })
            .await?;
        self.settings = change.settings;
        self.version = Some(version);
        Ok(self.publish())
    }

    async fn event(&mut self, event: Event) {
        let generation = event.generation;
        match event.kind {
            EventKind::Spawned { build, .. } => {
                self.running = self.build_of(&build);
                let Some(build) = self.running else {
                    return;
                };
                if self.state.highest_run.is_none_or(|highest| build > highest)
                    && let Err(error) = self
                        .change(move |_, state| {
                            state.highest_run = Some(build);
                            Ok(())
                        })
                        .await
                {
                    warn!(%error, "cannot record the highest build run");
                }
                self.probation_step(Input::Spawned { generation, build }, false)
                    .await;
            }
            EventKind::State { to, cause, .. } => {
                match to {
                    State::Online | State::Degraded => {
                        self.probation_step(Input::Ready { generation }, false).await;
                    }
                    State::Stopping => {
                        let end = End::Requested;
                        self.probation_step(Input::Ended { generation, end }, false).await;
                    }
                    State::Hung => {
                        let end = End::Hung;
                        self.probation_step(Input::Ended { generation, end }, true).await;
                    }
                    State::Crashed => {
                        self.running = None;
                        let end = match cause {
                            Cause::BindError { .. } => End::BindError,
                            Cause::Hang { .. } => End::Hung,
                            _ => End::Crashed,
                        };
                        self.probation_step(Input::Ended { generation, end }, false).await;
                    }
                    State::Stopped => {
                        self.running = None;
                        if matches!(cause, Cause::LicenceRejected { .. }) {
                            let end = End::LicenceRejected;
                            self.probation_step(Input::Ended { generation, end }, false).await;
                        }
                    }
                    State::CrashLoop => self.running = None,
                    State::Starting => {}
                }
                let ended = matches!(to, State::Crashed | State::Stopped | State::CrashLoop);
                if ended && self.rollback_after == Some(generation) {
                    self.rollback_after = None;
                    self.start_last_healthy().await;
                }
                self.publish();
            }
            _ => {}
        }
    }

    /// The build number of a folder FXServer was started from, when it is one of the artifacts
    /// folder's.
    fn build_of(&self, folder: &Path) -> Option<u32> {
        let folder = std::path::absolute(folder).ok()?;
        if folder.parent()? != self.root {
            return None;
        }
        folder
            .file_name()?
            .to_str()?
            .parse()
            .ok()
            .filter(|build| BUILDS.contains(build))
    }

    /// Runs the probation one step and does what it says. `still_running` is a run that failed
    /// before it ended, as a hang does: the last healthy build waits for its end.
    async fn probation_step(&mut self, input: Input, still_running: bool) {
        let now = self.clock.now();
        let (next, verdicts) = probation::step(self.probation.clone(), input, now, self.timings.stable_for);
        self.probation = next;
        let generation = match input {
            Input::Ended { generation, .. } => Some(generation),
            _ => None,
        };
        for verdict in verdicts {
            if let Err(error) = self.verdict(verdict, generation, still_running).await {
                warn!(%error, ?verdict, "cannot record the probation's verdict");
            }
        }
        self.publish();
    }

    async fn verdict(
        &mut self,
        verdict: Verdict,
        generation: Option<u32>,
        still_running: bool,
    ) -> Result<(), Error> {
        let now = self.clock.now();
        match verdict {
            Verdict::Opened { build, from } => {
                info!(build, ?from, "a probation opened");
                self.change(move |_, state| {
                    state.probation = Some(stored::OpenProbation { build, from });
                    Ok(())
                })
                .await
            }
            Verdict::Closed { .. } => {
                self.change(move |_, state| {
                    state.probation = None;
                    Ok(())
                })
                .await
            }
            Verdict::Passed { build } => {
                info!(build, "a probation passed");
                self.change(move |tx, state| {
                    state.probation = None;
                    state.last_healthy = Some(HealthyBuild { build, since: now });
                    system(AuditAction::ProbationPassed)
                        .details(json!({ "build": build }))
                        .write(tx, now)?;
                    Ok(())
                })
                .await
            }
            Verdict::Failed { build, from, reason } => {
                self.probation_failed(build, from, reason, generation, still_running)
                    .await
            }
        }
    }

    /// Goes back to the last healthy build when it may, and says why when it does not.
    async fn probation_failed(
        &mut self,
        build: u32,
        from: Option<u32>,
        reason: probation::Failure,
        generation: Option<u32>,
        still_running: bool,
    ) -> Result<(), Error> {
        let now = self.clock.now();
        let valid = from.is_some_and(|from| self.health(from) == Some(Ok(())));
        let target =
            probation::rollback_target(self.settings.auto_rollback, from, valid, self.state.highest_run);
        let failure = self
            .change(move |tx, state| {
                state.probation = None;
                let (kept, other) = match target {
                    // Chosen while the build was on probation, so already handed over: the
                    // supervisor's recovery starts it, and nothing goes back.
                    _ if state.selected != Some(build) => (NoticeReason::Replaced, state.selected),
                    Err(kept) => (kept.reason(), None),
                    Ok(to) => {
                        state.selected = Some(to);
                        state.notice = Some(ArtifactNotice {
                            kind: NoticeKind::RolledBack,
                            build,
                            other: Some(to),
                            reason: reason.reason(),
                            at: now,
                        });
                        system(AuditAction::BuildRolledBack)
                            .outcome(Outcome::Failure)
                            .details(json!({ "from": build, "to": to, "reason": reason.reason() }))
                            .write(tx, now)?;
                        return Ok(Fallback::RolledBack(to));
                    }
                };
                state.notice = Some(ArtifactNotice {
                    kind: NoticeKind::ProbationFailed,
                    build,
                    other,
                    reason: kept,
                    at: now,
                });
                system(AuditAction::ProbationFailed)
                    .outcome(Outcome::Failure)
                    .details(json!({ "build": build, "reason": reason.reason(), "why": kept }))
                    .write(tx, now)?;
                Ok(Fallback::Kept(kept))
            })
            .await?;
        match failure {
            Fallback::Kept(why) => {
                warn!(
                    build,
                    ?reason,
                    ?why,
                    "a build failed its probation: nothing goes back"
                );
            }
            Fallback::RolledBack(to) => {
                warn!(
                    build,
                    ?reason,
                    to,
                    "a build failed its probation: going back to the last healthy one"
                );
                self.servers.launch_changed();
                if still_running {
                    self.rollback_after = generation;
                } else {
                    self.start_last_healthy().await;
                }
            }
        }
        Ok(())
    }

    async fn start_last_healthy(&mut self) {
        if let Err(error) = self.supervision.start_after_rollback().await {
            warn!(%error, "cannot start the last healthy build");
        }
    }

    /// Takes in a change another writer made, the schedule's promotion of the pending build.
    async fn reload(&mut self) {
        match self.db.read(|conn| stored::state(conn, Servers::DEFAULT)).await {
            Ok(state) => {
                self.state = state;
                self.publish();
            }
            Err(error) => warn!(%error, "cannot read the artifact state"),
        }
    }

    /// Changes the stored state in one transaction, from what is stored now: the schedule's
    /// promotion writes it too.
    async fn change<T, F>(&mut self, change: F) -> Result<T, Error>
    where
        T: Send + 'static,
        F: FnOnce(&Transaction<'_>, &mut ArtifactState) -> Result<T, observer_store::StoreError>
            + Send
            + 'static,
    {
        let now = self.clock.now();
        let (state, value) = self
            .db
            .write(move |tx| -> Result<_, Error> {
                let mut state = stored::state(tx, Servers::DEFAULT)?;
                let value = change(tx, &mut state)?;
                let saved = stored::save_state(tx, Servers::DEFAULT, &state, now)?;
                Ok((saved, value))
            })
            .await?;
        self.probation.last_healthy = state.last_healthy.map(|healthy| healthy.build);
        self.probation.open = state.probation;
        self.state = state;
        Ok(value)
    }

    /// Looks at the folder, the volume and the manifests again. A build the command line
    /// installed shows up, with its fingerprint.
    async fn rescan(&mut self) {
        match self.installer.installed().await {
            Ok(installed) => self.installed = installed,
            Err(error) => warn!(%error, "cannot list the installed builds"),
        }
        let root = self.root.clone();
        self.disk = tokio::task::spawn_blocking(move || observer_platform::volume_space(&root))
            .await
            .ok()
            .and_then(Result::ok)
            .map(|space| DiskSpace {
                free: space.available,
                total: space.total,
            });
        let fingerprints: Vec<(u32, String)> = self
            .installed
            .iter()
            .filter_map(|scanned| {
                let manifest = scanned.manifest.as_ref()?;
                Some((scanned.build, manifest.origin.archive_sha256()?.to_owned()))
            })
            .collect();
        if fingerprints.is_empty() {
            return;
        }
        let (platform, now) = (BuildPlatform::host(), self.clock.now());
        let remembered = self
            .db
            .write(move |tx| {
                for (build, sha256) in &fingerprints {
                    stored::remember_fingerprint(tx, platform, *build, sha256, now)?;
                }
                Ok::<_, Error>(())
            })
            .await;
        if let Err(error) = remembered {
            warn!(%error, "cannot remember the fingerprints of the installed builds");
        }
    }

    /// Selects the build folder of the launch settings from before, when it already lies in the
    /// artifacts folder: it is the build FXServer ran, installed by the command line.
    async fn adopt(&mut self) -> Result<(), Error> {
        let Some(legacy) = self.state.legacy_build_path.clone() else {
            return Ok(());
        };
        if self.state.selected.is_some() {
            return Ok(());
        }
        let Some(build) = std::fs::canonicalize(&legacy).ok().and_then(|folder| {
            let root = std::fs::canonicalize(&self.root).ok()?;
            if folder.parent()? != root {
                return None;
            }
            folder.file_name()?.to_str()?.parse::<u32>().ok()
        }) else {
            return Ok(());
        };
        if self.health(build) != Some(Ok(())) {
            return Ok(());
        }
        let now = self.clock.now();
        self.change(move |tx, state| {
            state.selected = Some(build);
            state.last_healthy = Some(HealthyBuild { build, since: now });
            // It ran on this server's data, which the KVP guard must know before its next spawn.
            state.highest_run = state.highest_run.max(Some(build));
            state.legacy_build_path = None;
            system(AuditAction::BuildSelected)
                .details(json!({ "build": build, "adopted": true }))
                .write(tx, now)?;
            Ok(())
        })
        .await?;
        info!(
            build,
            "the build folder of the launch settings is the selected build"
        );
        Ok(())
    }

    fn health(&self, build: u32) -> Option<Result<(), observer_domain::artifacts::BrokenReason>> {
        let scanned = self.installed.iter().find(|scanned| scanned.build == build)?;
        Some(match scanned.health {
            Health::Valid => Ok(()),
            Health::Broken(broken) => Err(view::broken_reason(broken)),
        })
    }

    fn bytes(&self, build: u32) -> u64 {
        self.installed
            .iter()
            .find(|scanned| scanned.build == build)
            .map_or(0, |scanned| scanned.bytes)
    }

    /// Why each build in use may not be deleted.
    fn protections(&self) -> BTreeMap<u32, Vec<Protection>> {
        let mut protected: BTreeMap<u32, Vec<Protection>> = BTreeMap::new();
        let mut protect = |build: Option<u32>, reason| {
            if let Some(build) = build {
                protected.entry(build).or_default().push(reason);
            }
        };
        protect(self.state.selected, Protection::Selected);
        protect(
            self.state.pending.as_ref().map(|pending| pending.build),
            Protection::Pending,
        );
        protect(self.running, Protection::Running);
        protect(
            self.state.last_healthy.map(|healthy| healthy.build),
            Protection::LastHealthy,
        );
        protect(
            self.state.probation.map(|probation| probation.build),
            Protection::Probation,
        );
        protected
    }

    async fn audit(&self, record: Record) -> Result<(), Error> {
        let now = self.clock.now();
        let record = record.target(Servers::DEFAULT);
        self.db
            .write(move |tx| record.write(tx, now).map_err(Error::from))
            .await
    }

    fn status(&self) -> ArtifactsStatus {
        let probation = self
            .probation
            .open
            .map(|open| observer_domain::artifacts::Probation {
                build: open.build,
                from: open.from,
                ready_at: self.probation.run.and_then(|run| run.ready_at),
                passes_at: probation::passes_at(&self.probation, self.timings.stable_for),
            });
        ArtifactsStatus {
            platform: BuildPlatform::host(),
            installed: self.installed.iter().map(view::installed).collect(),
            selected: self.state.selected,
            pending: self.state.pending.clone(),
            running: self.running,
            last_healthy: self.state.last_healthy,
            probation,
            highest_run: self.state.highest_run,
            notice: self.state.notice.clone(),
            job: self.job.as_ref().map(|slot| slot.job.clone()),
            settings: self.settings.clone(),
            version: self.version,
            legacy_folder: self.state.legacy_build_path.is_some(),
            long_path: self
                .installer
                .long_path()
                .and_then(|length| u32::try_from(length).ok()),
            disk: self.disk,
        }
    }

    /// Republishes the status when it changed, a running job's progress included, and returns it.
    fn publish(&mut self) -> ArtifactsStatus {
        if let Some(slot) = self.job.as_mut().filter(|slot| slot.task.is_some()) {
            refresh(slot);
        }
        let next = self.status();
        self.status.send_if_modified(|current| {
            let changed = *current != next;
            if changed {
                current.clone_from(&next);
            }
            changed
        });
        next
    }
}

enum Removal {
    InUse,
    Busy,
    Failed(Error),
}

/// What a failed probation led to.
enum Fallback {
    /// Nothing went back, for this reason.
    Kept(NoticeReason),
    /// The last healthy build is selected again.
    RolledBack(u32),
}

fn system(action: AuditAction) -> Record {
    Record::operator(ActorKind::System, action).target(Servers::DEFAULT)
}

/// What a job reported, kept for the task to read.
fn record(live: &Mutex<Live>, progress: &Progress) {
    let mut live = live.lock().unwrap_or_else(PoisonError::into_inner);
    live.step = Some(view::step(progress));
    match progress {
        Progress::Downloading { received, total } => {
            live.received = *received;
            live.total = *total;
        }
        Progress::Extracting { total, done } => live.entries = Some((Arc::clone(done), *total)),
        Progress::Copying { total, done } => live.entries = Some((Arc::clone(done), Some(*total))),
        Progress::Checking | Progress::Verifying | Progress::Validating | Progress::Committing => {}
    }
}

/// Brings a job's step and counters up to what it reported, with the rate over the last seconds.
fn refresh(slot: &mut Slot) {
    let live = slot.live.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(step) = live.step {
        slot.job.step = step;
    }
    if live.total > 0 {
        let now = Instant::now();
        slot.samples.push_back((now, live.received));
        while slot
            .samples
            .front()
            .is_some_and(|(at, _)| now.duration_since(*at) > RATE_WINDOW)
        {
            slot.samples.pop_front();
        }
        let rate = match (slot.samples.front(), slot.samples.back()) {
            (Some((first, from)), Some((last, to))) if last > first => {
                let seconds = last.duration_since(*first).as_secs_f64();
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    clippy::cast_precision_loss,
                    reason = "a rate of bytes per second, shown to a tenth of a megabyte"
                )]
                let rate = (to.saturating_sub(*from) as f64 / seconds) as u64;
                rate
            }
            _ => 0,
        };
        slot.job.download = Some(DownloadProgress {
            received: live.received,
            total: live.total,
            rate,
        });
    }
    if let Some((done, total)) = &live.entries {
        slot.job.entries = Some(EntryProgress {
            done: done.load(Ordering::Relaxed),
            total: *total,
        });
    }
}

/// Everything wrong with artifact settings, at most one problem per branch.
fn settings_issues(settings: &ArtifactSettings) -> Vec<ArtifactSettingsIssue> {
    let mut issues = Vec::new();
    if !(1..=MAX_KEEP_BUILDS).contains(&settings.keep_builds) {
        issues.push(ArtifactSettingsIssue {
            field: ArtifactSettingsField::KeepBuilds,
            index: None,
            problem: ArtifactSettingsProblem::OutOfRange,
        });
    }
    if settings.branches.len() > MAX_BRANCHES {
        issues.push(ArtifactSettingsIssue {
            field: ArtifactSettingsField::Branches,
            index: None,
            problem: ArtifactSettingsProblem::TooMany,
        });
    }
    for (index, name) in settings.branches.iter().enumerate().take(MAX_BRANCHES) {
        let problem = match name.parse::<observer_artifacts::Branch>() {
            Err(_) => Some(ArtifactSettingsProblem::Invalid),
            Ok(branch) if branch.is_master() => Some(ArtifactSettingsProblem::Master),
            Ok(_) if settings.branches[..index].contains(name) => Some(ArtifactSettingsProblem::Duplicate),
            Ok(_) => None,
        };
        if let Some(problem) = problem {
            issues.push(ArtifactSettingsIssue {
                field: ArtifactSettingsField::Branches,
                index: Some(index),
                problem,
            });
        }
    }
    issues
}

/// Each setting that differs, with its value before and after.
fn setting_changes(before: &ArtifactSettings, after: &ArtifactSettings) -> Value {
    let fields = |settings: &ArtifactSettings| match serde_json::to_value(settings) {
        Ok(Value::Object(fields)) => fields,
        _ => Map::new(),
    };
    let old = fields(before);
    let changed: Map<String, Value> = fields(after)
        .into_iter()
        .filter_map(|(name, value)| {
            let previous = old.get(&name).cloned().unwrap_or(Value::Null);
            (previous != value).then(|| (name, json!({ "before": previous, "after": value })))
        })
        .collect();
    Value::Object(changed)
}

/// What the status watch holds until the first real status is made.
fn placeholder() -> ArtifactsStatus {
    ArtifactsStatus {
        platform: BuildPlatform::host(),
        installed: Vec::new(),
        selected: None,
        pending: None,
        running: None,
        last_healthy: None,
        probation: None,
        highest_run: None,
        notice: None,
        job: None,
        settings: ArtifactSettings::default(),
        version: None,
        legacy_folder: false,
        long_path: None,
        disk: None,
    }
}

/// What a scheduled restart did with the pending build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Promotion {
    Applied {
        build: u32,
        from: Option<u32>,
    },
    /// Deleted or broken since it was chosen: the restart goes ahead on the selected build.
    Dropped {
        build: u32,
    },
}

/// Makes the pending build the selected one, when it is still installed and valid, or drops it
/// with a notice. It runs in the transaction of the schedule's restart, before the launch is
/// handed over, so that the restart starts the build it applied.
pub(crate) fn promote_pending(
    tx: &Transaction<'_>,
    root: &Path,
    now: Timestamp,
) -> Result<Option<Promotion>, Error> {
    let state = stored::state(tx, Servers::DEFAULT)?;
    let Some(pending) = state.pending.clone() else {
        return Ok(None);
    };
    let folder = root.join(pending.build.to_string());
    let (_, health) = observer_artifacts::inspect(&folder, pending.build, Platform::host());
    let mut next = ArtifactState {
        pending: None,
        ..state.clone()
    };
    let promotion = if health == Health::Valid {
        next.selected = Some(pending.build);
        system(AuditAction::PendingBuildApplied)
            .details(json!({ "build": pending.build, "from": state.selected, "at": now }))
            .write(tx, now)?;
        Promotion::Applied {
            build: pending.build,
            from: state.selected,
        }
    } else {
        let reason = if std::fs::symlink_metadata(&folder).is_err() {
            NoticeReason::NotInstalled
        } else {
            NoticeReason::Broken
        };
        next.notice = Some(ArtifactNotice {
            kind: NoticeKind::PendingDropped,
            build: pending.build,
            other: None,
            reason,
            at: now,
        });
        system(AuditAction::PendingBuildDropped)
            .outcome(Outcome::Failure)
            .details(json!({ "build": pending.build, "reason": reason }))
            .write(tx, now)?;
        Promotion::Dropped { build: pending.build }
    };
    stored::save_state(tx, Servers::DEFAULT, &next, now)?;
    Ok(Some(promotion))
}

#[cfg(test)]
mod tests;
