//! The artifacts service over a real database and artifacts folder, with a local server in place
//! of Cfx.re and a catalog that lists what each test needs.

use std::fs;
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::time::Duration;

use jiff::SignedDuration;
use observer_artifacts::catalog::{Availability as Offered, Catalog, CatalogBuild, Listing, PolicyDate};
use observer_artifacts::samples::host_build;
use observer_artifacts::test_server::{Reply, TestServer};
use observer_artifacts::{Branch, Http, Manifest, Origin, Platform};
use observer_domain::artifacts::{
    ArtifactsStatus, BrokenReason, BuildHealth, InstallFailure, JobState, JobStep,
};
use observer_domain::audit::{ActorKind, AuditAction, AuditEntry, AuditFilter, Outcome};
use observer_domain::error::ErrorCode;
use observer_domain::schedule::{ScheduleSettings, ScheduleStatus};
use observer_store::Database;
use observer_store::artifacts::{self as stored, ArtifactState};
use serde_json::json;
use tempfile::TempDir;
use tokio::sync::watch;

use super::*;
use crate::{AuditLog, Client, PasswordCost, Running, Setup};

const COMMIT: &str = "0cfcdb5ead57a4a26fcb6ebad98adbcd0273e607";
const WAIT: Duration = Duration::from_secs(20);

struct Lab {
    dir: TempDir,
    db: Database,
    clock: Clock,
    accounts: Accounts,
    audit: AuditLog,
    servers: Servers,
    supervision: Supervision,
    _running: Running,
    owner: Caller,
    server: TestServer,
    artifacts: Artifacts,
    task: Option<ArtifactsTask>,
}

impl Lab {
    /// Serves `archive` to every download, `/slow` cut after a few bytes so that its download
    /// waits to try again, and lists the builds `listed` returns for the server.
    async fn new(archive: Vec<u8>, listed: impl FnOnce(&TestServer) -> Vec<CatalogBuild>) -> Self {
        Self::prepared(archive, listed, |_| {}).await
    }

    async fn prepared(
        archive: Vec<u8>,
        listed: impl FnOnce(&TestServer) -> Vec<CatalogBuild>,
        before: impl FnOnce(&Path),
    ) -> Self {
        let server = TestServer::start(move |request, _| {
            let reply = Reply::ok(archive.clone());
            if request.path == "/slow" {
                reply.cut_after(16)
            } else {
                reply
            }
        })
        .await;
        let dir = tempfile::tempdir().unwrap();
        before(dir.path());
        let db = Database::open(dir.path().join("observer.sqlite3"), dir.path().join("backups"))
            .await
            .unwrap();
        let clock = Clock::manual("2025-01-06T09:00:00Z".parse().unwrap());
        let accounts = Accounts::new(db.clone(), clock.clone(), PasswordCost::TEST)
            .await
            .unwrap();
        let servers = Servers::new(db.clone(), clock.clone());
        let (supervision, running) = Supervision::start(
            Setup {
                db: db.clone(),
                clock: clock.clone(),
                accounts: accounts.clone(),
                servers: servers.clone(),
                data_dir: dir.path().join("data"),
                artifacts: dir.path().join("data/artifacts"),
                console_logs: dir.path().join("data/logs/console"),
                log: None,
            },
            |_| {},
        )
        .await
        .unwrap();
        let owner = owner(&accounts).await;
        let builds = listed(&server);
        let root = dir.path().join("data/artifacts");
        let base = Base {
            db: &db,
            clock: &clock,
            accounts: &accounts,
            servers: &servers,
            supervision: &supervision,
        };
        let (artifacts, task) = start(base, &server, &root, builds).await;
        Self {
            audit: AuditLog::new(db.clone(), clock.clone()),
            dir,
            db,
            clock,
            accounts,
            servers,
            supervision,
            _running: running,
            owner,
            server,
            artifacts,
            task: Some(task),
        }
    }

    fn root(&self) -> PathBuf {
        std::path::absolute(self.dir.path().join("data/artifacts")).unwrap()
    }

