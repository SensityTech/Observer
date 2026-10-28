//! Builds switched under a running server, against the fake installed as builds of their own in
//! Observer's artifacts folder: a switch at the next start and its probation, a build that crashes
//! and the return to the last healthy one, a build waiting for the scheduled restart, and the
//! process check that keeps a build in use.
// The helpers below are test code too, though not inside a `#[test]` function.
#![allow(clippy::unwrap_used)]

use std::fs;
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, PoisonError};
use std::time::Duration;

use jiff::{SignedDuration, Timestamp, Unit};
use observer_app::{
    Accounts, ArtifactTimings, Artifacts, ArtifactsSetup, ArtifactsTask, AuditLog, Caller, Client, Clock,
    PasswordCost, Running, Schedule, ScheduleSetup, ScheduleTask, Servers, Setup, Supervision,
};
use observer_artifacts::catalog::{Catalog, Listing};
use observer_artifacts::{Branch, Http, Manifest, Origin, Platform};
use observer_domain::artifacts::{
    ApplyMode, ArtifactsStatus, HealthyBuild, NoticeKind, NoticeReason, SelectBuild,
};
use observer_domain::audit::{ActorKind, AuditAction, AuditEntry, AuditFilter};
use observer_domain::error::ErrorCode;
use observer_domain::launch::{ExtraArgument, LaunchSettings, OneSync};
use observer_domain::schedule::OneOffRequest;
use observer_domain::server::{ServerState, ServerStatus};
use observer_fxserver::health::HealthLimits;
use observer_fxserver::recovery::RecoveryPolicy;
use observer_fxserver::supervisor::StopTimings;
use observer_store::Database;
use observer_store::artifacts::{self as stored, ArtifactState};
use serde_json::json;
use tempfile::TempDir;
use tokio::time::timeout;

const WAIT: Duration = Duration::from_secs(30);
const HEALTHY: u32 = 35245;
const NEW: u32 = 37900;
const SELECTION_ROUTE: (&str, &str) = ("PUT", "/api/servers/default/artifacts/selection");

/// Ports of their own, apart from the other test binaries'.
static PORTS: LazyLock<Mutex<Vec<u16>>> = LazyLock::new(|| Mutex::new((41960..=41962).collect()));

struct Port(u16);

