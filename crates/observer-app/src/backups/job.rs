//! One backup from start to end: the dump into the work folder, a copy to each destination, the
//! retention of each destination that took its copy, and the record and audit entry of how it
//! ended, whatever happened on the way.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use jiff::{SignedDuration, Timestamp};
use observer_backups::Failure;
use observer_backups::archive::Written;
use observer_backups::destination::{self, Upload, local};
use observer_backups::dump::{self, Client, DumpRequest};
use observer_domain::audit::{ActorKind, AuditAction, Outcome};
use observer_domain::backups::{
    self as rules, BackupDestination, BackupError, BackupOutcome, BackupPhase, DatabaseConnection,
    DeleteReason, DestinationKind, DestinationTarget, Secret,
};
use observer_domain::tools::sha256_hex;
use observer_store::Database;
use observer_store::backups::{self as store, FinishedRun, RunSource};
use serde_json::json;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use super::{Asker, Dumped, Progress, blocking};
use crate::audit::Record;
use crate::{Clock, Error, Servers};

/// Runs whose copies are all gone are forgotten after this.
const FORGET_AFTER: SignedDuration = SignedDuration::from_hours(180 * 24);

pub(super) struct Job {
    pub db: Database,
    pub clock: Clock,
    pub work: PathBuf,
    pub run: i64,
    pub started_at: Timestamp,
    pub asker: Asker,
    /// The server's folder, which no local destination may be in.
    pub server_folder: Option<PathBuf>,
    pub connection: Result<DatabaseConnection, Failure>,
    pub client: Result<Client, Failure>,
    /// The enabled destinations, with their secrets.
    pub destinations: Vec<(BackupDestination, Option<Secret>)>,
    pub progress: Arc<Progress>,
    pub cancel: CancellationToken,
    pub dumped: watch::Sender<Option<Dumped>>,
    pub server_started: Arc<AtomicBool>,
}

/// A dump that made its file.
struct Made {
    file: PathBuf,
    name: String,
    database: String,
    written: Written,
}

/// What became of the copies.
#[derive(Default)]
struct Copies {
    stored: u32,
    failed: u32,
    first_failure: Option<Failure>,
    cancelled: bool,
}

impl Job {
    /// Runs the backup and records how it ended. An error here is the record's own: the run then
    /// stays unfinished until the next start of Observer ends it as interrupted.
    pub(super) async fn run(self) -> Result<(), Error> {
        let made = self.dump().await;
        let made = match made {
            Ok(made) => made,
            Err(failure) => {
                self.dumped.send_replace(Some(Dumped::Failed(failure.error)));
                let outcome = if failure.error == BackupError::Cancelled {
                    BackupOutcome::Cancelled
                } else {
                    BackupOutcome::Failed
                };
                return self
                    .finish(outcome, None, Some(failure), &Copies::default())
                    .await;
            }
        };
        self.dumped.send_replace(Some(Dumped::Done {
            database: made.database.clone(),
            file_bytes: made.written.file_bytes,
        }));

        let mut copies = Copies::default();
        let mut took = Vec::new();
        for (destination, secret) in &self.destinations {
            if self.cancel.is_cancelled() {
                copies.cancelled = true;
                break;
            }
            match self.copy(destination, secret.as_ref(), &made).await? {
                Ok(()) => {
                    copies.stored += 1;
                    took.push((destination, secret));
                }
                Err(failure) => {
                    if failure.error == BackupError::Cancelled {
                        copies.cancelled = true;
                    }
                    warn!(run = self.run, destination = destination.id, error = %failure, "a backup's copy failed");
                    copies.failed += 1;
                    copies.first_failure.get_or_insert(failure);
                }
            }
        }
        self.progress.phase(BackupPhase::Pruning);
        for (destination, secret) in took {
            if let Err(error) = self.prune(destination, secret.as_ref()).await {
                warn!(destination = destination.id, %error, "a destination's old backups could not be let go");
            }
        }
        let outcome = if copies.cancelled {
            BackupOutcome::Cancelled
        } else if copies.failed == 0 {
            BackupOutcome::Succeeded
        } else if copies.stored > 0 {
            BackupOutcome::Partial
        } else {
            BackupOutcome::Failed
        };
        let failure = if copies.cancelled {
            Some(Failure::new(BackupError::Cancelled))
        } else {
            copies.first_failure.clone()
        };
        let finished = self.finish(outcome, Some(&made), failure, &copies).await;
        let file = made.file;
        if let Err(error) = blocking(move || std::fs::remove_file(file)).await? {
            warn!(%error, "a backup's file could not be removed from the work folder");
        }
        finished
    }