    /// Stops the service and starts it again on the same database and folder, as an Observer
    /// restart does.
    async fn restart(&mut self, builds: Vec<CatalogBuild>) {
        self.task.take().unwrap().close().await;
        let base = Base {
            db: &self.db,
            clock: &self.clock,
            accounts: &self.accounts,
            servers: &self.servers,
            supervision: &self.supervision,
        };
        let (artifacts, task) = start(base, &self.server, &self.root(), builds).await;
        self.artifacts = artifacts;
        self.task = Some(task);
    }

    async fn until(&self, done: impl Fn(&ArtifactsStatus) -> bool) -> ArtifactsStatus {
        let mut status = self.artifacts.status();
        tokio::time::timeout(WAIT, status.wait_for(|status| done(status)))
            .await
            .expect("the status never got there")
            .unwrap()
            .clone()
    }

    async fn job_done(&self) -> ArtifactsStatus {
        self.until(|status| {
            status
                .job
                .as_ref()
                .is_some_and(|job| job.state != JobState::Running)
        })
        .await
    }

    async fn entries(&self, action: AuditAction) -> Vec<AuditEntry> {
        // The audit log reads up to a second before its clock.
        self.clock.advance(SignedDuration::from_secs(2));
        let filter = AuditFilter {
            action: Some(action),
            ..AuditFilter::default()
        };
        self.audit.page(filter).await.unwrap().entries
    }

    fn staging_is_empty(&self) -> bool {
        fs::read_dir(self.root().join(".staging")).map_or(true, |mut entries| entries.next().is_none())
    }
}

/// What the artifacts service stands on.
struct Base<'a> {
    db: &'a Database,
    clock: &'a Clock,
    accounts: &'a Accounts,
    servers: &'a Servers,
    supervision: &'a Supervision,
}

async fn start(
    base: Base<'_>,
    server: &TestServer,
    root: &Path,
    builds: Vec<CatalogBuild>,
) -> (Artifacts, ArtifactsTask) {
    let listing = Listing {
        platform: Platform::host(),
        builds,
        warnings: Vec::new(),
    };
    let next = ScheduleStatus {
        settings: ScheduleSettings::default(),
        version: None,
        zone: "UTC".to_owned(),
        next: None,
        upcoming: Vec::new(),
        one_off: None,
    };
    Artifacts::start(
        ArtifactsSetup {
            db: base.db.clone(),
            clock: base.clock.clone(),
            root: root.to_path_buf(),
            accounts: base.accounts.clone(),
            servers: base.servers.clone(),
            supervision: base.supervision.clone(),
            schedule: watch::channel(next).1,
            catalog: Catalog::fixed(Ok(listing)),
            http: Http::local(server.port),
        },
        |_| {},
    )
    .await
    .unwrap()
}

async fn owner(accounts: &Accounts) -> Caller {
    let client = Client::new(IpAddr::V4(Ipv4Addr::LOCALHOST), Some("test"));
    let token = accounts
        .create_setup_token(ActorKind::System)
        .await
        .unwrap()
        .unwrap();
    let signed_in = accounts
        .complete_setup(&token, "ada", "correct horse battery staple".to_owned(), &client)
        .await
        .unwrap();
    accounts
        .authenticate(&signed_in.token, &signed_in.key, client)
        .await
        .unwrap()
}

/// A downloadable build the catalog lists, served from `path`.
fn listed(server: &TestServer, build: u32, path: &str) -> CatalogBuild {
    CatalogBuild {
        build,
        commit: Some(COMMIT.parse().unwrap()),
        branch: Some(Branch::master()),
        recommended: false,
        latest: false,
        availability: Offered::Downloadable {
            url: server.url(path),
            kind: Platform::host().archive(),
            size: None,
        },
        published_at: None,
        end_of_support: None,
        end_of_life: None,
        predates_policy: false,
        listed: true,
    }
}

