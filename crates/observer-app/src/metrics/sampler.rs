//! The sampler. It wakes at the end of every five-second slot, measures the operating system's
//! sources in parallel, and assembles the slot's sample from them, from what the supervisor's
//! probes read in the middle of the slot, and from the events it reported during it.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use observer_domain::metrics::{ChildProcess, LiveSample, SLOT_MS, Series, next_slot_end, slot_of};
use observer_fxserver::perf::{Thread, TickReport};
use observer_fxserver::supervisor::{Event, EventKind, RunReadings};
use observer_platform::{CpuTimes, HostUsage, Interface, Space, TreeUsage, Usage};
use observer_store::metrics::samples::{self, Sample};
use observer_store::metrics::{Column, Values, series};
use tokio::sync::broadcast::error::RecvError;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use super::runs::{Change, Runs};
use super::{DatabaseFiles, Metrics, Sources};
use crate::supervision::server_state;

/// The wake comes this long after the slot ends, so that the slot's last events are in.
const WAKE_AFTER_MS: i64 = 100;
/// Each operating system source gets this long; one that takes longer is left out of the slot.
const SOURCE_LIMIT: Duration = Duration::from_secs(2);
/// `/dynamic.json` answers every second; an older answer says nothing of this slot.
const DYNAMIC_FRESH: Duration = Duration::from_secs(5);
/// The bridge reports the JS heap every ten seconds.
const HEAP_FRESH: Duration = Duration::from_secs(25);

const TICK_COLUMNS: [(Thread, [Column; 3]); 3] = [
    (
        Thread::Main,
        [Column::MainTicks, Column::MainTickMs, Column::MainOver],
    ),
    (
        Thread::Network,
        [Column::NetworkTicks, Column::NetworkTickMs, Column::NetworkOver],
    ),
    (
        Thread::Sync,
        [Column::SyncTicks, Column::SyncTickMs, Column::SyncOver],
    ),
];

pub(super) struct Sampler {
    metrics: Metrics,
    sources: Sources,
}

impl Sampler {
    pub(super) fn new(metrics: Metrics, sources: Sources) -> Self {
        Self { metrics, sources }
    }

    pub(super) async fn run(self, stop: CancellationToken) {
        let Self { metrics, sources } = self;
        let supervisor = sources.supervision.supervisor().clone();
        let mut events = supervisor.events();
        let status = supervisor.status();
        let readings = supervisor.readings();
        let clock = metrics.inner.clock.clone();
        let mut runs = Runs::new(metrics.inner.db.clone(), metrics.inner.started_ms);
        let mut assembler = Assembler::new(Instant::now());
        let mut busy = Busy::default();
        let mut events_open = true;
        let mut writer = Writer::default();
        let mut wake = Wake::after(clock.now().as_millisecond());

        loop {
            if let Some(wait) = wake.wait(clock.now().as_millisecond()) {
                tokio::select! {
                () = stop.cancelled() => break,
                event = events.recv(), if events_open => {
                    match event {
                        Ok(event) => fold(&mut assembler, &mut runs, &event).await,
                        Err(RecvError::Lagged(missed)) => {
                            if !assembler.slot.lagged {
                                warn!(missed, "the statistics missed supervisor events; this slot's counts are left out");
                            }
                            assembler.lagged();
                        }
                        Err(RecvError::Closed) => events_open = false,
                    }
                    continue;
                }
                () = tokio::time::sleep(Duration::from_millis(wait)) => {}
                }
                continue;
            }

            let measured_at = Instant::now();
            let ts = slot_just_ended(clock.now().as_millisecond());
            let (state, generation, build) = {
                let status = status.borrow();
                (server_state(status.state), status.generation, status.build)
            };
            let run = readings.borrow().run.clone();
            if let (Some(build), true) = (
                build,
                run.as_ref().is_some_and(|run| run.generation == generation),
            ) {
                runs.build(generation, build).await;
            }
            let measures = measure(&mut busy, run.as_ref(), &sources.data_dir, &sources.databases).await;
            let collect_ms = u64::try_from(measured_at.elapsed().as_millis()).unwrap_or(u64::MAX);
            let (values, children) = assembler.sample(Taken {
                at: measured_at,
                run,
                measures,
                sockets: *sources.sockets.borrow(),
                collect_ms,
            });
            let sample = Sample { ts, state, values };
            writer.write(&metrics, &sample).await;
            metrics.publish(live(&sample, children, assembler.processors));
            wake = Wake::after(clock.now().as_millisecond());
        }
        // The stop of FXServer that came just before is part of its run.
        while let Ok(event) = events.try_recv() {
            fold(&mut assembler, &mut runs, &event).await;
        }
    }
}

