use std::net::SocketAddr;
use std::time::{Duration, Instant, SystemTime};

use anyhow::Context;
use observer_app::{Accounts, AuditLog, Clock, PasswordCost};
use observer_store::{BackupKind, Database};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::config::Config;
use crate::{lock, logging, setup, shutdown};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const DAILY: Duration = Duration::from_hours(24);
const BACKUP_CHECK_INTERVAL: Duration = Duration::from_hours(1);
const UPKEEP_INTERVAL: Duration = Duration::from_mins(1);

pub async fn run(config: Config) -> anyhow::Result<()> {
    let paths = config.paths();
    let _lock = lock::acquire(&paths.root)?;
    let _log_guard = logging::init(&config.log.level, &paths.logs)?;

    info!(version = VERSION, data_dir = %paths.root.display(), "starting Observer");

    let db = Database::open(&paths.database, &paths.backups)
        .await
        .with_context(|| format!("cannot open {}", paths.database.display()))?;
    let clock = Clock::system();
    let accounts = Accounts::new(db.clone(), clock.clone(), PasswordCost::STANDARD).await?;
    let audit = AuditLog::new(db.clone(), clock);
    let instance_id = db.instance_id().await?;

    let listener = TcpListener::bind(config.http.listen)
        .await
        .with_context(|| format!("cannot listen on {}", config.http.listen))?;
    announce(config.http.listen);
    setup::offer_link(&accounts, config.http.listen).await?;

    let stop = CancellationToken::new();
    let state = observer_web::AppState::new(VERSION, accounts.clone(), audit.clone(), &instance_id);
    let app = observer_web::router(state);
    let server = tokio::spawn({
        let stop = stop.clone();
        async move {
            axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
                .with_graceful_shutdown(stop.cancelled_owned())
                .await
        }
    });
    let backups = tokio::spawn(daily_backups(db.clone(), stop.clone()));
    let upkeep = tokio::spawn(upkeep(accounts, audit, stop.clone()));

    shutdown::requested().await;
    info!("stopping Observer");
    stop.cancel();

    if let Err(error) = server.await.context("the HTTP server task failed")? {
        error!(%error, "the HTTP server stopped with an error");
    }
    backups.await.context("the backup task failed")?;
    upkeep.await.context("the upkeep task failed")?;
    db.close().await.context("cannot close the database")?;

    info!("Observer stopped");
    Ok(())
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
