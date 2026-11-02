use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use anyhow::Context;
use axum::Router;
use jiff::tz::TimeZone;
use observer_app::{
    Accounts, Artifacts, ArtifactsSetup, ArtifactsTask, AuditLog, Changes, ChangesSetup, Clock,
    DatabaseFiles, Diagnostics, DiagnosticsSetup, Drops, DropsSetup, Features, FeaturesSetup, Metrics,
    MetricsSetup, OwnPaths, PasswordCost, Players, PlayersSetup, Running, Schedule, ScheduleSetup,
    ScheduleTask, Servers, Setup, Sources, Supervision, Switched, Tools, ToolsHook, ToolsSetup,
};
use observer_artifacts::Http;
use observer_artifacts::catalog::Catalog;
use observer_domain::features::Feature;
use observer_fxserver::console::{ConsoleLog, LogLimits};
use observer_store::{BackupKind, ChangesDatabase, Database};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::config::{Config, DataPaths, HttpConfig};
use crate::shutdown::{Signals, StopRequest};
use crate::{BUILD, lock, logging, setup};

const DAILY: Duration = Duration::from_hours(24);
const BACKUP_CHECK_INTERVAL: Duration = Duration::from_hours(1);
const UPKEEP_INTERVAL: Duration = Duration::from_mins(1);
const AUDIT_FOLLOW_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug, Default, clap::Args)]
pub struct RunArgs {
    /// Show neither the status window nor the tray icon.
    #[cfg(windows)]
    #[arg(long)]
    no_window: bool,
}