/// A build of the host's platform the layout check passes, with a manifest, as an install leaves.
fn place(root: &Path, build: u32, archive_sha256: &str) {
    let dir = root.join(build.to_string());
    let required = match Platform::host() {
        Platform::Windows => vec![dir.join("FXServer.exe")],
        Platform::Linux => vec![
            dir.join("run.sh"),
            dir.join("alpine/opt/cfx-server/FXServer"),
            dir.join("alpine/opt/cfx-server/ld-musl-x86_64.so.1"),
        ],
    };
    let citizen = match Platform::host() {
        Platform::Windows => dir.join("citizen"),
        Platform::Linux => dir.join("alpine/opt/cfx-server/citizen"),
    };
    fs::create_dir_all(&citizen).unwrap();
    for file in required {
        fs::write(&file, b"#!").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&file, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
    Manifest {
        format: 2,
        build,
        platform: Platform::host(),
        installed_at: "2025-01-05T09:00:00Z".parse().unwrap(),
        installed_by: None,
        files: 3,
        bytes: 1000,
        origin: Origin::Download {
            commit: COMMIT.parse().unwrap(),
            branch: Branch::master(),
            source_url: format!("https://runtime.fivem.net/artifacts/fivem/build_server_windows/master/{build}-{COMMIT}/server.zip"),
            archive_bytes: 64,
            archive_sha256: archive_sha256.to_owned(),
        },
    }
    .write(&dir)
    .unwrap();
}

#[tokio::test]
async fn an_install_reports_each_step_then_installs() {
    let lab = Lab::new(host_build(), |server| vec![listed(server, 37872, "/archive")]).await;

    let started = lab.artifacts.install(&lab.owner, 37872, false).await.unwrap();
    let done = lab.job_done().await;

    let job = done.job.unwrap();
    assert_eq!(started.job.unwrap().state, JobState::Running);
    assert_eq!(job.started_by, "ada");
    assert!(
        job.download
            .is_some_and(|download| download.received == download.total)
    );
    assert!(lab.staging_is_empty());
    let finished = lab.entries(AuditAction::BuildInstallFinished).await;
    assert_eq!(
        lab.entries(AuditAction::BuildInstallStarted).await[0].details,
        json!({ "build": 37872, "kind": "install" })
    );
    if cfg!(windows) {
        // The sample's FXServer.exe carries no signature: it fails where a real build passes.
        assert_eq!(job.state, JobState::Failed);
        assert_eq!(job.step, JobStep::Validate);
        assert_eq!(job.failure, Some(InstallFailure::Signature));
        assert_eq!(
            job.entries.map(|entries| (entries.done, entries.total)),
            Some((3, Some(3)))
        );
        assert_eq!(finished[0].outcome, Outcome::Failure);
        assert_eq!(
            finished[0].details,
            json!({ "build": 37872, "kind": "install", "step": "validate", "reason": "signature" })
        );
        assert_eq!(done.installed, []);
    } else {
        assert_eq!(job.state, JobState::Installed);
        assert_eq!(job.step, JobStep::Commit);
        assert_eq!(
            job.entries.map(|entries| (entries.done, entries.total)),
            Some((6, None))
        );
        assert_eq!(finished[0].outcome, Outcome::Success);
        assert_eq!(finished[0].details["bytes"], json!(done.installed[0].bytes));
        assert_eq!(done.installed[0].build, 37872);
        assert_eq!(done.installed[0].installed_by.as_deref(), Some("ada"));
    }
}

#[tokio::test]
async fn the_free_space_is_known_before_the_first_install() {
    let lab = Lab::new(host_build(), |_| Vec::new()).await;

    let status = lab.until(|status| status.disk.is_some()).await;

    assert!(lab.root().is_dir());
    assert!(status.disk.is_some_and(|disk| disk.free <= disk.total));
}

#[tokio::test]
async fn a_cancelled_install_leaves_nothing_in_staging() {
    let lab = Lab::new(host_build(), |server| vec![listed(server, 37872, "/slow")]).await;
    lab.artifacts.install(&lab.owner, 37872, false).await.unwrap();
    lab.until(|status| {
        status
            .job
            .as_ref()
            .is_some_and(|job| job.step == JobStep::Download)
    })
    .await;

    let cancelled = lab.artifacts.cancel_job(&lab.owner).await.unwrap();
    let done = lab.job_done().await;
    let again = lab.artifacts.cancel_job(&lab.owner).await;

    assert!(cancelled.job.unwrap().cancel_requested);
    let job = done.job.unwrap();
    assert_eq!(job.state, JobState::Cancelled);
    assert_eq!(job.failure, Some(InstallFailure::Cancelled));
    assert_eq!(again.unwrap_err().code(), Some(ErrorCode::NoJob));
    assert!(lab.staging_is_empty());
    assert!(!lab.root().join("37872").exists());
    let cancellations = lab.entries(AuditAction::BuildInstallCancelled).await;
    assert_eq!(
        cancellations[0].details,
        json!({ "build": 37872, "kind": "install", "step": "download" })
    );
    assert_eq!(cancellations[0].actor_name.as_deref(), Some("ada"));
    let finished = lab.entries(AuditAction::BuildInstallFinished).await;
    assert_eq!(finished[0].details["reason"], "cancelled");
}

#[tokio::test]
async fn a_job_cut_short_by_observer_stopping_ends_in_the_audit_log() {
    let mut lab = Lab::new(host_build(), |server| vec![listed(server, 37872, "/slow")]).await;
    lab.artifacts.install(&lab.owner, 37872, false).await.unwrap();
    lab.until(|status| {
        status
            .job
            .as_ref()
            .is_some_and(|job| job.step == JobStep::Download)
    })
    .await;

    lab.task.take().unwrap().close().await;

    assert!(lab.staging_is_empty());
    assert_eq!(lab.entries(AuditAction::BuildInstallCancelled).await.len(), 0);
    let finished = lab.entries(AuditAction::BuildInstallFinished).await;
    assert_eq!(finished.len(), 1);
    assert_eq!(finished[0].outcome, Outcome::Failure);
    assert_eq!(
        finished[0].details,
        json!({ "build": 37872, "kind": "install", "step": "download", "reason": "interrupted" })
    );
}

#[tokio::test]
async fn a_second_job_is_refused_while_one_runs() {
    let lab = Lab::new(host_build(), |server| {
        vec![listed(server, 37872, "/slow"), listed(server, 37900, "/archive")]
    })
    .await;
    lab.artifacts.install(&lab.owner, 37872, false).await.unwrap();

    let second = lab.artifacts.install(&lab.owner, 37900, false).await;
    let folder = lab.dir.path().join("fxserver");
    let import = lab
        .artifacts
        .import(&lab.owner, folder.to_str().unwrap(), 37901)
        .await;

    assert_eq!(second.unwrap_err().code(), Some(ErrorCode::JobRunning));
    assert_eq!(import.unwrap_err().code(), Some(ErrorCode::JobRunning));
    lab.artifacts.cancel_job(&lab.owner).await.unwrap();
    lab.job_done().await;
}

#[tokio::test]
async fn an_end_of_life_build_needs_the_allowance() {
    let lab = Lab::new(host_build(), |server| {
        let past = "2024-06-01T00:00:00Z".parse().unwrap();
        vec![CatalogBuild {
            end_of_life: Some(PolicyDate::At(past)),
            end_of_support: Some(PolicyDate::At(past)),
            ..listed(server, 5848, "/archive")
        }]
    })
    .await;

    let refused = lab.artifacts.install(&lab.owner, 5848, false).await;
    let allowed = lab.artifacts.install(&lab.owner, 5848, true).await;

    assert_eq!(refused.unwrap_err().code(), Some(ErrorCode::EndOfLife));
    assert!(allowed.is_ok(), "{allowed:?}");
    lab.job_done().await;
    assert_eq!(
        lab.entries(AuditAction::BuildInstallStarted).await[0].details,
        json!({ "build": 5848, "kind": "install", "allowEndOfLife": true })
    );
}

#[tokio::test]
async fn a_build_the_catalog_does_not_offer_is_refused_before_any_job() {
    let lab = Lab::new(host_build(), |server| {
        vec![CatalogBuild {
            commit: None,
            branch: None,
            availability: Offered::NoCommit,
            ..listed(server, 37739, "/archive")
        }]
    })
    .await;

    let unlisted = lab.artifacts.install(&lab.owner, 37000, false).await;
    let failed = lab.artifacts.install(&lab.owner, 37739, false).await;

    assert_eq!(unlisted.unwrap_err().code(), Some(ErrorCode::BuildNotListed));
    let Err(Error::Refused(refusal)) = failed else {
        panic!("a build only the policy names is not downloadable");
    };
    assert_eq!(refusal.code, ErrorCode::NotDownloadable);
    assert_eq!(refusal.params["reason"], "noCommit");
    assert!(lab.artifacts.status().borrow().job.is_none());
}

#[cfg(unix)]
#[tokio::test]
async fn the_first_build_installed_is_selected() {
    let lab = Lab::new(host_build(), |server| {
        vec![
            listed(server, 37872, "/archive"),
            listed(server, 37900, "/archive"),
        ]
    })
    .await;

    lab.artifacts.install(&lab.owner, 37872, false).await.unwrap();
    lab.job_done().await;
    lab.artifacts.install(&lab.owner, 37900, false).await.unwrap();
    let done = lab
        .until(|status| {
            status.installed.len() == 2
                && status
                    .job
                    .as_ref()
                    .is_some_and(|job| job.state != JobState::Running)
        })
        .await;

    assert_eq!(done.selected, Some(37872));
    let selected = lab.entries(AuditAction::BuildSelected).await;
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].actor_kind, ActorKind::System);
    assert_eq!(selected[0].details, json!({ "build": 37872, "first": true }));
}

