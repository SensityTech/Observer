//! The updates service over a real database, with a local server in place of GitHub that serves a
//! release signed by the test key.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use jiff::SignedDuration;
use observer_artifacts::releases::fake::{FakeRelease, latest_path, test_key};
use observer_artifacts::test_server::{Reply, TestServer};
use observer_domain::audit::{ActorKind, AuditAction, AuditEntry, AuditFilter, Outcome};
use observer_domain::schedule::{ScheduleSettings, ScheduleStatus};
use observer_domain::updates::{SIGNATURE, UpdateFailure};
use tempfile::TempDir;

use super::*;
use crate::{Accounts, AuditLog, Client, PasswordCost, Running, Servers, Setup};

const WAIT: Duration = Duration::from_secs(20);
const EXECUTABLE: &[u8] = b"the new Observer";

struct Lab {
    dir: TempDir,
    db: Database,
    clock: Clock,
    audit: AuditLog,
    owner: Caller,
    updates: Updates,
    task: Option<UpdatesTask>,
    takeovers: mpsc::Receiver<Takeover>,
    server: TestServer,
    _running: Running,
}

impl Lab {
    /// An Observer that runs `current`, with `release` on the server.
    async fn new(current: &str, release: FakeRelease) -> Self {
        Self::serving(current, release, false).await
    }

