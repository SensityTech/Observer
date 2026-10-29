//! What processes, the host and a volume use, for the statistics. Every call blocks on the
//! operating system and is meant for a blocking task.

use std::time::Duration;

/// One process's resources. Times and byte counts are totals since the process started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Usage {
    pub user: Duration,
    pub kernel: Duration,
    pub working_set: u64,
    /// The most the working set has been.
    pub working_set_peak: Option<u64>,
    /// Memory only this process uses: Windows' private bytes, Linux's anonymous resident and
    /// swapped pages.
    pub private: u64,
    /// Page faults of every kind: Windows' soft and hard ones together, Linux's minor and major.
    pub faults: Option<u64>,
    /// Faults that read from the disk, which only Linux tells apart.
    pub hard_faults: Option<u64>,
    pub threads: Option<u32>,
    /// Windows handles, or Linux file descriptors.
    pub handles: Option<u32>,
    /// Every byte read and written through the I/O system: files, pipes and sockets.
    pub read: Option<u64>,
    pub written: Option<u64>,
    /// When the process started, in the system's own unit. With the pid it tells a process from a
    /// later one that reused the pid.
    pub started: u64,
}

impl Usage {
    /// User and kernel time together.
    pub fn cpu(&self) -> Duration {
        self.user + self.kernel
    }
}

/// FXServer's process tree: the main process, and the processes it started kept apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeUsage {
    /// `None` once the main process is gone and only children are left.
    pub main: Option<Usage>,
    pub children: Vec<Member>,
    /// The main process's threads; `None` once it is gone or when they cannot be listed.
    pub threads: Option<Vec<ThreadUsage>>,
    pub totals: Option<TreeTotals>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub pid: u32,
    pub name: String,
    pub usage: Usage,
}

/// One thread of the main process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadUsage {
    pub id: u32,
    /// When the thread started, in the system's own unit: with the id it tells a thread from a
    /// later one that reused the id.
    pub started: u64,
    /// As the system gives it, empty when the thread has none. Linux cuts names to 15 bytes.
    pub name: String,
    /// User and kernel time together.
    pub cpu: Duration,
}

/// The whole tree since it started, the processes that already ended included.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TreeTotals {
    /// On Windows every process that was ever in the tree; on Linux the members now in it and the
    /// children each of them waited for.
    pub cpu: Duration,
    /// Windows only: the processes started in the tree, the ones gone included.
    pub processes_started: Option<u64>,
    /// Windows only: the most memory the tree's processes committed together.
    pub private_peak: Option<u64>,
}

/// The machine. Each part is `None` when the system would not say.
#[derive(Debug, Clone, PartialEq)]
pub struct HostUsage {
    pub cpu: Option<CpuTimes>,
    /// The logical processors `cpu` counts: every one of the machine's, whatever this process may
    /// use.
    pub processors: Option<u32>,
    pub memory: Option<Memory>,
    pub interfaces: Option<Vec<Interface>>,
    pub clock: Option<Clock>,
    pub pressure: Option<Pressure>,
}

/// Processor time of every core since boot, in the system's own unit: only the ratio of two
/// deltas means something.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuTimes {
    pub busy: u64,
    pub total: u64,
    /// Linux only: time a hypervisor gave another guest while this one had work, inside `busy` and
    /// `total`.
    pub steal: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Memory {
    pub used: u64,
    pub total: u64,
}

/// A physical network adapter's byte counters since boot. The key stays the same while the
/// adapter exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Interface {
    pub key: String,
    pub received: u64,
    pub sent: u64,
}

/// The processors' clock, averaged over them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Clock {
    pub mhz: f64,
    /// The highest the system states: Windows' base clock, which a boosting processor passes,
    /// Linux's maximum with boost.
    pub max_mhz: f64,
    /// The share of that maximum the processors are allowed now, in percent: under 100 when the
    /// firmware or the system throttles them.
    pub limit: Option<f64>,
}

/// Linux's pressure stall information: microseconds since boot during which some task waited for
/// each resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pressure {
    pub cpu: Option<u64>,
    pub memory: Option<u64>,
    pub io: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Space {
    /// What this user may still write.
    pub available: u64,
    pub total: u64,
}

/// The reads and writes a volume completed since boot, and the time they took added up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskCounters {
    pub transfers: u64,
    pub busy: Duration,
}
