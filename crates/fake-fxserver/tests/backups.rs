//! The backups service against `fake-dump`, copied in as `mariadb-dump` and `mariadb`: the runs and
//! their records, the restart that waits for its dump and the one that does not wait past its
//! limit, the retention, the downloads, and the secrets that never come back out.
// The helpers below are test code too, though not inside a `#[test]` function.
#![allow(clippy::unwrap_used)]

use std::fs;
use std::io::Read;
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use jiff::{SignedDuration, Timestamp};
use observer_app::{
    Accounts, AuditLog, Backups, BackupsHook, BackupsSetup, BackupsTask, BeforeSpawn, Caller, Client, Clock,
    PasswordCost, Servers, ToolsHook,
};
use observer_domain::audit::{ActorKind, AuditAction, AuditFilter};
use observer_domain::backups::{
    BackupError, BackupOutcome, BackupRun, BackupSettings, BackupSettingsChange, BackupTrigger,
    BeforeRestarts, CopyState, DatabaseEngine, DeleteReason, DestinationDraft, DestinationTarget,
    ManualSource, Retention, Secret, SourceKind,
};
use observer_domain::error::ErrorCode;
use observer_domain::launch::{LaunchSettings, OneSync};
use observer_fxserver::cfg;
use observer_fxserver::launch::ServerLaunch;
use observer_fxserver::layout::{Flavour, Layout};
use observer_fxserver::pre_launch::{PreLaunch, PreLaunchRequest};
use observer_fxserver::server_data::Resources;
use observer_fxserver::supervisor::Cause;
use observer_store::Database;
use observer_store::backups::{self as store, NewRun};
use tempfile::TempDir;
use tokio::io::AsyncReadExt;

const PASSWORD: &str = "hunter2-db";

struct Lab {
    dir: TempDir,
    audit: AuditLog,
    owner: Caller,
    backups: Backups,
    task: Option<BackupsTask>,
}

impl Lab {
    async fn new() -> Self {
        Self::with(|_| {}, Clock::system()).await
    }

