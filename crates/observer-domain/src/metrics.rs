//! Statistics: the five-second slots every sample belongs to, the series the API, the live topic
//! and the panel share, and the ranges the panel charts.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::server::ServerState;

/// A slot starts at a multiple of five seconds of the wall clock; its sample is stored under the
/// slot's start, in Unix seconds.
pub const SLOT_MS: i64 = 5_000;
/// The probes that feed the statistics fire this long into each slot.
pub const MID_SLOT_MS: i64 = SLOT_MS / 2;

/// The start, in Unix seconds, of the slot holding `ms` (Unix milliseconds).
pub fn slot_of(ms: i64) -> i64 {
    ms.div_euclid(SLOT_MS) * SLOT_MS / 1000
}

/// The first middle of a slot strictly after `ms`.
pub fn next_mid_slot(ms: i64) -> i64 {
    let mid = ms.div_euclid(SLOT_MS) * SLOT_MS + MID_SLOT_MS;
    if mid > ms { mid } else { mid + SLOT_MS }
}

/// The first end of a slot, which is the next one's start, strictly after `ms`.
pub fn next_slot_end(ms: i64) -> i64 {
    (ms.div_euclid(SLOT_MS) + 1) * SLOT_MS
}

/// What a series measures, as the API, the live topic and the panel name it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Series {
    Players,
    Slots,
    PingAvg,
    PingP95,
    PingMax,
    Joins,
    Leaves,
    /// Milliseconds per tick of each thread.
    MainTick,
    NetworkTick,
    SyncTick,
    /// The share of ticks over the thread's budget, in percent.
    MainOverBudget,
    NetworkOverBudget,
    SyncOverBudget,
    HitchesMain,
    HitchesNetwork,
    HitchesSync,
    Barks,
    HitchMax,
    /// Started resources, the bridge and `hardcap` included.
    Resources,
    JsHeapUsed,
    JsHeapTotal,
    /// Percent of one core; past 100 when FXServer uses more than one.
    FxsCpu,
    FxsWorkingSet,
    FxsPrivate,
    FxsThreads,
    FxsHandles,
    /// Bytes per second through the I/O system: files, pipes and sockets.
    FxsRead,
    FxsWrite,
    Children,
    ChildrenCpu,
    ChildrenWorkingSet,
    ChildrenPrivate,
    /// Percent of the whole machine.
    HostCpu,
    HostMemoryUsed,
    HostMemoryTotal,
    /// The volume of Observer's data folder.
    HostDiskFree,
    HostDiskTotal,
    HostReceive,
    HostSend,
    ObserverCpu,
    ObserverWorkingSet,
    ObserverPrivate,
    StateDatabase,
    MetricsDatabase,
    DashboardClients,
    /// Milliseconds the sampler took to gather the slot's sample.
    SampleTime,
    /// The kernel part of `FxsCpu`, in percent of one core.
    FxsKernel,
    /// Page faults in the slot, soft and hard together.
    FxsFaults,
    /// Linux only: faults that read from the disk.
    FxsHardFaults,
    FxsWorkingSetPeak,
    /// The main process's threads by group, in percent of one core.
    MainThreadCpu,
    NetworkThreadCpu,
    SyncThreadCpu,
    JsThreadsCpu,
    MonoThreadsCpu,
    OtherThreadsCpu,
    /// The whole tree, the processes that ended included, in percent of one core.
    TreeCpu,
    /// Windows only: processes started in the tree in the slot.
    TreeStarted,
    /// Windows only: the most memory the tree has committed.
    TreePrivatePeak,
    /// MHz, averaged over the processors.
    HostClock,
    /// The highest clock the system states: Windows' base clock, Linux's maximum with boost.
    HostClockMax,
    /// The share of that maximum the processors are allowed, in percent.
    HostClockLimit,
    /// Milliseconds per read or write of the data folder's volume.
    HostDiskLatency,
    /// Reads and writes of that volume in the slot.
    HostDiskTransfers,
    /// Linux only, percent of the whole machine.
    HostSteal,
    /// Linux only: the share of the slot some task waited for each resource, in percent.
    HostPressureCpu,
    HostPressureMemory,
    HostPressureIo,
}

/// The most series one request may ask for.
pub const MAX_SERIES: usize = 16;