    /// The dump into the work folder, read back, under its final name there.
    async fn dump(&self) -> Result<Made, Failure> {
        self.progress.phase(BackupPhase::Preparing);
        let connection = self.connection.clone()?;
        let client = self.client.clone()?;
        let name = rules::file_name(&connection.database, self.started_at, self.run);
        let host = connection
            .socket
            .clone()
            .unwrap_or_else(|| format!("{}:{}", connection.host, connection.port));
        let source = (connection.database.clone(), host, client.describe(), name.clone());
        let run = self.run;
        self.db
            .write(move |tx| {
                store::set_run_source(
                    tx,
                    run,
                    &RunSource {
                        database: &source.0,
                        host: &source.1,
                        client: &source.2,
                        file_name: &source.3,
                    },
                )
            })
            .await
            .map_err(|error| Failure::with(BackupError::Io, error.to_string()))?;

        self.progress.phase(BackupPhase::Dumping);
        let partial = self.work.join(format!("{}.partial", self.run));
        let written = dump::dump(DumpRequest {
            client: &client,
            connection: &connection,
            work: &self.work,
            name: &self.run.to_string(),
            output: &partial,
            progress: Arc::clone(&self.progress.sql),
            cancel: self.cancel.clone(),
        })
        .await?;
        self.progress.phase(BackupPhase::Verifying);
        let file = self.work.join(&name);
        tokio::fs::rename(&partial, &file).await?;
        info!(run = self.run, database = %connection.database, bytes = written.file_bytes, "database dumped");
        Ok(Made {
            file,
            name,
            database: connection.database,
            written,
        })
    }

    /// One copy, recorded as it starts and as it ends. The outer error is the record's.
    async fn copy(
        &self,
        destination: &BackupDestination,
        secret: Option<&Secret>,
        made: &Made,
    ) -> Result<Result<(), Failure>, Error> {
        let location = destination::location(&destination.target, &made.name);
        let now = self.clock.now();
        let (run, id, at) = (self.run, destination.id, location.clone());
        let copy = self
            .db
            .write(move |tx| store::insert_copy(tx, run, id, &at, now))
            .await?;
        self.progress.phase(BackupPhase::Copying);
        self.progress.copying(destination.id);
        let uploaded = match self.refusal(&destination.target).await? {
            Some(failure) => Err(failure),
            None => {
                destination::upload(
                    &destination.target,
                    secret,
                    Upload {
                        file: &made.file,
                        bytes: made.written.file_bytes,
                        sha256: made.written.file_sha256,
                    },
                    &location,
                    Arc::clone(&self.progress.copied),
                    self.cancel.clone(),
                )
                .await
            }
        };
        let now = self.clock.now();
        let result = match uploaded {
            Ok(bytes) if bytes != made.written.file_bytes => Err(Failure::with(
                BackupError::VerifyFailed,
                format!(
                    "the destination holds {bytes} bytes of {}",
                    made.written.file_bytes
                ),
            )),
            Ok(_) => Ok(()),
            Err(failure) => Err(failure),
        };
        let (recorded, bytes) = (result.clone(), made.written.file_bytes);
        self.db
            .write(move |tx| match &recorded {
                Ok(()) => store::copy_stored(tx, copy, bytes, now),
                Err(failure) => store::copy_failed(tx, copy, failure.error, failure.detail.as_deref()),
            })
            .await?;
        Ok(result)
    }

    /// Why a destination may not take a copy, before anything is sent: a local folder inside the
    /// server's, as the launch settings say now.
    async fn refusal(&self, target: &DestinationTarget) -> Result<Option<Failure>, Error> {
        let (DestinationTarget::Local { folder }, Some(server)) = (target, &self.server_folder) else {
            return Ok(None);
        };
        let (folder, server) = (PathBuf::from(folder), server.clone());
        let inside = blocking(move || local::inside(&folder, &server)).await?;
        Ok(inside.then(|| Failure::new(BackupError::InsideServerFolder)))
    }

