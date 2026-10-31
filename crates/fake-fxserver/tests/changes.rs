//! What changed between starts, recorded the way `observer run` records it: a snapshot of the
//! server folder at each spawn of the fake, compared with the one before.
// The helpers below are test code too, though not inside a `#[test]` function.
#![allow(clippy::unwrap_used)]

use std::fs;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, PoisonError};
use std::time::{Duration, Instant};

use jiff::Timestamp;
use observer_app::{
    Accounts, Caller, Changes, ChangesSetup, ChangesTask, Client, Clock, PasswordCost, Running, Servers,
    Setup, Supervision,
};
use observer_domain::audit::ActorKind;
use observer_domain::changes::{FileKind, LiveChangeKind, SnapshotKind, SnapshotState, StartCause};
use observer_domain::launch::{LaunchSettings, OneSync};
use observer_domain::server::{ServerState, ServerStatus};
use observer_fxserver::changes::tree::TreeEntry;
use observer_fxserver::health::HealthLimits;
use observer_fxserver::supervisor::StopTimings;
use observer_store::artifacts::{self, ArtifactState};
use observer_store::changes::snapshots::{self, Snapshot};
use observer_store::changes::{live, resources, texts, trees};
use observer_store::{ChangesDatabase, Database, StoreError};
use tempfile::TempDir;
use tokio::time::timeout;

const WAIT: Duration = Duration::from_secs(30);
const BUILD: u32 = 35245;
const CONSOLE_ROUTE: (&str, &str) = ("POST", "/api/servers/default/console");
/// A webhook planted in a resource, which no stored text may hold.
const WEBHOOK: &str = "https://discord.com/api/webhooks/123456/PlantedTokenThatMustNeverBeStored";

static PORTS: LazyLock<Mutex<Vec<u16>>> = LazyLock::new(|| Mutex::new((41940..=41942).collect()));

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
    store: ChangesDatabase,
    supervision: Supervision,
    running: Option<Running>,
    following: Option<ChangesTask>,
    owner: Caller,
}