impl Series {
    pub const ALL: [Self; 68] = [
        Self::Players,
        Self::Slots,
        Self::PingAvg,
        Self::PingP95,
        Self::PingMax,
        Self::Joins,
        Self::Leaves,
        Self::MainTick,
        Self::NetworkTick,
        Self::SyncTick,
        Self::MainOverBudget,
        Self::NetworkOverBudget,
        Self::SyncOverBudget,
        Self::HitchesMain,
        Self::HitchesNetwork,
        Self::HitchesSync,
        Self::Barks,
        Self::HitchMax,
        Self::Resources,
        Self::JsHeapUsed,
        Self::JsHeapTotal,
        Self::FxsCpu,
        Self::FxsWorkingSet,
        Self::FxsPrivate,
        Self::FxsThreads,
        Self::FxsHandles,
        Self::FxsRead,
        Self::FxsWrite,
        Self::Children,
        Self::ChildrenCpu,
        Self::ChildrenWorkingSet,
        Self::ChildrenPrivate,
        Self::HostCpu,
        Self::HostMemoryUsed,
        Self::HostMemoryTotal,
        Self::HostDiskFree,
        Self::HostDiskTotal,
        Self::HostReceive,
        Self::HostSend,
        Self::ObserverCpu,
        Self::ObserverWorkingSet,
        Self::ObserverPrivate,
        Self::StateDatabase,
        Self::MetricsDatabase,
        Self::DashboardClients,
        Self::SampleTime,
        Self::FxsKernel,
        Self::FxsFaults,
        Self::FxsHardFaults,
        Self::FxsWorkingSetPeak,
        Self::MainThreadCpu,
        Self::NetworkThreadCpu,
        Self::SyncThreadCpu,
        Self::JsThreadsCpu,
        Self::MonoThreadsCpu,
        Self::OtherThreadsCpu,
        Self::TreeCpu,
        Self::TreeStarted,
        Self::TreePrivatePeak,
        Self::HostClock,
        Self::HostClockMax,
        Self::HostClockLimit,
        Self::HostDiskLatency,
        Self::HostDiskTransfers,
        Self::HostSteal,
        Self::HostPressureCpu,
        Self::HostPressureMemory,
        Self::HostPressureIo,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Players => "players",
            Self::Slots => "slots",
            Self::PingAvg => "pingAvg",
            Self::PingP95 => "pingP95",
            Self::PingMax => "pingMax",
            Self::Joins => "joins",
            Self::Leaves => "leaves",
            Self::MainTick => "mainTick",
            Self::NetworkTick => "networkTick",
            Self::SyncTick => "syncTick",
            Self::MainOverBudget => "mainOverBudget",
            Self::NetworkOverBudget => "networkOverBudget",
            Self::SyncOverBudget => "syncOverBudget",
            Self::HitchesMain => "hitchesMain",
            Self::HitchesNetwork => "hitchesNetwork",
            Self::HitchesSync => "hitchesSync",
            Self::Barks => "barks",
            Self::HitchMax => "hitchMax",
            Self::Resources => "resources",
            Self::JsHeapUsed => "jsHeapUsed",
            Self::JsHeapTotal => "jsHeapTotal",
            Self::FxsCpu => "fxsCpu",
            Self::FxsWorkingSet => "fxsWorkingSet",
            Self::FxsPrivate => "fxsPrivate",
            Self::FxsThreads => "fxsThreads",
            Self::FxsHandles => "fxsHandles",
            Self::FxsRead => "fxsRead",
            Self::FxsWrite => "fxsWrite",
            Self::Children => "children",
            Self::ChildrenCpu => "childrenCpu",
            Self::ChildrenWorkingSet => "childrenWorkingSet",
            Self::ChildrenPrivate => "childrenPrivate",
            Self::HostCpu => "hostCpu",
            Self::HostMemoryUsed => "hostMemoryUsed",
            Self::HostMemoryTotal => "hostMemoryTotal",
            Self::HostDiskFree => "hostDiskFree",
            Self::HostDiskTotal => "hostDiskTotal",
            Self::HostReceive => "hostReceive",
            Self::HostSend => "hostSend",
            Self::ObserverCpu => "observerCpu",
            Self::ObserverWorkingSet => "observerWorkingSet",
            Self::ObserverPrivate => "observerPrivate",
            Self::StateDatabase => "stateDatabase",
            Self::MetricsDatabase => "metricsDatabase",
            Self::DashboardClients => "dashboardClients",
            Self::SampleTime => "sampleTime",
            Self::FxsKernel => "fxsKernel",
            Self::FxsFaults => "fxsFaults",
            Self::FxsHardFaults => "fxsHardFaults",
            Self::FxsWorkingSetPeak => "fxsWorkingSetPeak",
            Self::MainThreadCpu => "mainThreadCpu",
            Self::NetworkThreadCpu => "networkThreadCpu",
            Self::SyncThreadCpu => "syncThreadCpu",
            Self::JsThreadsCpu => "jsThreadsCpu",
            Self::MonoThreadsCpu => "monoThreadsCpu",
            Self::OtherThreadsCpu => "otherThreadsCpu",
            Self::TreeCpu => "treeCpu",
            Self::TreeStarted => "treeStarted",
            Self::TreePrivatePeak => "treePrivatePeak",
            Self::HostClock => "hostClock",
            Self::HostClockMax => "hostClockMax",
            Self::HostClockLimit => "hostClockLimit",
            Self::HostDiskLatency => "hostDiskLatency",
            Self::HostDiskTransfers => "hostDiskTransfers",
            Self::HostSteal => "hostSteal",
            Self::HostPressureCpu => "hostPressureCpu",
            Self::HostPressureMemory => "hostPressureMemory",
            Self::HostPressureIo => "hostPressureIo",
        }
    }

    /// Reads `players,slots,…`: one to [`MAX_SERIES`] distinct names, in the order given.
    pub fn parse_list(list: &str) -> Result<Vec<Self>, InvalidSeries> {
        let mut series = Vec::new();
        for name in list.split(',') {
            let known = Self::ALL
                .into_iter()
                .find(|series| series.name() == name)
                .ok_or(InvalidSeries)?;
            if series.contains(&known) {
                return Err(InvalidSeries);
            }
            series.push(known);
        }
        if series.len() > MAX_SERIES {
            return Err(InvalidSeries);
        }
        Ok(series)
    }
}

