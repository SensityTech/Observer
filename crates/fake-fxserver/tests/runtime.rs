//! A server run the way `observer run` runs it: launch settings stored through observer-app, then
//! started, commanded and stopped through its supervision, against the fake.
// The helpers below are test code too, though not inside a `#[test]` function.
#![allow(clippy::unwrap_used)]

mod common;

use std::fs;
use std::io::{Read as _, Write as _};
use std::net::{IpAddr, Ipv4Addr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::{Port, Ports};
use jiff::{SignedDuration, Timestamp, Unit};
use observer_app::{
    Accounts, Admission, AdmissionSetup, AdmissionTask, AuditLog, Bans, BansSetup, BansTask, Caller, Client,
    Clock, DatabaseFiles, Diagnostics, DiagnosticsSetup, DiagnosticsTask, Drops, DropsSetup, Entities,
    EntitiesSetup, Error, LiveMap, LogQuery, MapSetup, Metrics, MetricsSetup, PasswordCost, Players,
    PlayersSetup, PlayersTask, Running, Sampling, Schedule, ScheduleSetup, ScheduleTask, Servers, Setup,
    Sources, Supervision,
};
use observer_domain::allowlist::AllowlistSettings;
use observer_domain::audit::{ActorKind, AuditAction, AuditEntry, AuditFilter, Outcome};
use observer_domain::bans::{BanFilter, BanRequest};
use observer_domain::changes::LiveChangeKind;
use observer_domain::diagnostics::{
    Alert, AlertKind, AnalysisState, CrashFileKind, CrashKind, DiagnosticsEvent, ListenerRole, ProcessKind,
    Protocol, Scope,
};
use observer_domain::drops::{DropCategory, DropsRange, TileKind};
use observer_domain::entities::{
    EntitiesLive, EntitiesUnavailable, EntityDeleteRequest, EntityFilters, EntityKind, EntityOutcome,
    EntityPage, EntitySort, EntityTarget, VehicleClass,
};
use observer_domain::error::ErrorCode;
use observer_domain::launch::{ExtraArgument, LaunchSettings, OneSync};
use observer_domain::maintenance::{AddPass, MaintenanceSettings, SetMaintenance};
use observer_domain::metrics::{LiveSample, MarkerKind, Range, Series, ThreadGroup, ThreadShares};
use observer_domain::permissions::Permissions;
use observer_domain::players::{Confirmation, IdentifierKind, PlayerLabel, PlayerProfile, PlayerSort};
use observer_domain::resources::{
    ResourceAck, ResourceOrigin, ResourceRow, ResourceState, ResourceWarnings, ResourcesStage, ResourcesView,
};
use observer_domain::schedule::{
    OneOffRequest, OneOffRestart, RestartSource, ScheduleChange, ScheduleSettings,
};
use observer_domain::server::{
    CommandAck, ConsoleLine, Delivery, LINE_MASKED, LineKind, ServerState, ServerStatus, StateCause,
};
use observer_domain::server_data::{CfgCheck, FindingKind};
use observer_fxserver::console::plain;
use observer_fxserver::health::HealthLimits;
use observer_fxserver::recovery::RecoveryPolicy;
use observer_fxserver::supervisor::StopTimings;
use observer_store::artifacts::{self, ArtifactState};
use observer_store::{Database, StoreError};
use serde_json::json;
use tempfile::TempDir;
use tokio::sync::{broadcast, watch};
use tokio::task::JoinHandle;
use tokio::time::timeout;

const WAIT: Duration = Duration::from_secs(20);
/// The build the fake stands in for, installed in Observer's artifacts folder and selected.
const BUILD: u32 = 35245;
const CONSOLE_ROUTE: (&str, &str) = ("POST", "/api/servers/default/console");
/// A quick heartbeat, three players and ticks of 12 ms.
const FAKE: [(&str, &str); 3] = [
    ("fake_heartbeat_ms", "200"),
    ("fake_players", "3"),
    ("fake_tick_ms", "12"),
];

/// The Observers' ports, lent to three at a time.
static PORTS: Ports = Ports::new(3);

struct Observer {
    dir: TempDir,
    port: Port,
    /// The schedule's clock, which the tests move on rather than wait.
    clock: Clock,
    /// The schedule's longest sleep.
    longest_sleep: Duration,
    db: Database,
    audit: AuditLog,
    servers: Servers,
    supervision: Supervision,
    running: Option<Running>,
    schedule: Schedule,
    scheduling: Option<ScheduleTask>,
    diagnostics: Diagnostics,
    diagnosing: Option<DiagnosticsTask>,
    owner: Caller,
}

/// The services `observer run` starts, on the database of `dir`.
struct Services {
    db: Database,
    audit: AuditLog,
    servers: Servers,
    accounts: Accounts,
}

impl Services {
    async fn open(dir: &Path) -> Self {
        let db = Database::open(dir.join("observer.sqlite3"), dir.join("backups"))
            .await
            .unwrap();
        let clock = Clock::system();
        let accounts = Accounts::new(db.clone(), clock.clone(), PasswordCost::TEST)
            .await
            .unwrap();
        Self {
            audit: AuditLog::new(db.clone(), clock.clone()),
            servers: Servers::new(db.clone(), clock),
            accounts,
            db,
        }
    }

    /// The supervision, the schedule and the diagnostics, with timings short enough for tests.
    async fn supervise(
        &self,
        dir: &Path,
        clock: &Clock,
        longest_sleep: Duration,
    ) -> (
        Supervision,
        Running,
        Schedule,
        ScheduleTask,
        Diagnostics,
        DiagnosticsTask,
    ) {
        let setup = Setup {
            db: self.db.clone(),
            clock: Clock::system(),
            accounts: self.accounts.clone(),
            servers: self.servers.clone(),
            data_dir: dir.join("observer-data"),
            panel: common::panel(),
            artifacts: dir.join("observer-data/artifacts"),
            console_logs: dir.join("observer-data/logs/console"),
            log: None,
        };
        let (supervision, running) = Supervision::start(setup, |settings| {
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
                backoff: vec![Duration::from_millis(100)],
                backoff_cap: Duration::from_millis(100),
                ..RecoveryPolicy::default()
            };
        })
        .await
        .unwrap();
        let (schedule, scheduling) = Schedule::start(
            ScheduleSetup {
                db: self.db.clone(),
                clock: clock.clone(),
                supervision: supervision.clone(),
            },
            |timings| timings.longest_sleep = longest_sleep,
        )
        .await
        .unwrap();
        let (diagnostics, diagnosing) = Diagnostics::start(DiagnosticsSetup {
            db: self.db.clone(),
            clock: Clock::system(),
            supervision: supervision.clone(),
            data_dir: dir.join("observer-data"),
        })
        .await
        .unwrap();
        (
            supervision,
            running,
            schedule,
            scheduling,
            diagnostics,
            diagnosing,
        )
    }
}

impl Observer {
    /// An Observer whose owner `ada` saved launch settings for the fake.
    async fn new(auto_start: bool) -> Self {
        Self::with_longest_sleep(auto_start, Duration::from_millis(50)).await
    }

    async fn with_longest_sleep(auto_start: bool, longest_sleep: Duration) -> Self {
        Self::launched(auto_start, longest_sleep, &FAKE).await
    }

    /// The fake takes `arguments` instead of [`FAKE`]'s.
    async fn launched(auto_start: bool, longest_sleep: Duration, arguments: &[(&str, &str)]) -> Self {
        let port = PORTS.lease().await;
        let dir = tempfile::tempdir().unwrap();
        let build = dir.path().join("observer-data/artifacts").join(BUILD.to_string());
        let program = fake_program(&build);
        fs::create_dir_all(program.parent().unwrap()).unwrap();
        fs::copy(env!("CARGO_BIN_EXE_fake-fxserver"), &program).unwrap();
        let data = dir.path().join("server-data");
        fs::create_dir_all(&data).unwrap();
        fs::write(
            data.join("server.cfg"),
            format!(
                "endpoint_add_tcp \"127.0.0.1:{0}\"\nendpoint_add_udp \"127.0.0.1:{0}\"\nsv_maxclients 8\nsets sv_projectName \"^2Observer ^7lab\"\n",
                port.number
            ),
        )
        .unwrap();

        let services = Services::open(dir.path()).await;
        let accounts = &services.accounts;
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
        let owner = accounts
            .authenticate(&signed_in.token, &signed_in.key, client)
            .await
            .unwrap();
        let launch = LaunchSettings {
            data_path: data.to_str().unwrap().to_owned(),
            cfg_file: "server.cfg".to_owned(),
            onesync: OneSync::On,
            auto_start,
            txadmin_events: false,
            extra_arguments: arguments
                .iter()
                .map(|&(name, value)| ExtraArgument {
                    name: name.to_owned(),
                    value: value.to_owned(),
                })
                .collect(),
            hang_detection: true,
            hang_timeout_seconds: 360,
        };
        services
            .servers
            .change_launch_settings(&owner, Servers::DEFAULT, launch, None)
            .await
            .unwrap();
        let selected = ArtifactState {
            selected: Some(BUILD),
            ..ArtifactState::default()
        };
        services
            .db
            .write(move |tx| artifacts::save_state(tx, Servers::DEFAULT, &selected, Timestamp::now()))
            .await
            .unwrap();

        // On a whole second, as a one-off planned in minutes is.
        let clock = Clock::manual(Timestamp::now().round(Unit::Second).unwrap());
        let (supervision, running, schedule, scheduling, diagnostics, diagnosing) =
            services.supervise(dir.path(), &clock, longest_sleep).await;
        Self {
            dir,
            port,
            clock,
            longest_sleep,
            db: services.db,
            audit: services.audit,
            servers: services.servers,
            supervision,
            running: Some(running),
            schedule,
            scheduling: Some(scheduling),
            diagnostics,
            diagnosing: Some(diagnosing),
            owner,
        }
    }

    async fn until(&self, wanted: impl Fn(&ServerStatus) -> bool) -> ServerStatus {
        let mut status = self.supervision.status();
        let found = timeout(WAIT, status.wait_for(|status| wanted(status)))
            .await
            .map(|found| found.unwrap().clone());
        found.unwrap_or_else(|_| panic!("the status never came; last {:#?}", status.borrow()))
    }

    async fn state(&self, state: ServerState) -> ServerStatus {
        self.until(|status| status.state == state).await
    }

    fn console(&self) -> Vec<ConsoleLine> {
        self.supervision.console(None).0.lines
    }

