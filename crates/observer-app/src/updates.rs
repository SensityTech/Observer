//! Observer's own updates: the check for a newer release on GitHub, and its install, which stages
//! and verifies the release's executable, then hands it to the binary, which stops Observer, swaps
//! the executable and starts the new one. When the owner chooses, it waits for the next scheduled
//! restart, so that resources hear the schedule's warnings. Whether the new version started, and
//! the return to the previous one when it did not, are the binary's: see [`handover`].

pub mod handover;
pub mod swap;
#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use jiff::Timestamp;
use observer_artifacts::releases::{Latest, Published, Releases, StageProgress};
use observer_domain::audit::{ActorKind, AuditAction, Outcome};
use observer_domain::error::{ApiError, ErrorCode};
use observer_domain::schedule::ScheduleStatus;
use observer_domain::server::ServerState;
use observer_domain::updates::{
    CheckFailure, InstallUpdate, MAX_NOTES_BYTES, Release, ReleaseAsset, UpdateBlock, UpdateFailure,
    UpdateJob, UpdateJobState, UpdateOutcome, UpdateResult, UpdateStep, UpdateWhen, UpdatesView,
};
use observer_store::{BackupKind, Database};
use serde_json::json;
use tokio::sync::{Mutex as AsyncMutex, mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::artifacts::DownloadRate;
use crate::audit::Record;
use crate::{Caller, Clock, Error, Supervision};

/// The first check waits a little, so that Observer's start does not wait on GitHub.
const FIRST_CHECK: Duration = Duration::from_secs(10);
const CHECK_EVERY: Duration = Duration::from_hours(6);
/// How often a job's progress is brought up to date for the panel.
const PROGRESS_EVERY: Duration = Duration::from_millis(500);

/// What [`Updates::start`] works with.
pub struct UpdatesSetup {
    pub db: Database,
    pub clock: Clock,
    pub data_dir: PathBuf,
    pub releases: Releases,
    /// Where Observer runs from: the file an install replaces.
    pub executable: PathBuf,
    /// The version this Observer runs.
    pub current: semver::Version,
    /// Built from source rather than a release, which an install would replace with a release.
    pub development: bool,
    pub supervision: Supervision,
    pub schedule: watch::Receiver<ScheduleStatus>,
    /// The tests' releases hold no executable that runs.
    #[cfg(test)]
    pub runs_executable: bool,
}

/// A verified executable that must take over: the binary stops Observer, as for any other stop,
/// then swaps it in and starts it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Takeover {
    pub from: String,
    pub to: String,
    pub staged: PathBuf,
    /// Of the staged executable, checked again before the swap.
    pub sha256: String,
    pub database_backup: PathBuf,
    /// FXServer runs now: the next Observer starts it again.
    pub fxserver: bool,
    /// It takes the place of a scheduled restart: resources hear `scheduled`.
    pub scheduled: bool,
}

/// The update checks and installs. Clones share them.
#[derive(Clone)]
pub struct Updates {
    inner: Arc<Inner>,
}

/// The check every six hours and the install under way, which [`UpdatesTask::close`] ends.
pub struct UpdatesTask {
    stop: CancellationToken,
    checker: JoinHandle<()>,
    inner: Arc<Inner>,
}

struct Inner {
    db: Database,
    clock: Clock,
    data_dir: PathBuf,
    releases: Releases,
    executable: PathBuf,
    current: semver::Version,
    blocked: Option<UpdateBlock>,
    supervision: Supervision,
    schedule: watch::Receiver<ScheduleStatus>,
    takeovers: mpsc::Sender<Takeover>,
    #[cfg(test)]
    runs_executable: bool,
    state: Mutex<State>,
    /// One check at a time: a check asked during another waits for it and answers the same.
    checking: AsyncMutex<()>,
    job: Mutex<Option<JoinHandle<()>>>,
    stop: CancellationToken,
}