/// The series list of a request names none, one unknown, one twice, or too many.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidSeries;

/// The ranges the panel charts. Each ends now and has a fixed step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum Range {
    #[serde(rename = "1h")]
    Hour,
    #[serde(rename = "6h")]
    SixHours,
    #[serde(rename = "24h")]
    Day,
    #[serde(rename = "7d")]
    Week,
    #[serde(rename = "30d")]
    Month,
}

impl Range {
    pub const ALL: [Self; 5] = [Self::Hour, Self::SixHours, Self::Day, Self::Week, Self::Month];

    pub fn name(self) -> &'static str {
        match self {
            Self::Hour => "1h",
            Self::SixHours => "6h",
            Self::Day => "24h",
            Self::Week => "7d",
            Self::Month => "30d",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|range| range.name() == name)
    }

    /// Seconds per bucket.
    pub fn step(self) -> i64 {
        match self {
            Self::Hour => 5,
            Self::SixHours => 30,
            Self::Day => 120,
            Self::Week => 900,
            Self::Month => 3600,
        }
    }

    pub fn buckets(self) -> i64 {
        match self {
            Self::Week => 672,
            _ => 720,
        }
    }

    pub fn seconds(self) -> i64 {
        self.step() * self.buckets()
    }

    /// The first bucket's start and the end of the grid for a range read at `now` (Unix seconds):
    /// the grid ends with the bucket that holds `now`, on UTC multiples of the step.
    pub fn grid(self, now: i64) -> (i64, i64) {
        let end = (now.div_euclid(self.step()) + 1) * self.step();
        (end - self.seconds(), end)
    }
}

/// `GET /api/metrics/series`: the range on a regular grid, bucket `i` starting at
/// `from + i × step`, every bucket present.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MetricsSeries {
    pub range: Range,
    /// Seconds per bucket.
    pub step: i64,
    /// Unix seconds.
    pub from: i64,
    pub buckets: i64,
    /// The newest slot the buckets include; live samples continue after it.
    pub last: Option<i64>,
    pub series: Vec<SeriesData>,
    pub markers: Vec<RunMarker>,
    /// The samples Observer took in each bucket.
    pub observed: Vec<u32>,
    /// Of those, the ones taken while the server was online or degraded. A bucket observed but
    /// never online is a stretch the server was down.
    pub online: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SeriesData {
    pub series: Series,
    /// The average of a gauge, the total of a count, the ratio of a derived series; `null` where
    /// nothing was measured.
    pub values: Vec<Option<f64>>,
    /// For gauges over buckets of more than one sample.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub min: Option<Vec<Option<f64>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub max: Option<Vec<Option<f64>>>,
    /// The samples with a value in each bucket.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub samples: Option<Vec<u32>>,
}

/// A moment of the server's runs a chart marks: a start, a requested restart, or a crash with the
/// time the automatic restart brought it back online.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RunMarker {
    /// Unix seconds.
    pub at: i64,
    pub kind: MarkerKind,
    pub back_online_at: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum MarkerKind {
    Start,
    Restart,
    Crash,
}

/// The `metrics.live` topic's snapshot: the newest sample, if any was taken yet.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MetricsSnapshot {
    pub latest: Option<LiveSample>,
}

/// One slot's sample as it is taken.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LiveSample {
    /// The slot's start, in Unix seconds.
    pub ts: i64,
    pub state: ServerState,
    /// The series measured in this slot; an unmeasured one is absent.
    pub values: BTreeMap<Series, f64>,
    pub children: Vec<ChildProcess>,
    /// The machine's logical processors, all of those the host's CPU share counts, for shares of
    /// the whole machine. Unknown while the host has not said.
    pub cores: Option<u32>,
}