#[tokio::test]
async fn a_fingerprint_survives_a_restart_of_observer() {
    let first = "f".repeat(64);
    let root = |dir: &Path| dir.join("data/artifacts");
    let mut lab = Lab::prepared(
        host_build(),
        |server| vec![listed(server, 37872, "/archive")],
        |dir| place(&root(dir), 37872, &"f".repeat(64)),
    )
    .await;
    assert_eq!(lab.artifacts.status().borrow().installed[0].build, 37872);
    lab.artifacts.delete(&lab.owner, 37872).await.unwrap();
    let platform = BuildPlatform::host();
    let kept = lab
        .db
        .read(move |conn| stored::fingerprint(conn, platform, 37872))
        .await
        .unwrap();
    assert_eq!(kept, Some(first));
    let builds = vec![listed(&lab.server, 37872, "/archive")];
    lab.restart(builds).await;

    lab.artifacts.install(&lab.owner, 37872, false).await.unwrap();
    let done = lab.job_done().await;

    let job = done.job.unwrap();
    assert_eq!(job.failure, Some(InstallFailure::ArchiveChanged));
    assert_eq!(job.step, JobStep::Verify);
    assert!(lab.staging_is_empty());
    let deleted = lab.entries(AuditAction::BuildDeleted).await;
    assert_eq!(deleted[0].details, json!({ "build": 37872, "bytes": 1000 }));
}