#[derive(Default)]
struct State {
    checked_at: Option<Timestamp>,
    checking: bool,
    failure: Option<CheckFailure>,
    etag: Option<String>,
    latest: Option<Published>,
    job: Option<Slot>,
    last: Option<UpdateOutcome>,
}

struct Slot {
    view: UpdateJob,
    cancel: CancellationToken,
    rate: DownloadRate,
}

impl Updates {
    /// Starts the checks. The receiver gets the executable that must take over, once an install
    /// is verified and its time has come.
    pub fn start(setup: UpdatesSetup) -> (Self, UpdatesTask, mpsc::Receiver<Takeover>) {
        let (takeovers, receiver) = mpsc::channel(1);
        let writable = setup.executable.parent().is_some_and(swap::writable);
        let blocked = if setup.development {
            Some(UpdateBlock::DevelopmentBuild)
        } else if !setup.releases.has_key() {
            Some(UpdateBlock::NoReleaseKey)
        } else if !writable {
            Some(UpdateBlock::ReadOnly)
        } else {
            None
        };
        let state = State {
            last: handover::read_last(&setup.data_dir),
            ..State::default()
        };
        let stop = CancellationToken::new();
        let inner = Arc::new(Inner {
            db: setup.db,
            clock: setup.clock,
            data_dir: setup.data_dir,
            releases: setup.releases,
            executable: setup.executable,
            current: setup.current,
            blocked,
            supervision: setup.supervision,
            schedule: setup.schedule,
            takeovers,
            #[cfg(test)]
            runs_executable: setup.runs_executable,
            state: Mutex::new(state),
            checking: AsyncMutex::new(()),
            job: Mutex::new(None),
            stop: stop.clone(),
        });
        let updates = Self {
            inner: Arc::clone(&inner),
        };
        let checker = tokio::spawn(check_regularly(updates.clone(), stop.clone()));
        (updates, UpdatesTask { stop, checker, inner }, receiver)
    }

    pub fn view(&self) -> UpdatesView {
        let state = self.inner.state();
        UpdatesView {
            current: self.inner.current.to_string(),
            checked_at: state.checked_at,
            checking: state.checking,
            check_failure: state.failure.clone(),
            available: state
                .latest
                .as_ref()
                .filter(|release| release.version > self.inner.current)
                .map(release_view),
            job: state.job.as_ref().map(|slot| slot.view.clone()),
            last: state.last.clone(),
            blocked: self.inner.blocked,
            next_restart_at: self.inner.next_restart(),
        }
    }

    /// How the install that restarted Observer ended, once this one settled it as it started.
    pub fn settled(&self, outcome: UpdateOutcome) {
        self.inner.state().last = Some(outcome);
    }

    /// Asks GitHub now, and answers once it has.
    pub async fn check(&self) -> UpdatesView {
        self.check_now().await;
        self.view()
    }

    async fn check_now(&self) {
        let _one = self.inner.checking.lock().await;
        let etag = {
            let mut state = self.inner.state();
            state.checking = true;
            state.etag.clone()
        };
        let answer = self.inner.releases.latest(etag.as_deref()).await;
        let now = self.inner.clock.now();
        let mut state = self.inner.state();
        state.checking = false;
        match answer {
            Ok(Latest::Unchanged) => {
                state.checked_at = Some(now);
                state.failure = None;
            }
            Ok(Latest::Found { release, etag }) => {
                if release.version > self.inner.current
                    && state
                        .latest
                        .as_ref()
                        .is_none_or(|latest| latest.version != release.version)
                {
                    info!(version = %release.version, "a newer release of Observer is out");
                }
                state.checked_at = Some(now);
                state.failure = None;
                state.etag = etag;
                state.latest = Some(release);
            }
            Ok(Latest::None) => {
                state.failure = Some(CheckFailure::NoRelease);
                state.etag = None;
                state.latest = None;
            }
            Err(error) => {
                warn!(%error, "cannot check for a newer release of Observer");
                state.failure = Some(check_failure(&error));
            }
        }
    }