/// A process FXServer started, as the last sample saw it.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChildProcess {
    pub pid: u32,
    pub name: String,
    /// Percent of one core; `None` the first time the process is seen.
    pub cpu: Option<f64>,
    pub working_set: u64,
    pub private: u64,
}

/// The groups the main process's threads are counted in, by their names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ThreadGroup {
    Main,
    Network,
    Sync,
    Js,
    Mono,
    Other,
}

impl ThreadGroup {
    pub const ALL: [Self; 6] = [
        Self::Main,
        Self::Network,
        Self::Sync,
        Self::Js,
        Self::Mono,
        Self::Other,
    ];

    /// The group of a thread by its name, as FXServer's libraries set it: the libuv loops are
    /// `UV loop: <tag>`, Node's and Mono's threads keep their runtime's names. Each pattern
    /// survives Linux's cut to 15 bytes (`UV loop: svNetw`). `libuv-worker`, which FXServer's
    /// loops and Node share, the `[Cfx]` threads and unnamed ones are `Other`.
    pub fn of(name: &str) -> Self {
        const JS: [&str; 5] = ["v8", "node", "delayedtask", "traceevent", "inspector"];
        const MONO: [&str; 4] = ["mono", "finalizer", "sgen", "thread pool"];
        if name.contains("svMain") {
            return Self::Main;
        }
        if name.contains("svNetw") {
            return Self::Network;
        }
        if name.contains("svSync") {
            return Self::Sync;
        }
        let lower = name.to_ascii_lowercase();
        if JS.iter().any(|pattern| lower.contains(pattern)) {
            Self::Js
        } else if MONO.iter().any(|pattern| lower.contains(pattern)) {
            Self::Mono
        } else {
            Self::Other
        }
    }
}

/// `GET /api/metrics/threads`: CPU per thread name of FXServer's main process over the hour of
/// slots that ends with the newest one.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ThreadShares {
    /// Unix seconds.
    pub from: i64,
    pub to: i64,
    /// The slots whose threads were measured.
    pub measured: u32,
    /// FXServer's average CPU over those slots, in percent of one core.
    pub process: Option<f64>,
    /// The busiest names first.
    pub threads: Vec<ThreadShare>,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ThreadShare {
    /// Empty for threads without a name.
    pub name: String,
    pub group: ThreadGroup,
    /// The average over the measured slots, in percent of one core; the threads of that name
    /// together.
    pub cpu: f64,
}

/// Growth is fitted from this long after a run started, once its resources have loaded, in
/// seconds.
pub const GROWTH_WARM_UP: i64 = 30 * 60;
/// A leak is only suspected over at least an hour of minutes, thirty of them or more, that a
/// straight line fits this well.
pub const LEAK_SPAN: i64 = 3600;
pub const LEAK_POINTS: u32 = 30;
pub const LEAK_FIT: f64 = 0.6;
/// Private bytes: at least 32 MB an hour, and at least 0.5% of the fitted start an hour.
pub const PRIVATE_LEAK: LeakThreshold = LeakThreshold {
    floor: 33_554_432.0,
    share: 0.005,
};
/// Handles: at least 100 an hour, and at least 2% of the fitted start an hour.
pub const HANDLES_LEAK: LeakThreshold = LeakThreshold {
    floor: 100.0,
    share: 0.02,
};

/// The growth per hour past which a steady line reads as a leak: the larger of a floor and a share
/// of where the line started.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LeakThreshold {
    pub floor: f64,
    pub share: f64,
}

/// A straight line through weighted points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fit {
    pub slope: f64,
    pub intercept: f64,
    /// How much of the points' spread the line explains, from 0 to 1.
    pub r2: f64,
}

/// Weighted least squares over `(x, y, weight)` points, on sums centred on the weighted means.
/// `None` under two points or when `x` does not vary; `r2` is 0 when `y` does not.
pub fn fit(points: &[(f64, f64, f64)]) -> Option<Fit> {
    if points.len() < 2 {
        return None;
    }
    let weight: f64 = points.iter().map(|&(_, _, weight)| weight).sum();
    if weight <= 0.0 {
        return None;
    }
    let mean_x = points.iter().map(|&(x, _, weight)| x * weight).sum::<f64>() / weight;
    let mean_y = points.iter().map(|&(_, y, weight)| y * weight).sum::<f64>() / weight;
    let (mut xx, mut xy, mut yy) = (0.0, 0.0, 0.0);
    for &(x, y, weight) in points {
        let (dx, dy) = (x - mean_x, y - mean_y);
        xx += weight * dx * dx;
        xy += weight * dx * dy;
        yy += weight * dy * dy;
    }
    if xx <= 0.0 {
        return None;
    }
    let slope = xy / xx;
    Some(Fit {
        slope,
        intercept: mean_y - slope * mean_x,
        r2: if yy > 0.0 { xy * xy / (xx * yy) } else { 0.0 },
    })
}