#[expect(
    clippy::too_many_lines,
    reason = "Observer's start, then its stop, each step in the order it must take"
)]
pub async fn run(config: Config, args: RunArgs) -> anyhow::Result<()> {
    let paths = config.paths();
    let _lock = lock::acquire(&paths.root)?;
    let log_guard = logging::init(&config.log.level, &paths.logs)?;
    let mut signals = Signals::listen().context("cannot listen for stop requests")?;

    info!(version = BUILD.version, commit = BUILD.commit, data_dir = %paths.root.display(), "starting Observer");

    let db = Database::open(&paths.database, &paths.backups)
        .await
        .with_context(|| format!("cannot open {}", paths.database.display()))?;
    let clock = Clock::system();
    let observer_started = clock.now();
    // Before every optional feature: one that is off now is never built.
    let features = Features::load(FeaturesSetup {
        db: db.clone(),
        clock: clock.clone(),
        host_off: config.host_off(),
        sftp: config.sftp.as_ref().map(|sftp| sftp.listen),
    })
    .await?;
    announce_features(&features);
    let accounts = Accounts::new(db.clone(), clock.clone(), PasswordCost::STANDARD).await?;
    let audit = AuditLog::new(db.clone(), clock.clone());
    let servers = Servers::new(db.clone(), clock.clone());
    let instance_id = db.instance_id().await?;
    let console_logs = paths.logs.join("console");
    let console_log = ConsoleLog::start(&console_logs, LogLimits::default(), TimeZone::system())
        .context("cannot open the console log")?;
    // The server tools run before every spawn; they need the supervision's status, so they are
    // bound to the hook once both exist, well before FXServer first starts.
    let tools_hook = ToolsHook::switched(features.clone());
    let diagnosing_at_start = features.runs(Feature::Diagnostics);
    let (supervision, mut supervised) = Supervision::start(
        Setup {
            db: db.clone(),
            clock: clock.clone(),
            accounts: accounts.clone(),
            servers: servers.clone(),
            data_dir: paths.root.clone(),
            panel: config.http.listen,
            artifacts: paths.artifacts.clone(),
            console_logs,
            log: Some(&console_log),
        },
        |settings| {
            settings.pre_launch = Some(Arc::new(tools_hook.clone()));
            // A hung server's snapshot is the diagnostics' to read.
            if !diagnosing_at_start {
                settings.hang_snapshot = None;
            }
        },
    )
    .await?;
    // Before FXServer boots, so that the first run's tree is seen.
    let (diagnostics, diagnosing) = if features.runs(Feature::Diagnostics) {
        let (diagnostics, task) = Diagnostics::start(DiagnosticsSetup {
            db: db.clone(),
            clock: clock.clone(),
            supervision: supervision.clone(),
            data_dir: paths.root.clone(),
        })
        .await?;
        let supervision = supervision.clone();
        let switched = features.switched(Feature::Diagnostics, move || async move {
            supervision.skip_hang_snapshots().await;
            task.close().await;
        });
        (Some(diagnostics), Some(switched))
    } else {
        (None, None)
    };
    // Before FXServer boots, so that its first start gets a snapshot.
    let (changes, changes_db, following) = if features.runs(Feature::Changes) {
        let changes_db = ChangesDatabase::open(&paths.changes)
            .await
            .with_context(|| format!("cannot open {}", paths.changes.display()))?;
        let (changes, task) = Changes::start(ChangesSetup {
            db: db.clone(),
            store: changes_db.clone(),
            clock: clock.clone(),
            supervision: supervision.clone(),
            data_dir: paths.root.clone(),
            artifacts: paths.artifacts.clone(),
        })
        .await?;
        let switched = features.switched(Feature::Changes, move || task.close());
        (Some(changes), Some(changes_db), Some(switched))
    } else {
        (None, None, None)
    };
    // Before FXServer boots, so that Observer's own start counts as one for the schedule.
    let (schedule, scheduling) = Schedule::start(
        ScheduleSetup {
            db: db.clone(),
            clock: clock.clone(),
            supervision: supervision.clone(),
        },
        |_| {},
    )
    .await?;
    let (artifacts, building) =
        builds(&db, &clock, &accounts, &servers, &supervision, &schedule, &paths).await?;
    // The statistics' database also holds the players' sessions, which the drops are drawn from.
    let metrics = if [Feature::Metrics, Feature::Players, Feature::Drops]
        .into_iter()
        .any(|feature| features.runs(feature))
    {
        let metrics = Metrics::open(MetricsSetup {
            path: paths.metrics.clone(),
            clock: Clock::system(),
        })
        .await
        .with_context(|| format!("cannot open {}", paths.metrics.display()))?;
        Some(metrics)
    } else {
        None
    };
    // Before FXServer boots, so that the first run's players are followed from its start.
    let (players, playing) = match &metrics {
        Some(metrics) if features.runs(Feature::Players) => {
            let (players, task) = Players::start(PlayersSetup {
                db: db.clone(),
                metrics: metrics.clone(),
                clock: clock.clone(),
                supervision: supervision.clone(),
                schedule: schedule.status(),
                data_dir: paths.root.clone(),
            })
            .await?;
            let switched = features.switched(Feature::Players, move || task.close());
            (Some(players), Some(switched))
        }
        _ => (None, None),
    };
    // After the players, whose start closes what an earlier Observer left open.
    let (drops, dropping) = match (&metrics, &players) {
        (Some(metrics), Some(players)) if features.runs(Feature::Drops) => {
            let (drops, task) = Drops::start(DropsSetup {
                db: db.clone(),
                metrics: metrics.clone(),
                clock: clock.clone(),
                schedule: schedule.status(),
                players: players.clone(),
            });
            let switched = features.switched(Feature::Drops, move || task.close());
            (Some(drops), Some(switched))
        }
        _ => (None, None),
    };

    let listener = TcpListener::bind(config.http.listen)
        .await
        .with_context(|| format!("cannot listen on {}", config.http.listen))?;
    announce(&config.http);
    let files = observer_app::Files::new(db.clone(), clock.clone(), accounts.clone());
    // Observer's own folders and files, which neither SFTP nor the tools reach.
    let own_paths = OwnPaths::new(&paths.root, &config.path);
    let tools = Tools::new(ToolsSetup {
        db: db.clone(),
        clock: clock.clone(),
        data_dir: paths.root.clone(),
        artifacts: paths.artifacts.clone(),
        status: Some(supervision.status()),
        gate: files.commit_gate(),
        own_paths: own_paths.clone(),
    });
    tools_hook.bind(tools.clone());
    // Whether the tools are on or not, an interrupted write leaves a file half Observer's. Before
    // the uploads' sweep, which would remove a tool's temporary file all the same: the journal
    // then tells from the file itself whether the write landed.
    if let Err(error) = tools.recover(observer_started).await {
        warn!(%error, "the server tools' interrupted writes could not be settled");
    }
    // Right after the panel's port, so that a port taken stops Observer before FXServer starts.
    let sftp_listener = match &config.sftp {
        Some(sftp) if features.runs(Feature::Sftp) => {
            // A crash may have left uploads, which go before anyone can upload again.
            if let Err(error) = observer_sftp::sweep_uploads(&files).await {
                warn!(%error, "the temporary SFTP uploads could not all be removed");
            }
            let host_key = observer_sftp::host_key(&files)
                .await
                .context("cannot read or create the SFTP host key")?;
            let listener = TcpListener::bind(sftp.listen)
                .await
                .with_context(|| format!("cannot listen for SFTP on {}", sftp.listen))?;
            info!(
                listen = %sftp.listen,
                fingerprint = %host_key.fingerprint(observer_sftp::FINGERPRINT),
                "SFTP listening"
            );
            Some((listener, host_key))
        }
        _ => None,
    };
    let setup_token = setup::offer_link(&accounts, config.http.listen).await?;

    let stop = CancellationToken::new();
    let services = observer_web::Services {
        accounts: accounts.clone(),
        audit: audit.clone(),
        servers,
        supervision: supervision.clone(),
        schedule: schedule.clone(),
        artifacts,
        files: files.clone(),
        tools: tools.clone(),
        features: features.clone(),
        diagnostics,
        metrics: metrics.clone(),
        players,
        drops,
        changes,
    };
    let state = observer_web::AppState::new(
        BUILD,
        services,
        &instance_id,
        &config.http.hosts,
        &config.http.trusted_proxies,
    );
    let live = state.live.clone();
    // Before FXServer starts with Observer, so that the first samples show its start.
    let sampling = match &metrics {
        Some(metrics) if features.runs(Feature::Metrics) => {
            let sampling = metrics.start(Sources {
                supervision: supervision.clone(),
                sockets: live.sockets(),
                data_dir: paths.root.clone(),
                databases: DatabaseFiles {
                    state: paths.database.clone(),
                    metrics: paths.metrics.clone(),
                },
            });
            Some(features.switched(Feature::Metrics, move || sampling.close()))
        }
        _ => None,
    };
    let server = tokio::spawn(serve(listener, observer_web::router(state), stop.clone()));
    let backups = tokio::spawn(daily_backups(db.clone(), stop.clone()));
    let upkeep = tokio::spawn(upkeep(
        accounts.clone(),
        audit.clone(),
        files.clone(),
        features.clone(),
        stop.clone(),
    ));
    let sftp = sftp_listener.map(|(listener, host_key)| {
        let closing = CancellationToken::new();
        let task = tokio::spawn(observer_sftp::serve(
            listener,
            files.clone(),
            host_key,
            own_paths.paths().to_vec(),
            closing.clone(),
        ));
        features.switched(Feature::Sftp, move || async move {
            closing.cancel();
            match task.await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => error!(%error, "the SFTP server stopped with an error"),
                Err(error) => error!(%error, "the SFTP server task failed"),
            }
        })
    });
    let audit_feed = tokio::spawn(publish_audit(audit.clone(), stop.clone()));
    let host = Host::start(HostSetup {
        args,
        supervision: &supervision,
        schedule,
        accounts,
        listen: config.http.listen,
        data_dir: &paths.root,
        setup_token,
        signals: &signals,
    });
    if let Err(error) = supervision.boot().await {
        error!(%error, "cannot start FXServer with Observer");
    }

    let first = signals.next().await;
    info!(shortened = first.shortened, from = ?first.from, "stopping Observer");
    host.stopping();
    // Before FXServer's stop: a start queued behind a fix or an undo from the panel waits for it,
    // and the stop waits for that start, so the run ends between two files rather than at its last.
    tools.stopping();
    // Before FXServer stops, so that the log names who stopped it even if the stop goes wrong.
    if let Err(error) = audit.record_stop(first.from).await {
        warn!(%error, "cannot audit the stop of Observer");
    }
    stop_fxserver(
        &supervision,
        building,
        scheduling,
        &mut supervised,
        &mut signals,
        first,
    )
    .await;
    // Once FXServer is down: waits for a fix or an undo from the panel to end between two files.
    tools.close().await;
    supervised.close().await;
    close(diagnosing).await;
    // After the supervision, so that FXServer's stop closes the sessions.
    close(playing).await;
    close(dropping).await;
    // After the supervision: a stop of FXServer ends a snapshot's job, which this joins.
    close(following).await;
    // After the supervision, so that FXServer's stop is in the statistics.
    close(sampling).await;
    stop.cancel();
    close(sftp).await;
    live.shutdown().await;

    join(server, [backups, upkeep, audit_feed]).await?;
    console_log.close().await;
    if let Some(metrics) = metrics {
        metrics
            .close()
            .await
            .context("cannot close the metrics database")?;
    }
    if let Some(changes_db) = changes_db {
        changes_db
            .close()
            .await
            .context("cannot close the changes database")?;
    }
    db.close().await.context("cannot close the database")?;
    // Before the release, so that the tray icon leaves the notification area while Windows lets
    // the process run.
    host.close().await;

    info!("Observer stopped");
    drop(log_guard);
    signals.release();
    Ok(())
}

