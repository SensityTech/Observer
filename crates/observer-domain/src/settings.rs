//! What the Settings page shows of Observer's own configuration: `observer.toml`, read once at
//! start and changed only in that file, and how long the statistics are kept, which nothing changes.

use serde::Serialize;
use ts_rs::TS;

/// `GET /api/settings/observer`, the owner's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ObserverSettings {
    /// The file read at start, as the host names it.
    pub config_file: String,
    /// The panel's address, `[http] listen`.
    pub listen: String,
    pub hosts: Vec<String>,
    /// Addresses and CIDR blocks such as `10.0.0.0/8`.
    pub trusted_proxies: Vec<String>,
    pub statistics: StatisticsKept,
}

/// How long the statistics keep each tier, in seconds. Hour aggregates and runs are kept for good.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StatisticsKept {
    /// The five-second samples.
    pub samples: u32,
    /// The minute aggregates.
    pub minutes: u32,
}