#[tokio::test]
async fn a_build_from_an_archive_other_than_the_first_one_is_broken_and_not_selected() {
    let mut lab = Lab::prepared(
        host_build(),
        |_| Vec::new(),
        |dir| place(&dir.join("data/artifacts"), 37872, &"a".repeat(64)),
    )
    .await;
    let (platform, now) = (BuildPlatform::host(), lab.clock.now());
    lab.db
        .write(move |tx| {
            tx.execute("DELETE FROM artifact_fingerprints", [])?;
            stored::remember_fingerprint(tx, platform, 37872, &"b".repeat(64), now)
        })
        .await
        .unwrap();
    lab.restart(Vec::new()).await;

    let status = lab.artifacts.status().borrow().clone();
    let selected = lab
        .artifacts
        .select(
            &lab.owner,
            SelectBuild {
                build: 37872,
                apply: ApplyMode::NextStart,
                confirm_kvp_rollback: false,
                allow_end_of_life: false,
            },
            "PUT",
            "/api/servers/default/artifacts/selection",
        )
        .await;

    assert_eq!(
        status.installed[0].health,
        BuildHealth::Broken {
            reason: BrokenReason::ArchiveChanged
        }
    );
    let Err(Error::Refused(refusal)) = selected else {
        panic!("a build from another archive was selected: {selected:?}");
    };
    assert_eq!(refusal.code, ErrorCode::BuildBroken);
    assert_eq!(refusal.params["reason"], json!("archiveChanged"));
    let kept = lab
        .db
        .read(move |conn| stored::fingerprint(conn, platform, 37872))
        .await
        .unwrap();
    assert_eq!(kept, Some("b".repeat(64)), "the first fingerprint stays");
}