    /// Downloads and verifies the release the owner saw, then hands it over now or at the next
    /// scheduled restart. The job runs on: the answer comes once it has begun.
    pub async fn install(&self, caller: &Caller, request: InstallUpdate) -> Result<UpdatesView, Error> {
        if let Some(reason) = self.inner.blocked {
            return Err(ApiError::new(ErrorCode::UpdateBlocked)
                .with("reason", json!(reason))
                .into());
        }
        if request.when == UpdateWhen::ScheduledRestart && self.inner.next_restart().is_none() {
            return Err(ErrorCode::NoScheduledRestart.into());
        }
        let now = self.inner.clock.now();
        let (release, cancel) = {
            let mut state = self.inner.state();
            if state
                .job
                .as_ref()
                .is_some_and(|slot| slot.view.state == UpdateJobState::Running)
            {
                return Err(ErrorCode::UpdateRunning.into());
            }
            let release = state
                .latest
                .clone()
                .filter(|release| release.version > self.inner.current)
                .filter(|release| release.version.to_string() == request.version)
                .ok_or(ErrorCode::UpdateNotAvailable)?;
            let cancel = self.inner.stop.child_token();
            state.job = Some(Slot {
                view: UpdateJob {
                    version: request.version.clone(),
                    when: request.when,
                    step: UpdateStep::Download,
                    state: UpdateJobState::Running,
                    started_by: caller.username.clone(),
                    started_at: now,
                    finished_at: None,
                    download: None,
                    restart_at: None,
                    failure: None,
                },
                cancel: cancel.clone(),
                rate: DownloadRate::default(),
            });
            (release, cancel)
        };
        let record = Record::by(caller, AuditAction::ObserverUpdateStarted)
            .target(request.version.clone())
            .details(json!({ "applies": request.when }));
        self.inner.audit(record).await;
        let job = tokio::spawn(run_job(Arc::clone(&self.inner), release, request.when, cancel));
        *self.inner.job.lock().unwrap_or_else(PoisonError::into_inner) = Some(job);
        Ok(self.view())
    }

    /// Stops the install under way, until Observer begins to restart.
    pub async fn cancel(&self, caller: &Caller) -> Result<UpdatesView, Error> {
        let (version, step) = {
            let mut state = self.inner.state();
            let Some(slot) = state
                .job
                .as_mut()
                .filter(|slot| slot.view.state == UpdateJobState::Running)
            else {
                return Err(ErrorCode::NoUpdate.into());
            };
            if slot.view.step == UpdateStep::Restart {
                return Err(ErrorCode::UpdateRestarting.into());
            }
            slot.cancel.cancel();
            slot.view.state = UpdateJobState::Cancelled;
            slot.view.finished_at = Some(self.inner.clock.now());
            (slot.view.version.clone(), slot.view.step)
        };
        self.inner.supervision.keep_next_restart();
        let record = Record::by(caller, AuditAction::ObserverUpdateCancelled)
            .target(version)
            .details(json!({ "step": step }));
        self.inner.audit(record).await;
        Ok(self.view())
    }
}

impl UpdatesTask {
    /// Ends the checks, and an install that has not handed over yet.
    pub async fn close(self) {
        self.stop.cancel();
        self.inner.supervision.keep_next_restart();
        let _ = self.checker.await;
        let job = self
            .inner
            .job
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(job) = job {
            let _ = job.await;
        }
    }
}

impl Inner {
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The restart an install can wait for: the schedule's next, while FXServer runs.
    fn next_restart(&self) -> Option<Timestamp> {
        let running = runs(self.supervision.status().borrow().state);
        running
            .then(|| self.schedule.borrow().next.as_ref().map(|next| next.at))
            .flatten()
    }