/// When the sampler wakes next: 100 ms after the end of the slot under way. It is set once per
/// sample, so that an event right after a slot ends does not push the wake to the next slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Wake {
    at: i64,
}

impl Wake {
    fn after(now: i64) -> Self {
        Self {
            at: next_slot_end(now) + WAKE_AFTER_MS,
        }
    }

    /// The milliseconds left before the wake, or `None` once it is due. A clock set back by more
    /// than a slot finds the next wake again rather than wait for the clock to catch up.
    fn wait(&mut self, now: i64) -> Option<u64> {
        if self.at - now > SLOT_MS + WAKE_AFTER_MS {
            *self = Self::after(now);
        }
        u64::try_from(self.at - now).ok().filter(|&left| left > 0)
    }
}

/// The slot a wake at `ms` samples. A wake late by more than a slot, after a suspend or under
/// load, samples the one that just ended; those it missed stay gaps.
fn slot_just_ended(ms: i64) -> i64 {
    slot_of(ms - SLOT_MS)
}

async fn fold(assembler: &mut Assembler, runs: &mut Runs, event: &Event) {
    assembler.event(event, Instant::now());
    if let Some(change) = Change::of(event) {
        runs.apply(event.generation, change).await;
    }
}

/// Writes the samples, warning once per streak of failures, and once each time the clock went
/// back over slots already written.
#[derive(Default)]
struct Writer {
    newest: Option<i64>,
    behind: bool,
    failing: bool,
}

impl Writer {
    async fn write(&mut self, metrics: &Metrics, sample: &Sample) {
        if self.newest.is_some_and(|newest| sample.ts <= newest) {
            if !self.behind {
                warn!(
                    ts = sample.ts,
                    "the clock went back: samples already written are replaced"
                );
            }
            self.behind = true;
        } else {
            self.behind = false;
            self.newest = Some(sample.ts);
        }
        let stored = sample.clone();
        let written = metrics
            .inner
            .db
            .write(move |tx| samples::insert(tx, &stored))
            .await;
        match written {
            Ok(()) => self.failing = false,
            Err(error) => {
                if !self.failing {
                    warn!(%error, "a statistics sample could not be written; it is shown live only");
                }
                self.failing = true;
            }
        }
    }
}

fn live(sample: &Sample, children: Vec<ChildProcess>, cores: Option<u32>) -> LiveSample {
    LiveSample {
        ts: sample.ts,
        state: sample.state,
        values: Series::ALL
            .into_iter()
            .filter_map(|wanted| Some((wanted, series::value(wanted, &sample.values)?)))
            .collect(),
        children,
        cores,
    }
}

/// What the operating system said at one wake. `None` for a source that failed, took too long,
/// or had nothing to measure.
#[derive(Debug, Clone, Default)]
pub(super) struct Measures {
    pub tree: Option<TreeUsage>,
    pub observer: Option<Usage>,
    pub host: Option<HostUsage>,
    pub disk: Option<Disk>,
}

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Disk {
    pub space: Option<Space>,
    pub state_db: Option<u64>,
    pub metrics_db: Option<u64>,
}

/// The sources still busy from an earlier slot, which are not started again until they finish.
#[derive(Default)]
struct Busy {
    tree: Option<JoinHandle<io::Result<TreeUsage>>>,
    observer: Option<JoinHandle<io::Result<Usage>>>,
    host: Option<JoinHandle<HostUsage>>,
    disk: Option<JoinHandle<Disk>>,
}