#[tokio::test]
async fn a_legacy_folder_inside_the_artifacts_folder_is_adopted() {
    let lab = Lab::prepared(
        host_build(),
        |_| Vec::new(),
        |dir| {
            place(&dir.join("data/artifacts"), 35245, &"a".repeat(64));
        },
    )
    .await;
    let legacy = lab.root().join("35245").to_str().unwrap().to_owned();
    let now = lab.clock.now();
    lab.db
        .write(move |tx| {
            let state = ArtifactState {
                legacy_build_path: Some(legacy),
                ..ArtifactState::default()
            };
            stored::save_state(tx, Servers::DEFAULT, &state, now)
        })
        .await
        .unwrap();
    let mut lab = lab;

    lab.restart(Vec::new()).await;

    let status = lab.artifacts.status().borrow().clone();
    assert_eq!(status.selected, Some(35245));
    assert_eq!(status.last_healthy.map(|healthy| healthy.build), Some(35245));
    assert_eq!(
        status.highest_run,
        Some(35245),
        "it ran before, as the KVP guard must know"
    );
    assert!(!status.legacy_folder);
    let adopted = lab.entries(AuditAction::BuildSelected).await;
    assert_eq!(adopted[0].details, json!({ "build": 35245, "adopted": true }));
    assert_eq!(adopted[0].actor_kind, ActorKind::System);
}

#[tokio::test]
async fn a_legacy_folder_elsewhere_stays_until_the_owner_imports_it() {
    let lab = Lab::new(host_build(), |_| Vec::new()).await;
    let elsewhere = lab.dir.path().join("fxserver").to_str().unwrap().to_owned();
    let now = lab.clock.now();
    lab.db
        .write(move |tx| {
            let state = ArtifactState {
                legacy_build_path: Some(elsewhere),
                ..ArtifactState::default()
            };
            stored::save_state(tx, Servers::DEFAULT, &state, now)
        })
        .await
        .unwrap();
    let mut lab = lab;

    lab.restart(Vec::new()).await;

    let status = lab.artifacts.status().borrow().clone();
    assert_eq!(status.selected, None);
    assert!(status.legacy_folder);
}