impl Port {
    async fn lease() -> Self {
        loop {
            if let Some(port) = PORTS.lock().unwrap_or_else(PoisonError::into_inner).pop() {
                return Self(port);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

impl Drop for Port {
    fn drop(&mut self) {
        PORTS.lock().unwrap_or_else(PoisonError::into_inner).push(self.0);
    }
}

struct Lab {
    dir: TempDir,
    _port: Port,
    /// The schedule's clock, which a test moves on rather than wait.
    clock: Clock,
    audit: AuditLog,
    supervision: Supervision,
    running: Option<Running>,
    schedule: Schedule,
    scheduling: Option<ScheduleTask>,
    artifacts: Artifacts,
    building: Option<ArtifactsTask>,
    owner: Caller,
}

impl Lab {
    /// An Observer whose owner `ada` saved launch settings for the fake, with 35245 selected and
    /// healthy, and 37900 installed beside it doing what `new_build` says.
    async fn new(stable_for: Duration, new_build: &str) -> Self {
        let port = Port::lease().await;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("observer-data/artifacts");
        install_fake(&root, HEALTHY, "");
        install_fake(&root, NEW, &format!("set fake_build {NEW}\n{new_build}"));
        let data = dir.path().join("server-data");
        fs::create_dir_all(&data).unwrap();
        fs::write(
            data.join("server.cfg"),
            format!(
                "endpoint_add_tcp \"127.0.0.1:{0}\"\nendpoint_add_udp \"127.0.0.1:{0}\"\nsv_maxclients 8\n",
                port.0
            ),
        )
        .unwrap();

        let db = Database::open(dir.path().join("observer.sqlite3"), dir.path().join("backups"))
            .await
            .unwrap();
        let system = Clock::system();
        let accounts = Accounts::new(db.clone(), system.clone(), PasswordCost::TEST)
            .await
            .unwrap();
        let servers = Servers::new(db.clone(), system.clone());
        let owner = owner(&accounts).await;
        seed(&db, &servers, &owner, &data).await;
        let (supervision, running) = supervise(&db, &accounts, &servers, dir.path()).await;
        // On a whole second, as a one-off planned in minutes is.
        let clock = Clock::manual(Timestamp::now().round(Unit::Second).unwrap());
        let (schedule, scheduling) = Schedule::start(
            ScheduleSetup {
                db: db.clone(),
                clock: clock.clone(),
                supervision: supervision.clone(),
            },
            |timings| timings.longest_sleep = Duration::from_millis(50),
        )
        .await
        .unwrap();
        let listing = Listing {
            platform: Platform::host(),
            builds: Vec::new(),
            warnings: Vec::new(),
        };
        let (artifacts, building) = Artifacts::start(
            ArtifactsSetup {
                db: db.clone(),
                clock: system.clone(),
                root,
                accounts,
                servers,
                supervision: supervision.clone(),
                schedule: schedule.status(),
                catalog: Catalog::fixed(Ok(listing)),
                http: Http::new().unwrap(),
            },
            |timings: &mut ArtifactTimings| timings.stable_for = stable_for,
        )
        .await
        .unwrap();
        Self {
            audit: AuditLog::new(db, system),
            dir,
            _port: port,
            clock,
            supervision,
            running: Some(running),
            schedule,
            scheduling: Some(scheduling),
            artifacts,
            building: Some(building),
            owner,
        }
    }

    fn root(&self) -> PathBuf {
        self.dir.path().join("observer-data/artifacts")
    }

    async fn until(&self, wanted: impl Fn(&ServerStatus) -> bool) -> ServerStatus {
        let mut status = self.supervision.status();
        let found = timeout(WAIT, status.wait_for(|status| wanted(status)))
            .await
            .map(|found| found.unwrap().clone());
        found.unwrap_or_else(|_| panic!("the status never came; last {:#?}", status.borrow()))
    }

    async fn builds_until(&self, wanted: impl Fn(&ArtifactsStatus) -> bool) -> ArtifactsStatus {
        let mut status = self.artifacts.status();
        let found = timeout(WAIT, status.wait_for(|status| wanted(status)))
            .await
            .map(|found| found.unwrap().clone());
        found.unwrap_or_else(|_| panic!("the builds never got there; last {:#?}", status.borrow()))
    }

    /// The run of `build` as `/info.json` reports it, online.
    async fn online_on(&self, build: u32) -> ServerStatus {
        self.until(|status| status.state == ServerState::Online && status.build == Some(build))
            .await
    }

    async fn select(&self, build: u32, apply: ApplyMode) -> ArtifactsStatus {
        let request = SelectBuild {
            build,
            apply,
            confirm_kvp_rollback: false,
            allow_end_of_life: false,
        };
        self.artifacts
            .select(&self.owner, request, SELECTION_ROUTE.0, SELECTION_ROUTE.1)
            .await
            .unwrap()
    }

    async fn audited(&self, action: AuditAction) -> Vec<AuditEntry> {
        // The audit log reads up to a second before the present.
        tokio::time::sleep(Duration::from_millis(1100)).await;
        let filter = AuditFilter {
            action: Some(action),
            ..AuditFilter::default()
        };
        self.audit.page(filter).await.unwrap().entries
    }

    async fn stop(&mut self) {
        self.building.take().unwrap().close().await;
        self.scheduling.take().unwrap().close().await;
        self.supervision.shutdown(false).await;
        self.running.take().unwrap().close().await;
    }
}

/// Launch settings for the fake, saved by the owner, and 35245 selected and healthy, as an Observer
/// that ran it before has them.
async fn seed(db: &Database, servers: &Servers, owner: &Caller, data: &Path) {
    let launch = LaunchSettings {
        data_path: data.to_str().unwrap().to_owned(),
        cfg_file: "server.cfg".to_owned(),
        onesync: OneSync::On,
        auto_start: false,
        txadmin_events: false,
        extra_arguments: vec![ExtraArgument {
            name: "fake_heartbeat_ms".to_owned(),
            value: "200".to_owned(),
        }],
    };
    servers
        .change_launch_settings(owner, Servers::DEFAULT, launch, None)
        .await
        .unwrap();
    let now = Timestamp::now();
    db.write(move |tx| {
        let state = ArtifactState {
            selected: Some(HEALTHY),
            last_healthy: Some(HealthyBuild {
                build: HEALTHY,
                since: now,
            }),
            ..ArtifactState::default()
        };
        stored::save_state(tx, Servers::DEFAULT, &state, now)
    })
    .await
    .unwrap();
}

/// The supervision of the default server, with timings short enough for tests.
async fn supervise(
    db: &Database,
    accounts: &Accounts,
    servers: &Servers,
    dir: &Path,
) -> (Supervision, Running) {
    Supervision::start(
        Setup {
            db: db.clone(),
            clock: Clock::system(),
            accounts: accounts.clone(),
            servers: servers.clone(),
            data_dir: dir.join("observer-data"),
            artifacts: dir.join("observer-data/artifacts"),
            console_logs: dir.join("observer-data/logs/console"),
            log: None,
        },
        |settings| {
            settings.health = HealthLimits {
                boot_grace: Duration::from_secs(2),
                heartbeat_late: Duration::from_secs(1),
                heartbeat_fatal: Duration::from_secs(2),
                http_late: Duration::from_secs(2),
                http_fatal: Duration::from_secs(5),
            };
            settings.stop = StopTimings {
                notice: Duration::from_millis(300),
                quit: Duration::from_secs(3),
                shortened: Duration::from_secs(1),
                fatal_exit: Duration::from_secs(1),
            };
            settings.recovery = RecoveryPolicy {
                backoff: vec![Duration::from_millis(500)],
                backoff_cap: Duration::from_millis(500),
                ..RecoveryPolicy::default()
            };
        },
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

/// Where the fake goes in a build: FXServer.exe on Windows, the musl loader FXServer starts from
/// on Linux.
fn fake_program(build: &Path) -> PathBuf {
    if cfg!(windows) {
        build.join("FXServer.exe")
    } else {
        build.join("alpine/opt/cfx-server/ld-musl-x86_64.so.1")
    }
}

/// The fake as build `build` of the artifacts folder: the layout the host checks, a manifest, and
/// `fake-build.cfg` next to the program with what this build does.
fn install_fake(root: &Path, build: u32, cfg: &str) {
    let dir = root.join(build.to_string());
    let program = fake_program(&dir);
    fs::create_dir_all(program.parent().unwrap().join("citizen")).unwrap();
    fs::copy(env!("CARGO_BIN_EXE_fake-fxserver"), &program).unwrap();
    if cfg!(unix) {
        for file in [dir.join("run.sh"), dir.join("alpine/opt/cfx-server/FXServer")] {
            fs::write(&file, b"#!/bin/sh\n").unwrap();
            executable(&file);
        }
    }
    fs::write(program.parent().unwrap().join("fake-build.cfg"), cfg).unwrap();
    Manifest {
        format: 2,
        build,
        platform: Platform::host(),
        installed_at: Timestamp::now(),
        installed_by: Some("ada".to_owned()),
        files: 3,
        bytes: 1000,
        origin: Origin::Download {
            commit: "0cfcdb5ead57a4a26fcb6ebad98adbcd0273e607".parse().unwrap(),
            branch: Branch::master(),
            source_url: "https://runtime.fivem.net/artifacts/fivem/build_server_windows/master/".to_owned(),
            archive_bytes: 64,
            archive_sha256: "a".repeat(64),
        },
    }
    .write(&dir)
    .unwrap();
}

#[cfg(unix)]
fn executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(not(unix))]
fn executable(_path: &Path) {}

#[tokio::test]
async fn a_switch_at_the_next_start_runs_the_new_build_and_passes_probation() {
    let mut lab = Lab::new(Duration::from_secs(2), "").await;
    lab.supervision.start_server(&lab.owner).await.unwrap();
    lab.online_on(HEALTHY).await;

    let selected = lab.select(NEW, ApplyMode::NextStart).await;
    let still = lab.supervision.status().borrow().build;
    lab.supervision.restart_server(&lab.owner).await.unwrap();
    lab.online_on(NEW).await;
    let on_probation = lab
        .builds_until(|status| {
            status
                .probation
                .is_some_and(|probation| probation.ready_at.is_some())
        })
        .await;
    let passed = lab
        .builds_until(|status| status.last_healthy.is_some_and(|healthy| healthy.build == NEW))
        .await;

    assert_eq!((selected.selected, selected.running), (Some(NEW), Some(HEALTHY)));
    assert_eq!(
        still,
        Some(HEALTHY),
        "a selection never changes the build that runs"
    );
    let probation = on_probation.probation.unwrap();
    assert_eq!((probation.build, probation.from), (NEW, Some(HEALTHY)));
    assert!(probation.passes_at > probation.ready_at);
    assert_eq!(passed.probation, None);
    assert_eq!(passed.running, Some(NEW));
    assert_eq!(passed.highest_run, Some(NEW));
    let verdicts = lab.audited(AuditAction::ProbationPassed).await;
    assert_eq!(verdicts.len(), 1);
    assert_eq!(verdicts[0].actor_kind, ActorKind::System);
    assert_eq!(verdicts[0].details, json!({ "build": NEW }));
    let selections = lab.audited(AuditAction::BuildSelected).await;
    assert_eq!(
        selections[0].details,
        json!({ "build": NEW, "from": HEALTHY, "apply": "nextStart" })
    );
    lab.stop().await;
}

#[tokio::test]
async fn a_build_that_crashes_in_probation_is_rolled_back_and_the_last_healthy_one_starts() {
    let mut lab = Lab::new(Duration::from_mins(5), "set fake_crash_after_ms 2500").await;
    lab.supervision.start_server(&lab.owner).await.unwrap();
    lab.online_on(HEALTHY).await;

    lab.select(NEW, ApplyMode::Now).await;
    lab.online_on(NEW).await;
    let back = lab
        .builds_until(|status| status.notice.is_some() && status.running == Some(HEALTHY))
        .await;
    let online = lab.online_on(HEALTHY).await;

    assert_eq!(back.selected, Some(HEALTHY));
    assert_eq!(back.probation, None);
    assert_eq!(back.last_healthy.map(|healthy| healthy.build), Some(HEALTHY));
    let notice = back.notice.unwrap();
    assert_eq!(
        (notice.kind, notice.build, notice.other, notice.reason),
        (NoticeKind::RolledBack, NEW, Some(HEALTHY), NoticeReason::Crashed)
    );
    assert_eq!(online.state, ServerState::Online);
    let rollbacks = lab.audited(AuditAction::BuildRolledBack).await;
    assert_eq!(rollbacks.len(), 1);
    assert_eq!(
        rollbacks[0].details,
        json!({ "from": NEW, "to": HEALTHY, "reason": "crashed" })
    );
    assert!(
        lab.root().join(NEW.to_string()).exists(),
        "a failed build is never deleted"
    );
    lab.stop().await;
}

#[tokio::test]
async fn a_pending_build_is_applied_at_the_scheduled_restart_only() {
    let mut lab = Lab::new(Duration::from_mins(5), "").await;
    lab.supervision.start_server(&lab.owner).await.unwrap();
    lab.online_on(HEALTHY).await;
    let one_off = OneOffRequest {
        in_minutes: Some(2),
        at: None,
    };
    lab.schedule
        .restart_once(&lab.owner, one_off.clone())
        .await
        .unwrap();

    let pending = lab.select(NEW, ApplyMode::ScheduledRestart).await;
    lab.supervision.restart_server(&lab.owner).await.unwrap();
    let by_hand = lab
        .until(|status| {
            status.generation == 2 && status.state == ServerState::Online && status.build.is_some()
        })
        .await;
    lab.schedule.restart_once(&lab.owner, one_off).await.unwrap();
    lab.clock.advance(SignedDuration::from_mins(1));
    tokio::time::sleep(Duration::from_millis(300)).await;
    lab.clock.advance(SignedDuration::from_mins(1));
    let scheduled = lab
        .until(|status| {
            status.generation == 3 && status.state == ServerState::Online && status.build.is_some()
        })
        .await;
    let applied = lab.builds_until(|status| status.pending.is_none()).await;

    assert_eq!(pending.pending.as_ref().map(|pending| pending.build), Some(NEW));
    assert_eq!(pending.selected, Some(HEALTHY));
    assert_eq!(
        by_hand.build,
        Some(HEALTHY),
        "a restart by hand keeps the selected build"
    );
    assert_eq!(scheduled.build, Some(NEW));
    assert_eq!(applied.selected, Some(NEW));
    let entries = lab.audited(AuditAction::PendingBuildApplied).await;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].actor_kind, ActorKind::System);
    assert_eq!(entries[0].details["build"], json!(NEW));
    assert_eq!(entries[0].details["from"], json!(HEALTHY));
    let set = lab.audited(AuditAction::PendingBuildSet).await;
    assert_eq!(set[0].details, json!({ "build": NEW }));
    lab.stop().await;
}

#[tokio::test]
async fn a_build_a_process_runs_from_is_not_deleted() {
    let mut lab = Lab::new(Duration::from_mins(5), "").await;
    install_fake(&lab.root(), 37800, "");
    let program = fake_program(&lab.root().join("37800"));
    let spawned = observer_platform::spawn(&observer_platform::Launch {
        program,
        args: Vec::new(),
        cwd: lab.dir.path().to_path_buf(),
    })
    .unwrap();

    // The catalog's rescan lets the service see the build the test installed after its start.
    let _ = lab.artifacts.catalog(false).await;
    let in_use = lab.artifacts.delete(&lab.owner, 37800).await.unwrap_err().code();
    spawned.tree.kill().unwrap();
    let mut exit = spawned.exit;
    exit.wait().await;
    assert!(spawned.tree.wait_empty(WAIT).await.unwrap());
    let deleted = lab.artifacts.delete(&lab.owner, 37800).await.unwrap();

    assert_eq!(in_use, Some(ErrorCode::BuildInUse));
    assert!(deleted.installed.iter().all(|build| build.build != 37800));
    assert!(!lab.root().join("37800").exists());
    lab.stop().await;
}