    fn update_job(&self, change: impl FnOnce(&mut Slot)) {
        if let Some(slot) = self.state().job.as_mut() {
            change(slot);
        }
    }

    async fn audit(&self, record: Record) {
        let now = self.clock.now();
        if let Err(error) = self.db.write(move |tx| record.write(tx, now)).await {
            warn!(%error, "cannot audit an update of Observer");
        }
    }

    /// Marks the job failed with `failure`, and audits it.
    async fn fail(&self, version: &str, failure: UpdateFailure, error: &(dyn std::fmt::Display + Sync)) {
        warn!(%error, version, "the update of Observer stopped");
        let now = self.clock.now();
        self.update_job(|slot| {
            slot.view.state = UpdateJobState::Failed;
            slot.view.finished_at = Some(now);
            slot.view.failure = Some(failure.clone());
        });
        let record = Record::operator(ActorKind::System, AuditAction::ObserverUpdateFinished)
            .target(version.to_owned())
            .outcome(Outcome::Failure)
            .details(json!({ "from": self.current.to_string(), "to": version, "reason": failure }));
        self.audit(record).await;
    }
}

/// Whether FXServer runs, or comes back by itself: an install that waits for a restart keeps
/// waiting meanwhile.
fn runs(state: ServerState) -> bool {
    !matches!(state, ServerState::Stopped | ServerState::CrashLoop)
}

async fn check_regularly(updates: Updates, stop: CancellationToken) {
    let mut wait = FIRST_CHECK;
    loop {
        tokio::select! {
            () = stop.cancelled() => return,
            () = tokio::time::sleep(wait) => {}
        }
        tokio::select! {
            () = stop.cancelled() => return,
            () = updates.check_now() => {}
        }
        wait = CHECK_EVERY;
    }
}

async fn run_job(inner: Arc<Inner>, release: Published, when: UpdateWhen, cancel: CancellationToken) {
    let version = release.version.to_string();
    let Some(staged) = stage(&inner, &release, &cancel).await else {
        return;
    };
    inner.update_job(|slot| slot.view.step = UpdateStep::Verify);
    #[cfg(test)]
    let run_it = inner.runs_executable;
    #[cfg(not(test))]
    let run_it = true;
    if run_it && let Err(error) = staged.check(&inner.executable).await {
        inner.fail(&version, failure(&error), &error).await;
        return;
    }
    if cancel.is_cancelled() {
        return;
    }
    let sha256 = match swap::sha256(&staged.executable) {
        Ok(sha256) => sha256,
        Err(error) => return inner.fail(&version, UpdateFailure::Io, &error).await,
    };
    let scheduled = match when {
        UpdateWhen::Now => false,
        UpdateWhen::ScheduledRestart => match wait_for_restart(&inner, &cancel).await {
            Some(scheduled) => scheduled,
            None => return,
        },
    };
    if cancel.is_cancelled() {
        return;
    }
    let fxserver = scheduled || runs(inner.supervision.status().borrow().state);
    inner.update_job(|slot| slot.view.step = UpdateStep::Restart);
    let database_backup = match inner.db.backup(BackupKind::PreUpdate).await {
        Ok(backup) => backup,
        Err(error) => return inner.fail(&version, UpdateFailure::Io, &error).await,
    };
    info!(version, scheduled, "Observer restarts into its new release");
    let takeover = Takeover {
        from: inner.current.to_string(),
        to: version.clone(),
        staged: staged.executable,
        sha256,
        database_backup,
        fxserver,
        scheduled,
    };
    if inner.takeovers.send(takeover).await.is_err() {
        inner
            .fail(&version, UpdateFailure::Interrupted, &"Observer is stopping")
            .await;
    }
}