/// What the host shows of Observer besides its console: the status window and the tray icon on
/// Windows, the state of the service to systemd on Linux.
struct Host {
    #[cfg(windows)]
    desktop: Option<crate::desktop::Desktop>,
    #[cfg(target_os = "linux")]
    readiness: crate::systemd::Readiness,
}

#[cfg_attr(
    not(windows),
    expect(dead_code, reason = "the window takes what Linux leaves")
)]
struct HostSetup<'a> {
    args: RunArgs,
    supervision: &'a Supervision,
    schedule: Schedule,
    accounts: Accounts,
    listen: SocketAddr,
    data_dir: &'a std::path::Path,
    setup_token: Option<observer_app::SetupToken>,
    signals: &'a Signals,
}

impl Host {
    /// Once the panel listens, before FXServer starts with Observer.
    #[cfg_attr(
        not(windows),
        expect(clippy::needless_pass_by_value, reason = "the window takes what Linux leaves")
    )]
    fn start(setup: HostSetup<'_>) -> Self {
        #[cfg(windows)]
        {
            let desktop = (!setup.args.no_window).then(|| {
                crate::desktop::Desktop::start(crate::desktop::DesktopSetup {
                    runtime: tokio::runtime::Handle::current(),
                    supervision: setup.supervision.clone(),
                    schedule: setup.schedule,
                    accounts: setup.accounts,
                    listen: setup.listen,
                    data_dir: setup.data_dir.to_path_buf(),
                    setup: setup.setup_token,
                    stop: setup.signals.requests(),
                    build: BUILD,
                })
            });
            Self { desktop }
        }
        #[cfg(not(windows))]
        {
            Self {
                #[cfg(target_os = "linux")]
                readiness: crate::systemd::Readiness::start(setup.supervision, setup.listen.port()),
            }
        }
    }

    fn stopping(&self) {
        #[cfg(windows)]
        if let Some(desktop) = &self.desktop {
            desktop.stopping();
        }
        #[cfg(target_os = "linux")]
        self.readiness.stopping();
    }

    async fn close(self) {
        #[cfg(windows)]
        if let Some(desktop) = self.desktop {
            desktop.close().await;
        }
        #[cfg(target_os = "linux")]
        self.readiness.close().await;
    }
}