    async fn console_until(&self, needle: &str) -> ConsoleLine {
        let deadline = Instant::now() + WAIT;
        loop {
            if let Some(line) = self
                .console()
                .into_iter()
                .find(|line| plain(&line.5).contains(needle))
            {
                return line;
            }
            assert!(
                Instant::now() < deadline,
                "no {needle:?} in {:#?}",
                self.console()
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn audited(&self, action: AuditAction) -> Vec<AuditEntry> {
        let filter = AuditFilter {
            action: Some(action),
            ..AuditFilter::default()
        };
        self.audit.page(filter).await.unwrap().entries
    }

    /// Has the owner save the settings again with `cfg_file`, a cfg of the data folder that runs
    /// the usual one, then prints its own name.
    async fn save_cfg(&self, cfg_file: &str) {
        let data = self.dir.path().join("server-data");
        fs::write(
            data.join(cfg_file),
            format!(
                "exec server.cfg
fake_print running {cfg_file}
"
            ),
        )
        .unwrap();
        let stored = self
            .servers
            .launch_settings(Servers::DEFAULT)
            .await
            .unwrap()
            .unwrap();
        let settings = LaunchSettings {
            cfg_file: cfg_file.to_owned(),
            ..stored.settings
        };
        self.servers
            .change_launch_settings(&self.owner, Servers::DEFAULT, settings, Some(stored.version))
            .await
            .unwrap();
    }

    /// Crashes the server, then waits for its restart of `generation` to be online and returns
    /// the cfg it ran.
    async fn crash_and_restart(&self, generation: u32) -> String {
        self.supervision
            .command(&self.owner, "fake_crash 3", CONSOLE_ROUTE.0, CONSOLE_ROUTE.1)
            .await
            .unwrap();
        self.until(|status| status.generation == generation && status.state == ServerState::Online)
            .await;
        self.cfg_run_by(generation).await
    }

    async fn cfg_run_by(&self, generation: u32) -> String {
        let deadline = Instant::now() + WAIT;
        loop {
            let ran = self.console().into_iter().find_map(|line| {
                let text = plain(&line.5);
                let cfg = text.split_once("] running ")?.1.to_owned();
                (line.2 == generation).then_some(cfg)
            });
            if let Some(cfg) = ran {
                return cfg;
            }
            assert!(
                Instant::now() < deadline,
                "generation {generation} ran no cfg of its own"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn stop_observer(&mut self) {
        if let Some(scheduling) = self.scheduling.take() {
            scheduling.close().await;
        }
        self.supervision.shutdown(false).await;
        let running = self.running.take().unwrap();
        timeout(WAIT, running.close())
            .await
            .expect("the supervisor did not stop");
        if let Some(diagnosing) = self.diagnosing.take() {
            diagnosing.close().await;
        }
    }

    /// Stops this Observer, then starts another on the same data, as after an update.
    async fn restart_observer(&mut self) {
        self.stop_observer().await;
        self.db.clone().close().await.unwrap();
        let services = Services::open(self.dir.path()).await;
        let (supervision, running, schedule, scheduling, diagnostics, diagnosing) = services
            .supervise(self.dir.path(), &self.clock, self.longest_sleep)
            .await;
        self.diagnostics = diagnostics;
        self.diagnosing = Some(diagnosing);
        self.db = services.db;
        self.audit = services.audit;
        self.servers = services.servers;
        self.supervision = supervision;
        self.running = Some(running);
        self.schedule = schedule;
        self.scheduling = Some(scheduling);
    }

    /// The position of the first console line holding `needle`, once there is one.
    async fn line_of(&self, needle: &str) -> u64 {
        self.console_until(needle).await.0
    }

    /// Players coming and going, as `sv_maxclients` changing stands in for them on the fake, from
    /// the first change the status shows. A change a little over every second stays under the
    /// console's ten commands in ten seconds, so the changes never pause.
    async fn churn(&self) -> JoinHandle<()> {
        let supervision = self.supervision.clone();
        let owner = self.owner.clone();
        let churn = tokio::spawn(async move {
            for max in 9..200 {
                let line = format!("set sv_maxclients {max}");
                let _ = supervision
                    .command(&owner, &line, CONSOLE_ROUTE.0, CONSOLE_ROUTE.1)
                    .await;
                tokio::time::sleep(Duration::from_millis(1100)).await;
            }
        });
        self.until(|status| {
            status
                .players
                .is_some_and(|players| players.max.is_some_and(|max| max >= 10))
        })
        .await;
        churn
    }

    /// Makes every write of the schedule's plan fail, or lets them through again.
    async fn hold_the_plan(&self, held: bool) {
        let sql = if held {
            "CREATE TRIGGER hold_the_plan BEFORE UPDATE ON schedule_plan
             BEGIN SELECT RAISE(ABORT, 'the plan is held'); END"
        } else {
            "DROP TRIGGER hold_the_plan"
        };
        self.db
            .write(move |tx| tx.execute_batch(sql).map_err(StoreError::from))
            .await
            .unwrap();
    }
}

fn fake_program(build: &Path) -> PathBuf {
    if cfg!(windows) {
        build.join("FXServer.exe")
    } else {
        build.join("alpine/opt/cfx-server/ld-musl-x86_64.so.1")
    }
}

#[tokio::test]
async fn a_server_started_from_the_panel_runs_a_command_then_stops_gracefully() {
    let mut observer = Observer::new(false).await;

    observer.supervision.start_server(&observer.owner).await.unwrap();
    let online = observer
        .until(|status| status.state == ServerState::Online && status.build.is_some())
        .await;
    let delivery = observer
        .supervision
        .command(
            &observer.owner,
            "fake_print hello from the panel",
            CONSOLE_ROUTE.0,
            CONSOLE_ROUTE.1,
        )
        .await
        .unwrap();
    let echo = observer.console_until("fake_print hello").await;
    let printed = observer.console_until("] hello from the panel").await;

    assert_eq!(online.cause, Some(StateCause::Ready));
    assert_eq!(online.build, Some(35245));
    assert_eq!(online.name.as_deref(), Some("Observer lab"));
    assert_eq!(online.port, Some(observer.port.number));
    assert!(online.started_at.is_some());
    assert_eq!(
        delivery,
        CommandAck {
            delivery: Delivery::Written,
            masked: false
        }
    );
    assert_eq!((echo.3, echo.4.as_deref()), (LineKind::Command, Some("ada")));
    assert!(
        echo.0 < printed.0,
        "the echo comes before what the command printed"
    );
    let commands = observer.audited(AuditAction::ConsoleCommandSent).await;
    assert_eq!(commands.len(), 1);
    assert_eq!(
        commands[0].details,
        json!({ "command": "fake_print hello from the panel" })
    );
    assert_eq!(commands[0].actor_name.as_deref(), Some("ada"));

    observer.supervision.stop_server(&observer.owner).await.unwrap();
    let stopped = observer.state(ServerState::Stopped).await;

    assert_eq!(
        stopped.cause,
        Some(StateCause::Requested { by: "ada".to_owned() }),
        "who stopped it says more than its exit code"
    );
    assert_eq!(
        (stopped.started_at, stopped.players, stopped.build),
        (None, None, None)
    );
    let notice = observer.console_until("observer:shuttingDown").await;
    let quit = observer.console_until("-> Quitting: stop").await;
    assert!(notice.0 < quit.0);
    assert!(plain(&notice.5).contains(r#""by":"ada""#), "{notice:?}");
    for action in [AuditAction::ServerStarted, AuditAction::ServerStopped] {
        let entries = observer.audited(action).await;
        assert_eq!(entries.len(), 1, "{action:?}");
        assert_eq!(entries[0].outcome, Outcome::Success);
        assert_eq!(entries[0].target.as_deref(), Some("default"));
    }
    observer.stop_observer().await;
}

#[tokio::test]
async fn a_start_from_the_host_window_is_audited_as_local() {
    let mut observer = Observer::new(false).await;
    let local = || "local".to_owned();

    observer.supervision.start_local().await.unwrap();
    let starting = observer.state(ServerState::Starting).await;
    observer.state(ServerState::Online).await;
    observer.supervision.restart_local().await.unwrap();
    let restarting = observer.state(ServerState::Stopping).await;
    observer.state(ServerState::Online).await;
    observer.supervision.stop_local().await.unwrap();
    let stopped = observer.state(ServerState::Stopped).await;

    assert_eq!(starting.cause, Some(StateCause::Requested { by: local() }));
    assert_eq!(restarting.cause, Some(StateCause::Restart { by: local() }));
    assert_eq!(stopped.cause, Some(StateCause::Requested { by: local() }));
    let notice = observer.console_until("observer:shuttingDown").await;
    assert!(plain(&notice.5).contains(r#""by":"local""#), "{notice:?}");
    for action in [
        AuditAction::ServerStarted,
        AuditAction::ServerRestarted,
        AuditAction::ServerStopped,
    ] {
        let entries = observer.audited(action).await;
        assert_eq!(entries.len(), 1, "{action:?}");
        assert_eq!(
            (entries[0].actor_kind, entries[0].actor_name.as_deref()),
            (ActorKind::Local, None),
            "{action:?}"
        );
        assert_eq!(entries[0].target.as_deref(), Some("default"));
    }
    observer.stop_observer().await;
}

#[tokio::test]
async fn a_server_started_with_observer_stops_with_it() {
    let mut observer = Observer::new(true).await;

    observer.supervision.boot().await.unwrap();
    observer.state(ServerState::Online).await;
    observer.stop_observer().await;

    let stopped = observer.supervision.status().borrow().clone();
    assert_eq!(stopped.state, ServerState::Stopped);
    assert_eq!(stopped.cause, Some(StateCause::Shutdown));
    let notice = observer.console_until("observer:shuttingDown").await;
    assert!(plain(&notice.5).contains(r#""reason":"shutdown""#), "{notice:?}");
    observer.console_until("-> Quitting: shutdown").await;
    let started = observer.audited(AuditAction::ServerStarted).await;
    assert_eq!(started.len(), 1);
    assert_eq!(started[0].actor_kind, ActorKind::System);
    assert_eq!(started[0].details, json!({ "automatic": true }));
}

#[tokio::test]
async fn a_command_whose_audit_entry_cannot_be_written_never_reaches_fxserver() {
    let mut observer = Observer::new(false).await;
    observer.supervision.start_server(&observer.owner).await.unwrap();
    observer.state(ServerState::Online).await;

    observer.db.clone().close().await.unwrap();
    let sent = observer
        .supervision
        .command(
            &observer.owner,
            "fake_print never audited",
            CONSOLE_ROUTE.0,
            CONSOLE_ROUTE.1,
        )
        .await;
    tokio::time::sleep(Duration::from_secs(1)).await;

    assert!(sent.is_err());
    assert!(
        !observer
            .console()
            .iter()
            .any(|line| line.5.contains("never audited")),
        "{:#?}",
        observer.console()
    );
    observer.stop_observer().await;
}

#[tokio::test]
async fn a_restart_after_a_crash_launches_the_settings_saved_while_the_server_ran() {
    let mut observer = Observer::new(false).await;
    observer.save_cfg("a.cfg").await;
    observer.supervision.start_server(&observer.owner).await.unwrap();
    observer.state(ServerState::Online).await;
    assert_eq!(observer.cfg_run_by(1).await, "a.cfg");

    observer.save_cfg("b.cfg").await;
    let after_a_save = observer.crash_and_restart(2).await;
    observer.save_cfg("c.cfg").await;
    let refused = observer.supervision.start_server(&observer.owner).await;
    let after_a_refused_start = observer.crash_and_restart(3).await;

    assert_eq!(after_a_save, "b.cfg", "saved settings apply from the next spawn");
    assert!(
        matches!(&refused, Err(Error::Refused(error)) if error.code == ErrorCode::ServerRunning),
        "{refused:?}"
    );
    assert_eq!(
        after_a_refused_start, "c.cfg",
        "the stored settings, whatever was asked in between"
    );
    observer.stop_observer().await;
}

#[tokio::test]
async fn a_stop_while_the_server_already_stops_is_refused_and_not_audited() {
    let mut observer = Observer::new(false).await;
    observer.supervision.start_server(&observer.owner).await.unwrap();
    observer.state(ServerState::Online).await;

    observer.supervision.stop_server(&observer.owner).await.unwrap();
    observer.state(ServerState::Stopping).await;
    let again = observer.supervision.stop_server(&observer.owner).await;
    observer.state(ServerState::Stopped).await;

    assert!(
        matches!(&again, Err(Error::Refused(error)) if error.code == ErrorCode::ServerStopping),
        "{again:?}"
    );
    assert_eq!(observer.audited(AuditAction::ServerStopped).await.len(), 1);
    observer.stop_observer().await;
}

#[tokio::test]
async fn a_tripped_crash_loop_breaker_is_audited() {
    let mut observer = Observer::new(false).await;
    observer.supervision.start_server(&observer.owner).await.unwrap();

    for generation in 1..=5 {
        observer
            .until(|status| status.generation == generation && status.started_at.is_some())
            .await;
        observer
            .supervision
            .command(&observer.owner, "fake_crash 3", CONSOLE_ROUTE.0, CONSOLE_ROUTE.1)
            .await
            .unwrap();
    }
    observer.state(ServerState::CrashLoop).await;

    let tripped = observer.audited(AuditAction::ServerCrashLoop).await;
    assert_eq!(tripped.len(), 1);
    assert_eq!(
        (tripped[0].actor_kind, tripped[0].outcome),
        (ActorKind::System, Outcome::Failure)
    );
    assert_eq!(tripped[0].target.as_deref(), Some("default"));
    assert_eq!(tripped[0].details, json!({ "failures": 5 }));
    observer.stop_observer().await;
}

#[tokio::test]
async fn a_secret_set_in_an_unusual_spelling_is_masked_in_the_echo_and_the_audit_log() {
    let mut observer = Observer::new(false).await;
    observer.supervision.start_server(&observer.owner).await.unwrap();
    observer.state(ServerState::Online).await;

    let ack = observer
        .supervision
        .command(
            &observer.owner,
            "set sv_tebexSecret\"hunter2\"; set /**/rcon_password hunter2",
            CONSOLE_ROUTE.0,
            CONSOLE_ROUTE.1,
        )
        .await
        .unwrap();
    let echo = observer.console_until("sv_tebexSecret").await;

    assert_eq!(
        echo.5,
        "set sv_tebexSecret [REDACTED]; set rcon_password [REDACTED]"
    );
    assert_eq!(echo.6 & LINE_MASKED, LINE_MASKED);
    assert!(ack.masked, "the panel keeps the line out of its history");
    let commands = observer.audited(AuditAction::ConsoleCommandSent).await;
    assert_eq!(
        commands[0].details,
        json!({ "command": "set sv_tebexSecret [REDACTED]; set rcon_password [REDACTED]" })
    );
    assert!(!format!("{:?}", observer.console()).contains("hunter2"));
    observer.stop_observer().await;
}

/// The next live sample that `wanted` accepts, within a few slots.
async fn sample_where(
    live: &mut broadcast::Receiver<LiveSample>,
    wanted: impl Fn(&LiveSample) -> bool,
) -> LiveSample {
    let found = timeout(Duration::from_secs(60), async {
        loop {
            let sample = live.recv().await.unwrap();
            if wanted(&sample) {
                return sample;
            }
        }
    })
    .await;
    found.expect("no such sample came")
}

/// What the sample of a fake spinning half of its network loop, and the hour's thread shares, say
/// of its threads and its tree.
fn assert_threads_and_tree(running: &LiveSample, threads: &ThreadShares) {
    let value = |series| running.values.get(&series).copied();
    // The fake's network loop spins half of every 100 ms. A loaded machine gives it less, never
    // more, so the floor only tells a measured thread from an idle one.
    let network = value(Series::NetworkThreadCpu).unwrap();
    assert!((10.0..=65.0).contains(&network), "{running:?}");
    let cpu = value(Series::FxsCpu).unwrap();
    assert!(
        value(Series::FxsKernel).is_some_and(|kernel| kernel <= cpu),
        "{running:?}"
    );
    // The tree's totals and the main process are read a moment apart.
    assert!(
        value(Series::TreeCpu).is_some_and(|tree| tree + 1.0 >= cpu),
        "{running:?}"
    );
    if cfg!(windows) {
        assert!(value(Series::HostDiskTransfers).is_some(), "{running:?}");
    }
    let named = threads
        .threads
        .iter()
        .find(|thread| thread.name.starts_with("UV loop: svNetw"))
        .unwrap_or_else(|| panic!("{threads:?}"));
    assert_eq!(named.group, ThreadGroup::Network);
    assert!(threads.measured >= 1);
}

#[tokio::test]
async fn the_statistics_follow_a_run_from_before_its_start_to_after_its_stop() {
    let [heartbeat, players, tick] = FAKE;
    let busy = [heartbeat, players, tick, ("fake_busy_percent", "50")];
    let mut observer = Observer::launched(false, Duration::from_millis(50), &busy).await;
    let path = observer.dir.path().join("metrics.sqlite3");
    let metrics = Metrics::open(MetricsSetup {
        path: path.clone(),
        clock: Clock::system(),
    })
    .await
    .unwrap();
    let (_sockets, sockets) = watch::channel(1);
    let sampling = metrics.start(Sources {
        supervision: observer.supervision.clone(),
        sockets,
        data_dir: observer.dir.path().to_path_buf(),
        databases: DatabaseFiles {
            state: observer.dir.path().join("observer.sqlite3"),
            metrics: path,
        },
    });
    let (_, mut live) = metrics.live();

    let before = sample_where(&mut live, |_| true).await;
    observer.supervision.start_server(&observer.owner).await.unwrap();
    observer.state(ServerState::Online).await;
    // Thread shares need two listings of the same process.
    let running = sample_where(&mut live, |sample| {
        sample.values.contains_key(&Series::PingAvg) && sample.values.contains_key(&Series::NetworkThreadCpu)
    })
    .await;
    let threads = metrics.threads().await.unwrap();
    observer.supervision.stop_server(&observer.owner).await.unwrap();
    observer.state(ServerState::Stopped).await;
    let after = sample_where(&mut live, |sample| sample.state == ServerState::Stopped).await;
    sampling.close().await;

    assert_eq!(before.state, ServerState::Stopped);
    assert!(!before.values.contains_key(&Series::Players), "{before:?}");
    assert!(before.values[&Series::ObserverWorkingSet] > 0.0);
    assert!(before.values[&Series::HostMemoryTotal] > 0.0);
    assert_eq!(before.values[&Series::DashboardClients], 1.0);
    let value = |series| running.values.get(&series).copied();
    assert_eq!(value(Series::Players), Some(3.0));
    assert_eq!(value(Series::Slots), Some(8.0));
    // The fake's pings are 25, 30 and 35 ms.
    assert_eq!(
        (value(Series::PingAvg), value(Series::PingP95)),
        (Some(30.0), Some(35.0))
    );
    assert_eq!(value(Series::MainTick), Some(12.0));
    assert_eq!(value(Series::MainOverBudget), Some(0.0));
    assert!(value(Series::FxsCpu).is_some_and(|cpu| cpu >= 0.0), "{running:?}");
    assert!(value(Series::FxsWorkingSet).is_some_and(|bytes| bytes > 0.0));
    assert_eq!(value(Series::HitchesMain), Some(0.0));
    assert_eq!(
        value(Series::Resources),
        Some(1.0),
        "the fake starts the bridge alone"
    );
    assert_threads_and_tree(&running, &threads);
    for series in [
        Series::Players,
        Series::FxsCpu,
        Series::MainTick,
        Series::FxsWorkingSet,
        Series::NetworkThreadCpu,
        Series::TreeCpu,
    ] {
        assert!(
            !after.values.contains_key(&series),
            "{series:?} after the stop: {after:?}"
        );
    }

    let hour = metrics
        .series(Range::Hour, &[Series::Players, Series::FxsCpu])
        .await
        .unwrap();
    let players = &hour.series[0].values;
    assert!(players.contains(&Some(3.0)));
    assert_eq!(players.last(), Some(&None), "the open slot has no sample yet");
    assert_eq!(
        hour.markers.iter().map(|marker| marker.kind).collect::<Vec<_>>(),
        [MarkerKind::Start]
    );
    metrics.close().await.unwrap();
    observer.stop_observer().await;
}

#[tokio::test]
async fn a_one_off_warns_the_bridge_then_restarts_on_schedule() {
    let mut observer = Observer::new(false).await;
    observer.supervision.start_server(&observer.owner).await.unwrap();
    observer.state(ServerState::Online).await;

    let planned = observer
        .schedule
        .restart_once(
            &observer.owner,
            OneOffRequest {
                in_minutes: Some(2),
                at: None,
            },
        )
        .await
        .unwrap();
    let two_minutes = observer
        .line_of(r#""reason":"oneOff","secondsRemaining":120}"#)
        .await;
    observer.clock.advance(SignedDuration::from_mins(1));
    let one_minute = observer
        .line_of(r#""reason":"oneOff","secondsRemaining":60}"#)
        .await;
    observer.clock.advance(SignedDuration::from_mins(1));
    let restarted = observer
        .until(|status| status.generation == 2 && status.state == ServerState::Online)
        .await;

    let at = observer.clock.now();
    assert_eq!(
        planned.one_off,
        Some(OneOffRestart {
            at,
            by: "ada".to_owned()
        })
    );
    assert_eq!(planned.next.map(|next| next.source), Some(RestartSource::OneOff));
    let notice = observer.line_of("observer:shuttingDown").await;
    let quit = observer.line_of("-> Quitting: scheduled").await;
    let note = observer.line_of("restarting as ada planned").await;
    assert!(two_minutes < one_minute && one_minute < notice && notice < quit);
    assert!(note > one_minute);
    let shutting_down = observer
        .console()
        .into_iter()
        .find(|line| line.0 == notice)
        .unwrap();
    assert!(
        plain(&shutting_down.5).contains(r#""by":"ada","delayMs":300,"reason":"scheduled""#),
        "{shutting_down:?}"
    );
    assert!(restarted.started_at.is_some());
    let planned_entries = observer.audited(AuditAction::OneOffRestartPlanned).await;
    assert_eq!(planned_entries.len(), 1);
    assert_eq!(planned_entries[0].actor_name.as_deref(), Some("ada"));
    assert_eq!(planned_entries[0].details, json!({ "at": at }));
    let restarts = observer.audited(AuditAction::ServerRestarted).await;
    assert_eq!(restarts.len(), 1);
    assert_eq!(restarts[0].actor_kind, ActorKind::System);
    assert_eq!(
        restarts[0].details,
        json!({ "scheduled": { "at": at, "source": "oneOff", "plannedBy": "ada" } })
    );
    assert_eq!(observer.schedule.status().borrow().one_off, None);
    observer.stop_observer().await;
}

#[tokio::test]
async fn the_schedule_never_starts_a_stopped_server() {
    let mut observer = Observer::new(false).await;
    let soon = observer.clock.now() + SignedDuration::from_mins(2);
    let time = soon.strftime("%H:%M").to_string();
    let settings = ScheduleSettings {
        enabled: true,
        times: vec![time],
        time_zone: Some("UTC".to_owned()),
        ..ScheduleSettings::default()
    };
    let saved = observer
        .schedule
        .change_settings(
            &observer.owner,
            ScheduleChange {
                settings,
                version: None,
            },
        )
        .await
        .unwrap();
    let first = saved.next.unwrap().at;

    observer.clock.advance(SignedDuration::from_mins(3));
    let mut status = observer.schedule.status();
    let moved = timeout(
        WAIT,
        status.wait_for(|status| status.next.as_ref().is_some_and(|next| next.at > first)),
    )
    .await
    .map(|status| status.unwrap().clone());
    tokio::time::sleep(Duration::from_millis(300)).await;

    assert!(
        first <= soon && soon.duration_since(first) < SignedDuration::from_mins(1),
        "{first} for {soon}"
    );
    assert_eq!(
        moved.unwrap().next.unwrap().at,
        first + SignedDuration::from_hours(24)
    );
    let server = observer.supervision.status().borrow().clone();
    assert_eq!((server.state, server.generation), (ServerState::Stopped, 0));
    for action in [
        AuditAction::ScheduledRestartMissed,
        AuditAction::ServerStarted,
        AuditAction::ServerRestarted,
    ] {
        assert_eq!(observer.audited(action).await, [], "{action:?}");
    }
    assert_eq!(observer.audited(AuditAction::ScheduleChanged).await.len(), 1);
    observer.stop_observer().await;
}

#[tokio::test]
async fn a_planned_one_off_survives_a_restart_of_observer() {
    let mut observer = Observer::new(true).await;
    observer.supervision.boot().await.unwrap();
    observer.state(ServerState::Online).await;
    let planned = observer
        .schedule
        .restart_once(
            &observer.owner,
            OneOffRequest {
                in_minutes: Some(30),
                at: None,
            },
        )
        .await
        .unwrap()
        .one_off
        .unwrap();

    observer.restart_observer().await;
    let read_back = observer.schedule.status().borrow().one_off.clone();
    observer.supervision.boot().await.unwrap();
    observer.state(ServerState::Online).await;
    observer.clock.advance(SignedDuration::from_mins(30));
    observer
        .until(|status| status.generation == 2 && status.state == ServerState::Online)
        .await;

    assert_eq!(read_back, Some(planned.clone()));
    let restarts = observer.audited(AuditAction::ServerRestarted).await;
    assert_eq!(restarts.len(), 1, "{restarts:#?}");
    assert_eq!(
        restarts[0].details,
        json!({ "scheduled": { "at": planned.at, "source": "oneOff", "plannedBy": "ada" } })
    );
    observer.stop_observer().await;
}

/// The schedule sleeps on a monotonic clock while its restarts are wall-clock instants. A clock
/// that jumps past a restart's time makes it due at the next turn of the schedule's loop, and the
/// changes of a busy server's status are such turns: they must not each start the wait over.
#[tokio::test]
async fn a_restart_the_clock_jumped_past_begins_at_once_while_the_status_keeps_changing() {
    let mut observer = Observer::with_longest_sleep(false, Duration::from_secs(30)).await;
    observer.supervision.start_server(&observer.owner).await.unwrap();
    observer.state(ServerState::Online).await;
    observer
        .schedule
        .restart_once(
            &observer.owner,
            OneOffRequest {
                in_minutes: Some(2),
                at: None,
            },
        )
        .await
        .unwrap();
    observer
        .line_of(r#""reason":"oneOff","secondsRemaining":120}"#)
        .await;
    let churn = observer.churn().await;

    observer
        .clock
        .advance(SignedDuration::from_mins(2) + SignedDuration::from_secs(1));
    let jumped = Instant::now();
    observer.state(ServerState::Stopping).await;
    let waited = jumped.elapsed();
    churn.abort();
    observer.stop_observer().await;

    assert!(
        waited < Duration::from_secs(5),
        "the restart began {waited:?} after the clock passed its time"
    );
}

/// A step whose writes fail is tried again after the longest sleep, and the turns a busy server's
/// status gives the schedule's loop meanwhile must not each start that wait over.
#[tokio::test]
async fn a_restart_whose_step_could_not_be_written_is_tried_again_while_the_status_keeps_changing() {
    let longest_sleep = Duration::from_secs(4);
    let held = Duration::from_secs(3);
    let mut observer = Observer::with_longest_sleep(false, longest_sleep).await;
    observer.supervision.start_server(&observer.owner).await.unwrap();
    observer.state(ServerState::Online).await;
    observer
        .schedule
        .restart_once(
            &observer.owner,
            OneOffRequest {
                in_minutes: Some(2),
                at: None,
            },
        )
        .await
        .unwrap();
    observer
        .line_of(r#""reason":"oneOff","secondsRemaining":120}"#)
        .await;
    let churn = observer.churn().await;

    observer.hold_the_plan(true).await;
    observer
        .clock
        .advance(SignedDuration::from_mins(2) + SignedDuration::from_secs(1));
    let jumped = Instant::now();
    tokio::time::sleep(held).await;
    observer.hold_the_plan(false).await;
    observer.state(ServerState::Stopping).await;
    let waited = jumped.elapsed();
    churn.abort();
    observer.stop_observer().await;

    assert!(
        waited > held,
        "the restart began {waited:?} after the clock passed its time, while its plan could not be written"
    );
    assert!(
        waited < held + longest_sleep + Duration::from_secs(1),
        "the restart began {waited:?} after the clock passed its time"
    );
}

/// The next diagnostics event that `wanted` accepts, skipping the others.
async fn diagnosed<T>(
    events: &mut broadcast::Receiver<DiagnosticsEvent>,
    wanted: impl Fn(DiagnosticsEvent) -> Option<T>,
) -> T {
    let found = timeout(WAIT, async {
        loop {
            if let Some(found) = wanted(events.recv().await.unwrap()) {
                return found;
            }
        }
    })
    .await;
    found.expect("the diagnostics never said it")
}

impl Observer {
    async fn type_line(&self, line: &str) {
        self.supervision
            .command(&self.owner, line, CONSOLE_ROUTE.0, CONSOLE_ROUTE.1)
            .await
            .unwrap();
    }

    async fn alert(&self, wanted: impl Fn(&Alert) -> bool) -> Alert {
        let mut alerts = self.diagnostics.alerts();
        let found = timeout(WAIT, alerts.wait_for(|list| list.alerts.iter().any(&wanted)))
            .await
            .map(|list| list.unwrap().alerts.iter().find(|alert| wanted(alert)).cloned());
        found.unwrap().unwrap()
    }
}

/// A shell that lives a few seconds, as a resource would start one.
fn shell_line() -> &'static str {
    if cfg!(windows) {
        "fake_spawn cmd /d /c \"ping -n 3 127.0.0.1 >nul\""
    } else {
        "fake_spawn /bin/sh -c \"sleep 2; true\""
    }
}

#[tokio::test]
async fn a_shell_the_server_starts_is_recorded_audited_noted_and_alerted() {
    let mut observer = Observer::new(false).await;
    let (_, mut events) = observer.diagnostics.follow().await.unwrap();
    observer.supervision.start_server(&observer.owner).await.unwrap();
    observer.state(ServerState::Online).await;

    observer.type_line(shell_line()).await;

    let shell = diagnosed(&mut events, |event| match event {
        DiagnosticsEvent::Process { process } if process.kind == ProcessKind::Shell => Some(process),
        _ => None,
    })
    .await;
    let note = observer
        .console_until("security alert: FXServer's tree started")
        .await;
    let alert = observer.alert(|alert| alert.kind == AlertKind::Shell).await;
    let ended = diagnosed(&mut events, |event| match event {
        DiagnosticsEvent::Process { process } if process.id == shell.id && process.ended_at.is_some() => {
            Some(process)
        }
        _ => None,
    })
    .await;

    assert_eq!(shell.generation, 1);
    assert!(
        shell
            .command_line
            .as_deref()
            .is_some_and(|line| line.contains('3') || line.contains('2'))
    );
    assert_eq!(
        (note.3, note.4.as_deref()),
        (LineKind::Observer, Some("observer"))
    );
    assert_eq!(alert.count, 1);
    assert!(ended.ended_at.is_some());
    // The shell, and the program it ran, which FXServer never starts either.
    let audited = observer.audited(AuditAction::TreeProcessStarted).await;
    let kinds: Vec<&str> = audited
        .iter()
        .map(|entry| entry.details["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"shell") && kinds.contains(&"other"), "{kinds:?}");
    assert!(audited.iter().all(|entry| entry.actor_kind == ActorKind::System));
    observer.stop_observer().await;
}

#[tokio::test]
async fn a_crash_dump_is_moved_recorded_analysed_and_published() {
    let mut observer = Observer::new(false).await;
    let (_, mut events) = observer.diagnostics.follow().await.unwrap();
    observer.supervision.start_server(&observer.owner).await.unwrap();
    observer.state(ServerState::Online).await;

    observer.type_line("fake_crash_dump 1").await;
    observer.type_line("fake_crash").await;

    let analysed = diagnosed(&mut events, |event| match event {
        DiagnosticsEvent::Crash { crash } if crash.analysis_state != AnalysisState::Pending => Some(crash),
        _ => None,
    })
    .await;
    let alert = observer
        .alert(|alert| {
            alert
                .crash
                .as_ref()
                .is_some_and(|crash| crash.exception.is_some())
        })
        .await;

    assert_eq!(analysed.kind, CrashKind::Crash);
    assert_eq!(analysed.analysis_state, AnalysisState::Done);
    assert_eq!(
        analysed.signature.as_deref(),
        Some("0xC0000005 citizen-scripting-lua.dll+0x1a2b3")
    );
    if cfg!(windows) {
        assert_eq!(analysed.exit_code.as_deref(), Some("0xFFFFFFFE"));
    }
    assert_eq!(analysed.files.len(), 1);
    assert!(analysed.files[0].kept);
    let details = alert.crash.unwrap();
    assert_eq!(details.exception.unwrap().code, "0xC0000005");
    assert_eq!(details.module.as_deref(), Some("citizen-scripting-lua.dll"));
    assert_eq!(details.thread.as_deref(), Some("UV loop: svMain"));
    observer.stop_observer().await;
}

#[tokio::test]
async fn a_hang_is_recorded_with_the_snapshot_taken_before_the_kill_and_audited() {
    let mut observer = Observer::new(false).await;
    let (_, mut events) = observer.diagnostics.follow().await.unwrap();
    observer.supervision.start_server(&observer.owner).await.unwrap();
    observer.state(ServerState::Online).await;

    observer.type_line("fake_hang").await;

    let hang = diagnosed(&mut events, |event| match event {
        DiagnosticsEvent::Crash { crash } if crash.analysis_state != AnalysisState::Pending => Some(crash),
        _ => None,
    })
    .await;
    observer.console_until("before killing it").await;

    assert_eq!(hang.kind, CrashKind::Hang);
    assert_eq!(hang.analysis_state, AnalysisState::Done, "{hang:?}");
    assert!(
        hang.signature
            .as_deref()
            .is_some_and(|signature| signature.starts_with("hang UV loop: svMain"))
    );
    let snapshot = if cfg!(windows) {
        CrashFileKind::HangDump
    } else {
        CrashFileKind::HangReport
    };
    assert!(
        hang.files.iter().any(|file| file.kind == snapshot && file.kept),
        "{:?}",
        hang.files
    );
    let audited = observer.audited(AuditAction::HangSnapshotTaken).await;
    assert_eq!(audited.len(), 1);
    assert_eq!(audited[0].outcome, Outcome::Success);
    assert_eq!(
        audited[0].details["kind"],
        if cfg!(windows) { "dump" } else { "report" }
    );
    observer.stop_observer().await;
}

#[tokio::test]
async fn a_connection_the_server_holds_reaches_the_topic_open_then_closed() {
    let mut observer = Observer::new(false).await;
    let (_, mut events) = observer.diagnostics.follow().await.unwrap();
    let peer = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = peer.local_addr().unwrap();
    observer.supervision.start_server(&observer.owner).await.unwrap();
    observer.state(ServerState::Online).await;

    observer.type_line(&format!("fake_connect {address}")).await;
    let _accepted = peer.accept().unwrap();
    let destination = |open: u32| {
        move |event: DiagnosticsEvent| match event {
            DiagnosticsEvent::Connections {
                destinations,
                listeners,
            } => destinations
                .into_iter()
                .find(|destination| destination.port == address.port() && destination.open == open)
                .map(|destination| (destination, listeners)),
            _ => None,
        }
    };
    let (open, listeners) = diagnosed(&mut events, destination(1)).await;
    observer.type_line("fake_disconnect").await;
    let (closed, _) = diagnosed(&mut events, destination(0)).await;

    assert_eq!(open.address, "127.0.0.1");
    assert_eq!(open.scope, Scope::Loopback);
    assert_eq!(open.protocol, Protocol::Tcp);
    assert_eq!(
        open.process,
        if cfg!(windows) { "FXServer.exe" } else { "FXServer" }
    );
    assert_eq!(closed.peak, 1);
    assert!(
        listeners
            .iter()
            .any(|listener| listener.role == Some(ListenerRole::Game)),
        "{listeners:?}"
    );
    observer.stop_observer().await;
}

#[tokio::test]
async fn a_started_server_has_its_cfg_noted_and_keeps_its_port_out_of_the_setup_check() {
    let mut observer = Observer::new(false).await;
    let data = observer.dir.path().join("server-data");
    fs::create_dir_all(data.join("resources/chat")).unwrap();
    fs::write(
        data.join("resources/chat/fxmanifest.lua"),
        "fx_version 'cerulean'\n",
    )
    .unwrap();
    let check = CfgCheck {
        data_path: data.to_str().unwrap().to_owned(),
        cfg_file: "server.cfg".to_owned(),
        onesync: OneSync::On,
    };

    observer.supervision.start_server(&observer.owner).await.unwrap();
    observer.state(ServerState::Online).await;
    let note = observer.console_until("server.cfg sets no licence key").await;
    let report = observer.supervision.check_cfg(check).await.unwrap();

    assert_eq!(
        (note.2, note.3, note.4.as_deref()),
        (1, LineKind::Observer, Some("observer"))
    );
    let endpoint = &report.facts.unwrap().endpoints[0].address;
    assert!(
        std::net::TcpListener::bind(endpoint).is_err(),
        "FXServer holds {endpoint}"
    );
    let kinds: Vec<_> = report.findings.iter().map(|finding| finding.kind).collect();
    assert_eq!(
        kinds,
        [FindingKind::LicenceMissing],
        "a running server's own ports are not in use"
    );
    observer.stop_observer().await;
}

/// Every file Observer wrote under `dir`, the databases and their write-ahead logs included,
/// holding `needle`. The builds folder holds the fake itself, which knows its players' address.
fn files_holding(dir: &Path, needle: &[u8]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for entry in fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.ends_with("artifacts") {
            continue;
        }
        if path.is_dir() {
            found.extend(files_holding(&path, needle));
        } else if fs::read(&path)
            .is_ok_and(|bytes| bytes.windows(needle.len()).any(|window| window == needle))
        {
            found.push(path);
        }
    }
    found
}

/// The statistics and the players service over `observer`'s data, as `observer run` starts them.
async fn follow_players(observer: &Observer) -> (Metrics, Sampling, Players, PlayersTask) {
    let path = observer.dir.path().join("metrics.sqlite3");
    let metrics = Metrics::open(MetricsSetup {
        path: path.clone(),
        clock: Clock::system(),
    })
    .await
    .unwrap();
    let (_sockets, sockets) = watch::channel(1);
    let sampling = metrics.start(Sources {
        supervision: observer.supervision.clone(),
        sockets,
        data_dir: observer.dir.path().to_path_buf(),
        databases: DatabaseFiles {
            state: observer.dir.path().join("observer.sqlite3"),
            metrics: path,
        },
    });
    let (players, playing) = Players::start(PlayersSetup {
        db: observer.db.clone(),
        metrics: metrics.clone(),
        clock: Clock::system(),
        supervision: observer.supervision.clone(),
        schedule: observer.schedule.status(),
        data_dir: observer.dir.path().join("observer-data"),
    })
    .await
    .unwrap();
    (metrics, sampling, players, playing)
}

#[tokio::test]
async fn players_are_confirmed_followed_and_credited_and_their_address_is_kept_nowhere() {
    let mut observer = Observer::new(false).await;
    let (metrics, sampling, players, playing) = follow_players(&observer).await;
    let confirmed = |count: usize| {
        let players = players.clone();
        async move {
            let deadline = Instant::now() + WAIT;
            loop {
                let online = players.online().await.unwrap();
                let done = online
                    .players
                    .iter()
                    .filter(|player| {
                        player.confirmation == Confirmation::Confirmed && player.player.is_some()
                    })
                    .count();
                if done == count && online.players.len() == count {
                    return online;
                }
                assert!(Instant::now() < deadline, "never {count} confirmed: {online:?}");
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
    };

    observer.supervision.start_server(&observer.owner).await.unwrap();
    observer.state(ServerState::Online).await;
    for line in ["fake_join Ada", "fake_join Grace"] {
        observer
            .supervision
            .command(&observer.owner, line, CONSOLE_ROUTE.0, CONSOLE_ROUTE.1)
            .await
            .unwrap();
    }
    let both = confirmed(2).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    observer
        .supervision
        .command(
            &observer.owner,
            "fake_drop 1 2 - Exiting",
            CONSOLE_ROUTE.0,
            CONSOLE_ROUTE.1,
        )
        .await
        .unwrap();
    let one = confirmed(1).await;
    let deadline = Instant::now() + WAIT;
    let page = loop {
        let page = players
            .page(None, PlayerSort::PlayTime, None, Permissions::all())
            .await
            .unwrap();
        if page.players.iter().any(|player| player.sessions == 1) {
            break page;
        }
        assert!(
            Instant::now() < deadline,
            "Ada's session was never credited: {page:?}"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    let grace = one.players[0].player.unwrap();
    let profile = players.profile(grace, Permissions::all()).await.unwrap();
    observer.supervision.stop_server(&observer.owner).await.unwrap();
    observer.state(ServerState::Stopped).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let after_stop = players.profile(grace, Permissions::all()).await.unwrap();
    playing.close().await;
    sampling.close().await;
    metrics.close().await.unwrap();
    observer.stop_observer().await;

    assert_eq!(both.players[0].name.as_deref(), Some("Ada"));
    assert_eq!(page.players.len(), 2);
    let ada = page.players.iter().find(|player| player.name == "Ada").unwrap();
    assert_eq!(ada.sessions, 1);
    assert!(ada.play_ms >= 2000, "{ada:?}");
    assert!(profile.online);
    assert!(profile.in_progress_ms > 0);
    let kinds: Vec<_> = profile
        .identifiers
        .unwrap()
        .iter()
        .map(|shown| shown.kind)
        .collect();
    assert!(kinds.contains(&IdentifierKind::Ip), "{kinds:?}");
    assert!(kinds.contains(&IdentifierKind::Token), "{kinds:?}");
    assert_eq!(after_stop.sessions, 1);
    assert!(!after_stop.online);
    let shown = serde_json::to_string(&(&both, &page, &profile_json(&after_stop))).unwrap();
    assert!(!shown.contains("192.0.2.77"));
    assert_eq!(
        files_holding(observer.dir.path(), b"192.0.2.77"),
        Vec::<PathBuf>::new()
    );
}

#[tokio::test]
async fn a_banned_player_is_refused_at_connection_and_dropped_when_banned_while_playing() {
    let mut observer = Observer::new(false).await;
    let (bans, banning) = Bans::start(BansSetup {
        db: observer.db.clone(),
        clock: Clock::system(),
        supervision: observer.supervision.clone(),
        schedule: observer.schedule.status(),
    });
    let (_admission, admitting) = Admission::start(AdmissionSetup {
        db: observer.db.clone(),
        clock: Clock::system(),
        supervision: observer.supervision.clone(),
        bans: bans.clone(),
        schedule: observer.schedule.status(),
    })
    .await
    .unwrap();
    let cheater = format!("license:{}", "a".repeat(40));
    let ada = format!("license:{}", "b".repeat(40));
    let ban = |license: &str| BanRequest {
        player: None,
        identifiers: vec![license.to_owned()],
        reason: "Aimbot".to_owned(),
        minutes: None,
    };
    let first = bans.ban(&observer.owner, ban(&cheater)).await.unwrap();
    let send = |line: String| {
        let observer = &observer;
        async move {
            observer
                .supervision
                .command(&observer.owner, &line, CONSOLE_ROUTE.0, CONSOLE_ROUTE.1)
                .await
                .unwrap();
        }
    };

    observer.supervision.start_server(&observer.owner).await.unwrap();
    observer.state(ServerState::Online).await;
    send(format!("fake_connecting Cheater {cheater}")).await;
    let refused = observer.console_until("refused ").await;
    let refused_event = observer.console_until("observer:playerRefused").await;
    send(format!("fake_connecting Ada {ada}")).await;
    observer.console_until("Player Ada connected").await;
    let second = bans.ban(&observer.owner, ban(&ada)).await.unwrap();
    let banned_event = observer.console_until("observer:playerBanned").await;
    let listed = bans
        .page(BanFilter::Active, None, None, Permissions::all())
        .await
        .unwrap();
    observer.supervision.stop_server(&observer.owner).await.unwrap();
    observer.state(ServerState::Stopped).await;
    admitting.close().await;
    banning.close().await;
    observer.stop_observer().await;

    let message = format!(
        "You are banned from this server.
Reason: Aimbot
Ends: never
Ban ID: {}",
        first.code
    );
    assert!(
        plain(&refused.5).contains(&json!(message).to_string()),
        "{}",
        refused.5
    );
    assert!(
        plain(&refused_event.5).contains(&first.code),
        "{}",
        refused_event.5
    );
    assert!(
        plain(&refused_event.5).contains("\"cause\":\"ban\""),
        "{}",
        refused_event.5
    );
    assert!(
        plain(&banned_event.5).contains(&second.code),
        "{}",
        banned_event.5
    );
    assert!(
        plain(&banned_event.5).contains("\"player\":1"),
        "{}",
        banned_event.5
    );
    assert!(
        !observer
            .console()
            .iter()
            .any(|line| plain(&line.5).contains("Player Cheater connected")),
        "the cheater got in"
    );
    let refusals: Vec<(String, u32)> = listed
        .bans
        .iter()
        .map(|ban| (ban.code.clone(), ban.refused))
        .collect();
    assert_eq!(refusals, [(second.code.clone(), 1), (first.code.clone(), 1)]);
}

#[tokio::test]
async fn the_allowlist_refuses_a_player_until_their_request_is_approved() {
    let mut observer = Observer::new(false).await;
    let (admission, admitting, banning) = admit(&observer).await;
    admission
        .change_allowlist_settings(
            &observer.owner,
            AllowlistSettings {
                enabled: true,
                message: "Approved players only. Ask the staff: {id}".to_owned(),
            },
        )
        .await
        .unwrap();
    let ada = format!("license:{}", "c".repeat(40));
    let send = |line: String| {
        let observer = &observer;
        async move {
            observer
                .supervision
                .command(&observer.owner, &line, CONSOLE_ROUTE.0, CONSOLE_ROUTE.1)
                .await
                .unwrap();
        }
    };

    observer.supervision.start_server(&observer.owner).await.unwrap();
    observer.state(ServerState::Online).await;
    send(format!("fake_connecting Ada {ada}")).await;
    let refused = observer.console_until("refused ").await;
    let refused_event = observer.console_until("observer:playerRefused").await;
    let waiting = admission.requests(None, None).await.unwrap().requests;
    let got_in_before = observer
        .console()
        .iter()
        .any(|line| plain(&line.5).contains("Player Ada connected"));
    admission
        .approve_request(&observer.owner, &waiting[0].code)
        .await
        .unwrap();
    send(format!("fake_connecting Ada {ada}")).await;
    observer.console_until("Player Ada connected").await;
    observer.supervision.stop_server(&observer.owner).await.unwrap();
    observer.state(ServerState::Stopped).await;
    admitting.close().await;
    banning.close().await;
    observer.stop_observer().await;

    let code = &waiting[0].code;
    assert_eq!((waiting.len(), waiting[0].license.as_str()), (1, ada.as_str()));
    assert_eq!(waiting[0].name.as_deref(), Some("Ada"));
    let message = json!(format!("Approved players only. Ask the staff: {code}")).to_string();
    assert!(plain(&refused.5).contains(&message), "{}", refused.5);
    for part in [
        "\"cause\":\"allowlist\"".to_owned(),
        format!("\"request\":\"{code}\""),
        format!("\"license\":\"{ada}\""),
    ] {
        assert!(plain(&refused_event.5).contains(&part), "{}", refused_event.5);
    }
    assert!(!got_in_before, "Ada got in before her request was approved");
}

/// What the server's game port answers at `path`, as FiveM's server list reads it. The fake
/// closes the connection after each answer.
fn served(port: u16, path: &str) -> serde_json::Value {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream.set_read_timeout(Some(WAIT)).unwrap();
    write!(stream, "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let body = response.split_once("\r\n\r\n").map_or("", |(_, body)| body);
    serde_json::from_str(body).unwrap()
}

/// The names FiveM's server list reads from the server: its project name and its host name.
fn listed_names(port: u16) -> (String, String) {
    let name = |value: &serde_json::Value| value.as_str().unwrap_or_default().to_owned();
    (
        name(&served(port, "/info.json")["vars"]["sv_projectName"]),
        name(&served(port, "/dynamic.json")["hostname"]),
    )
}

/// The bans and the admission over `observer`'s data, as `observer run` starts them.
async fn admit(observer: &Observer) -> (Admission, AdmissionTask, BansTask) {
    let (bans, banning) = Bans::start(BansSetup {
        db: observer.db.clone(),
        clock: Clock::system(),
        supervision: observer.supervision.clone(),
        schedule: observer.schedule.status(),
    });
    let (admission, admitting) = Admission::start(AdmissionSetup {
        db: observer.db.clone(),
        clock: Clock::system(),
        supervision: observer.supervision.clone(),
        bans,
        schedule: observer.schedule.status(),
    })
    .await
    .unwrap();
    (admission, admitting, banning)
}

/// What the resources heard of maintenance in the run `generation`, the latest last.
fn heard_of_maintenance(observer: &Observer, generation: u32) -> Vec<String> {
    observer
        .console()
        .into_iter()
        .filter(|line| line.2 == generation)
        .map(|line| plain(&line.5))
        .filter(|line| line.contains("observer:maintenance "))
        .collect()
}

/// Waits until `met`, or fails saying `what` never came.
async fn eventually(what: &str, met: impl Fn() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !met() {
        assert!(Instant::now() < deadline, "{what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn maintenance_refuses_at_connection_and_tells_the_resources_at_each_start() {
    let mut observer = Observer::new(false).await;
    let (admission, admitting, banning) = admit(&observer).await;
    let (ada, grace) = (
        format!("license:{}", "c".repeat(40)),
        format!("license:{}", "d".repeat(40)),
    );
    admission
        .add_pass(
            &observer.owner,
            AddPass {
                license: grace.clone(),
                note: None,
            },
        )
        .await
        .unwrap();
    let send = |line: String| {
        let observer = &observer;
        async move {
            observer
                .supervision
                .command(&observer.owner, &line, CONSOLE_ROUTE.0, CONSOLE_ROUTE.1)
                .await
                .unwrap();
        }
    };
    let switch = |on: bool| admission.set_maintenance(&observer.owner, SetMaintenance { on });

    observer.supervision.start_server(&observer.owner).await.unwrap();
    observer.state(ServerState::Online).await;
    let before = heard_of_maintenance(&observer, 1);
    switch(true).await.unwrap();
    observer.console_until("observer:maintenance ").await;
    let since = admission.status().borrow().maintenance.since.unwrap();
    send(format!("fake_connecting Ada {ada}")).await;
    let refused = observer.console_until("refused ").await;
    let refused_event = observer.console_until("observer:playerRefused").await;
    send(format!("fake_connecting Grace {grace}")).await;
    observer.console_until("Player Grace connected").await;
    observer
        .supervision
        .restart_server(&observer.owner)
        .await
        .unwrap();
    observer
        .until(|status| status.generation == 2 && status.state == ServerState::Online)
        .await;
    eventually("the second start heard nothing of maintenance", || {
        !heard_of_maintenance(&observer, 2).is_empty()
    })
    .await;
    switch(false).await.unwrap();
    eventually("the end of maintenance was not raised", || {
        heard_of_maintenance(&observer, 2).len() == 2
    })
    .await;
    send(format!("fake_connecting Ada {ada}")).await;
    observer.console_until("Player Ada connected").await;
    let (first, second) = (
        heard_of_maintenance(&observer, 1),
        heard_of_maintenance(&observer, 2),
    );
    let renames = observer
        .console()
        .iter()
        .filter(|line| plain(&line.5).contains("observer_cmd serverName"))
        .count();
    observer.supervision.stop_server(&observer.owner).await.unwrap();
    observer.state(ServerState::Stopped).await;
    admitting.close().await;
    banning.close().await;
    observer.stop_observer().await;

    assert_eq!(before, [] as [String; 0]);
    let turned_on = format!("\"at\":{},\"by\":\"ada\",\"on\":true", since.as_millisecond());
    assert_eq!(first.len(), 1, "{first:?}");
    assert!(first[0].contains(&turned_on), "{first:?}");
    assert_eq!(second.len(), 2, "{second:?}");
    assert!(second[0].contains(&turned_on), "{second:?}");
    assert!(second[1].contains("\"on\":false"), "{second:?}");
    let message = json!(MaintenanceSettings::default().message).to_string();
    assert!(plain(&refused.5).contains(&message), "{}", refused.5);
    for part in [
        "\"cause\":\"maintenance\"".to_owned(),
        format!("\"license\":\"{ada}\""),
    ] {
        assert!(plain(&refused_event.5).contains(&part), "{}", refused_event.5);
    }
    assert_eq!(
        renames, 1,
        "without a maintenance name, only its end gives a name back"
    );
}

#[tokio::test]
async fn the_server_carries_its_maintenance_name_in_the_list_and_gets_its_own_back() {
    let mut observer = Observer::new(false).await;
    let (admission, admitting, banning) = admit(&observer).await;
    admission
        .change_maintenance_settings(
            &observer.owner,
            MaintenanceSettings {
                server_name: Some("Observer lab | maintenance".to_owned()),
                ..MaintenanceSettings::default()
            },
        )
        .await
        .unwrap();
    let switch = |on: bool| admission.set_maintenance(&observer.owner, SetMaintenance { on });
    let maintenance = (
        "Observer lab | maintenance".to_owned(),
        "Observer lab | maintenance".to_owned(),
    );

    observer.supervision.start_server(&observer.owner).await.unwrap();
    let port = observer.state(ServerState::Online).await.port.unwrap();
    let own = listed_names(port);
    switch(true).await.unwrap();
    eventually("the server kept its own name", || {
        listed_names(port) == maintenance
    })
    .await;
    observer
        .supervision
        .restart_server(&observer.owner)
        .await
        .unwrap();
    observer
        .until(|status| status.generation == 2 && status.state == ServerState::Online)
        .await;
    eventually("the second start kept its own name", || {
        listed_names(port) == maintenance
    })
    .await;
    // `/info.json` is read once a run, after the rename as likely as before it.
    tokio::time::sleep(Duration::from_secs(6)).await;
    let shown = observer.supervision.status().borrow().name.clone();
    admission
        .change_maintenance_settings(
            &observer.owner,
            MaintenanceSettings {
                server_name: Some("Back at 8".to_owned()),
                ..MaintenanceSettings::default()
            },
        )
        .await
        .unwrap();
    eventually(
        "a name changed during maintenance did not reach the server",
        || listed_names(port).0 == "Back at 8",
    )
    .await;
    switch(false).await.unwrap();
    eventually("the server kept its maintenance name", || {
        listed_names(port) == own
    })
    .await;
    observer.supervision.stop_server(&observer.owner).await.unwrap();
    observer.state(ServerState::Stopped).await;
    admitting.close().await;
    banning.close().await;
    observer.stop_observer().await;

    assert_eq!(
        own,
        (
            "^2Observer ^7lab".to_owned(),
            "FXServer, but unconfigured".to_owned()
        )
    );
    assert_eq!(shown.as_deref(), Some("Observer lab"));
}

#[tokio::test]
async fn the_map_places_the_connected_players_while_someone_watches_it() {
    let mut observer = Observer::new(false).await;
    let (metrics, sampling, players, playing) = follow_players(&observer).await;
    let (map, mapping) = LiveMap::start(MapSetup {
        db: observer.db.clone(),
        clock: Clock::system(),
        supervision: observer.supervision.clone(),
        players: players.clone(),
        data_dir: observer.dir.path().join("observer-data"),
    });
    observer.supervision.start_server(&observer.owner).await.unwrap();
    observer.state(ServerState::Online).await;
    for line in ["fake_join Ada", "fake_move 1 412.6 -1208.3 32.1"] {
        observer
            .supervision
            .command(&observer.owner, line, CONSOLE_ROUTE.0, CONSOLE_ROUTE.1)
            .await
            .unwrap();
    }
    confirmed(&players, 1).await;

    let (_, mut live, watching) = map.watch();
    let placed = timeout(WAIT, async {
        loop {
            live.changed().await.unwrap();
            let current = live.borrow_and_update().clone();
            if !current.players.is_empty() {
                return current;
            }
        }
    })
    .await
    .expect("no position came while the map was watched");
    drop(watching);
    observer.supervision.stop_server(&observer.owner).await.unwrap();
    observer.state(ServerState::Stopped).await;
    mapping.close().await;
    playing.close().await;
    sampling.close().await;
    metrics.close().await.unwrap();
    observer.stop_observer().await;

    let ada = &placed.players[0];
    assert_eq!(ada.name.as_deref(), Some("Ada"));
    assert!(ada.player.is_some());
    assert_eq!((ada.x, ada.y, ada.z), (412.6, -1208.3, 32.1));
    assert!(placed.onesync);
    assert_eq!(placed.waiting, 0);
    assert!(
        !observer
            .console()
            .iter()
            .any(|line| plain(&line.5).contains("observer_cmd positions")),
        "the asks for positions fill the console"
    );
}

#[tokio::test]
async fn a_player_gets_in_when_observer_does_not_answer_in_time() {
    let mut observer =
        Observer::launched(false, Duration::from_millis(50), &[("fake_verdict_ms", "300")]).await;

    observer.supervision.start_server(&observer.owner).await.unwrap();
    observer.state(ServerState::Online).await;
    observer
        .supervision
        .command(
            &observer.owner,
            "fake_connecting Ada",
            CONSOLE_ROUTE.0,
            CONSOLE_ROUTE.1,
        )
        .await
        .unwrap();
    observer.console_until("Player Ada connected").await;
    observer.supervision.stop_server(&observer.owner).await.unwrap();
    observer.state(ServerState::Stopped).await;
    observer.stop_observer().await;
}

fn profile_json(profile: &PlayerProfile) -> serde_json::Value {
    serde_json::to_value(profile).unwrap()
}

/// Waits until `count` players are online, every one of them confirmed.
async fn confirmed(players: &Players, count: usize) {
    let deadline = Instant::now() + WAIT;
    loop {
        let online = players.online().await.unwrap();
        let done = online
            .players
            .iter()
            .filter(|player| player.confirmation == Confirmation::Confirmed && player.player.is_some())
            .count();
        if done == count && online.players.len() == count {
            return;
        }
        assert!(Instant::now() < deadline, "never {count} confirmed: {online:?}");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

#[tokio::test]
async fn a_label_a_resource_gives_shows_online_stays_on_the_profile_and_finds_the_player() {
    let mut observer = Observer::new(false).await;
    let (metrics, sampling, players, playing) = follow_players(&observer).await;
    let (supervision, owner) = (observer.supervision.clone(), observer.owner.clone());
    let command = |line: &'static str| {
        let (supervision, owner) = (supervision.clone(), owner.clone());
        async move {
            supervision
                .command(&owner, line, CONSOLE_ROUTE.0, CONSOLE_ROUTE.1)
                .await
                .unwrap();
        }
    };
    observer.supervision.start_server(&observer.owner).await.unwrap();
    observer.state(ServerState::Online).await;

    command("fake_join Ada").await;
    command("fake_label 1 CHAR-1234 John Doe").await;
    confirmed(&players, 1).await;
    let deadline = Instant::now() + WAIT;
    let found = loop {
        let found = players
            .page(
                Some("char-1234"),
                PlayerSort::PlayTime,
                None,
                Permissions::PLAYERS_VIEW,
            )
            .await
            .unwrap();
        if !found.players.is_empty() {
            break found;
        }
        assert!(Instant::now() < deadline, "the label was never kept");
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    let online = players.online().await.unwrap();
    command("fake_label 1").await;
    let deadline = Instant::now() + WAIT;
    while players.online().await.unwrap().players[0].label.is_some() {
        assert!(Instant::now() < deadline, "the label was never cleared");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let profile = players
        .profile(found.players[0].id, Permissions::PLAYERS_VIEW)
        .await
        .unwrap();
    observer.supervision.stop_server(&observer.owner).await.unwrap();
    observer.state(ServerState::Stopped).await;
    playing.close().await;
    sampling.close().await;
    metrics.close().await.unwrap();
    observer.stop_observer().await;

    let john = PlayerLabel {
        name: Some("John Doe".to_owned()),
        id: Some("CHAR-1234".to_owned()),
        resource: Some("fake".to_owned()),
    };
    assert_eq!(online.players[0].name.as_deref(), Some("Ada"));
    assert_eq!(online.players[0].label.as_ref(), Some(&john));
    assert_eq!(found.players[0].name, "Ada");
    assert_eq!(found.players[0].label.as_ref(), Some(&john));
    assert_eq!(profile.name, "Ada");
    assert_eq!(profile.labels.len(), 1);
    assert_eq!(profile.labels[0].id.as_deref(), Some("CHAR-1234"));
}

#[tokio::test]
async fn a_crash_and_a_kick_reach_the_drops_with_their_signature_and_reason_group() {
    let mut observer = Observer::new(false).await;
    let (metrics, sampling, players, playing) = follow_players(&observer).await;
    observer.supervision.start_server(&observer.owner).await.unwrap();
    observer.state(ServerState::Online).await;
    for line in ["fake_join Ada", "fake_join Grace"] {
        observer
            .supervision
            .command(&observer.owner, line, CONSOLE_ROUTE.0, CONSOLE_ROUTE.1)
            .await
            .unwrap();
    }
    confirmed(&players, 2).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    for line in [
        "fake_drop 1 2 - Game crashed: kilo-ohio-lemon (GTA5_b3095.exe+1A2B3C)",
        "fake_drop 2 1 afk AFK for 31 minutes",
    ] {
        observer
            .supervision
            .command(&observer.owner, line, CONSOLE_ROUTE.0, CONSOLE_ROUTE.1)
            .await
            .unwrap();
    }
    confirmed(&players, 0).await;
    // The players' flush writes the closures within a second; the drops' first round then
    // classifies them.
    tokio::time::sleep(Duration::from_secs(2)).await;
    let (drops, dropping) = Drops::start(DropsSetup {
        db: observer.db.clone(),
        metrics: metrics.clone(),
        clock: Clock::system(),
        schedule: observer.schedule.status(),
        players: players.clone(),
    });
    let deadline = Instant::now() + WAIT;
    let log = loop {
        let log = drops.log(&LogQuery::default(), Permissions::all()).await.unwrap();
        if log.rows.iter().any(|row| row.signature.is_some()) {
            break log;
        }
        assert!(
            Instant::now() < deadline,
            "the crash was never classified: {log:?}"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    let signatures = drops
        .signatures(DropsRange::Week, Permissions::all())
        .await
        .unwrap();
    let resources = drops
        .resources(DropsRange::Week, Permissions::all())
        .await
        .unwrap();
    let summary = drops
        .summary(DropsRange::Week, Permissions::DROPS_VIEW)
        .await
        .unwrap();
    dropping.close().await;
    observer.supervision.stop_server(&observer.owner).await.unwrap();
    observer.state(ServerState::Stopped).await;
    playing.close().await;
    sampling.close().await;
    metrics.close().await.unwrap();
    observer.stop_observer().await;

    let crash = log
        .rows
        .iter()
        .find(|row| row.category == DropCategory::Crash)
        .unwrap();
    assert_eq!(crash.signature.as_deref(), Some("hash:GTA5_b3095.exe+1A2B3C"));
    assert_eq!(crash.label.as_deref(), Some("kilo-ohio-lemon"));
    assert_eq!(
        crash.player.as_ref().and_then(|player| player.name.as_deref()),
        Some("Ada")
    );
    let entry = &signatures.signatures[0];
    assert_eq!(
        (entry.module.as_deref(), entry.game_build),
        (Some("GTA5.exe"), Some(3095))
    );
    assert_eq!((entry.drops, entry.players), (1, 1));
    let afk = &resources.resources[0];
    assert_eq!(afk.resource.as_deref(), Some("afk"));
    assert_eq!(afk.reasons.as_ref().unwrap()[0].template, "AFK for {n} minutes");
    let crashes = summary
        .tiles
        .iter()
        .find(|tile| tile.kind == TileKind::Crashes)
        .unwrap();
    assert_eq!(crashes.drops, 1);
}

/// What the resources tests start with: a library, one resource that depends on it, one the cfg
/// does not start, one FXServer cannot start and one it warns about, listed two a page.
const RESOURCES_CFG: &str = "set fake_resource_page 2\nset fake_fail_start mechanic\nset fake_heavy docks\nset fake_missing docks\nensure ox_lib\nensure ox_target\nensure mechanic\nensure docks\n";

impl Observer {
    /// A resource of the server folder, under `resources`, with its manifest.
    fn resource(&self, folder: &str, manifest: &str) {
        let path = self.dir.path().join("server-data/resources").join(folder);
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("fxmanifest.lua"), manifest).unwrap();
    }

    /// An Observer whose server folder holds the resources of [`RESOURCES_CFG`], not started yet.
    async fn with_resources() -> Self {
        let observer = Self::new(false).await;
        observer.resource("[core]/ox_lib", "fx_version 'cerulean'\nversion '3.30.6'\n");
        observer.resource(
            "[core]/[ox]/ox_target",
            "version '1.17.2'\ndependencies {\n    '/onesync',\n    'ox_lib',\n}\n",
        );
        observer.resource("[jobs]/taxi", "fx_version 'cerulean'\n");
        observer.resource("[jobs]/mechanic", "version '0.8.0'\n");
        observer.resource("[maps]/docks", "version '0.9'\n");
        let cfg = observer.dir.path().join("server-data/server.cfg");
        let mut text = fs::read_to_string(&cfg).unwrap();
        text.push_str(RESOURCES_CFG);
        fs::write(cfg, text).unwrap();
        observer
    }

    async fn resources_until(&self, wanted: impl Fn(&ResourcesView) -> bool) -> ResourcesView {
        let mut view = self.supervision.resources();
        let found = timeout(WAIT, view.wait_for(|view| wanted(view)))
            .await
            .map(|found| found.unwrap().clone());
        found.unwrap_or_else(|_| panic!("the resources never came to that; last {:#?}", view.borrow()))
    }

    /// Starts the server and waits for its resources to be listed.
    async fn listed(&self) -> ResourcesView {
        self.supervision.start_server(&self.owner).await.unwrap();
        self.resources_until(|view| view.stage == ResourcesStage::Listed)
            .await
    }
}

fn row<'a>(view: &'a ResourcesView, name: &str) -> &'a ResourceRow {
    view.resources
        .iter()
        .find(|row| row.name == name)
        .unwrap_or_else(|| panic!("no resource {name} in {view:#?}"))
}

fn is(view: &ResourcesView, name: &str, state: ResourceState) -> bool {
    view.resources
        .iter()
        .any(|row| row.name == name && row.state == state)
}

#[tokio::test]
async fn the_resources_of_a_running_server_are_listed_with_what_fxserver_says_of_each() {
    let mut observer = Observer::with_resources().await;
    let before = observer.supervision.resources().borrow().clone();

    let view = observer.listed().await;

    assert_eq!(before, ResourcesView::default(), "no server, no resource");
    assert_eq!((view.generation, view.truncated, &view.rescan), (1, false, &None));
    let rows: Vec<(&str, &str, ResourceState, Option<&str>)> = view
        .resources
        .iter()
        .map(|row| {
            (
                row.name.as_str(),
                row.folder.as_str(),
                row.state,
                row.version.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            ("docks", "[maps]", ResourceState::Started, Some("0.9")),
            ("mechanic", "[jobs]", ResourceState::Stopped, Some("0.8.0")),
            ("ox_lib", "[core]", ResourceState::Started, Some("3.30.6")),
            ("ox_target", "[core]/[ox]", ResourceState::Started, Some("1.17.2")),
            ("taxi", "[jobs]", ResourceState::Stopped, None),
        ],
        "every resource of the folder, in two pages and a half, and the bridge left out"
    );
    for listed in &view.resources {
        assert_eq!(listed.origin, ResourceOrigin::Server, "{}", listed.name);
        assert!(listed.controllable, "{}", listed.name);
        assert_eq!(
            listed.last, None,
            "{}: nothing happened since the cfg ran",
            listed.name
        );
    }
    assert_eq!(row(&view, "ox_lib").dependants, ["ox_target"]);
    assert_eq!(row(&view, "ox_target").dependants, Vec::<String>::new());
    assert_eq!(
        row(&view, "docks").warnings,
        Some(ResourceWarnings {
            count: 3,
            heavy: 2,
            oversized: 1,
            missing: 1
        })
    );
    assert!(
        row(&view, "mechanic").failed_at.is_some(),
        "FXServer said it could not start it"
    );
    assert_eq!(row(&view, "taxi").failed_at, None, "the cfg never started it");

    observer.supervision.stop_server(&observer.owner).await.unwrap();
    let stopped = observer
        .resources_until(|view| view.stage == ResourcesStage::Stopped)
        .await;
    assert_eq!(stopped.resources, Vec::new());
    observer.stop_observer().await;
}

/// What last happened to a resource: how, who asked for it, and the resource it stopped with.
fn last(row: &ResourceRow) -> (LiveChangeKind, Option<String>, Option<String>) {
    let last = row.last.clone().unwrap();
    (last.kind, last.by, last.with)
}

#[tokio::test]
async fn a_stop_takes_what_depends_on_the_resource_and_a_start_brings_what_it_needs() {
    let mut observer = Observer::with_resources().await;
    observer.listed().await;
    let owner = observer.owner.clone();
    let ada = Some("ada".to_owned());

    let stopped = observer
        .supervision
        .stop_resource(&owner, "ox_lib")
        .await
        .unwrap();
    let view = observer
        .resources_until(|view| is(view, "ox_lib", ResourceState::Stopped))
        .await;

    assert_eq!(
        stopped,
        ResourceAck {
            delivery: Delivery::Written,
            dependants: vec!["ox_target".to_owned()]
        }
    );
    let (lib, target) = (row(&view, "ox_lib"), row(&view, "ox_target"));
    assert_eq!(
        target.state,
        ResourceState::Stopped,
        "FXServer stopped it with ox_lib"
    );
    assert_eq!(last(lib), (LiveChangeKind::Stopped, ada.clone(), None));
    assert_eq!(lib.asked, None, "FXServer answered the order");
    assert_eq!(
        last(target),
        (LiveChangeKind::Stopped, None, Some("ox_lib".to_owned()))
    );
    let echo = observer.console_until("stop ox_lib").await;
    assert_eq!((echo.3, echo.4.as_deref()), (LineKind::Command, Some("ada")));
    let audited = observer.audited(AuditAction::ResourceStopped).await;
    assert_eq!(audited.len(), 1);
    assert_eq!(
        (audited[0].target.as_deref(), &audited[0].details),
        (
            Some("ox_lib"),
            &json!({ "command": "stop ox_lib", "dependants": ["ox_target"] })
        )
    );

    // FXServer starts what a resource depends on before the resource.
    observer
        .supervision
        .start_resource(&owner, "ox_target")
        .await
        .unwrap();
    let view = observer
        .resources_until(|view| is(view, "ox_target", ResourceState::Started))
        .await;

    assert_eq!(row(&view, "ox_lib").state, ResourceState::Started);
    assert_eq!(
        last(row(&view, "ox_target")),
        (LiveChangeKind::Started, ada, None)
    );
    assert_eq!(row(&view, "ox_lib").dependants, ["ox_target"]);
    let audited = observer.audited(AuditAction::ResourceStarted).await;
    assert_eq!(audited.len(), 1);
    assert_eq!(audited[0].details, json!({ "command": "start ox_target" }));
    observer.stop_observer().await;
}

#[tokio::test]
async fn a_restart_starts_again_what_depends_on_the_resource_when_asked_to() {
    let mut observer = Observer::with_resources().await;
    observer.listed().await;
    let ada = Some("ada".to_owned());

    let restarted = observer
        .supervision
        .restart_resource(&observer.owner, "ox_lib", true)
        .await
        .unwrap();
    let view = observer
        .resources_until(|view| {
            let lib = row(view, "ox_lib");
            let back = lib.state == ResourceState::Started
                && lib
                    .last
                    .as_ref()
                    .is_some_and(|last| last.kind == LiveChangeKind::Restarted);
            // Its stop shows first when the two lines fall in two reports of the bridge.
            back && row(view, "ox_target")
                .last
                .as_ref()
                .is_some_and(|last| last.kind != LiveChangeKind::Stopped)
        })
        .await;

    assert_eq!(restarted.dependants, ["ox_target"]);
    assert_eq!(
        last(row(&view, "ox_lib")),
        (LiveChangeKind::Restarted, ada.clone(), None)
    );
    let target = row(&view, "ox_target");
    assert_eq!(target.state, ResourceState::Started);
    assert_eq!(
        target.last.as_ref().unwrap().by,
        ada,
        "started again by the same order"
    );
    let restart = observer.line_of("restart ox_lib").await;
    let again = observer.console_until("start ox_target").await;
    assert!(restart < again.0, "the resource first, then what depends on it");
    assert_eq!((again.3, again.4.as_deref()), (LineKind::Command, Some("ada")));
    let audited = observer.audited(AuditAction::ResourceRestarted).await;
    assert_eq!(
        audited[0].details,
        json!({ "command": "restart ox_lib", "dependants": ["ox_target"], "startedAgain": true })
    );
    assert_eq!(
        observer.audited(AuditAction::ResourceStarted).await.len(),
        0,
        "one order, one entry"
    );
    observer.stop_observer().await;
}

#[tokio::test]
async fn a_restart_alone_leaves_stopped_what_fxserver_stopped_with_the_resource() {
    let mut observer = Observer::with_resources().await;
    observer.listed().await;

    observer
        .supervision
        .restart_resource(&observer.owner, "ox_lib", false)
        .await
        .unwrap();
    let view = observer
        .resources_until(|view| {
            row(view, "ox_lib")
                .last
                .as_ref()
                .is_some_and(|last| last.kind == LiveChangeKind::Restarted)
        })
        .await;

    assert_eq!(row(&view, "ox_lib").state, ResourceState::Started);
    let target = row(&view, "ox_target");
    assert_eq!(target.state, ResourceState::Stopped);
    assert_eq!(
        target.last.as_ref().and_then(|last| last.with.as_deref()),
        Some("ox_lib")
    );
    assert_eq!(row(&view, "ox_lib").dependants, Vec::<String>::new());
    observer.stop_observer().await;
}

#[tokio::test]
async fn an_order_names_only_a_resource_fxserver_listed_and_never_the_bridge() {
    let mut observer = Observer::with_resources().await;
    observer.resource("bad name", "version '1'\n");
    let cfg = observer.dir.path().join("server-data/server.cfg");
    let mut text = fs::read_to_string(&cfg).unwrap();
    text.push_str("ensure \"bad name\"\n");
    fs::write(cfg, text).unwrap();
    let owner = observer.owner.clone();
    let refused = |result: Result<ResourceAck, Error>| result.unwrap_err().code();

    assert_eq!(
        refused(observer.supervision.stop_resource(&owner, "ox_lib").await),
        Some(ErrorCode::ServerNotRunning)
    );
    assert_eq!(
        refused(observer.supervision.rescan_resources(&owner).await),
        Some(ErrorCode::ServerNotRunning)
    );
    let view = observer.listed().await;
    let started = plain(&observer.console_until("Started resource sys_").await.5);
    let bridge = started.rsplit(' ').next().unwrap().to_owned();

    assert!(!row(&view, "bad name").controllable);
    assert!(
        view.resources.iter().all(|listed| listed.name != bridge),
        "the bridge is Observer's own"
    );
    for name in [
        bridge.as_str(),
        &bridge.to_ascii_uppercase(),
        "ghost",
        "OX_LIB",
        "ox_lib;quit",
    ] {
        assert_eq!(
            refused(observer.supervision.stop_resource(&owner, name).await),
            Some(ErrorCode::ResourceNotFound),
            "{name}"
        );
        assert_eq!(
            refused(observer.supervision.restart_resource(&owner, name, true).await),
            Some(ErrorCode::ResourceNotFound),
            "{name}"
        );
    }
    assert_eq!(
        refused(observer.supervision.stop_resource(&owner, "bad name").await),
        Some(ErrorCode::ResourceNameRefused)
    );
    for action in [AuditAction::ResourceStopped, AuditAction::ResourceRestarted] {
        assert_eq!(observer.audited(action).await.len(), 0, "{action:?}");
    }
    assert!(
        observer.console().iter().all(|line| line.3 != LineKind::Command),
        "nothing reached the console"
    );
    observer.stop_observer().await;
}

#[tokio::test]
async fn an_order_is_taken_for_the_state_it_needs_and_one_at_a_time_for_a_resource() {
    let mut observer = Observer::with_resources().await;
    observer.listed().await;
    let owner = observer.owner.clone();
    let refused = |result: Result<ResourceAck, Error>| result.unwrap_err().code();

    assert_eq!(
        refused(observer.supervision.start_resource(&owner, "ox_lib").await),
        Some(ErrorCode::ResourceChanged),
        "it is up"
    );
    assert_eq!(
        refused(observer.supervision.stop_resource(&owner, "taxi").await),
        Some(ErrorCode::ResourceChanged),
        "it is not"
    );
    assert_eq!(
        refused(observer.supervision.restart_resource(&owner, "taxi", true).await),
        Some(ErrorCode::ResourceChanged)
    );

    observer
        .supervision
        .stop_resource(&owner, "ox_lib")
        .await
        .unwrap();
    let second = refused(observer.supervision.stop_resource(&owner, "ox_lib").await);
    // FXServer answers within half a second, and the resource is stopped from then on.
    assert!(
        matches!(second, Some(ErrorCode::ResourceBusy | ErrorCode::ResourceChanged)),
        "{second:?}"
    );
    let view = observer
        .resources_until(|view| is(view, "ox_lib", ResourceState::Stopped))
        .await;

    assert_eq!(row(&view, "ox_lib").asked, None, "FXServer answered");
    assert_eq!(
        observer.audited(AuditAction::ResourceStopped).await.len(),
        1,
        "a refused order is not one"
    );
    let written = observer
        .console()
        .iter()
        .filter(|line| line.3 == LineKind::Command)
        .count();
    assert_eq!(written, 1, "one line reached the console");
    observer.stop_observer().await;
}

#[tokio::test]
async fn a_rescan_says_what_fxserver_found_and_a_failed_start_shows_as_one() {
    let mut observer = Observer::with_resources().await;
    observer.listed().await;
    let owner = observer.owner.clone();

    observer.resource("[jobs]/bank", "version '1.0.0'\n");
    observer.resource("[jobs]/taxi", "fx_version 'cerulean'\nversion '2.0.0'\n");
    let asked = observer.supervision.rescan_resources(&owner).await.unwrap();
    let view = observer.resources_until(|view| view.rescan.is_some()).await;

    assert_eq!(asked.delivery, Delivery::Written);
    assert!(!view.rescanning, "the list came again");
    let rescan = view.rescan.clone().unwrap();
    assert_eq!(
        (rescan.by.as_deref(), rescan.added.as_slice(), rescan.added_count),
        (Some("ada"), ["bank".to_owned()].as_slice(), 1)
    );
    assert_eq!(
        (row(&view, "bank").state, row(&view, "bank").folder.as_str()),
        (ResourceState::Stopped, "[jobs]")
    );
    assert_eq!(row(&view, "taxi").version.as_deref(), Some("2.0.0"));
    assert_eq!(
        row(&view, "ox_lib").state,
        ResourceState::Started,
        "nothing stopped"
    );
    let echo = observer.console_until("refresh").await;
    assert_eq!((echo.3, echo.4.as_deref()), (LineKind::Command, Some("ada")));
    observer
        .console_until("Found 1 new resources, and refreshed 1/")
        .await;
    let view = observer
        .resources_until(|view| {
            view.rescan
                .as_ref()
                .is_some_and(|rescan| rescan.changed.is_some())
        })
        .await;
    assert_eq!(view.rescan.unwrap().changed, Some(1), "taxi's manifest");
    let audited = observer.audited(AuditAction::ResourcesRescanned).await;
    assert_eq!(
        (audited.len(), audited[0].target.as_deref()),
        (1, Some("default"))
    );

    observer
        .supervision
        .command(
            &owner,
            "set fake_fail_start bank",
            CONSOLE_ROUTE.0,
            CONSOLE_ROUTE.1,
        )
        .await
        .unwrap();
    observer.supervision.start_resource(&owner, "bank").await.unwrap();
    let view = observer
        .resources_until(|view| row(view, "bank").failed_at.is_some())
        .await;
    assert_eq!(row(&view, "bank").state, ResourceState::Stopped);

    observer
        .supervision
        .command(
            &owner,
            "set fake_fail_start nothing",
            CONSOLE_ROUTE.0,
            CONSOLE_ROUTE.1,
        )
        .await
        .unwrap();
    observer.supervision.start_resource(&owner, "bank").await.unwrap();
    let view = observer
        .resources_until(|view| is(view, "bank", ResourceState::Started))
        .await;
    assert_eq!(row(&view, "bank").failed_at, None, "it started since");
    observer.stop_observer().await;
}

/// The whole list, by net id.
async fn entities_page(entities: &Entities, observer: &Observer) -> EntityPage {
    entities
        .page(
            &observer.owner,
            &EntityFilters::default(),
            EntitySort::NetId,
            false,
            0,
        )
        .await
        .unwrap()
}

/// The list's state once `wanted` holds, which a sweep of the bridge brings.
async fn entities_live(
    live: &mut watch::Receiver<EntitiesLive>,
    wanted: impl FnMut(&EntitiesLive) -> bool,
) -> EntitiesLive {
    timeout(WAIT, live.wait_for(wanted))
        .await
        .expect("the bridge sent no such sweep while the list was followed")
        .unwrap()
        .clone()
}

fn entity_target(page: &EntityPage, net_id: u16) -> EntityTarget {
    let item = page
        .items
        .iter()
        .find(|item| item.net_id == net_id)
        .unwrap_or_else(|| panic!("no entity {net_id} in {:?}", page.items));
    EntityTarget {
        net_id,
        serial: item.serial,
        model: item.model,
    }
}

#[tokio::test]
#[expect(
    clippy::too_many_lines,
    reason = "one run of a server: its list, a deletion and what the audit log keeps of it"
)]
async fn the_entities_are_listed_while_someone_looks_and_deleted_one_by_one() {
    let mut observer = Observer::new(false).await;
    let (metrics, sampling, players, playing) = follow_players(&observer).await;
    let (entities, listing) = Entities::start(EntitiesSetup {
        db: observer.db.clone(),
        clock: Clock::system(),
        supervision: observer.supervision.clone(),
        players: Some(players.clone()),
    });
    let send = |line: &'static str| {
        let observer = &observer;
        async move {
            observer
                .supervision
                .command(&observer.owner, line, CONSOLE_ROUTE.0, CONSOLE_ROUTE.1)
                .await
                .unwrap();
        }
    };
    observer.supervision.start_server(&observer.owner).await.unwrap();
    observer.state(ServerState::Online).await;
    // A car from a resource that Ada's game simulates, a train, two objects, and a helicopter
    // Ada sits in: net ids 1 to 5.
    send(
        "fake_join Ada; fake_entity vehicle 412.6 -1208.3 32.1 qb-garages 1 2; fake_entity train 0 0 0; \
         fake_entity object 1 2 3 ox_inventory; fake_entity object 5 6 7; fake_entity heli 9 9 9; \
         fake_enter 1 5",
    )
    .await;
    confirmed(&players, 1).await;

    let (before, mut live, watching) = entities.watch();
    let listed = entities_live(&mut live, |live| live.run.is_some()).await;
    let page = entities_page(&entities, &observer).await;
    // Ada steps into the car after the sweep: the list does not know, the bridge does.
    send("fake_enter 1 1").await;
    let deletion = entities
        .delete(
            &observer.owner,
            EntityDeleteRequest {
                run: listed.run.clone().unwrap(),
                targets: vec![
                    entity_target(&page, 5),
                    entity_target(&page, 2),
                    entity_target(&page, 3),
                    entity_target(&page, 1),
                    EntityTarget {
                        model: 7,
                        ..entity_target(&page, 4)
                    },
                ],
            },
        )
        .await
        .unwrap();
    let after = entities_page(&entities, &observer).await;
    send("fake_entity_gone 4").await;
    let gone = entities
        .delete(
            &observer.owner,
            EntityDeleteRequest {
                run: listed.run.clone().unwrap(),
                targets: vec![entity_target(&page, 4)],
            },
        )
        .await
        .unwrap();
    drop(watching);
    let forgotten = entities_live(&mut live, |live| live.run.is_none()).await;
    let audited = observer.audited(AuditAction::EntitiesDeleted).await;
    observer.supervision.stop_server(&observer.owner).await.unwrap();
    observer.state(ServerState::Stopped).await;
    listing.close().await;
    playing.close().await;
    sampling.close().await;
    metrics.close().await.unwrap();
    observer.stop_observer().await;

    assert_eq!(before, EntitiesLive::default());
    assert_eq!((listed.sweep, listed.listed, listed.every_ms), (1, 5, 15_000));
    let counts = listed.counts.unwrap();
    assert_eq!(
        (counts.vehicles, counts.objects, counts.peds, counts.player_peds),
        (Some(3), Some(2), Some(1), 1)
    );
    assert!(!listed.truncated && !listed.spaced && listed.unavailable.is_none());
    assert_eq!(page.items.len(), 5);
    let car = &page.items[0];
    assert_eq!(
        (car.net_id, car.kind, car.class),
        (1, EntityKind::Vehicle, Some(VehicleClass::Car))
    );
    assert_eq!((car.x, car.y, car.z, car.bucket), (412.6, -1208.3, 32.1, 2));
    assert_eq!(car.resource.as_deref(), Some("qb-garages"));
    assert_eq!(
        (car.owned, car.owner, car.owner_name.as_deref(), car.occupied),
        (true, Some(1), Some("Ada"), false)
    );
    assert_eq!(page.items[1].class, Some(VehicleClass::Train));
    assert!(page.items[4].occupied);
    assert_eq!(page.facets.kinds.vehicles, 3);
    let outcomes = |deletion: &observer_domain::entities::EntityDeletion| -> Vec<(u16, EntityOutcome)> {
        deletion
            .results
            .iter()
            .map(|result| (result.net_id, result.outcome))
            .collect()
    };
    assert_eq!(
        outcomes(&deletion),
        [
            (5, EntityOutcome::Occupied),
            (2, EntityOutcome::Train),
            (3, EntityOutcome::Deleted),
            (1, EntityOutcome::Occupied),
            (4, EntityOutcome::Changed),
        ]
    );
    let left: Vec<u16> = after.items.iter().map(|item| item.net_id).collect();
    assert_eq!(
        left,
        [1, 2, 4, 5],
        "the list shows the deletion before the next sweep"
    );
    assert_eq!((after.live.listed, after.live.sweep), (4, 1));
    assert_eq!(outcomes(&gone), [(4, EntityOutcome::Gone)]);
    assert_eq!(forgotten, EntitiesLive::default());
    assert_eq!(audited.len(), 2);
    let first = &audited[1];
    assert_eq!(first.actor_name.as_deref(), Some("ada"));
    assert_eq!(first.outcome, Outcome::Success);
    assert_eq!(
        first.details,
        json!({
            "asked": 5,
            "deleted": 1,
            "outcomes": { "occupied": 2, "train": 1, "changed": 1 },
            "entities": [{ "model": "0xE0FAF6D2", "kind": "object", "resource": "ox_inventory", "count": 1 }],
        })
    );
    assert_eq!(audited[0].details["outcomes"], json!({ "gone": 1 }));
    let console: Vec<String> = observer.console().iter().map(|line| plain(&line.5)).collect();
    assert!(
        console
            .iter()
            .any(|line| line.contains("observer_cmd deleteEntities")),
        "a deletion shows in the console"
    );
    assert!(
        !console.iter().any(|line| line.contains("observer_cmd entities")),
        "the asks for the list fill the console"
    );
}

#[tokio::test]
async fn the_server_lists_its_entities_only_while_someone_looks() {
    let mut observer = Observer::launched(
        false,
        Duration::from_millis(50),
        &[("fake_heartbeat_ms", "200"), ("fake_entities_every_ms", "100")],
    )
    .await;
    let (entities, listing) = Entities::start(EntitiesSetup {
        db: observer.db.clone(),
        clock: Clock::system(),
        supervision: observer.supervision.clone(),
        players: None,
    });
    observer.supervision.start_server(&observer.owner).await.unwrap();
    observer.state(ServerState::Online).await;
    observer
        .supervision
        .command(
            &observer.owner,
            "fake_entities 4 3 20; fake_entity_stays 1",
            CONSOLE_ROUTE.0,
            CONSOLE_ROUTE.1,
        )
        .await
        .unwrap();

    let (_, mut live, watching) = entities.watch();
    let swept = entities_live(&mut live, |live| live.sweep >= 3).await;
    let page = entities_page(&entities, &observer).await;
    let kept = entities
        .delete(
            &observer.owner,
            EntityDeleteRequest {
                run: swept.run.clone().unwrap(),
                targets: vec![entity_target(&page, 1)],
            },
        )
        .await
        .unwrap();
    drop(watching);
    entities_live(&mut live, |live| live.run.is_none()).await;
    // Twenty sweeps' worth of time, had the bridge gone on without anyone looking.
    tokio::time::sleep(Duration::from_secs(2)).await;
    let (_, mut live, watching) = entities.watch();
    let again = entities_live(&mut live, |live| live.run.is_some()).await;
    drop(watching);
    observer.supervision.stop_server(&observer.owner).await.unwrap();
    observer.state(ServerState::Stopped).await;
    let stopped = entities_page(&entities, &observer).await;
    listing.close().await;
    observer.stop_observer().await;

    assert_eq!(swept.listed, 27);
    assert_eq!(swept.run, again.run, "the same server, the same bridge");
    assert_eq!(kept.results[0].outcome, EntityOutcome::Unconfirmed);
    assert!(
        again.sweep <= swept.sweep + 8,
        "the bridge went on sweeping with nobody looking: {} then {}",
        swept.sweep,
        again.sweep
    );
    assert_eq!(stopped.live, EntitiesLive::default());
}

#[tokio::test]
async fn a_server_without_onesync_has_no_entities_to_list() {
    let mut observer = Observer::new(false).await;
    let cfg = observer.dir.path().join("server-data/server.cfg");
    let mut text = fs::read_to_string(&cfg).unwrap();
    text.push_str("set onesync off\nfake_entities 2 2 2\n");
    fs::write(&cfg, text).unwrap();
    let (entities, listing) = Entities::start(EntitiesSetup {
        db: observer.db.clone(),
        clock: Clock::system(),
        supervision: observer.supervision.clone(),
        players: None,
    });
    observer.supervision.start_server(&observer.owner).await.unwrap();
    observer.state(ServerState::Online).await;

    let (_, mut live, watching) = entities.watch();
    let told = entities_live(&mut live, |live| live.unavailable.is_some()).await;
    let page = entities_page(&entities, &observer).await;
    drop(watching);
    observer.supervision.stop_server(&observer.owner).await.unwrap();
    observer.state(ServerState::Stopped).await;
    listing.close().await;
    observer.stop_observer().await;

    assert_eq!(told.unavailable, Some(EntitiesUnavailable::Onesync));
    assert_eq!((told.run, told.listed, told.counts), (None, 0, None));
    assert_eq!(page.items, []);
}