    async fn serving(current: &str, release: FakeRelease, slow: bool) -> Self {
        let release = Arc::new(release);
        let server = TestServer::start(move |request, _| {
            let base = format!("http://{}", request.header("Host").unwrap_or_default());
            match release.answer(&request.path, &base) {
                Some(body) if slow && request.path != latest_path() && body.len() > 1024 => {
                    Reply::ok(body).cut_after(16)
                }
                Some(body) => Reply::ok(body),
                None => Reply::status(404),
            }
        })
        .await;
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path().join("observer.sqlite3"), dir.path().join("backups"))
            .await
            .unwrap();
        let clock = Clock::manual("2026-11-09T09:00:00Z".parse().unwrap());
        let accounts = Accounts::new(db.clone(), clock.clone(), PasswordCost::TEST)
            .await
            .unwrap();
        let servers = Servers::new(db.clone(), clock.clone());
        let (supervision, running) = Supervision::start(
            Setup {
                db: db.clone(),
                clock: clock.clone(),
                accounts: accounts.clone(),
                servers,
                data_dir: dir.path().join("data"),
                panel: SocketAddr::from(([127, 0, 0, 1], 41976)),
                artifacts: dir.path().join("data/artifacts"),
                console_logs: dir.path().join("data/logs/console"),
                log: None,
            },
            |_| {},
        )
        .await
        .unwrap();
        let owner = owner(&accounts).await;
        let executable = dir.path().join("bin").join("observer");
        std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
        std::fs::write(&executable, b"the running Observer").unwrap();
        let schedule = ScheduleStatus {
            settings: ScheduleSettings::default(),
            version: None,
            zone: "UTC".to_owned(),
            next: None,
            upcoming: Vec::new(),
            one_off: None,
        };
        let (updates, task, takeovers) = Updates::start(UpdatesSetup {
            db: db.clone(),
            clock: clock.clone(),
            data_dir: dir.path().join("data"),
            releases: Releases::local(server.port, Some(test_key())),
            executable,
            current: current.parse().unwrap(),
            development: false,
            supervision,
            schedule: watch::channel(schedule).1,
            runs_executable: false,
        });
        Self {
            audit: AuditLog::new(db.clone(), clock.clone()),
            db,
            dir,
            clock,
            owner,
            updates,
            task: Some(task),
            takeovers,
            server,
            _running: running,
        }
    }

    async fn install(&self, version: &str) -> Result<UpdatesView, Error> {
        self.updates
            .install(
                &self.owner,
                InstallUpdate {
                    version: version.to_owned(),
                    when: UpdateWhen::Now,
                },
            )
            .await
    }

    async fn job_ended(&self) -> UpdateJob {
        tokio::time::timeout(WAIT, async {
            loop {
                let job = self.updates.view().job.unwrap();
                if job.state != UpdateJobState::Running {
                    return job;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the install never ended")
    }

    async fn entries(&self, action: AuditAction) -> Vec<AuditEntry> {
        self.clock.advance(SignedDuration::from_secs(2));
        let filter = AuditFilter {
            action: Some(action),
            ..AuditFilter::default()
        };
        self.audit.page(filter).await.unwrap().entries
    }
}

async fn owner(accounts: &Accounts) -> Caller {
    let client = Client::new(IpAddr::V4(Ipv4Addr::LOCALHOST), Some("test"));
    let token = accounts
        .create_setup_token(ActorKind::System)
        .await
        .unwrap()
        .unwrap()
        .token;
    let signed_in = accounts
        .complete_setup(&token, "ada", "correct horse battery staple".to_owned(), &client)
        .await
        .unwrap();
    accounts
        .authenticate(&signed_in.token, &signed_in.key, client)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_newer_release_is_offered_with_its_notes_and_an_older_one_is_not() {
    let lab = Lab::new("0.1.0", FakeRelease::new("0.2.0", EXECUTABLE)).await;

    let view = lab.updates.check().await;

    let release = view.available.unwrap();
    assert_eq!(view.current, "0.1.0");
    assert_eq!(release.version, "0.2.0");
    assert!(release.signed);
    assert!(release.notes.contains("## What's Changed"));
    assert_eq!(release.asset.unwrap().size, {
        let fake = FakeRelease::new("0.2.0", EXECUTABLE);
        fake.file(&fake.archive_name()).unwrap().len() as u64
    });
    assert!(view.checked_at.is_some());
    assert_eq!(view.blocked, None);

    let same = Lab::new("0.2.0", FakeRelease::new("0.2.0", EXECUTABLE)).await;
    assert_eq!(same.updates.check().await.available, None);
}

#[tokio::test]
async fn a_verified_release_is_handed_over_with_a_backup_of_the_database() {
    let mut lab = Lab::new("0.1.0", FakeRelease::new("0.2.0", EXECUTABLE)).await;
    lab.updates.check().await;

    lab.install("0.2.0").await.unwrap();
    let takeover = tokio::time::timeout(WAIT, lab.takeovers.recv())
        .await
        .unwrap()
        .unwrap();

    assert_eq!(takeover.from, "0.1.0");
    assert_eq!(takeover.to, "0.2.0");
    assert_eq!(std::fs::read(&takeover.staged).unwrap(), EXECUTABLE);
    assert_eq!(takeover.sha256, swap::sha256(&takeover.staged).unwrap());
    assert!(takeover.database_backup.exists());
    assert!(!takeover.fxserver);
    assert!(!takeover.scheduled);
    assert_eq!(lab.updates.view().job.unwrap().step, UpdateStep::Restart);
    let started = lab.entries(AuditAction::ObserverUpdateStarted).await;
    assert_eq!(started.len(), 1);
    assert_eq!(started[0].actor_name.as_deref(), Some("ada"));
    assert_eq!(started[0].target.as_deref(), Some("0.2.0"));
    let refused = lab.updates.cancel(&lab.owner).await.unwrap_err();
    assert_eq!(refused.code(), Some(ErrorCode::UpdateRestarting));
}

#[tokio::test]
async fn a_release_whose_signature_fails_is_not_installed_and_the_failure_is_audited() {
    let mut release = FakeRelease::new("0.2.0", EXECUTABLE);
    release.set(SIGNATURE, vec![1; 64]);
    let mut lab = Lab::new("0.1.0", release).await;
    lab.updates.check().await;

    lab.install("0.2.0").await.unwrap();
    let job = lab.job_ended().await;

    assert_eq!(job.state, UpdateJobState::Failed);
    assert_eq!(job.failure, Some(UpdateFailure::BadSignature));
    assert!(lab.takeovers.try_recv().is_err());
    let finished = lab.entries(AuditAction::ObserverUpdateFinished).await;
    assert_eq!(finished.len(), 1);
    assert_eq!(finished[0].outcome, Outcome::Failure);
    assert_eq!(finished[0].details["reason"]["code"], "badSignature");
}

#[tokio::test]
async fn only_the_release_found_can_be_installed_and_one_at_a_time() {
    let lab = Lab::serving("0.1.0", FakeRelease::new("0.2.0", EXECUTABLE), true).await;

    let unchecked = lab.install("0.2.0").await.unwrap_err();
    assert_eq!(unchecked.code(), Some(ErrorCode::UpdateNotAvailable));
    lab.updates.check().await;
    let other = lab.install("0.3.0").await.unwrap_err();
    assert_eq!(other.code(), Some(ErrorCode::UpdateNotAvailable));

    lab.install("0.2.0").await.unwrap();
    let second = lab.install("0.2.0").await.unwrap_err();
    assert_eq!(second.code(), Some(ErrorCode::UpdateRunning));
    let at_restart = lab
        .updates
        .install(
            &lab.owner,
            InstallUpdate {
                version: "0.2.0".to_owned(),
                when: UpdateWhen::ScheduledRestart,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(at_restart.code(), Some(ErrorCode::NoScheduledRestart));
}

#[tokio::test]
async fn an_install_cancelled_during_its_download_hands_nothing_over() {
    let mut lab = Lab::serving("0.1.0", FakeRelease::new("0.2.0", EXECUTABLE), true).await;
    lab.updates.check().await;
    lab.install("0.2.0").await.unwrap();

    let view = lab.updates.cancel(&lab.owner).await.unwrap();

    assert_eq!(view.job.unwrap().state, UpdateJobState::Cancelled);
    lab.task.take().unwrap().close().await;
    assert!(lab.takeovers.try_recv().is_err());
    let cancelled = lab.entries(AuditAction::ObserverUpdateCancelled).await;
    assert_eq!(cancelled.len(), 1);
    assert!(lab.updates.cancel(&lab.owner).await.is_err());
}

#[tokio::test]
async fn no_release_on_github_is_said_as_such() {
    let server = TestServer::start(|_, _| Reply::status(404)).await;
    let lab = Lab::new("0.1.0", FakeRelease::new("0.2.0", EXECUTABLE)).await;
    let (updates, _task, _takeovers) = Updates::start(UpdatesSetup {
        releases: Releases::local(server.port, Some(test_key())),
        ..lab_setup(&lab)
    });

    let view = updates.check().await;

    assert_eq!(view.available, None);
    assert_eq!(view.check_failure, Some(CheckFailure::NoRelease));
}

#[tokio::test]
async fn a_development_build_is_told_it_cannot_install_a_release() {
    let lab = Lab::new("0.1.0", FakeRelease::new("0.2.0", EXECUTABLE)).await;
    let setup = lab_setup(&lab);
    let (updates, _task, _takeovers) = Updates::start(UpdatesSetup {
        development: true,
        ..setup
    });
    updates.check().await;

    assert_eq!(updates.view().blocked, Some(UpdateBlock::DevelopmentBuild));
    let refused = updates
        .install(
            &lab.owner,
            InstallUpdate {
                version: "0.2.0".to_owned(),
                when: UpdateWhen::Now,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Some(ErrorCode::UpdateBlocked));
}

#[test]
fn notes_are_cut_at_a_character_boundary() {
    assert_eq!(cut("short", 64), ("short".to_owned(), false));
    assert_eq!(cut("éé", 3), ("é".to_owned(), true));
}

/// The setup of a second service beside the lab's, on the same database and server.
fn lab_setup(lab: &Lab) -> UpdatesSetup {
    let schedule = ScheduleStatus {
        settings: ScheduleSettings::default(),
        version: None,
        zone: "UTC".to_owned(),
        next: None,
        upcoming: Vec::new(),
        one_off: None,
    };
    UpdatesSetup {
        db: lab.db.clone(),
        clock: lab.clock.clone(),
        data_dir: lab.dir.path().join("data"),
        releases: Releases::local(lab.server.port, Some(test_key())),
        executable: lab.dir.path().join("bin").join("observer"),
        current: "0.1.0".parse().unwrap(),
        development: false,
        supervision: lab.updates.inner.supervision.clone(),
        schedule: watch::channel(schedule).1,
        runs_executable: false,
    }
}