async fn measure(
    busy: &mut Busy,
    run: Option<&RunReadings>,
    data_dir: &Path,
    databases: &DatabaseFiles,
) -> Measures {
    // Upgraded for this measurement only: the tree goes as soon as its run is over.
    let tree = run
        .and_then(|run| run.tree.upgrade())
        .map(|tree: Arc<_>| move || tree.usage());
    let disk = {
        let (data_dir, state, metrics) = (
            data_dir.to_path_buf(),
            databases.state.clone(),
            databases.metrics.clone(),
        );
        move || Disk {
            space: observer_platform::volume_space(&data_dir).ok(),
            state_db: database_size(&state),
            metrics_db: database_size(&metrics),
        }
    };
    let (tree, observer, host, disk) = tokio::join!(
        source(&mut busy.tree, tree),
        source(&mut busy.observer, Some(observer_platform::own_usage)),
        source(&mut busy.host, Some(observer_platform::host_usage)),
        source(&mut busy.disk, Some(disk)),
    );
    Measures {
        tree: tree.and_then(Result::ok),
        observer: observer.and_then(Result::ok),
        host,
        disk,
    }
}

/// Runs one source on a blocking thread, unless it is still busy from an earlier slot.
async fn source<T, F>(busy: &mut Option<JoinHandle<T>>, job: Option<F>) -> Option<T>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    if busy.as_ref().is_some_and(|task| !task.is_finished()) {
        return None;
    }
    let mut task = tokio::task::spawn_blocking(job?);
    if let Ok(done) = tokio::time::timeout(SOURCE_LIMIT, &mut task).await {
        *busy = None;
        done.ok()
    } else {
        *busy = Some(task);
        None
    }
}

/// A database file with its write-ahead log.
fn database_size(path: &Path) -> Option<u64> {
    let main = std::fs::metadata(path).ok()?.len();
    let mut wal = path.as_os_str().to_owned();
    wal.push("-wal");
    Some(main + std::fs::metadata(PathBuf::from(wal)).map_or(0, |metadata| metadata.len()))
}

/// What one wake of the sampler gathered.
pub(super) struct Taken {
    pub at: Instant,
    pub run: Option<RunReadings>,
    pub measures: Measures,
    pub sockets: usize,
    pub collect_ms: u64,
}

/// The events of the slot under way.
#[derive(Debug, Default)]
struct SlotEvents {
    /// A run existed at some point of the slot.
    run: bool,
    /// Events were lost: the counts below are not the slot's.
    lagged: bool,
    hitches: [u64; 3],
    longest_hitch: Option<u64>,
    barks: u64,
}

#[derive(Debug, Clone, Copy)]
struct Measured {
    at: Instant,
    usage: Usage,
}

struct Heap {
    generation: u32,
    at: Instant,
    used: u64,
    total: u64,
}

type Children = HashMap<(u32, u64), Measured>;

/// Turns each wake into a sample, keeping what the next one is measured against: rates and CPU
/// shares are deltas between two wakes of the same process, joins and ticks deltas between two
/// readings of the same run.
pub(super) struct Assembler {
    previous_wake: Instant,
    /// The main process, known by its generation and start stamp.
    main: Option<(u32, Measured)>,
    /// The children by pid and start stamp, as the run's last measure of the tree saw them.
    children: Option<(u32, Children)>,
    observer: Option<Measured>,
    host_cpu: Option<CpuTimes>,
    /// The machine's processors, as the host's CPU share counts them.
    processors: Option<u32>,
    interfaces: Option<(Instant, Vec<Interface>)>,
    perf: Option<(u32, TickReport)>,
    players: Option<(u32, HashSet<u32>)>,
    heap: Option<Heap>,
    resources: Option<(u32, usize)>,
    slot: SlotEvents,
}

impl Assembler {
    pub(super) fn new(started: Instant) -> Self {
        Self {
            previous_wake: started,
            main: None,
            children: None,
            observer: None,
            host_cpu: None,
            processors: None,
            interfaces: None,
            perf: None,
            players: None,
            heap: None,
            resources: None,
            slot: SlotEvents::default(),
        }
    }