/// `GET /api/metrics/growth`: how the main process's memory and handles grew over the open run,
/// from the minute averages after the warm-up. `null` without an open run.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RunGrowth {
    /// Unix seconds.
    pub started_at: i64,
    /// The first minute fitted, once the warm-up is over.
    pub from: i64,
    /// The end of the newest minute fitted.
    pub to: i64,
    pub private: Option<Growth>,
    pub handles: Option<Growth>,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Growth {
    /// Bytes, or handles, per hour.
    pub per_hour: f64,
    /// The fit's r², from 0 to 1.
    pub fit: f64,
    /// The fitted value at `from`.
    pub start: f64,
    /// The minutes fitted.
    pub points: u32,
    /// Steady growth past the threshold: a possible leak.
    pub leak: bool,
    /// How far the points stray from the line: the standard deviation of what it leaves out.
    /// `None` under three points.
    pub residual: Option<f64>,
    /// The standard error of `per_hour`, widened for minutes that follow each other closely.
    pub slope_error: Option<f64>,
    /// The mean of the fitted points' times, in seconds since `from`: where the line is surest.
    pub center: f64,
}

impl Growth {
    /// The line through `(seconds since from, value, samples)` points spanning `span` seconds, and
    /// the verdict against `threshold`. Points come in time order.
    pub fn of(points: &[(f64, f64, f64)], span: i64, threshold: LeakThreshold) -> Option<Self> {
        let line = fit(points)?;
        let per_hour = line.slope * 3600.0;
        let count = u32::try_from(points.len()).unwrap_or(u32::MAX);
        let leak = count >= LEAK_POINTS
            && span >= LEAK_SPAN
            && line.r2 >= LEAK_FIT
            && per_hour >= threshold.floor.max(threshold.share * line.intercept);
        let spread = Spread::of(points, line);
        Some(Self {
            per_hour,
            fit: line.r2,
            start: line.intercept,
            points: count,
            leak,
            residual: spread.map(|spread| spread.residual),
            slope_error: spread.map(|spread| spread.slope_error * 3600.0),
            center: spread.map_or(0.0, |spread| spread.center),
        })
    }

    /// Where the line puts the value `seconds` after `from`, and the 95 % band a new point would
    /// fall in: the points' own spread plus the slope's uncertainty, which grows away from the
    /// centre. `None` without the spread.
    pub fn forecast(&self, seconds: f64) -> Option<(f64, f64, f64)> {
        let (residual, slope_error) = (self.residual?, self.slope_error? / 3600.0);
        let mid = self.start + self.per_hour / 3600.0 * seconds;
        let away = seconds - self.center;
        let half = Z_95 * (residual * residual + away * away * slope_error * slope_error).sqrt();
        Some((mid - half, mid, mid + half))
    }
}

/// The normal quantile of a 95 % band.
const Z_95: f64 = 1.96;
/// Above this, neighbouring minutes are taken as moving together almost entirely.
const MOST_CORRELATION: f64 = 0.95;

/// How the points spread around their line.
#[derive(Debug, Clone, Copy)]
struct Spread {
    residual: f64,
    /// Per second.
    slope_error: f64,
    center: f64,
}

