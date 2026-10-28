//! A server run the way `observer run` runs it: launch settings stored through observer-app, then
//! started, commanded and stopped through its supervision, against the fake.
// The helpers below are test code too, though not inside a `#[test]` function.
#![allow(clippy::unwrap_used)]

use std::fs;
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, PoisonError};
use std::time::{Duration, Instant};

use jiff::{SignedDuration, Timestamp, Unit};
use observer_app::{
    Accounts, AuditLog, Caller, Client, Clock, DatabaseFiles, Error, Metrics, MetricsSetup, PasswordCost,
    Running, Schedule, ScheduleSetup, ScheduleTask, Servers, Setup, Sources, Supervision,
};
use observer_domain::audit::{ActorKind, AuditAction, AuditEntry, AuditFilter, Outcome};
use observer_domain::error::ErrorCode;
use observer_domain::launch::{ExtraArgument, LaunchSettings, OneSync};
use observer_domain::metrics::{LiveSample, MarkerKind, Range, Series};
use observer_domain::schedule::{
    OneOffRequest, OneOffRestart, RestartSource, ScheduleChange, ScheduleSettings,
};
use observer_domain::server::{
    CommandAck, ConsoleLine, Delivery, LINE_MASKED, LineKind, ServerState, ServerStatus, StateCause,
};
use observer_fxserver::console::plain;
use observer_fxserver::health::HealthLimits;
use observer_fxserver::recovery::RecoveryPolicy;
use observer_fxserver::supervisor::StopTimings;
use observer_store::{Database, StoreError};
use serde_json::json;
use tempfile::TempDir;
use tokio::sync::{broadcast, watch};
use tokio::task::JoinHandle;
use tokio::time::timeout;

const WAIT: Duration = Duration::from_secs(20);
const CONSOLE_ROUTE: (&str, &str) = ("POST", "/api/servers/default/console");
/// A quick heartbeat, three players and ticks of 12 ms.
const FAKE: [(&str, &str); 3] = [
    ("fake_heartbeat_ms", "200"),
    ("fake_players", "3"),
    ("fake_tick_ms", "12"),
];

/// Ports of their own: test binaries run one after the other, but this one's tests run together.
static PORTS: LazyLock<Mutex<Vec<u16>>> = LazyLock::new(|| Mutex::new((41926..=41928).collect()));

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

struct Observer {
    dir: TempDir,
    _port: Port,
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

    /// The supervision and the schedule, with timings short enough for tests.
    async fn supervise(
        &self,
        dir: &Path,
        clock: &Clock,
        longest_sleep: Duration,
    ) -> (Supervision, Running, Schedule, ScheduleTask) {
        let setup = Setup {
            db: self.db.clone(),
            clock: Clock::system(),
            accounts: self.accounts.clone(),
            servers: self.servers.clone(),
            data_dir: dir.join("observer-data"),
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
        (supervision, running, schedule, scheduling)
    }
}

impl Observer {
    /// An Observer whose owner `ada` saved launch settings for the fake.
    async fn new(auto_start: bool) -> Self {
        Self::with_longest_sleep(auto_start, Duration::from_millis(50)).await
    }

    async fn with_longest_sleep(auto_start: bool, longest_sleep: Duration) -> Self {
        let port = Port::lease().await;
        let dir = tempfile::tempdir().unwrap();
        let build = dir.path().join("build");
        let program = fake_program(&build);
        fs::create_dir_all(program.parent().unwrap()).unwrap();
        fs::copy(env!("CARGO_BIN_EXE_fake-fxserver"), &program).unwrap();
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
            build_path: build.to_str().unwrap().to_owned(),
            data_path: data.to_str().unwrap().to_owned(),
            cfg_file: "server.cfg".to_owned(),
            onesync: OneSync::On,
            auto_start,
            txadmin_events: false,
            extra_arguments: FAKE
                .map(|(name, value)| ExtraArgument {
                    name: name.to_owned(),
                    value: value.to_owned(),
                })
                .to_vec(),
        };
        services
            .servers
            .change_launch_settings(&owner, Servers::DEFAULT, launch, None)
            .await
            .unwrap();

        // On a whole second, as a one-off planned in minutes is.
        let clock = Clock::manual(Timestamp::now().round(Unit::Second).unwrap());
        let (supervision, running, schedule, scheduling) =
            services.supervise(dir.path(), &clock, longest_sleep).await;
        Self {
            dir,
            _port: port,
            clock,
            longest_sleep,
            db: services.db,
            audit: services.audit,
            servers: services.servers,
            supervision,
            running: Some(running),
            schedule,
            scheduling: Some(scheduling),
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
    }

    /// Stops this Observer, then starts another on the same data, as after an update.
    async fn restart_observer(&mut self) {
        self.stop_observer().await;
        self.db.clone().close().await.unwrap();
        let services = Services::open(self.dir.path()).await;
        let (supervision, running, schedule, scheduling) = services
            .supervise(self.dir.path(), &self.clock, self.longest_sleep)
            .await;
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

#[tokio::test]
async fn the_statistics_follow_a_run_from_before_its_start_to_after_its_stop() {
    let mut observer = Observer::new(false).await;
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
    let running = sample_where(&mut live, |sample| sample.values.contains_key(&Series::PingAvg)).await;
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
    for series in [
        Series::Players,
        Series::FxsCpu,
        Series::MainTick,
        Series::FxsWorkingSet,
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