    /// Lets go the copies the destination's retention no longer keeps, now that it holds this
    /// run's.
    async fn prune(&self, destination: &BackupDestination, secret: Option<&Secret>) -> Result<(), Error> {
        let id = destination.id;
        let rows = self.db.read(move |conn| store::stored_copies(conn, id)).await?;
        let stored: Vec<_> = rows.iter().filter_map(store::CopyRow::stored).collect();
        let now = self.clock.now();
        let expired = rules::expired(&stored, destination.retention, now);
        let mut deleted = Vec::new();
        for row in rows.iter().filter(|row| expired.contains(&row.id)) {
            match destination::delete(&destination.target, secret, &row.location).await {
                Ok(()) => deleted.push(row.id),
                Err(failure) => {
                    warn!(copy = row.id, error = %failure, "an old backup could not be deleted; it is tried again next time");
                }
            }
        }
        if deleted.is_empty() {
            return Ok(());
        }
        // The retention is the destination's rule, applied by Observer whoever asked for the run.
        let record = Record::operator(ActorKind::System, AuditAction::BackupsExpired)
            .target(destination.id.to_string())
            .details(json!({ "run": self.run, "deleted": deleted.len(), "destination": destination.name }));
        self.db
            .write(move |tx| -> Result<(), Error> {
                for copy in deleted {
                    store::copy_deleted(tx, copy, DeleteReason::Retention, now)?;
                }
                record.write(tx, now)?;
                Ok(())
            })
            .await
    }

    /// Records how the run ended, with its audit entry, and forgets the runs whose copies are all
    /// long gone.
    async fn finish(
        &self,
        outcome: BackupOutcome,
        made: Option<&Made>,
        failure: Option<Failure>,
        copies: &Copies,
    ) -> Result<(), Error> {
        let now = self.clock.now();
        let server_started = self.server_started.load(Ordering::Relaxed);
        let written = made.map(|made| &made.written);
        let (sql_sha256, file_sha256) = (
            written.map(|written| sha256_hex(&written.sql_sha256)),
            written.map(|written| sha256_hex(&written.file_sha256)),
        );
        let inspection = written
            .map(|written| written.inspection.clone())
            .unwrap_or_default();
        let mut details = json!({
            "trigger": self.asker.trigger().as_str(),
            "outcome": outcome.as_str(),
            "stored": copies.stored,
            "failed": copies.failed,
            "serverStarted": server_started,
        });
        if let Some(made) = made {
            details["database"] = json!(made.database);
            details["fileBytes"] = json!(made.written.file_bytes);
        }
        if let Some(failure) = &failure {
            details["error"] = json!(failure.error.as_str());
        }
        if let Asker::Restart { by } = &self.asker {
            details["by"] = json!(by);
        }
        let record = self
            .asker
            .record(AuditAction::BackupFinished)
            .target(self.run.to_string())
            .outcome(match outcome {
                BackupOutcome::Succeeded => Outcome::Success,
                _ => Outcome::Failure,
            })
            .details(details);
        let run = self.run;
        let cutoff = now - FORGET_AFTER;
        let (sql_bytes, file_bytes) = (
            written.map(|written| written.sql_bytes),
            written.map(|written| written.file_bytes),
        );
        self.db
            .write(move |tx| -> Result<(), Error> {
                store::finish_run(
                    tx,
                    run,
                    &FinishedRun {
                        at: now,
                        outcome,
                        sql_bytes,
                        file_bytes,
                        sql_sha256: sql_sha256.as_deref(),
                        file_sha256: file_sha256.as_deref(),
                        server_started,
                        loose_tables: &inspection.loose_tables,
                        loose_table_count: inspection.loose_table_count,
                        error: failure.as_ref().map(|failure| failure.error),
                        error_detail: failure.as_ref().and_then(|failure| failure.detail.as_deref()),
                    },
                )?;
                record.write(tx, now)?;
                store::forget_runs(tx, Servers::DEFAULT, cutoff)?;
                Ok(())
            })
            .await
    }
}

/// Copies to the host's own folders first: they are the quickest, and the one a restore reaches
/// first.
pub(super) fn order(destinations: &mut [(BackupDestination, Option<Secret>)]) {
    destinations.sort_by_key(|(destination, _)| destination.target.kind() != DestinationKind::Local);
}