    /// A server folder whose cfg sets the connection string of `es_extended`, the fake tools in
    /// their folder, and the settings pointing at them.
    async fn with(prepare: impl FnOnce(&Path), clock: Clock) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("server-data");
        fs::create_dir_all(data.join("resources")).unwrap();
        cfg(&data, "es_extended");
        let bin = dir.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        for name in ["mariadb-dump", "mariadb"] {
            fs::copy(
                env!("CARGO_BIN_EXE_fake-dump"),
                bin.join(format!("{name}{}", std::env::consts::EXE_SUFFIX)),
            )
            .unwrap();
        }
        prepare(dir.path());
        let db = Database::open(
            dir.path().join("observer.sqlite3"),
            dir.path().join("sqlite-backups"),
        )
        .await
        .unwrap();
        let accounts = Accounts::new(db.clone(), clock.clone(), PasswordCost::TEST)
            .await
            .unwrap();
        let owner = owner(&accounts).await;
        let servers = Servers::new(db.clone(), clock.clone());
        servers
            .change_launch_settings(&owner, Servers::DEFAULT, launch(&data), None)
            .await
            .unwrap();
        let (backups, task) = Backups::start(
            BackupsSetup {
                db: db.clone(),
                clock: clock.clone(),
                data_dir: dir.path().join("observer-data"),
                schedule: None,
            },
            |timings| timings.longest_sleep = Duration::from_millis(50),
        )
        .await
        .unwrap();
        let lab = Self {
            dir,
            audit: AuditLog::new(db, clock),
            owner,
            backups,
            task: Some(task),
        };
        lab.settings(|settings| {
            settings.client_folder = Some(bin.to_string_lossy().into_owned());
        })
        .await;
        lab
    }

    fn data(&self) -> PathBuf {
        self.dir.path().join("server-data")
    }

    fn copies(&self) -> PathBuf {
        self.dir.path().join("copies")
    }

    fn work(&self) -> PathBuf {
        self.dir
            .path()
            .join("observer-data")
            .join("database-backups")
            .join(".work")
    }

    async fn settings(&self, change: impl FnOnce(&mut BackupSettings)) {
        let view = self.backups.view().await.unwrap();
        let mut settings = view.settings;
        change(&mut settings);
        self.backups
            .change_settings(
                &self.owner,
                BackupSettingsChange {
                    settings,
                    version: view.version,
                    password: None,
                },
            )
            .await
            .unwrap();
    }

    async fn local(&self, retention: Retention) -> i64 {
        self.backups
            .create_destination(
                &self.owner,
                DestinationDraft {
                    name: "This host".to_owned(),
                    enabled: true,
                    target: DestinationTarget::Local {
                        folder: self.copies().to_string_lossy().into_owned(),
                    },
                    retention,
                    secret: None,
                },
            )
            .await
            .unwrap()
            .id
    }

    /// The run once it ended, and once the service took the next one.
    async fn ended(&self, id: i64) -> BackupRun {
        let started = Instant::now();
        loop {
            let run = self.backups.run(id).await.unwrap();
            let idle = self.backups.view().await.unwrap().running.is_none();
            if run.finished_at.is_some() && idle {
                return run;
            }
            assert!(
                started.elapsed() < Duration::from_secs(30),
                "run {id} did not end"
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    async fn backup(&self) -> BackupRun {
        let run = self.backups.run_now(&self.owner).await.unwrap();
        self.ended(run.id).await
    }

    async fn audited(&self, action: AuditAction) -> Vec<observer_domain::audit::AuditEntry> {
        self.audit
            .page(AuditFilter {
                action: Some(action),
                ..AuditFilter::default()
            })
            .await
            .unwrap()
            .entries
    }

    async fn close(mut self) {
        self.task.take().unwrap().close().await;
    }
}

fn cfg(data: &Path, database: &str) {
    fs::write(
        data.join("server.cfg"),
        format!(
            "endpoint_add_tcp \"0.0.0.0:30140\"\nendpoint_add_udp \"0.0.0.0:30140\"\n\
             set mysql_connection_string \"mysql://fivem:{PASSWORD}@127.0.0.1:3306/{database}?charset=utf8mb4\"\n"
        ),
    )
    .unwrap();
}

fn launch(data: &Path) -> LaunchSettings {
    LaunchSettings {
        data_path: data.to_str().unwrap().to_owned(),
        cfg_file: "server.cfg".to_owned(),
        onesync: OneSync::On,
        auto_start: false,
        txadmin_events: false,
        extra_arguments: Vec::new(),
        hang_detection: true,
        hang_timeout_seconds: 360,
        stop_hold_seconds: 60,
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

fn unpacked(path: &Path) -> String {
    let mut text = String::new();
    flate2::read::MultiGzDecoder::new(fs::File::open(path).unwrap())
        .read_to_string(&mut text)
        .unwrap();
    text
}

const KEEP_ALL: Retention = Retention {
    keep_last: Some(1000),
    keep_days: None,
};

#[tokio::test]
async fn a_backup_from_the_cfg_lands_in_a_local_folder_recorded_and_audited() {
    let lab = Lab::new().await;
    let destination = lab.local(KEEP_ALL).await;

    let run = lab.backup().await;

    assert_eq!(run.outcome, Some(BackupOutcome::Succeeded), "{run:?}");
    assert_eq!(run.trigger, BackupTrigger::Manual);
    assert_eq!(run.actor.as_deref(), Some("ada"));
    assert_eq!(run.database.as_deref(), Some("es_extended"));
    assert_eq!(run.host.as_deref(), Some("127.0.0.1:3306"));
    assert_eq!(run.client.as_deref(), Some("mariadb-dump 11.7.2-MariaDB"));
    let copy = &run.copies[0];
    assert_eq!(
        (copy.destination_id, copy.state),
        (destination, CopyState::Stored)
    );
    assert_eq!(copy.bytes, run.file_bytes);
    let text = unpacked(Path::new(&copy.location));
    assert!(
        text.contains("-- Host: 127.0.0.1:3306    Database: es_extended"),
        "{text}"
    );
    assert!(text.contains("-- User: fivem"));
    assert!(
        text.trim_end()
            .ends_with("-- Dump completed on 2026-11-02  4:00:01")
    );
    assert_eq!(
        fs::read_dir(lab.work()).unwrap().count(),
        0,
        "the work folder is left empty"
    );

    let view = lab.backups.view().await.unwrap();
    assert_eq!(view.runs_last_30_days, 1);
    let last = view.destinations[0].last.clone().unwrap();
    assert_eq!((last.run, last.state), (run.id, CopyState::Stored));
    let finished = lab.audited(AuditAction::BackupFinished).await;
    assert_eq!(finished.len(), 1);
    assert_eq!(finished[0].actor_name.as_deref(), Some("ada"));
    assert_eq!(finished[0].details["outcome"], "succeeded");
    assert_eq!(finished[0].details["stored"], 1);
    lab.close().await;
}

#[tokio::test]
async fn a_refused_login_fails_the_run_without_its_password_and_keeps_no_file() {
    let lab = Lab::new().await;
    lab.local(KEEP_ALL).await;
    cfg(&lab.data(), "es_fail");

    let run = lab.backup().await;

    assert_eq!(run.outcome, Some(BackupOutcome::Failed));
    assert_eq!(run.error, Some(BackupError::DumpFailed));
    let detail = run.error_detail.unwrap();
    assert!(detail.contains("Access denied for user 'fivem'"), "{detail}");
    assert!(!detail.contains(PASSWORD));
    assert_eq!(run.copies, []);
    assert_eq!(fs::read_dir(lab.work()).unwrap().count(), 0);
    lab.close().await;
}

#[tokio::test]
async fn a_dump_cut_short_is_never_copied() {
    let lab = Lab::new().await;
    lab.local(KEEP_ALL).await;
    cfg(&lab.data(), "es_cut");

    let run = lab.backup().await;

    assert_eq!(run.error, Some(BackupError::DumpIncomplete));
    assert_eq!(run.copies, []);
    assert!(!lab.copies().exists());
    lab.close().await;
}

#[tokio::test]
async fn tables_without_transactions_are_named() {
    let lab = Lab::new().await;
    lab.local(KEEP_ALL).await;
    cfg(&lab.data(), "es_loose");

    let run = lab.backup().await;

    assert_eq!(run.loose_tables, ["player_logs"]);
    assert_eq!(run.loose_table_count, 1);
    lab.close().await;
}

#[tokio::test]
async fn nothing_runs_without_a_destination_and_one_run_at_a_time() {
    let lab = Lab::new().await;

    let none = lab.backups.run_now(&lab.owner).await.unwrap_err();
    assert!(
        matches!(none, observer_app::Error::Refused(ref refusal) if refusal.code == ErrorCode::BackupNotConfigured)
    );

    lab.local(KEEP_ALL).await;
    cfg(&lab.data(), "es_slow5000");
    let first = lab.backups.run_now(&lab.owner).await.unwrap();
    let second = lab.backups.run_now(&lab.owner).await.unwrap_err();
    assert!(
        matches!(second, observer_app::Error::Refused(ref refusal) if refusal.code == ErrorCode::BackupRunning)
    );

    lab.backups.cancel(&lab.owner).await.unwrap();
    let cancelled = lab.ended(first.id).await;

    assert_eq!(cancelled.outcome, Some(BackupOutcome::Cancelled));
    assert_eq!(cancelled.error, Some(BackupError::Cancelled));
    assert_eq!(fs::read_dir(lab.work()).unwrap().count(), 0);
    assert_eq!(lab.audited(AuditAction::BackupCancelled).await.len(), 1);
    lab.close().await;
}

#[tokio::test]
async fn a_scheduled_restart_waits_for_its_dump() {
    let lab = Lab::new().await;
    lab.local(KEEP_ALL).await;
    cfg(&lab.data(), "es_slow300");

    let lines = lab
        .backups
        .before_restart(
            &Cause::Scheduled {
                by: "observer".to_owned(),
            },
            None,
            Arc::new(AtomicBool::new(false)),
        )
        .await;

    assert_eq!(lines.len(), 1);
    assert!(
        lines[0].starts_with("Backed up es_slow300 before the start"),
        "{lines:?}"
    );
    let run = lab.backups.runs(None, None).await.unwrap().runs.remove(0);
    let run = lab.ended(run.id).await;
    assert_eq!(run.trigger, BackupTrigger::Restart);
    assert_eq!(run.actor, None);
    assert!(!run.server_started);
    assert_eq!(run.outcome, Some(BackupOutcome::Succeeded));
    lab.close().await;
}

#[tokio::test]
async fn a_restart_never_waits_past_its_limit_and_the_dump_still_ends() {
    let lab = Lab::new().await;
    lab.local(KEEP_ALL).await;
    lab.settings(|settings| settings.restart_wait_seconds = 1).await;
    cfg(&lab.data(), "es_slow4000");
    let started = Instant::now();

    let lines = lab
        .backups
        .before_restart(
            &Cause::Scheduled { by: "ada".to_owned() },
            None,
            Arc::new(AtomicBool::new(false)),
        )
        .await;

    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
    assert!(
        lines[0].starts_with("The backup's dump goes on while FXServer starts"),
        "{lines:?}"
    );
    let run = lab.backups.runs(None, None).await.unwrap().runs.remove(0);
    let run = lab.ended(run.id).await;
    assert!(run.server_started);
    assert_eq!(run.actor.as_deref(), Some("ada"));
    assert_eq!(run.outcome, Some(BackupOutcome::Succeeded));
    let finished = lab.audited(AuditAction::BackupFinished).await;
    assert_eq!(finished[0].details["serverStarted"], true);
    lab.close().await;
}

#[tokio::test]
async fn the_work_before_a_spawn_backs_up_from_the_cfg_that_spawn_read() {
    let lab = Lab::new().await;
    lab.local(KEEP_ALL).await;
    let data = lab.data();
    let text = fs::read_to_string(data.join("server.cfg")).unwrap();
    let scan = cfg::scan(
        &data,
        "server.cfg",
        &text,
        &Resources::default(),
        &cfg::Context {
            panel: None,
            onesync: OneSync::On,
            probe_ports: false,
            system_resources: None,
            follow_outside: false,
            convars: None,
        },
    );
    // Edited after the spawn read it: the backup goes by what FXServer is about to run.
    cfg(&data, "es_fail");
    let hook = BackupsHook::default();
    hook.bind(lab.backups.clone());
    let work = BeforeSpawn {
        tools: ToolsHook::default(),
        backups: hook,
    };
    let build = lab.dir.path().join("build");

    let lines = work
        .run(PreLaunchRequest {
            launch: ServerLaunch {
                build: build.clone(),
                settings: launch(&data),
            },
            layout: Layout::new(build, Flavour::HOST),
            scan,
            cause: Cause::Scheduled {
                by: "observer".to_owned(),
            },
            generation: 2,
            cancel: Arc::new(AtomicBool::new(false)),
        })
        .await
        .unwrap();

    assert!(
        lines[0].starts_with("Backed up es_extended before the start"),
        "{lines:?}"
    );
    let run = lab.backups.runs(None, None).await.unwrap().runs.remove(0);
    assert_eq!(lab.ended(run.id).await.outcome, Some(BackupOutcome::Succeeded));
    lab.close().await;
}

#[tokio::test]
async fn crashes_plain_starts_and_restarts_by_hand_back_nothing_up_by_default() {
    let lab = Lab::new().await;
    lab.local(KEEP_ALL).await;
    let stop = Arc::new(AtomicBool::new(false));

    for cause in [
        Cause::Recovery,
        Cause::Requested { by: "ada".to_owned() },
        Cause::Restart { by: "ada".to_owned() },
    ] {
        let lines = lab.backups.before_restart(&cause, None, Arc::clone(&stop)).await;
        assert_eq!(lines, Vec::<String>::new(), "{cause:?}");
    }
    lab.settings(|settings| settings.before_restarts = BeforeRestarts::Off)
        .await;
    let off = lab
        .backups
        .before_restart(
            &Cause::Scheduled {
                by: "observer".to_owned(),
            },
            None,
            stop,
        )
        .await;

    assert_eq!(off, Vec::<String>::new());
    assert_eq!(lab.backups.runs(None, None).await.unwrap().runs, []);
    lab.close().await;
}

#[tokio::test]
async fn the_retention_lets_old_copies_go_after_a_success_only() {
    let lab = Lab::new().await;
    lab.local(Retention {
        keep_last: Some(2),
        keep_days: None,
    })
    .await;
    let first = lab.backup().await;
    lab.backup().await;
    let third = lab.backup().await;

    let first = lab.backups.run(first.id).await.unwrap();
    assert_eq!(first.copies[0].state, CopyState::Deleted);
    assert_eq!(first.copies[0].delete_reason, Some(DeleteReason::Retention));
    assert!(!Path::new(&first.copies[0].location).exists());
    assert_eq!(fs::read_dir(lab.copies()).unwrap().count(), 2);
    let expired = lab.audited(AuditAction::BackupsExpired).await;
    assert_eq!(expired.len(), 1);
    assert_eq!(expired[0].actor_kind, ActorKind::System);

    cfg(&lab.data(), "es_fail");
    lab.backup().await;
    assert_eq!(
        fs::read_dir(lab.copies()).unwrap().count(),
        2,
        "a failed run lets nothing go"
    );
    assert_eq!(
        lab.backups.run(third.id).await.unwrap().copies[0].state,
        CopyState::Stored
    );
    lab.close().await;
}

#[tokio::test]
async fn a_download_reads_the_copy_back_audited() {
    let lab = Lab::new().await;
    lab.local(KEEP_ALL).await;
    let run = lab.backup().await;
    let copy = &run.copies[0];

    let mut download = lab.backups.download(&lab.owner, copy.id).await.unwrap();
    let mut bytes = Vec::new();
    download.reader.read_to_end(&mut bytes).await.unwrap();

    assert_eq!(bytes, fs::read(&copy.location).unwrap());
    assert_eq!(download.file_name, run.file_name.unwrap());
    assert_eq!(download.bytes, copy.bytes);
    let downloaded = lab.audited(AuditAction::BackupDownloaded).await;
    assert_eq!(downloaded[0].target.as_deref(), Some(run.id.to_string().as_str()));
    lab.close().await;
}

#[tokio::test]
async fn a_deleted_backup_leaves_every_destination() {
    let lab = Lab::new().await;
    lab.local(KEEP_ALL).await;
    let run = lab.backup().await;

    let deleted = lab.backups.delete_run(&lab.owner, run.id).await.unwrap();

    assert_eq!(deleted.copies[0].state, CopyState::Deleted);
    assert_eq!(deleted.copies[0].delete_reason, Some(DeleteReason::Manual));
    assert_eq!(fs::read_dir(lab.copies()).unwrap().count(), 0);
    let download = lab
        .backups
        .download(&lab.owner, deleted.copies[0].id)
        .await
        .unwrap_err();
    assert!(
        matches!(download, observer_app::Error::Refused(ref refusal) if refusal.code == ErrorCode::BackupCopyUnavailable)
    );
    lab.close().await;
}

#[tokio::test]
async fn secrets_go_in_and_never_come_back_out() {
    let lab = Lab::new().await;
    let view = lab.backups.view().await.unwrap();
    let settings = BackupSettings {
        source: SourceKind::Manual,
        manual: Some(ManualSource {
            host: "127.0.0.1".to_owned(),
            port: 3307,
            user: "backup".to_owned(),
            database: "es_manual".to_owned(),
        }),
        ..view.settings
    };
    lab.backups
        .change_settings(
            &lab.owner,
            BackupSettingsChange {
                settings,
                version: view.version,
                password: Some(Secret::new("manual-secret")),
            },
        )
        .await
        .unwrap();
    let bucket = lab
        .backups
        .create_destination(
            &lab.owner,
            DestinationDraft {
                name: "Bucket".to_owned(),
                enabled: false,
                target: DestinationTarget::S3 {
                    endpoint: "https://s3.example.net".to_owned(),
                    region: "auto".to_owned(),
                    bucket: "sensity-backups".to_owned(),
                    prefix: String::new(),
                    access_key_id: "GKKEYID".to_owned(),
                    path_style: true,
                },
                retention: Retention::default(),
                secret: Some(Secret::new("s3-secret-key")),
            },
        )
        .await
        .unwrap();
    lab.local(KEEP_ALL).await;

    let run = lab.backup().await;
    let view = lab.backups.view().await.unwrap();
    let shown = serde_json::to_string(&view).unwrap();
    let audited =
        serde_json::to_string(&lab.audit.page(AuditFilter::default()).await.unwrap().entries).unwrap();

    assert!(bucket.has_secret);
    assert!(view.has_password);
    assert_eq!(view.source.database.as_deref(), Some("es_manual"));
    assert_eq!(run.host.as_deref(), Some("127.0.0.1:3307"));
    for secret in ["manual-secret", "s3-secret-key", PASSWORD] {
        assert!(!shown.contains(secret), "{secret} reached the view");
        assert!(!audited.contains(secret), "{secret} reached the audit log");
    }
    let changed = lab.audited(AuditAction::BackupSettingsChanged).await;
    assert_eq!(changed[0].details["passwordChanged"], true);
    lab.close().await;
}

#[tokio::test]
async fn a_local_destination_never_goes_inside_the_server_folder() {
    let lab = Lab::new().await;

    let refused = lab
        .backups
        .create_destination(
            &lab.owner,
            DestinationDraft {
                name: "Inside".to_owned(),
                enabled: true,
                target: DestinationTarget::Local {
                    folder: lab
                        .data()
                        .join("resources")
                        .join("dumps")
                        .to_string_lossy()
                        .into_owned(),
                },
                retention: Retention::default(),
                secret: None,
            },
        )
        .await
        .unwrap_err();

    let observer_app::Error::Refused(refusal) = refused else {
        panic!("{refused:?}");
    };
    assert_eq!(refusal.code, ErrorCode::InvalidBackupDestination);
    assert_eq!(refusal.params["issues"][0]["problem"], "insideServerFolder");
    lab.close().await;
}

#[tokio::test]
async fn runs_an_earlier_observer_left_are_ended_as_interrupted() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(
        dir.path().join("observer.sqlite3"),
        dir.path().join("sqlite-backups"),
    )
    .await
    .unwrap();
    let left = db
        .write(|tx| {
            store::insert_run(
                tx,
                &NewRun {
                    server_id: Servers::DEFAULT,
                    trigger: BackupTrigger::Schedule,
                    actor: None,
                    engine: DatabaseEngine::MariaDb,
                    started_at: Timestamp::now(),
                },
            )
        })
        .await
        .unwrap();
    let work = dir
        .path()
        .join("observer-data")
        .join("database-backups")
        .join(".work");
    fs::create_dir_all(&work).unwrap();
    fs::write(work.join(format!("{left}.cnf")), "[client]\npassword=\"left\"\n").unwrap();

    let (backups, task) = Backups::start(
        BackupsSetup {
            db: db.clone(),
            clock: Clock::system(),
            data_dir: dir.path().join("observer-data"),
            schedule: None,
        },
        |_| {},
    )
    .await
    .unwrap();

    let run = backups.run(left).await.unwrap();
    assert_eq!(run.outcome, Some(BackupOutcome::Interrupted));
    assert_eq!(
        fs::read_dir(&work).unwrap().count(),
        0,
        "a leftover option file is swept"
    );
    task.close().await;
}

#[tokio::test]
async fn a_backup_starts_at_its_time() {
    let start: Timestamp = "2026-11-02T03:58:30Z".parse().unwrap();
    let clock = Clock::manual(start);
    let lab = Lab::with(|_| {}, clock.clone()).await;
    lab.local(KEEP_ALL).await;
    let at = (start + SignedDuration::from_mins(1)).to_zoned(jiff::tz::TimeZone::system());
    let time = format!("{:02}:{:02}", at.hour(), at.minute());
    lab.settings(|settings| settings.times = vec![time]).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(lab.backups.runs(None, None).await.unwrap().runs, []);

    clock.advance(SignedDuration::from_mins(2));
    let started = Instant::now();
    let run = loop {
        if let Some(run) = lab.backups.runs(None, None).await.unwrap().runs.pop() {
            break run;
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "no timed backup started"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    };

    assert_eq!(run.trigger, BackupTrigger::Schedule);
    assert_eq!(lab.ended(run.id).await.outcome, Some(BackupOutcome::Succeeded));
    lab.close().await;
}
