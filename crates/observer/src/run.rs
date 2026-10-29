use std::io;
use std::net::SocketAddr;
use std::time::{Duration, Instant, SystemTime};

use anyhow::Context;
use axum::Router;
use jiff::tz::TimeZone;
use observer_app::{
    Accounts, AuditLog, Clock, DatabaseFiles, Diagnostics, DiagnosticsSetup, Metrics, MetricsSetup,
    PasswordCost, Running, Schedule, ScheduleSetup, ScheduleTask, Servers, Setup, Sources, Supervision,
};
use observer_fxserver::console::{ConsoleLog, LogLimits};
use observer_store::{BackupKind, Database};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::config::Config;
use crate::shutdown::{Shutdown, Signals};
use crate::{lock, logging, setup};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const DAILY: Duration = Duration::from_hours(24);
const BACKUP_CHECK_INTERVAL: Duration = Duration::from_hours(1);
const UPKEEP_INTERVAL: Duration = Duration::from_mins(1);
const AUDIT_FOLLOW_INTERVAL: Duration = Duration::from_secs(1);

#[expect(
    clippy::too_many_lines,
    reason = "every service starts and stops here, in an order that reads best in one place"
)]
pub async fn run(config: Config) -> anyhow::Result<()> {
    let paths = config.paths();
    let _lock = lock::acquire(&paths.root)?;
    let log_guard = logging::init(&config.log.level, &paths.logs)?;
    let mut signals = Signals::listen().context("cannot listen for stop requests")?;

    info!(version = VERSION, data_dir = %paths.root.display(), "starting Observer");

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
    let metrics = Metrics::open(MetricsSetup {
        path: paths.metrics.clone(),
        clock: Clock::system(),
    })
    .await
    .with_context(|| format!("cannot open {}", paths.metrics.display()))?;

    let listener = TcpListener::bind(config.http.listen)
        .await
        .with_context(|| format!("cannot listen on {}", config.http.listen))?;
    announce(config.http.listen);
    setup::offer_link(&accounts, config.http.listen).await?;

    let stop = CancellationToken::new();
    let services = observer_web::Services {
        accounts: accounts.clone(),
        audit: audit.clone(),
        servers,
        supervision: supervision.clone(),
        metrics: metrics.clone(),
        schedule,
        diagnostics,
    };
    let state = observer_web::AppState::new(VERSION, services, &instance_id, &config.http.hosts);
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
    let upkeep = tokio::spawn(upkeep(accounts, audit.clone(), stop.clone()));
    let audit_feed = tokio::spawn(publish_audit(audit, stop.clone()));
    if let Err(error) = supervision.boot().await {
        error!(%error, "cannot start FXServer with Observer");
    }

    stop_fxserver(&supervision, scheduling, &mut supervised, &mut signals).await;
    supervised.close().await;
    diagnosing.close().await;
    // After the supervision, so that FXServer's stop is in the statistics.
    sampling.close().await;
    stop.cancel();
    live.shutdown().await;

    if let Err(error) = server.await.context("the HTTP server task failed")? {
        error!(%error, "the HTTP server stopped with an error");
    }
    backups.await.context("the backup task failed")?;
    upkeep.await.context("the upkeep task failed")?;
    audit_feed.await.context("the audit feed task failed")?;
    console_log.close().await;
    metrics
        .close()
        .await
        .context("cannot close the metrics database")?;
    db.close().await.context("cannot close the database")?;

    info!("Observer stopped");
    drop(log_guard);
    signals.release();
    Ok(())
}

async fn serve(listener: TcpListener, app: Router, stop: CancellationToken) -> io::Result<()> {
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(stop.cancelled_owned())
        .await
}

/// Waits for a stop request, then stops FXServer for good. The panel keeps serving meanwhile, so
/// that it shows the stop as it happens.
async fn stop_fxserver(
    supervision: &Supervision,
    scheduling: ScheduleTask,
    supervised: &mut Running,
    signals: &mut Signals,
) {
    let mut shortened = signals.next().await == Shutdown::Shortened;
    info!(shortened, "stopping Observer");
    // The schedule goes first: it must not restart a server Observer is stopping.
    scheduling.close().await;
    supervision.shutdown(shortened).await;
    loop {
        tokio::select! {
            () = supervised.stopped() => return,
            // A console closed during a graceful stop leaves Windows' few seconds only.
            shutdown = signals.next(), if !shortened => {
                if shutdown == Shutdown::Shortened {
                    shortened = true;
                    info!("the console is closing; shortening the stop");
                    supervision.shutdown(true).await;
                }
            }
        }
    }
}

fn announce(listen: SocketAddr) {
    if listen.ip().is_unspecified() {
        info!(
            "panel listening on port {} of every interface (http://localhost:{})",
            listen.port(),
            listen.port()
        );
    } else {
        info!("panel listening on http://{listen}");
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
async fn upkeep(accounts: Accounts, audit: AuditLog, stop: CancellationToken) {
    let mut purged_at: Option<Instant> = None;
    loop {
        if let Err(error) = accounts.sweep().await {
            warn!(%error, "the account upkeep failed");
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
