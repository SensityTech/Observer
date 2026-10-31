use std::io;
use std::net::SocketAddr;
use std::time::{Duration, Instant, SystemTime};

use anyhow::Context;
use axum::Router;
use jiff::tz::TimeZone;
use observer_app::{
    Accounts, Artifacts, ArtifactsSetup, ArtifactsTask, AuditLog, Clock, DatabaseFiles, Diagnostics,
    DiagnosticsSetup, Metrics, MetricsSetup, PasswordCost, Running, Schedule, ScheduleSetup, ScheduleTask,
    Servers, Setup, Sources, Supervision,
};
use observer_artifacts::Http;
use observer_artifacts::catalog::Catalog;
use observer_fxserver::console::{ConsoleLog, LogLimits};
use observer_store::{BackupKind, Database};
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
    let accounts = Accounts::new(db.clone(), clock.clone(), PasswordCost::STANDARD).await?;
    let audit = AuditLog::new(db.clone(), clock.clone());
    let servers = Servers::new(db.clone(), clock.clone());
    let instance_id = db.instance_id().await?;
    let console_logs = paths.logs.join("console");
    let console_log = ConsoleLog::start(&console_logs, LogLimits::default(), TimeZone::system())
        .context("cannot open the console log")?;
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
        |_| {},
    )
    .await?;
    // Before FXServer boots, so that the first run's tree is seen.
    let (diagnostics, diagnosing) = Diagnostics::start(DiagnosticsSetup {
        db: db.clone(),
        clock: clock.clone(),
        supervision: supervision.clone(),
        data_dir: paths.root.clone(),
    })
    .await?;
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
    let metrics = Metrics::open(MetricsSetup {
        path: paths.metrics.clone(),
        clock: Clock::system(),
    })
    .await
    .with_context(|| format!("cannot open {}", paths.metrics.display()))?;

    let listener = TcpListener::bind(config.http.listen)
        .await
        .with_context(|| format!("cannot listen on {}", config.http.listen))?;
    announce(&config.http);
    let files = observer_app::Files::new(db.clone(), clock.clone(), accounts.clone());
    // Whether or not SFTP is on now, a crash may have left uploads from when it was.
    if let Err(error) = observer_sftp::sweep_uploads(&files).await {
        warn!(%error, "the temporary SFTP uploads could not all be removed");
    }
    // Right after the panel's port, so that a port taken stops Observer before FXServer starts.
    let sftp_listener = match &config.sftp {
        Some(sftp) => {
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
        None => None,
    };
    let setup_token = setup::offer_link(&accounts, config.http.listen).await?;

    let stop = CancellationToken::new();
    let services = observer_web::Services {
        accounts: accounts.clone(),
        audit: audit.clone(),
        servers,
        supervision: supervision.clone(),
        metrics: metrics.clone(),
        schedule: schedule.clone(),
        artifacts,
        diagnostics,
        files: files.clone(),
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
    let sampling = metrics.start(Sources {
        supervision: supervision.clone(),
        sockets: live.sockets(),
        data_dir: paths.root.clone(),
        databases: DatabaseFiles {
            state: paths.database.clone(),
            metrics: paths.metrics.clone(),
        },
    });
    let server = tokio::spawn(serve(listener, observer_web::router(state), stop.clone()));
    let backups = tokio::spawn(daily_backups(db.clone(), stop.clone()));
    let upkeep = tokio::spawn(upkeep(
        accounts.clone(),
        audit.clone(),
        files.clone(),
        stop.clone(),
    ));
    let sftp = sftp_listener.map(|(listener, host_key)| {
        // Observer's own folders and files, should the server folder hold them.
        let refused = [
            Some(paths.root.clone()),
            std::env::current_exe()
                .ok()
                .and_then(|exe| exe.parent().map(std::path::Path::to_path_buf)),
            Some(config.path.clone()),
        ]
        .into_iter()
        .flatten()
        .collect();
        tokio::spawn(observer_sftp::serve(
            listener,
            files.clone(),
            host_key,
            refused,
            stop.clone(),
        ))
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
    supervised.close().await;
    diagnosing.close().await;
    // After the supervision, so that FXServer's stop is in the statistics.
    sampling.close().await;
    stop.cancel();
    live.shutdown().await;

    join(server, [backups, upkeep, audit_feed]).await?;
    if let Some(sftp) = sftp
        && let Err(error) = sftp.await.context("the SFTP server task failed")?
    {
        error!(%error, "the SFTP server stopped with an error");
    }
    console_log.close().await;
    metrics
        .close()
        .await
        .context("cannot close the metrics database")?;
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
    Artifacts::start(setup, |_| {})
        .await
        .with_context(|| format!("cannot use {} for FXServer builds", root.display()))
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

/// Every minute, the accounts' upkeep: failed logins reach the audit log, expired sessions go.
/// At start and then daily, the audit entries past their retention go too.
async fn upkeep(accounts: Accounts, audit: AuditLog, files: observer_app::Files, stop: CancellationToken) {
    let mut purged_at: Option<Instant> = None;
    loop {
        if let Err(error) = accounts.sweep().await {
            warn!(%error, "the account upkeep failed");
        }
        if let Err(error) = files.sweep().await {
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