/// Points the stored legacy folder at `folder`, as the migration of an older Observer leaves it.
async fn legacy(lab: &Lab, folder: &Path) {
    let legacy = folder.to_str().unwrap().to_owned();
    let now = lab.clock.now();
    lab.db
        .write(move |tx| {
            let mut state = stored::state(tx, Servers::DEFAULT)?;
            state.legacy_build_path = Some(legacy);
            stored::save_state(tx, Servers::DEFAULT, &state, now)
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn a_legacy_folder_is_forgotten_once_a_build_is_selected() {
    let mut lab = Lab::prepared(
        host_build(),
        |_| Vec::new(),
        |dir| place(&dir.join("data/artifacts"), 35245, &"a".repeat(64)),
    )
    .await;
    legacy(&lab, &lab.dir.path().join("fxserver")).await;
    lab.restart(Vec::new()).await;
    assert!(lab.artifacts.status().borrow().legacy_folder);

    let selected = lab
        .artifacts
        .select(
            &lab.owner,
            SelectBuild {
                build: 35245,
                apply: ApplyMode::NextStart,
                confirm_kvp_rollback: false,
                allow_end_of_life: false,
            },
            "PUT",
            "/api/servers/default/artifacts/selection",
        )
        .await
        .unwrap();

    assert_eq!(selected.selected, Some(35245));
    assert!(!selected.legacy_folder);
    let state = lab
        .db
        .read(|conn| stored::state(conn, Servers::DEFAULT))
        .await
        .unwrap();
    assert_eq!(state.legacy_build_path, None);
}

#[cfg(unix)]
#[tokio::test]
async fn a_legacy_folder_is_forgotten_once_it_is_imported() {
    let mut lab = Lab::prepared(
        host_build(),
        |_| Vec::new(),
        |dir| {
            place(&dir.join("data/artifacts"), 35245, &"a".repeat(64));
            place(dir, 37000, &"b".repeat(64));
        },
    )
    .await;
    let folder = lab.dir.path().join("37000");
    legacy(&lab, &folder).await;
    let now = lab.clock.now();
    lab.db
        .write(move |tx| {
            let mut state = stored::state(tx, Servers::DEFAULT)?;
            state.selected = Some(35245);
            stored::save_state(tx, Servers::DEFAULT, &state, now)
        })
        .await
        .unwrap();
    lab.restart(Vec::new()).await;
    assert!(lab.artifacts.status().borrow().legacy_folder);

    lab.artifacts
        .import(&lab.owner, folder.to_str().unwrap(), 37000)
        .await
        .unwrap();
    let done = lab.job_done().await;

    assert_eq!(done.job.unwrap().state, JobState::Installed);
    assert_eq!(done.selected, Some(35245));
    assert!(!done.legacy_folder);
}

#[tokio::test]
async fn a_pending_build_deleted_since_is_dropped_at_the_scheduled_restart_with_a_notice() {
    let lab = Lab::new(host_build(), |_| Vec::new()).await;
    let now = lab.clock.now();
    lab.db
        .write(move |tx| {
            let state = ArtifactState {
                selected: Some(35245),
                pending: Some(PendingBuild {
                    build: 37950,
                    by: "ada".to_owned(),
                    at: now,
                }),
                ..ArtifactState::default()
            };
            stored::save_state(tx, Servers::DEFAULT, &state, now)
        })
        .await
        .unwrap();
    let root = lab.root();

    let promotion = lab
        .db
        .write(move |tx| promote_pending(tx, &root, now))
        .await
        .unwrap();

    assert_eq!(promotion, Some(Promotion::Dropped { build: 37950 }));
    let state = lab
        .db
        .read(|conn| stored::state(conn, Servers::DEFAULT))
        .await
        .unwrap();
    assert_eq!(state.selected, Some(35245));
    assert_eq!(state.pending, None);
    let notice = state.notice.unwrap();
    assert_eq!(notice.kind, NoticeKind::PendingDropped);
    assert_eq!(notice.reason, NoticeReason::NotInstalled);
    let dropped = lab.entries(AuditAction::PendingBuildDropped).await;
    assert_eq!(dropped[0].outcome, Outcome::Failure);
    assert_eq!(
        dropped[0].details,
        json!({ "build": 37950, "reason": "notInstalled" })
    );
}

#[tokio::test]
async fn a_protected_build_is_not_deleted_and_says_why() {
    let lab = Lab::prepared(
        host_build(),
        |_| Vec::new(),
        |dir| place(&dir.join("data/artifacts"), 35245, &"a".repeat(64)),
    )
    .await;
    let selection = SelectBuild {
        build: 35245,
        apply: ApplyMode::NextStart,
        confirm_kvp_rollback: false,
        allow_end_of_life: false,
    };
    lab.artifacts
        .select(
            &lab.owner,
            selection,
            "PUT",
            "/api/servers/default/artifacts/selection",
        )
        .await
        .unwrap();

    let refused = lab.artifacts.delete(&lab.owner, 35245).await;
    let missing = lab.artifacts.delete(&lab.owner, 37000).await;

    let Err(Error::Refused(refusal)) = refused else {
        panic!("the selected build was deleted");
    };
    assert_eq!(refusal.code, ErrorCode::BuildProtected);
    assert_eq!(refusal.params["reasons"], json!(["selected"]));
    assert_eq!(missing.unwrap_err().code(), Some(ErrorCode::BuildNotInstalled));
    assert!(lab.root().join("35245").exists());
}

#[tokio::test]
async fn a_cleanup_removes_what_its_preview_listed_and_refuses_another_list() {
    let lab = Lab::prepared(
        host_build(),
        |_| Vec::new(),
        |dir| {
            for build in [37000, 37100, 37200] {
                place(&dir.join("data/artifacts"), build, &"a".repeat(64));
            }
        },
    )
    .await;
    let settings = ArtifactSettingsChange {
        settings: ArtifactSettings {
            keep_builds: 1,
            ..ArtifactSettings::default()
        },
        version: None,
    };
    lab.artifacts.change_settings(&lab.owner, settings).await.unwrap();

    let preview = lab.artifacts.cleanup_preview().await.unwrap();
    let conflict = lab.artifacts.cleanup(&lab.owner, vec![37000]).await;
    let result = lab
        .artifacts
        .cleanup(&lab.owner, vec![37000, 37100])
        .await
        .unwrap();

    // Installed the same day, so the highest number stays.
    assert_eq!(
        preview.remove.iter().map(|build| build.build).collect::<Vec<_>>(),
        [37100, 37000]
    );
    assert_eq!(preview.freed, 2000);
    assert_eq!(conflict.unwrap_err().code(), Some(ErrorCode::Conflict));
    assert_eq!(result.deleted, [37100, 37000]);
    assert_eq!(result.freed, 2000);
    assert!(lab.root().join("37200").exists());
    let deleted = lab.entries(AuditAction::BuildDeleted).await;
    assert_eq!(deleted.len(), 2);
    assert_eq!(deleted[0].details["cleanup"], json!(true));
    let changed = lab.entries(AuditAction::ArtifactSettingsChanged).await;
    assert_eq!(
        changed[0].details,
        json!({ "keepBuilds": { "before": 3, "after": 1 } })
    );
}

#[tokio::test]
async fn settings_are_saved_once_checked_and_from_the_version_they_were_made_from() {
    let lab = Lab::new(host_build(), |_| Vec::new()).await;
    let change = |keep_builds, version| ArtifactSettingsChange {
        settings: ArtifactSettings {
            keep_builds,
            ..ArtifactSettings::default()
        },
        version,
    };

    let invalid = lab.artifacts.change_settings(&lab.owner, change(21, None)).await;
    let saved = lab
        .artifacts
        .change_settings(&lab.owner, change(5, None))
        .await
        .unwrap();
    let stale = lab.artifacts.change_settings(&lab.owner, change(6, None)).await;
    let same = lab
        .artifacts
        .change_settings(&lab.owner, change(5, Some(1)))
        .await
        .unwrap();

    assert_eq!(
        invalid.unwrap_err().code(),
        Some(ErrorCode::InvalidArtifactSettings)
    );
    assert_eq!((saved.settings.keep_builds, saved.version), (5, Some(1)));
    assert_eq!(stale.unwrap_err().code(), Some(ErrorCode::Conflict));
    assert_eq!(
        same.version,
        Some(1),
        "saving the same settings again changes nothing"
    );
    assert_eq!(lab.entries(AuditAction::ArtifactSettingsChanged).await.len(), 1);
}

#[test]
fn the_runtime_paths_of_both_crates_match() {
    assert_eq!(
        observer_artifacts::RUNTIME_PATHS,
        observer_fxserver::layout::RUNTIME_PATHS
    );
}

#[test]
fn settings_name_each_branch_they_refuse() {
    let settings = ArtifactSettings {
        keep_builds: 0,
        auto_rollback: true,
        branches: vec![
            "feature/network-improvements-clean".to_owned(),
            "master".to_owned(),
            "Feature X".to_owned(),
            "feature/network-improvements-clean".to_owned(),
        ],
    };

    let issues: Vec<(ArtifactSettingsField, Option<usize>, ArtifactSettingsProblem)> =
        settings_issues(&settings)
            .into_iter()
            .map(|issue| (issue.field, issue.index, issue.problem))
            .collect();

    assert_eq!(
        issues,
        [
            (
                ArtifactSettingsField::KeepBuilds,
                None,
                ArtifactSettingsProblem::OutOfRange
            ),
            (
                ArtifactSettingsField::Branches,
                Some(1),
                ArtifactSettingsProblem::Master
            ),
            (
                ArtifactSettingsField::Branches,
                Some(2),
                ArtifactSettingsProblem::Invalid
            ),
            (
                ArtifactSettingsField::Branches,
                Some(3),
                ArtifactSettingsProblem::Duplicate
            ),
        ]
    );
    let eleven = ArtifactSettings {
        branches: (0..11).map(|index| format!("feature/b{index}")).collect(),
        ..ArtifactSettings::default()
    };
    assert_eq!(
        settings_issues(&eleven)[0].problem,
        ArtifactSettingsProblem::TooMany
    );
    assert_eq!(settings_issues(&ArtifactSettings::default()), []);
}
