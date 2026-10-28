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
}

/// The most series one request may ask for.
pub const MAX_SERIES: usize = 16;

impl Series {
    pub const ALL: [Self; 46] = [
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
}