    pub(super) fn event(&mut self, event: &Event, at: Instant) {
        match &event.kind {
            EventKind::Spawned { .. } => self.slot.run = true,
            EventKind::Hitch { thread, ms } => {
                let index = match thread.as_str() {
                    "svMain" => 0,
                    "svNetwork" => 1,
                    "svSync" => 2,
                    _ => return,
                };
                self.slot.hitches[index] += 1;
                self.slot.longest_hitch = self.slot.longest_hitch.max(Some(*ms));
            }
            EventKind::WatchdogBark { .. } => self.slot.barks += 1,
            EventKind::Heap {
                heap_used,
                heap_total,
                ..
            } => {
                self.heap = Some(Heap {
                    generation: event.generation,
                    at,
                    used: *heap_used,
                    total: *heap_total,
                });
            }
            EventKind::Resources { started } => self.resources = Some((event.generation, started.len())),
            _ => {}
        }
    }

    pub(super) fn lagged(&mut self) {
        self.slot.lagged = true;
    }

    pub(super) fn sample(&mut self, wake: Taken) -> (Values, Vec<ChildProcess>) {
        let mut values = Values::default();
        let run = wake.run.as_ref();
        let generation = run.map(|run| run.generation);
        // A reading is the slot's when it came after the previous wake.
        let since = self.previous_wake;
        let of_slot = |at: Instant| at > since;

        if let Some(dynamic) = run
            .and_then(|run| run.dynamic.as_ref())
            .filter(|dynamic| wake.at.saturating_duration_since(dynamic.at) <= DYNAMIC_FRESH)
        {
            values.set(Column::Players, Some(real(dynamic.value.clients)));
            values.set(Column::Slots, dynamic.value.max_clients.map(real));
        }

        let main_ticks = self.ticks(&mut values, run, of_slot);
        self.players(&mut values, run, of_slot, main_ticks);

        if (self.slot.run || run.is_some()) && !self.slot.lagged {
            let [main, network, sync] = self.slot.hitches.map(real);
            values.set(Column::HitchMain, Some(main));
            values.set(Column::HitchNetwork, Some(network));
            values.set(Column::HitchSync, Some(sync));
            values.set(Column::Barks, Some(real(self.slot.barks)));
            values.set(Column::HitchMaxMs, self.slot.longest_hitch.map(real));
        }
        self.slot = SlotEvents {
            run: run.is_some(),
            ..SlotEvents::default()
        };

        if let Some(heap) = self.heap.as_ref().filter(|heap| {
            Some(heap.generation) == generation && wake.at.saturating_duration_since(heap.at) <= HEAP_FRESH
        }) {
            values.set(Column::JsHeapUsed, Some(real(heap.used)));
            values.set(Column::JsHeapTotal, Some(real(heap.total)));
        }
        if let Some((_, count)) = self.resources.filter(|(of, _)| Some(*of) == generation) {
            values.set(Column::Resources, Some(real(count as u64)));
        }

        let children = self.tree(&mut values, run, wake.measures.tree, wake.at);
        self.host(&mut values, wake.measures.host, wake.at);
        if let Some(disk) = wake.measures.disk {
            values.set(
                Column::HostDiskFree,
                disk.space.map(|space| real(space.available)),
            );
            values.set(Column::HostDiskTotal, disk.space.map(|space| real(space.total)));
            values.set(Column::ObsStateDb, disk.state_db.map(real));
            values.set(Column::ObsMetricsDb, disk.metrics_db.map(real));
        }
        if let Some(usage) = wake.measures.observer {
            let now = Measured { at: wake.at, usage };
            values.set(Column::ObsCpu, self.observer.and_then(|before| cpu(before, now)));
            values.set(Column::ObsWs, Some(real(usage.working_set)));
            values.set(Column::ObsPrivate, Some(real(usage.private)));
            self.observer = Some(now);
        }
        values.set(Column::ObsWsClients, Some(real(wake.sockets as u64)));
        values.set(Column::CollectMs, Some(real(wake.collect_ms)));
        self.previous_wake = wake.at;
        (values, children)
    }