/// FXServer's builds, listed and downloaded from Cfx.re. After the schedule, whose next restart a
/// build can wait for.
async fn builds(
    db: &Database,
    clock: &Clock,
    accounts: &Accounts,
    servers: &Servers,
    supervision: &Supervision,
    schedule: &Schedule,
    paths: &DataPaths,
) -> anyhow::Result<(Artifacts, ArtifactsTask)> {
    let root = &paths.artifacts;
    let (catalog, http) = sources()?;
    #[cfg(feature = "test-catalog")]
    let probation = test_probation()?;
    #[cfg(not(feature = "test-catalog"))]
    let probation = None;
    let setup = ArtifactsSetup {
        db: db.clone(),
        clock: clock.clone(),
        root: root.clone(),
        accounts: accounts.clone(),
        servers: servers.clone(),
        supervision: supervision.clone(),
        schedule: schedule.status(),
        catalog,
        http,
    };
    Artifacts::start(setup, |timings| {
        if let Some(stable_for) = probation {
            timings.stable_for = stable_for;
        }
    })
    .await
    .with_context(|| format!("cannot use {} for FXServer builds", root.display()))
}

/// How long a new build must stay up to pass its probation in the browser tests, so that they see
/// one pass: the seconds `OBSERVER_TEST_PROBATION` names, Observer's ten minutes without it.
#[cfg(feature = "test-catalog")]
fn test_probation() -> anyhow::Result<Option<Duration>> {
    let Some(seconds) = std::env::var_os("OBSERVER_TEST_PROBATION") else {
        return Ok(None);
    };
    let seconds: u64 = seconds
        .to_str()
        .and_then(|seconds| seconds.parse().ok())
        .with_context(|| {
            format!(
                "OBSERVER_TEST_PROBATION is not a number of seconds: {}",
                seconds.display()
            )
        })?;
    warn!(
        seconds,
        "a new build passes its probation after this many seconds up, not ten minutes"
    );
    Ok(Some(Duration::from_secs(seconds)))
}