/// The release's executable, staged and verified under `updates/staging`, or `None` once the job
/// failed or was cancelled.
async fn stage(
    inner: &Arc<Inner>,
    release: &Published,
    cancel: &CancellationToken,
) -> Option<observer_artifacts::releases::Staged> {
    let version = release.version.to_string();
    let dir = handover::folder(&inner.data_dir).join("staging");
    let prepared = tokio::task::spawn_blocking({
        let dir = dir.clone();
        move || {
            if dir.exists() {
                std::fs::remove_dir_all(&dir)?;
            }
            std::fs::create_dir_all(&dir)
        }
    })
    .await
    .map_err(std::io::Error::other)
    .and_then(|prepared| prepared);
    if let Err(error) = prepared {
        inner.fail(&version, UpdateFailure::Io, &error).await;
        return None;
    }
    let progress = Arc::new(Mutex::new(None));
    let staging = {
        let progress = Arc::clone(&progress);
        let releases = inner.releases.clone();
        let release = release.clone();
        let cancel = cancel.clone();
        tokio::spawn(async move {
            let mut report = |step: StageProgress| {
                *progress.lock().unwrap_or_else(PoisonError::into_inner) = Some(step);
            };
            releases.stage(&release, &dir, &cancel, &mut report).await
        })
    };
    tokio::pin!(staging);
    let staged = loop {
        tokio::select! {
            staged = &mut staging => break staged,
            () = tokio::time::sleep(PROGRESS_EVERY) => {}
        }
        let reported = *progress.lock().unwrap_or_else(PoisonError::into_inner);
        inner.update_job(|slot| match reported {
            Some(StageProgress::Downloading { received, total }) => {
                slot.view.download = Some(slot.rate.measure(received, total));
            }
            Some(StageProgress::Verifying) => slot.view.step = UpdateStep::Verify,
            None => {}
        });
    };
    match staged {
        Ok(Ok(staged)) => Some(staged),
        Ok(Err(observer_artifacts::Error::Cancelled)) => {
            if inner.stop.is_cancelled() {
                inner
                    .fail(&version, UpdateFailure::Interrupted, &"Observer is stopping")
                    .await;
            }
            None
        }
        Ok(Err(error)) => {
            inner.fail(&version, failure(&error), &error).await;
            None
        }
        Err(error) => {
            inner.fail(&version, UpdateFailure::Io, &error).await;
            None
        }
    }
}

/// Waits for the next scheduled restart, which the supervision hands over in its place: `true`
/// then. Should FXServer stop meanwhile, nobody plays, and the install goes on at once: `false`.
/// `None` once cancelled.
async fn wait_for_restart(inner: &Arc<Inner>, cancel: &CancellationToken) -> Option<bool> {
    let (sender, mut takeover) = oneshot::channel();
    inner.supervision.hand_next_restart(sender);
    let mut status = inner.supervision.status();
    let mut schedule = inner.schedule.clone();
    inner.update_job(|slot| {
        slot.view.step = UpdateStep::Wait;
        slot.view.restart_at = inner.next_restart();
    });
    loop {
        if !runs(status.borrow_and_update().state) {
            inner.supervision.keep_next_restart();
            return Some(false);
        }
        tokio::select! {
            taken = &mut takeover => return taken.is_ok().then_some(true),
            () = cancel.cancelled() => {
                inner.supervision.keep_next_restart();
                if inner.stop.is_cancelled() {
                    let version = inner.state().job.as_ref().map(|slot| slot.view.version.clone()).unwrap_or_default();
                    inner.fail(&version, UpdateFailure::Interrupted, &"Observer is stopping").await;
                }
                return None;
            }
            changed = status.changed() => {
                if changed.is_err() {
                    return None;
                }
            }
            changed = schedule.changed() => {
                if changed.is_err() {
                    return None;
                }
                let at = inner.next_restart();
                inner.update_job(|slot| slot.view.restart_at = at);
            }
        }
    }
}