    /// Each thread's ticks since the run's previous reading, and svMain's count of them.
    fn ticks(
        &mut self,
        values: &mut Values,
        run: Option<&RunReadings>,
        of_slot: impl Fn(Instant) -> bool,
    ) -> Option<u64> {
        let Some(run) = run else {
            self.perf = None;
            return None;
        };
        if self.perf.as_ref().is_some_and(|(of, _)| *of != run.generation) {
            self.perf = None;
        }
        // Without a reading the baseline stays, and the next delta spans both slots.
        let reading = run.perf.as_ref().filter(|reading| of_slot(reading.at))?;
        let deltas = self
            .perf
            .as_ref()
            .map(|(_, baseline)| reading.value.since(baseline));
        self.perf = Some((run.generation, reading.value.clone()));
        let deltas = deltas?;
        for (thread, [ticks, ms, over]) in TICK_COLUMNS {
            if let Some(delta) = deltas.get(&thread) {
                values.set(ticks, Some(real(delta.ticks)));
                values.set(ms, Some(real(delta.total_ms)));
                values.set(over, delta.over.map(real));
            }
        }
        deltas.get(&Thread::Main).map(|delta| delta.ticks)
    }

    /// Pings of the slot's player list, and who joined or left since the run's previous list.
    /// FXServer rebuilds the list from its main loop, so it says nothing while svMain stands still.
    fn players(
        &mut self,
        values: &mut Values,
        run: Option<&RunReadings>,
        of_slot: impl Fn(Instant) -> bool,
        main_ticks: Option<u64>,
    ) {
        let Some(run) = run else {
            self.players = None;
            return;
        };
        if self.players.as_ref().is_some_and(|(of, _)| *of != run.generation) {
            self.players = None;
        }
        let Some(list) = run
            .players
            .as_ref()
            .filter(|reading| of_slot(reading.at) && !reading.value.anonymous)
        else {
            return;
        };
        let ids: HashSet<u32> = list.value.players.iter().map(|&(id, _)| id).collect();
        let previous = self.players.replace((run.generation, ids.clone()));
        if main_ticks.is_none_or(|ticks| ticks == 0) {
            return;
        }
        let mut pings: Vec<u32> = list.value.players.iter().filter_map(|&(_, ping)| ping).collect();
        pings.sort_unstable();
        if let Some(&max) = pings.last() {
            let count = pings.len();
            let total: u64 = pings.iter().map(|&ping| u64::from(ping)).sum();
            // Nearest rank: the smallest ping at or above 95% of the players.
            let rank = (count * 95).div_ceil(100).max(1);
            values.set(Column::PingAvgMs, Some(real(total) / real(count as u64)));
            values.set(Column::PingP95Ms, Some(f64::from(pings[rank - 1])));
            values.set(Column::PingMaxMs, Some(f64::from(max)));
        }
        if let Some((_, before)) = previous {
            values.set(Column::Joins, Some(real(ids.difference(&before).count() as u64)));
            values.set(Column::Leaves, Some(real(before.difference(&ids).count() as u64)));
        }
    }

    fn tree(
        &mut self,
        values: &mut Values,
        run: Option<&RunReadings>,
        tree: Option<TreeUsage>,
        at: Instant,
    ) -> Vec<ChildProcess> {
        let Some(run) = run else {
            self.main = None;
            self.children = None;
            return Vec::new();
        };
        let Some(tree) = tree else {
            return Vec::new();
        };
        if let Some(usage) = tree.main {
            let now = Measured { at, usage };
            let before = self
                .main
                .filter(|(generation, before)| {
                    *generation == run.generation && before.usage.started == usage.started
                })
                .map(|(_, before)| before);
            values.set(Column::FxsCpu, before.and_then(|before| cpu(before, now)));
            values.set(
                Column::FxsReadBps,
                before.and_then(|before| rate(before, now, |usage| usage.read)),
            );
            values.set(
                Column::FxsWriteBps,
                before.and_then(|before| rate(before, now, |usage| usage.written)),
            );
            values.set(Column::FxsWs, Some(real(usage.working_set)));
            values.set(Column::FxsPrivate, Some(real(usage.private)));
            values.set(Column::FxsThreads, usage.threads.map(f64::from));
            values.set(Column::FxsHandles, usage.handles.map(f64::from));
            self.main = Some((run.generation, now));
        } else {
            self.main = None;
        }

        // A run's first tree has nothing to compare with, even when the last wake saw another run.
        let before = self
            .children
            .take()
            .filter(|(generation, _)| *generation == run.generation)
            .map(|(_, children)| children);
        let mut seen = HashMap::new();
        let mut listed = Vec::new();
        let mut shared = 0.0;
        for child in tree.children {
            let key = (child.pid, child.usage.started);
            let now = Measured {
                at,
                usage: child.usage,
            };
            let share = before
                .as_ref()
                .and_then(|before| before.get(&key))
                .and_then(|&before| cpu(before, now));
            shared += share.unwrap_or(0.0);
            listed.push(ChildProcess {
                pid: child.pid,
                name: child.name,
                cpu: share,
                working_set: child.usage.working_set,
                private: child.usage.private,
            });
            seen.insert(key, now);
        }
        values.set(Column::Children, Some(real(listed.len() as u64)));
        values.set(
            Column::ChildrenWs,
            Some(real(listed.iter().map(|child| child.working_set).sum())),
        );
        values.set(
            Column::ChildrenPrivate,
            Some(real(listed.iter().map(|child| child.private).sum())),
        );
        values.set(Column::ChildrenCpu, before.is_some().then_some(shared));
        self.children = Some((run.generation, seen));
        listed
    }