/// Where builds are listed and downloaded from: Cfx.re, or, in a build with the `test-catalog`
/// feature, the test catalog when `OBSERVER_TEST_CATALOG` names its port.
fn sources() -> anyhow::Result<(Catalog, Http)> {
    #[cfg(feature = "test-catalog")]
    if let Some(port) = std::env::var_os("OBSERVER_TEST_CATALOG") {
        return test_catalog(&port);
    }
    Ok((Catalog::new(Http::new()?), Http::new()?))
}

/// The browser tests' catalog: build 99001 alone, recommended and supported, whose archive the
/// fake FXServer serves on 127.0.0.1:`port`. Nothing else is listed, and Cfx.re is never reached.
#[cfg(feature = "test-catalog")]
fn test_catalog(port: &std::ffi::OsStr) -> anyhow::Result<(Catalog, Http)> {
    use observer_artifacts::catalog::{Availability, CatalogBuild, Listing, PolicyDate};
    use observer_artifacts::{Branch, Platform};

    const BUILD: u32 = 99001;
    const COMMIT: &str = "0cfcdb5ead57a4a26fcb6ebad98adbcd0273e607";

    let port: u16 = port
        .to_str()
        .and_then(|port| port.parse().ok())
        .with_context(|| format!("OBSERVER_TEST_CATALOG is not a port: {}", port.display()))?;
    let kind = Platform::host().archive();
    let url = format!("http://127.0.0.1:{port}/{BUILD}-{COMMIT}/{}", kind.file_name()).parse()?;
    let build = CatalogBuild {
        build: BUILD,
        commit: Some(COMMIT.parse().map_err(anyhow::Error::msg)?),
        branch: Some(Branch::master()),
        recommended: true,
        latest: false,
        availability: Availability::Downloadable {
            url,
            kind,
            size: None,
        },
        published_at: None,
        end_of_support: Some(PolicyDate::Never),
        end_of_life: Some(PolicyDate::Never),
        predates_policy: false,
        listed: true,
    };
    let listing = Listing {
        platform: Platform::host(),
        builds: vec![build],
        warnings: Vec::new(),
    };
    warn!(port, "builds come from the test catalog, not Cfx.re");
    Ok((Catalog::fixed(Ok(listing)), Http::local(port)))
}

/// Says which optional features this Observer starts without.
fn announce_features(features: &Features) {
    let off: Vec<&str> = Feature::ALL
        .into_iter()
        .filter(|&feature| !features.runs(feature))
        .map(Feature::as_str)
        .collect();
    if !off.is_empty() {
        info!(features = %off.join(", "), "optional features off");
    }
}

/// Closes a feature's tasks, when it started with Observer and still runs.
async fn close(switched: Option<Switched>) {
    if let Some(switched) = switched {
        switched.close().await;
    }
}

/// Waits for the tasks the stop token ends: the HTTP server, then the periodic ones.
async fn join(server: JoinHandle<io::Result<()>>, periodic: [JoinHandle<()>; 3]) -> anyhow::Result<()> {
    if let Err(error) = server.await.context("the HTTP server task failed")? {
        error!(%error, "the HTTP server stopped with an error");
    }
    for (task, name) in periodic.into_iter().zip(["backup", "upkeep", "audit feed"]) {
        task.await.with_context(|| format!("the {name} task failed"))?;
    }
    Ok(())
}

