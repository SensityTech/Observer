//! A server run the way `observer run` runs it: launch settings stored through observer-app, then
//! started, commanded and stopped through its supervision, against the fake.
// The helpers below are test code too, though not inside a `#[test]` function.
#![allow(clippy::unwrap_used)]

use std::fs;
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, PoisonError};
use std::time::{Duration, Instant};

use observer_app::{
    Accounts, AuditLog, Caller, Client, Clock, PasswordCost, Running, Servers, Setup, Supervision,
};
use observer_domain::audit::{ActorKind, AuditAction, AuditEntry, AuditFilter, Outcome};
use observer_domain::launch::{ExtraArgument, LaunchSettings, OneSync};
use observer_domain::server::{ConsoleLine, Delivery, LineKind, ServerState, ServerStatus, StateCause};
use observer_fxserver::console::plain;
use observer_fxserver::health::HealthLimits;
use observer_fxserver::supervisor::StopTimings;
use observer_store::Database;
use serde_json::json;
use tempfile::TempDir;
use tokio::time::timeout;

const WAIT: Duration = Duration::from_secs(20);
const CONSOLE_ROUTE: (&str, &str) = ("POST", "/api/servers/default/console");

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
    _dir: TempDir,
    _port: Port,
    db: Database,
    audit: AuditLog,
    supervision: Supervision,
    running: Option<Running>,
    owner: Caller,
}

impl Observer {
    /// An Observer whose owner `ada` saved launch settings for the fake.
    async fn new(auto_start: bool) -> Self {
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

        let db = Database::open(dir.path().join("observer.sqlite3"), dir.path().join("backups"))
            .await
            .unwrap();
        let clock = Clock::system();
        let accounts = Accounts::new(db.clone(), clock.clone(), PasswordCost::TEST)
            .await
            .unwrap();
        let audit = AuditLog::new(db.clone(), clock.clone());
        let servers = Servers::new(db.clone(), clock.clone());
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
            extra_arguments: vec![ExtraArgument {
                name: "fake_heartbeat_ms".to_owned(),
                value: "200".to_owned(),
            }],
        };
        servers
            .change_launch_settings(&owner, Servers::DEFAULT, launch, None)
            .await
            .unwrap();

        let setup = Setup {
            db: db.clone(),
            clock,
            accounts,
            servers,
            data_dir: dir.path().join("observer-data"),
            console_logs: dir.path().join("observer-data/logs/console"),
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
        })
        .await
        .unwrap();
        Self {
            _dir: dir,
            _port: port,
            db,
            audit,
            supervision,
            running: Some(running),
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

    async fn stop_observer(&mut self) {
        self.supervision.shutdown(false).await;
        let running = self.running.take().unwrap();
        timeout(WAIT, running.close())
            .await
            .expect("the supervisor did not stop");
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
    assert_eq!(delivery, Delivery::Written);
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

    assert!(matches!(stopped.cause, Some(StateCause::Exited { .. })));
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