impl Lab {
    async fn new() -> Self {
        let port = Port::lease().await;
        let dir = tempfile::tempdir().unwrap();
        let observer_data = dir.path().join("observer-data");
        let build = observer_data.join("artifacts").join(BUILD.to_string());
        let program = fake_program(&build);
        fs::create_dir_all(program.parent().unwrap()).unwrap();
        fs::copy(env!("CARGO_BIN_EXE_fake-fxserver"), &program).unwrap();
        let data = dir.path().join("server-data");
        plant(&data, port.0);
        let db = Database::open(dir.path().join("observer.sqlite3"), dir.path().join("backups"))
            .await
            .unwrap();
        let clock = Clock::system();
        let (accounts, servers, owner) = configure(&db, &clock, &data).await;

        let setup = Setup {
            db: db.clone(),
            clock: clock.clone(),
            accounts,
            servers,
            data_dir: observer_data.clone(),
            panel: SocketAddr::from(([127, 0, 0, 1], 41975)),
            artifacts: observer_data.join("artifacts"),
            console_logs: observer_data.join("logs/console"),
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
        let store = ChangesDatabase::open(observer_data.join("changes.sqlite3"))
            .await
            .unwrap();
        let (_, following) = Changes::start(ChangesSetup {
            db,
            store: store.clone(),
            clock,
            supervision: supervision.clone(),
            data_dir: observer_data.clone(),
            artifacts: observer_data.join("artifacts"),
        })
        .await
        .unwrap();
        Self {
            dir,
            _port: port,
            store,
            supervision,
            running: Some(running),
            following: Some(following),
            owner,
        }
    }

    fn data(&self) -> PathBuf {
        self.dir.path().join("server-data")
    }

    async fn until(&self, wanted: impl Fn(&ServerStatus) -> bool) -> ServerStatus {
        let mut status = self.supervision.status();
        let found = timeout(WAIT, status.wait_for(|status| wanted(status)))
            .await
            .map(|found| found.unwrap().clone());
        found.unwrap_or_else(|_| panic!("the status never came; last {:#?}", status.borrow()))
    }

    /// The `count`th start's snapshot, once its job is over.
    async fn settled_start(&self, count: usize) -> Snapshot {
        let deadline = Instant::now() + WAIT;
        loop {
            let starts = self
                .store
                .read(|conn| snapshots::starts(conn, Servers::DEFAULT, None, 50))
                .await
                .unwrap();
            if starts.len() >= count && starts[starts.len() - count].settled() {
                return starts[starts.len() - count].clone();
            }
            assert!(Instant::now() < deadline, "no settled start {count}: {starts:#?}");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn rows(&self, snapshot: &Snapshot) -> Vec<resources::ResourceRow> {
        let holder = snapshot.holder();
        self.store
            .read(move |conn| resources::rows(conn, holder))
            .await
            .unwrap()
    }

    async fn tree(&self, sha256: &str) -> Vec<TreeEntry> {
        let sha256 = sha256.to_owned();
        let listing = self
            .store
            .read(move |conn| trees::listing(conn, &sha256))
            .await
            .unwrap()
            .unwrap();
        serde_json::from_slice(&listing).unwrap()
    }

    async fn text(&self, digest: &str) -> String {
        let digest = digest.to_owned();
        self.store
            .read(move |conn| texts::get(conn, &digest))
            .await
            .unwrap()
            .unwrap()
            .text
            .unwrap()
    }

    /// Every stored text, decompressed.
    async fn every_text(&self) -> Vec<String> {
        let digests: Vec<String> = self
            .store
            .read(|conn| -> Result<Vec<String>, StoreError> {
                let mut statement = conn.prepare("SELECT digest FROM text_version WHERE body IS NOT NULL")?;
                let rows = statement.query_map([], |row| row.get(0))?;
                Ok(rows.collect::<Result<_, _>>()?)
            })
            .await
            .unwrap();
        let mut bodies = Vec::new();
        for digest in digests {
            bodies.push(self.text(&digest).await);
        }
        bodies
    }

    async fn command(&self, line: &str) {
        self.supervision
            .command(&self.owner, line, CONSOLE_ROUTE.0, CONSOLE_ROUTE.1)
            .await
            .unwrap();
    }

    async fn stop(&mut self) {
        self.supervision.shutdown(false).await;
        if let Some(running) = self.running.take() {
            timeout(WAIT, running.close())
                .await
                .expect("the supervisor did not stop");
        }
        if let Some(following) = self.following.take() {
            following.close().await;
        }
    }
}

/// A server folder with two resources, one in a category, and a secret in its cfg.
fn plant(data: &Path, port: u16) {
    file(
        data,
        "server.cfg",
        &format!(
            "endpoint_add_tcp \"127.0.0.1:{port}\"\nendpoint_add_udp \"127.0.0.1:{port}\"\n\
             set my_bot_token AlsoNeverStored\nensure chat\nensure [qb]\n"
        ),
    );
    file(
        data,
        "resources/chat/fxmanifest.lua",
        "fx_version 'cerulean'\nversion '1.0.0'\n",
    );
    file(data, "resources/chat/client.lua", "print('chat')\n");
    file(data, "resources/[qb]/qb-core/fxmanifest.lua", "version '1.2.0'\n");
    file(
        data,
        "resources/[qb]/qb-core/config.lua",
        "Config = {}\nConfig.Locale = 'fr'\n",
    );
    file(data, "resources/[qb]/qb-core/stream/car.ytd", "\0binary\0");
}

/// The owner `ada`, signed in, with launch settings for `data` and the fake's build selected.
async fn configure(db: &Database, clock: &Clock, data: &Path) -> (Accounts, Servers, Caller) {
    let accounts = Accounts::new(db.clone(), clock.clone(), PasswordCost::TEST)
        .await
        .unwrap();
    let servers = Servers::new(db.clone(), clock.clone());
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
        auto_start: false,
        txadmin_events: false,
        extra_arguments: Vec::new(),
    };
    servers
        .change_launch_settings(&owner, Servers::DEFAULT, launch, None)
        .await
        .unwrap();
    let selected = ArtifactState {
        selected: Some(BUILD),
        ..ArtifactState::default()
    };
    db.write(move |tx| artifacts::save_state(tx, Servers::DEFAULT, &selected, Timestamp::now()))
        .await
        .unwrap();
    (accounts, servers, owner)
}

fn file(root: &Path, path: &str, contents: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

fn fake_program(build: &Path) -> PathBuf {
    if cfg!(windows) {
        build.join("FXServer.exe")
    } else {
        build.join("alpine/opt/cfx-server/ld-musl-x86_64.so.1")
    }
}

#[tokio::test]
async fn the_first_start_is_a_baseline_of_every_resource_with_masked_texts() {
    let mut lab = Lab::new().await;

    lab.supervision.start_server(&lab.owner).await.unwrap();
    let first = lab.settled_start(1).await;

    assert_eq!(first.kind, SnapshotKind::Start);
    assert_eq!(first.state, SnapshotState::Complete, "{first:#?}");
    assert!(first.baseline);
    assert_eq!(first.cause, Some(StartCause::Requested));
    assert_eq!(first.by.as_deref(), Some("ada"));
    let rows = lab.rows(&first).await;
    let paths: Vec<&str> = rows.iter().map(|row| row.path.as_str()).collect();
    assert_eq!(paths, ["resources/[qb]/qb-core", "resources/chat"]);
    let core = lab.tree(rows[0].tree.as_deref().unwrap()).await;
    let kinds: Vec<(&str, FileKind)> = core
        .iter()
        .map(|entry| (entry.path.as_str(), entry.kind))
        .collect();
    assert_eq!(
        kinds,
        [
            ("resources/[qb]/qb-core/config.lua", FileKind::Text),
            ("resources/[qb]/qb-core/fxmanifest.lua", FileKind::Text),
            ("resources/[qb]/qb-core/stream/car.ytd", FileKind::Binary),
        ]
    );
    assert!(core[2].sha256.is_some());
    assert_eq!(core[0].sha256, None);
    let cfg = first.cfg["files"][0].clone();
    assert_eq!(cfg["path"], "server.cfg");
    assert_eq!(first.cfg["started"], serde_json::json!(["chat", "[qb]"]));
    for text in lab.every_text().await {
        assert!(!text.contains("AlsoNeverStored"), "{text}");
    }
    lab.stop().await;
}

#[tokio::test]
async fn a_restart_after_an_edit_records_the_new_files_with_their_secrets_hidden() {
    let mut lab = Lab::new().await;
    lab.supervision.start_server(&lab.owner).await.unwrap();
    let first = lab.settled_start(1).await;
    lab.until(|status| status.state == ServerState::Online).await;

    let data = lab.data();
    file(
        &data,
        "resources/chat/client.lua",
        &format!("print('chat')\nlocal hook = '{WEBHOOK}'\n"),
    );
    file(
        &data,
        "resources/[qb]/qb-core/fxmanifest.lua",
        "version '1.3.0'\n",
    );
    file(
        &data,
        "resources/[qb]/qb-housing/fxmanifest.lua",
        "version '0.1.0'\n",
    );
    lab.supervision.restart_server(&lab.owner).await.unwrap();
    let second = lab.settled_start(2).await;

    assert!(!second.baseline);
    assert_eq!(second.cause, Some(StartCause::Restart));
    assert_eq!(second.same_as, None);
    let before = lab.rows(&first).await;
    let after = lab.rows(&second).await;
    assert_eq!(after.len(), 3);
    let chat = after.iter().find(|row| row.name == "chat").unwrap();
    assert_ne!(
        chat.tree,
        before.iter().find(|row| row.name == "chat").unwrap().tree
    );
    let core = after.iter().find(|row| row.name == "qb-core").unwrap();
    assert_eq!(core.version(), Some("1.3.0"));
    let client = lab
        .tree(chat.tree.as_deref().unwrap())
        .await
        .into_iter()
        .find(|entry| entry.path.ends_with("client.lua"))
        .unwrap();
    let text = lab.text(client.digest().unwrap()).await;
    assert!(text.contains("webhooks/[REDACTED]"), "{text}");
    for text in lab.every_text().await {
        assert!(!text.contains("PlantedTokenThatMustNeverBeStored"), "{text}");
    }
    lab.stop().await;
}

#[tokio::test]
async fn an_unchanged_restart_shares_the_previous_rows_and_a_live_restart_is_attributed() {
    let mut lab = Lab::new().await;
    lab.supervision.start_server(&lab.owner).await.unwrap();
    let first = lab.settled_start(1).await;
    lab.until(|status| status.state == ServerState::Online).await;

    lab.command("restart chat").await;
    let deadline = Instant::now() + WAIT;
    let id = first.id;
    let changes = loop {
        let changes = lab
            .store
            .read(move |conn| live::list(conn, id, 10))
            .await
            .unwrap();
        if !changes.is_empty() || Instant::now() > deadline {
            break changes;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(changes.len(), 1, "{changes:#?}");
    assert_eq!(changes[0].name, "chat");
    assert_eq!(changes[0].change, LiveChangeKind::Restarted);
    assert_eq!(changes[0].by.as_deref(), Some("ada"));
    assert_eq!(changes[0].command.as_deref(), Some("restart chat"));

    lab.supervision.restart_server(&lab.owner).await.unwrap();
    let second = lab.settled_start(2).await;
    assert_eq!(second.same_as, Some(first.id));
    assert_eq!(lab.rows(&second).await, lab.rows(&first).await);
    lab.stop().await;
}