    fn host(&mut self, values: &mut Values, host: Option<HostUsage>, at: Instant) {
        let Some(host) = host else {
            return;
        };
        if let (Some(now), Some(before)) = (host.cpu, self.host_cpu) {
            let total = now.total.saturating_sub(before.total);
            if total > 0 && now.busy >= before.busy {
                values.set(
                    Column::HostCpu,
                    Some(real(now.busy - before.busy) / real(total) * 100.0),
                );
            }
        }
        self.host_cpu = host.cpu.or(self.host_cpu);
        self.processors = host.processors.or(self.processors);
        if let Some(memory) = host.memory {
            values.set(Column::HostMemUsed, Some(real(memory.used)));
            values.set(Column::HostMemTotal, Some(real(memory.total)));
        }
        if let (Some(now), Some((then, before))) = (&host.interfaces, &self.interfaces) {
            let before: BTreeMap<&str, &Interface> = before
                .iter()
                .map(|interface| (interface.key.as_str(), interface))
                .collect();
            // An adapter that came, or whose counters went back (reset, or a new one with its
            // key), says nothing of the slot.
            let deltas: Vec<(u64, u64)> = now
                .iter()
                .filter_map(|interface| {
                    let earlier = before.get(interface.key.as_str())?;
                    Some((
                        interface.received.checked_sub(earlier.received)?,
                        interface.sent.checked_sub(earlier.sent)?,
                    ))
                })
                .collect();
            let seconds = at.saturating_duration_since(*then).as_secs_f64();
            if !deltas.is_empty() && seconds > 0.0 {
                let received: u64 = deltas.iter().map(|delta| delta.0).sum();
                let sent: u64 = deltas.iter().map(|delta| delta.1).sum();
                values.set(Column::HostRxBps, Some(real(received) / seconds));
                values.set(Column::HostTxBps, Some(real(sent) / seconds));
            }
        }
        if let Some(interfaces) = host.interfaces {
            self.interfaces = Some((at, interfaces));
        }
    }
}

/// Percent of one core between two measures of the same process.
fn cpu(before: Measured, now: Measured) -> Option<f64> {
    let seconds = now.at.saturating_duration_since(before.at).as_secs_f64();
    let used = now.usage.cpu.checked_sub(before.usage.cpu)?;
    (seconds > 0.0).then(|| used.as_secs_f64() / seconds * 100.0)
}

/// Bytes per second of a counter between two measures of the same process.
fn rate(before: Measured, now: Measured, counter: impl Fn(&Usage) -> Option<u64>) -> Option<f64> {
    let seconds = now.at.saturating_duration_since(before.at).as_secs_f64();
    let bytes = counter(&now.usage)?.checked_sub(counter(&before.usage)?)?;
    (seconds > 0.0).then(|| real(bytes) / seconds)
}

/// Counts and bytes stay far below 2⁵³, where a float stops being exact.
#[allow(clippy::cast_precision_loss)]
fn real(value: u64) -> f64 {
    value as f64
}

#[cfg(test)]
mod tests;