impl Spread {
    fn of(points: &[(f64, f64, f64)], line: Fit) -> Option<Self> {
        if points.len() < 3 {
            return None;
        }
        #[allow(clippy::cast_precision_loss)]
        let count = points.len() as f64;
        let residuals: Vec<f64> = points
            .iter()
            .map(|&(x, y, _)| y - (line.intercept + line.slope * x))
            .collect();
        let squares: f64 = residuals.iter().map(|residual| residual * residual).sum();
        let center = points.iter().map(|&(x, _, _)| x).sum::<f64>() / count;
        let spread_x: f64 = points.iter().map(|&(x, _, _)| (x - center).powi(2)).sum();
        if spread_x <= 0.0 {
            return None;
        }
        let variance = squares / (count - 2.0);
        // Each minute sits close to the one before: as many minutes say less than as many
        // independent points would, by the lag-one correlation of what the line leaves out.
        let correlation = if squares > 0.0 {
            let lagged: f64 = residuals.windows(2).map(|pair| pair[0] * pair[1]).sum();
            (lagged / squares).clamp(0.0, MOST_CORRELATION)
        } else {
            0.0
        };
        let widened = (1.0 + correlation) / (1.0 - correlation);
        Some(Self {
            residual: variance.sqrt(),
            slope_error: (variance / spread_x * widened).sqrt(),
            center,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instants_fall_into_the_slot_that_starts_on_the_last_multiple_of_five_seconds() {
        assert_eq!(slot_of(1_761_642_000_000), 1_761_642_000);
        assert_eq!(slot_of(1_761_642_004_999), 1_761_642_000);
        assert_eq!(slot_of(1_761_642_005_000), 1_761_642_005);
        assert_eq!(slot_of(-1), -5);
    }

    #[test]
    fn the_next_mid_slot_and_slot_end_are_strictly_later() {
        let start = 1_761_642_000_000;

        assert_eq!(next_mid_slot(start), start + 2_500);
        assert_eq!(next_mid_slot(start + 2_499), start + 2_500);
        assert_eq!(next_mid_slot(start + 2_500), start + 7_500);
        assert_eq!(next_slot_end(start), start + 5_000);
        assert_eq!(next_slot_end(start + 4_999), start + 5_000);
        assert_eq!(next_slot_end(start + 5_000), start + 10_000);
    }

    #[test]
    fn every_series_has_the_name_it_is_serialised_with() {
        for series in Series::ALL {
            assert_eq!(serde_json::to_value(series).unwrap(), series.name());
        }
        let mut names: Vec<&str> = Series::ALL.iter().map(|series| series.name()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), Series::ALL.len());
    }

    #[test]
    fn a_series_list_takes_one_to_sixteen_distinct_known_names() {
        assert_eq!(
            Series::parse_list("players,slots,fxsCpu"),
            Ok(vec![Series::Players, Series::Slots, Series::FxsCpu])
        );
        let sixteen: Vec<&str> = Series::ALL[..16].iter().map(|series| series.name()).collect();
        assert_eq!(Series::parse_list(&sixteen.join(",")).unwrap().len(), 16);
        let seventeen: Vec<&str> = Series::ALL[..17].iter().map(|series| series.name()).collect();

        for invalid in [
            "",
            "players,",
            "players,players",
            "Players",
            "fxs_cpu",
            seventeen.join(",").as_str(),
        ] {
            assert_eq!(Series::parse_list(invalid), Err(InvalidSeries), "{invalid}");
        }
    }

    #[test]
    fn each_range_is_a_fixed_grid_ending_with_the_bucket_that_holds_now() {
        let now = 1_761_642_123;

        assert_eq!(Range::Hour.grid(now), (1_761_642_125 - 3600, 1_761_642_125));
        assert_eq!(Range::Day.grid(now), (1_761_642_240 - 86_400, 1_761_642_240));
        assert_eq!(Range::Week.grid(now), (1_761_642_900 - 604_800, 1_761_642_900));
        assert_eq!(Range::Month.seconds(), 30 * 86_400);
        for range in Range::ALL {
            assert_eq!(Range::parse(range.name()), Some(range));
            assert_eq!(serde_json::to_value(range).unwrap(), range.name());
            let (from, end) = range.grid(now);
            assert_eq!(from.rem_euclid(range.step()), 0);
            assert!(from <= now && now < end);
        }
        assert_eq!(Range::parse("2h"), None);
    }

    #[test]
    fn threads_fall_into_groups_by_the_names_of_the_lab_build_and_their_linux_cuts() {
        use ThreadGroup::{Js, Main, Mono, Network, Other, Sync};
        let names = [
            ("UV loop: svMain", Main),
            ("UV loop: svNetwork", Network),
            ("UV loop: svNetw", Network),
            ("UV loop: svSync", Sync),
            ("UV loop: default", Other),
            ("UV loop: defaul", Other),
            ("UV loop: tcp0", Other),
            ("UV loop: httpClient", Other),
            ("UV loop: httpCl", Other),
            ("UV loop: watchdog", Other),
            ("UV loop: watchd", Other),
            ("[Cfx] Server Thread", Other),
            ("[Cfx] Server Th", Other),
            ("[Cfx] Network Thread", Other),
            ("[Cfx] Network T", Other),
            ("[Cfx] StConThread", Other),
            ("[Cfx] StConThre", Other),
            ("[Cfx] Console Thread", Other),
            ("[Cfx] Console T", Other),
            ("[Mumble] Worker thread", Other),
            ("[Mumble] Worker", Other),
            ("DevCon Thread", Other),
            ("libuv-worker", Other),
            ("DelayedTaskSchedulerWorker", Js),
            ("DelayedTaskSche", Js),
            ("TraceEventWorker", Js),
            ("TraceEventWorke", Js),
            ("V8 DefaultWorker", Js),
            ("node", Js),
            ("Finalizer", Mono),
            ("Thread Pool Worker", Mono),
            ("Thread Pool Wor", Mono),
            ("mono-2.0-sgen", Mono),
            ("", Other),
        ];

        for (name, group) in names {
            assert_eq!(ThreadGroup::of(name), group, "{name}");
        }
        for group in ThreadGroup::ALL {
            let name = serde_json::to_value(group).unwrap();
            assert!(name.as_str().unwrap().chars().next().unwrap().is_lowercase());
        }
    }

    fn line(minutes: u32, start: f64, per_minute: f64) -> Vec<(f64, f64, f64)> {
        (0..minutes)
            .map(|minute| {
                let x = 60.0 * f64::from(minute) + 30.0;
                (x, start + per_minute * x / 60.0, 12.0)
            })
            .collect()
    }

    #[test]
    fn an_exact_line_is_fitted_exactly() {
        let fit = fit(&[(0.0, 10.0, 1.0), (60.0, 16.0, 1.0), (120.0, 22.0, 5.0)]).unwrap();

        assert!((fit.slope - 0.1).abs() < 1e-12);
        assert!((fit.intercept - 10.0).abs() < 1e-9);
        assert!((fit.r2 - 1.0).abs() < 1e-12);
    }

    #[test]
    fn weights_pull_the_line_towards_the_heavier_points() {
        let even = fit(&[(0.0, 0.0, 1.0), (1.0, 1.0, 1.0), (2.0, 0.0, 1.0), (3.0, 3.0, 1.0)]).unwrap();
        let weighted = fit(&[
            (0.0, 0.0, 1.0),
            (1.0, 1.0, 1.0),
            (2.0, 0.0, 1.0),
            (3.0, 3.0, 10.0),
        ])
        .unwrap();

        assert!(weighted.slope > even.slope, "{even:?} {weighted:?}");
        assert_eq!(fit(&[(0.0, 1.0, 1.0)]), None, "one point");
        assert_eq!(fit(&[(5.0, 1.0, 1.0), (5.0, 2.0, 1.0)]), None, "no spread in x");
        assert_eq!(fit(&[(0.0, 1.0, 0.0), (1.0, 2.0, 0.0)]), None, "no weight");
    }

    #[test]
    fn a_flat_series_has_no_slope_and_no_fit() {
        let flat = fit(&line(60, 500.0, 0.0)).unwrap();

        assert_eq!((flat.slope, flat.r2), (0.0, 0.0));
        assert!((flat.intercept - 500.0).abs() < 1e-9);
    }

    #[test]
    fn noise_around_a_flat_line_fits_poorly_and_is_never_a_leak() {
        // A sawtooth of 64 MB around 1 GB.
        let points: Vec<(f64, f64, f64)> = (0..90)
            .map(|minute| {
                let y = 1e9 + if minute % 2 == 0 { 64e6 } else { -64e6 };
                (60.0 * f64::from(minute), y, 12.0)
            })
            .collect();

        let growth = Growth::of(&points, 90 * 60, PRIVATE_LEAK).unwrap();

        assert!(growth.fit < 0.01, "{growth:?}");
        assert!(!growth.leak);
    }

    #[test]
    fn steady_growth_is_a_leak_only_past_every_threshold() {
        // 1 GB growing 40 MB an hour: past the 32 MB floor, and past 0.5% of 1 GB.
        let per_minute = 40.0 * 1_048_576.0 / 60.0;
        let leak = |minutes: u32, span: i64| {
            Growth::of(&line(minutes, 1e9, per_minute), span, PRIVATE_LEAK).unwrap()
        };

        let flagged = leak(60, 3600);
        assert!(flagged.leak, "{flagged:?}");
        assert!((flagged.per_hour - 40.0 * 1_048_576.0).abs() < 1.0);
        assert!((flagged.start - 1e9).abs() < 1.0);
        assert_eq!(flagged.points, 60);
        assert!(!leak(29, 3600).leak, "29 points");
        assert!(!leak(60, 3540).leak, "59 minutes");
    }

    #[test]
    fn a_line_that_fits_under_the_bar_is_no_leak() {
        // A rising line with alternating noise of 14, which keeps its r² at 0.59.
        let points: Vec<(f64, f64, f64)> = (0..60)
            .map(|minute| {
                let x = f64::from(minute);
                let noise = if minute % 2 == 0 { 14.0 } else { -14.0 };
                (x * 60.0, (x + noise) * 1e6, 1.0)
            })
            .collect();
        let growth = Growth::of(&points, 3600, PRIVATE_LEAK).unwrap();
        assert!(growth.fit < LEAK_FIT && growth.fit > 0.58, "{growth:?}");
        assert!(growth.per_hour > 32.0 * 1_048_576.0);
        assert!(!growth.leak);
    }

    #[test]
    fn growth_just_under_either_floor_is_no_leak() {
        let private_floor = PRIVATE_LEAK.floor / 60.0;
        let under_bytes = Growth::of(&line(60, 1e8, private_floor - 0.1 / 60.0), 3600, PRIVATE_LEAK).unwrap();
        let at_bytes = Growth::of(&line(60, 1e8, private_floor + 0.1 / 60.0), 3600, PRIVATE_LEAK).unwrap();
        assert!(!under_bytes.leak && at_bytes.leak, "{under_bytes:?} {at_bytes:?}");

        // 9.4 GB: 0.5% an hour is 47 MB, above the floor.
        let big = Growth::of(&line(60, 9.4e9, 40.0 * 1_048_576.0 / 60.0), 3600, PRIVATE_LEAK).unwrap();
        assert!(!big.leak, "{big:?}");

        let handles_floor = HANDLES_LEAK.floor / 60.0;
        let under = Growth::of(&line(60, 475.0, handles_floor - 0.1 / 60.0), 3600, HANDLES_LEAK).unwrap();
        let over = Growth::of(&line(60, 475.0, handles_floor + 0.1 / 60.0), 3600, HANDLES_LEAK).unwrap();
        assert!(!under.leak && over.leak, "{under:?} {over:?}");
        // 10,000 handles: 2% an hour is 200.
        let many = Growth::of(&line(60, 10_000.0, 150.0 / 60.0), 3600, HANDLES_LEAK).unwrap();
        assert!(!many.leak);
    }

    #[test]
    fn an_exact_line_forecasts_without_any_band() {
        let growth = Growth::of(&line(60, 1e9, 1e6), 3600, PRIVATE_LEAK).unwrap();

        let (low, mid, high) = growth.forecast(7200.0).unwrap();

        assert!((mid - (1e9 + 120.0 * 1e6)).abs() < 1.0, "{mid}");
        assert!((high - low).abs() < 1.0, "{low} {high}");
        assert!(
            (growth.center - 1800.0).abs() < 1e-6,
            "the minutes' middles average half an hour"
        );
    }

    #[test]
    fn the_band_widens_away_from_the_fitted_minutes() {
        // A rising line with a seesaw of 1 MB: residuals alternate, so no widening for correlation.
        let points: Vec<(f64, f64, f64)> = (0..60)
            .map(|minute| {
                let seesaw = if minute % 2 == 0 { 1e6 } else { -1e6 };
                (
                    60.0 * f64::from(minute),
                    1e9 + 1e6 * f64::from(minute) + seesaw,
                    12.0,
                )
            })
            .collect();
        let growth = Growth::of(&points, 3600, PRIVATE_LEAK).unwrap();
        let width = |seconds: f64| {
            let (low, _, high) = growth.forecast(seconds).unwrap();
            high - low
        };

        assert!((growth.residual.unwrap() - 1e6).abs() < 2e4, "{growth:?}");
        assert!(width(growth.center) < width(3600.0 * 3.0));
        assert!(width(3600.0 * 3.0) < width(3600.0 * 24.0));
        assert!(width(growth.center) >= 2.0 * 1.96 * growth.residual.unwrap() - 1.0);
    }

    #[test]
    fn minutes_that_drift_together_widen_the_slope_error() {
        // Slow waves around the line: neighbouring residuals share their sign.
        let wave = |minute: u32| 4e6 * (f64::from(minute) / 6.0).sin();
        let drifting: Vec<(f64, f64, f64)> = (0..120)
            .map(|minute| {
                (
                    60.0 * f64::from(minute),
                    1e9 + 1e6 * f64::from(minute) + wave(minute),
                    12.0,
                )
            })
            .collect();
        // The same residuals, shuffled into alternation by reversing every other one's sign.
        let alternating: Vec<(f64, f64, f64)> = (0..120)
            .map(|minute| {
                let sign = if minute % 2 == 0 { 1.0 } else { -1.0 };
                (
                    60.0 * f64::from(minute),
                    1e9 + 1e6 * f64::from(minute) + sign * wave(minute).abs(),
                    12.0,
                )
            })
            .collect();

        let drifting = Growth::of(&drifting, 7200, PRIVATE_LEAK).unwrap();
        let alternating = Growth::of(&alternating, 7200, PRIVATE_LEAK).unwrap();

        assert!(
            drifting.slope_error.unwrap() > 3.0 * alternating.slope_error.unwrap(),
            "{drifting:?} {alternating:?}"
        );
        assert_eq!(
            Growth::of(&line(2, 1e9, 1e6), 60, PRIVATE_LEAK).unwrap().residual,
            None
        );
    }
}