async fn serve(listener: TcpListener, app: Router, stop: CancellationToken) -> io::Result<()> {
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(stop.cancelled_owned())
        .await
}

/// Stops FXServer for good, as `first` asked. The panel keeps serving meanwhile, so that it shows
/// the stop as it happens.
async fn stop_fxserver(
    supervision: &Supervision,
    building: ArtifactsTask,
    scheduling: ScheduleTask,
    supervised: &mut Running,
    signals: &mut Signals,
    first: StopRequest,
) {
    let mut shortened = first.shortened;
    // A running job is cancelled first, then the schedule: it must not restart a server Observer
    // is stopping.
    building.close().await;
    scheduling.close().await;
    supervision.shutdown(shortened).await;
    loop {
        tokio::select! {
            () = supervised.stopped() => return,
            // A console closed during a graceful stop leaves Windows' few seconds only.
            request = signals.next(), if !shortened => {
                if request.shortened {
                    shortened = true;
                    info!("the console is closing; shortening the stop");
                    supervision.shutdown(true).await;
                }
            }
        }
    }
}

fn announce(http: &HttpConfig) {
    let listen = http.listen;
    if listen.ip().is_unspecified() {
        info!(
            "panel listening on port {} of every interface (http://localhost:{})",
            listen.port(),
            listen.port()
        );
    } else {
        info!("panel listening on http://{listen}");
    }
    if http.exposed_over_plain_http() {
        warn!(
            "the panel is served over plain HTTP on {listen}; put it behind an HTTPS proxy or reach it over a VPN"
        );
    }
}

/// Keeps one daily backup no older than a day. Checking every hour rather than sleeping a full day
/// means a host that was off at the usual time still gets its backup soon after it starts.
async fn daily_backups(db: Database, stop: CancellationToken) {
    loop {
        let due = match db.newest_backup(BackupKind::Daily).await {
            Ok(newest) => {
                newest.is_none_or(|at| SystemTime::now().duration_since(at).unwrap_or_default() >= DAILY)
            }
            Err(error) => {
                warn!(%error, "cannot list the database backups");
                false
            }
        };
        if due {
            match db.backup(BackupKind::Daily).await {
                Ok(path) => info!(backup = %path.display(), "database backed up"),
                Err(error) => error!(%error, "the daily database backup failed"),
            }
        }

        tokio::select! {
            () = stop.cancelled() => return,
            () = tokio::time::sleep(BACKUP_CHECK_INTERVAL) => {}
        }
    }
}

/// Every minute, the accounts' upkeep: failed logins reach the audit log, expired sessions go;
/// SFTP's too while it runs. At start and then daily, the audit entries past their retention go.
async fn upkeep(
    accounts: Accounts,
    audit: AuditLog,
    files: observer_app::Files,
    features: Features,
    stop: CancellationToken,
) {
    let mut purged_at: Option<Instant> = None;
    loop {
        if let Err(error) = accounts.sweep().await {
            warn!(%error, "the account upkeep failed");
        }
        if features.runs(Feature::Sftp)
            && let Err(error) = files.sweep().await
        {
            warn!(%error, "the SFTP login upkeep failed");
        }
        if purged_at.is_none_or(|at| at.elapsed() >= DAILY) {
            match audit.purge().await {
                Ok(0) => {}
                Ok(purged) => info!(purged, "expired audit entries deleted"),
                Err(error) => error!(%error, "the audit retention could not be applied"),
            }
            purged_at = Some(Instant::now());
        }

        tokio::select! {
            () = stop.cancelled() => return,
            () = tokio::time::sleep(UPKEEP_INTERVAL) => {}
        }
    }
}

/// Every second, the audit entries written since the last pass go to the panels that show them
/// live. Reading the log rather than its writers also catches what the command line writes.
async fn publish_audit(audit: AuditLog, stop: CancellationToken) {
    loop {
        if let Err(error) = audit.publish_new().await {
            warn!(%error, "new audit entries could not be read");
        }
        tokio::select! {
            () = stop.cancelled() => return,
            () = tokio::time::sleep(AUDIT_FOLLOW_INTERVAL) => {}
        }
    }
}
