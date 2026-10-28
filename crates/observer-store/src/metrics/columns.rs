//! The measures a sample holds, one column each in `sample_5s` and the aggregates of
//! `sample_1m` and `sample_1h`. This list drives the insert, the rollups and the series queries,
//! so a column name never comes from a request.

/// How a column's values combine over a bucket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A value at a time: buckets keep its average, minimum and maximum.
    Gauge,
    /// Events in the slot: buckets keep the total.
    Count,
}

/// How SQLite stores a column's values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Affinity {
    Integer,
    Real,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Column {
    Players,
    Slots,
    PingAvgMs,
    PingP95Ms,
    PingMaxMs,
    Joins,
    Leaves,
    MainTicks,
    MainTickMs,
    MainOver,
    NetworkTicks,
    NetworkTickMs,
    NetworkOver,
    SyncTicks,
    SyncTickMs,
    SyncOver,
    HitchMain,
    HitchNetwork,
    HitchSync,
    Barks,
    HitchMaxMs,
    Resources,
    JsHeapUsed,
    JsHeapTotal,
    FxsCpu,
    FxsWs,
    FxsPrivate,
    FxsThreads,
    FxsHandles,
    FxsReadBps,
    FxsWriteBps,
    Children,
    ChildrenCpu,
    ChildrenWs,
    ChildrenPrivate,
    HostCpu,
    HostMemUsed,
    HostMemTotal,
    HostDiskFree,
    HostDiskTotal,
    HostRxBps,
    HostTxBps,
    ObsCpu,
    ObsWs,
    ObsPrivate,
    ObsStateDb,
    ObsMetricsDb,
    ObsWsClients,
    CollectMs,
}

impl Column {
    pub const ALL: [Self; 49] = [
        Self::Players,
        Self::Slots,
        Self::PingAvgMs,
        Self::PingP95Ms,
        Self::PingMaxMs,
        Self::Joins,
        Self::Leaves,
        Self::MainTicks,
        Self::MainTickMs,
        Self::MainOver,
        Self::NetworkTicks,
        Self::NetworkTickMs,
        Self::NetworkOver,
        Self::SyncTicks,
        Self::SyncTickMs,
        Self::SyncOver,
        Self::HitchMain,
        Self::HitchNetwork,
        Self::HitchSync,
        Self::Barks,
        Self::HitchMaxMs,
        Self::Resources,
        Self::JsHeapUsed,
        Self::JsHeapTotal,
        Self::FxsCpu,
        Self::FxsWs,
        Self::FxsPrivate,
        Self::FxsThreads,
        Self::FxsHandles,
        Self::FxsReadBps,
        Self::FxsWriteBps,
        Self::Children,
        Self::ChildrenCpu,
        Self::ChildrenWs,
        Self::ChildrenPrivate,
        Self::HostCpu,
        Self::HostMemUsed,
        Self::HostMemTotal,
        Self::HostDiskFree,
        Self::HostDiskTotal,
        Self::HostRxBps,
        Self::HostTxBps,
        Self::ObsCpu,
        Self::ObsWs,
        Self::ObsPrivate,
        Self::ObsStateDb,
        Self::ObsMetricsDb,
        Self::ObsWsClients,
        Self::CollectMs,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Players => "players",
            Self::Slots => "slots",
            Self::PingAvgMs => "ping_avg_ms",
            Self::PingP95Ms => "ping_p95_ms",
            Self::PingMaxMs => "ping_max_ms",
            Self::Joins => "joins",
            Self::Leaves => "leaves",
            Self::MainTicks => "main_ticks",
            Self::MainTickMs => "main_tick_ms",
            Self::MainOver => "main_over",
            Self::NetworkTicks => "network_ticks",
            Self::NetworkTickMs => "network_tick_ms",
            Self::NetworkOver => "network_over",
            Self::SyncTicks => "sync_ticks",
            Self::SyncTickMs => "sync_tick_ms",
            Self::SyncOver => "sync_over",
            Self::HitchMain => "hitch_main",
            Self::HitchNetwork => "hitch_network",
            Self::HitchSync => "hitch_sync",
            Self::Barks => "barks",
            Self::HitchMaxMs => "hitch_max_ms",
            Self::Resources => "resources",
            Self::JsHeapUsed => "js_heap_used",
            Self::JsHeapTotal => "js_heap_total",
            Self::FxsCpu => "fxs_cpu",
            Self::FxsWs => "fxs_ws",
            Self::FxsPrivate => "fxs_private",
            Self::FxsThreads => "fxs_threads",
            Self::FxsHandles => "fxs_handles",
            Self::FxsReadBps => "fxs_read_bps",
            Self::FxsWriteBps => "fxs_write_bps",
            Self::Children => "children",
            Self::ChildrenCpu => "children_cpu",
            Self::ChildrenWs => "children_ws",
            Self::ChildrenPrivate => "children_private",
            Self::HostCpu => "host_cpu",
            Self::HostMemUsed => "host_mem_used",
            Self::HostMemTotal => "host_mem_total",
            Self::HostDiskFree => "host_disk_free",
            Self::HostDiskTotal => "host_disk_total",
            Self::HostRxBps => "host_rx_bps",
            Self::HostTxBps => "host_tx_bps",
            Self::ObsCpu => "obs_cpu",
            Self::ObsWs => "obs_ws",
            Self::ObsPrivate => "obs_private",
            Self::ObsStateDb => "obs_state_db",
            Self::ObsMetricsDb => "obs_metrics_db",
            Self::ObsWsClients => "obs_ws_clients",
            Self::CollectMs => "collect_ms",
        }
    }

    pub fn kind(self) -> Kind {
        match self {
            Self::Joins
            | Self::Leaves
            | Self::MainTicks
            | Self::MainTickMs
            | Self::MainOver
            | Self::NetworkTicks
            | Self::NetworkTickMs
            | Self::NetworkOver
            | Self::SyncTicks
            | Self::SyncTickMs
            | Self::SyncOver
            | Self::HitchMain
            | Self::HitchNetwork
            | Self::HitchSync
            | Self::Barks => Kind::Count,
            _ => Kind::Gauge,
        }
    }

    pub fn affinity(self) -> Affinity {
        match self {
            Self::PingAvgMs
            | Self::FxsCpu
            | Self::FxsReadBps
            | Self::FxsWriteBps
            | Self::ChildrenCpu
            | Self::HostCpu
            | Self::HostRxBps
            | Self::HostTxBps
            | Self::ObsCpu => Affinity::Real,
            _ => Affinity::Integer,
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// One value per column; `None` where the measure was not taken.
#[derive(Debug, Clone, PartialEq)]
pub struct Values([Option<f64>; Column::ALL.len()]);

impl Default for Values {
    fn default() -> Self {
        Self([None; Column::ALL.len()])
    }
}

impl Values {
    pub fn get(&self, column: Column) -> Option<f64> {
        self.0[column.index()]
    }

    pub fn set(&mut self, column: Column, value: Option<f64>) {
        self.0[column.index()] = value.filter(|value| value.is_finite());
    }

    /// The columns with a value, in the list's order.
    pub fn measured(&self) -> impl Iterator<Item = (Column, f64)> + '_ {
        Column::ALL
            .into_iter()
            .filter_map(|column| Some((column, self.get(column)?)))
    }
}