fn release_view(release: &Published) -> Release {
    let (notes, notes_cut) = cut(&release.notes, MAX_NOTES_BYTES);
    Release {
        version: release.version.to_string(),
        name: release.name.clone(),
        published_at: release.published_at,
        notes,
        notes_cut,
        url: release.url.clone(),
        asset: release.archive().map(|asset| ReleaseAsset {
            name: asset.name.clone(),
            size: asset.size,
        }),
        signed: release.signed(),
    }
}

/// `text` within `limit` bytes, cut at a character's boundary.
fn cut(text: &str, limit: usize) -> (String, bool) {
    if text.len() <= limit {
        return (text.to_owned(), false);
    }
    let end = (0..=limit)
        .rev()
        .find(|&at| text.is_char_boundary(at))
        .unwrap_or(0);
    (text[..end].to_owned(), true)
}

fn check_failure(error: &observer_artifacts::Error) -> CheckFailure {
    use observer_artifacts::Error as E;
    match error {
        E::RateLimited { until } => CheckFailure::RateLimited { until: *until },
        E::Status { status, .. } => CheckFailure::HttpStatus { status: *status },
        E::ReleaseFormat(_) => CheckFailure::Malformed,
        _ => CheckFailure::Unreachable,
    }
}

/// The code of a failed install; the full text goes to Observer's log.
fn failure(error: &observer_artifacts::Error) -> UpdateFailure {
    use observer_artifacts::Error as E;
    match error {
        E::Network { .. } | E::UrlRefused { .. } | E::TooManyRedirects { .. } | E::RateLimited { .. } => {
            UpdateFailure::Unreachable
        }
        E::Status { status, .. } => UpdateFailure::HttpStatus { status: *status },
        E::LengthMissing { .. } | E::LengthChanged { .. } | E::Overlong { .. } | E::BadRange { .. } => {
            UpdateFailure::BadLength
        }
        E::Truncated { .. } => UpdateFailure::Truncated,
        E::TooLarge { .. } => UpdateFailure::TooLarge,
        E::Unsigned | E::NoReleaseKey => UpdateFailure::Unsigned,
        E::BadSignature => UpdateFailure::BadSignature,
        E::NotListed { .. } | E::NoArchive { .. } => UpdateFailure::NotListed,
        E::ChecksumMismatch { .. } => UpdateFailure::ChecksumMismatch,
        E::DigestMismatch { .. } => UpdateFailure::DigestMismatch,
        E::Damaged(_) | E::Unsafe { .. } | E::ReleaseFormat(_) => UpdateFailure::Damaged,
        E::WontRun(_) => UpdateFailure::WontRun,
        E::ExecutableSignature(_) => UpdateFailure::Signature,
        E::DiskSpace {
            needed_mib,
            available_mib,
            ..
        } => UpdateFailure::DiskSpace {
            needed_mib: *needed_mib,
            available_mib: *available_mib,
        },
        E::Cancelled | E::Interrupted => UpdateFailure::Interrupted,
        _ => UpdateFailure::Io,
    }
}

/// How the install ended, once the next Observer settled it: kept for the panel, and audited.
pub async fn record_outcome(
    db: &Database,
    clock: &Clock,
    data_dir: &std::path::Path,
    outcome: &UpdateOutcome,
) {
    if let Err(error) = handover::write_last(data_dir, outcome) {
        warn!(%error, "cannot keep how the update of Observer ended");
    }
    let (result, details) = match outcome.result {
        UpdateResult::Installed => (
            Outcome::Success,
            json!({ "from": outcome.from, "to": outcome.to }),
        ),
        UpdateResult::RolledBack { reason } => (
            Outcome::Failure,
            json!({ "from": outcome.from, "to": outcome.to, "rolledBack": true, "reason": reason }),
        ),
    };
    let record = Record::operator(ActorKind::System, AuditAction::ObserverUpdateFinished)
        .target(outcome.to.clone())
        .outcome(result)
        .details(details);
    let now = clock.now();
    if let Err(error) = db.write(move |tx| record.write(tx, now)).await {
        warn!(%error, "cannot audit how the update of Observer ended");
    }
}
