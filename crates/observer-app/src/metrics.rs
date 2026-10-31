//! The statistics: a sample of FXServer, the host and Observer at the end of every five-second
//! slot, kept in the metrics database with its aggregates, and sent live to the dashboards.

mod runs;
mod sampler;
mod upkeep;

use std::path::PathBuf;
use std::sync::Arc;

use observer_domain::metrics::{LiveSample, MetricsSeries, Range, RunGrowth, Series, ThreadShares};
use observer_store::MetricsDatabase;
use observer_store::metrics::{growth, runs as stored_runs, series, threads};
use tokio::sync::{broadcast, watch};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::{Clock, Error, Supervision};

/// Live samples a socket may fall behind by before it is told to start over.
const LIVE_SAMPLES: usize = 16;

pub struct MetricsSetup {
    pub path: PathBuf,
    pub clock: Clock,
}

/// What the sampler measures besides the supervisor's server.
pub struct Sources {
    pub supervision: Supervision,
    /// The open live sockets.
    pub sockets: watch::Receiver<usize>,
    /// Observer's data folder, whose volume the host's disk figures are about.
    pub data_dir: PathBuf,
    pub databases: DatabaseFiles,
}

/// The database files whose size the statistics follow, each with its write-ahead log.
pub struct DatabaseFiles {
    pub state: PathBuf,
    pub metrics: PathBuf,
}

/// The statistics service. Clones share it.
#[derive(Clone)]
pub struct Metrics {
    inner: Arc<Inner>,
}

struct Inner {
    db: MetricsDatabase,
    clock: Clock,
    /// When this service started, in Unix milliseconds: runs are numbered from it.
    started_ms: i64,
    latest: watch::Sender<Option<LiveSample>>,
    live: broadcast::Sender<LiveSample>,
}

/// The sampler and the upkeep, until [`Sampling::close`].
pub struct Sampling {
    stop: CancellationToken,
    tasks: Vec<JoinHandle<()>>,
}

impl Sampling {
    pub async fn close(self) {
        self.stop.cancel();
        for task in self.tasks {
            if let Err(error) = task.await {
                warn!(%error, "a statistics task failed");
            }
        }
    }
}

impl Metrics {
    /// Opens the metrics database, and ends the runs an earlier Observer could not.
    pub async fn open(setup: MetricsSetup) -> Result<Self, Error> {
        let db = MetricsDatabase::open(setup.path).await?;
        let lost = db.write(stored_runs::close_lost).await?;
        if lost > 0 {
            info!(lost, "runs left open by an earlier Observer were closed");
        }
        Ok(Self {
            inner: Arc::new(Inner {
                db,
                started_ms: setup.clock.now().as_millisecond(),
                clock: setup.clock,
                latest: watch::channel(None).0,
                live: broadcast::channel(LIVE_SAMPLES).0,
            }),
        })
    }

    /// Starts sampling and the database upkeep.
    pub fn start(&self, sources: Sources) -> Sampling {
        let stop = CancellationToken::new();
        let sampler = sampler::Sampler::new(self.clone(), sources);
        let tasks = vec![
            tokio::spawn(sampler.run(stop.clone())),
            tokio::spawn(upkeep::run(
                self.inner.db.clone(),
                self.inner.clock.clone(),
                stop.clone(),
            )),
        ];
        Sampling { stop, tasks }
    }

    /// The range ending now, on its fixed grid.
    pub async fn series(&self, range: Range, wanted: &[Series]) -> Result<MetricsSeries, Error> {
        let now = self.inner.clock.now().as_second();
        let wanted = wanted.to_vec();
        Ok(self
            .inner
            .db
            .read(move |conn| series::read(conn, range, &wanted, now))
            .await?)
    }

    /// CPU per thread name of FXServer's main process over the last hour.
    pub async fn threads(&self) -> Result<ThreadShares, Error> {
        let now = self.inner.clock.now().as_second();
        Ok(self.inner.db.read(move |conn| threads::read(conn, now)).await?)
    }

    /// How FXServer's memory and handles grew over the run under way, if one is.
    pub async fn growth(&self) -> Result<Option<RunGrowth>, Error> {
        let now = self.inner.clock.now().as_second();
        let observer_started = self.inner.started_ms;
        Ok(self
            .inner
            .db
            .read(move |conn| growth::read(conn, observer_started, now))
            .await?)
    }

    pub(crate) fn database(&self) -> &MetricsDatabase {
        &self.inner.db
    }

    /// When this service started: runs are numbered from it.
    pub(crate) fn observer_started(&self) -> i64 {
        self.inner.started_ms
    }

    /// The newest sample, and the ones that follow it.
    pub fn live(&self) -> (Option<LiveSample>, broadcast::Receiver<LiveSample>) {
        let next = self.inner.live.subscribe();
        (self.inner.latest.borrow().clone(), next)
    }

    /// Closes the database for every clone, once the sampling is closed.
    pub async fn close(self) -> Result<(), Error> {
        Ok(self.inner.db.clone().close().await?)
    }

    fn publish(&self, sample: LiveSample) {
        self.inner.latest.send_replace(Some(sample.clone()));
        let _ = self.inner.live.send(sample);
    }
}
