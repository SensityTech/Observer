//! What processes, the host and a volume use, for the statistics. Every call blocks on the
//! operating system and is meant for a blocking task.

use std::time::Duration;

/// One process's resources. Times and byte counts are totals since the process started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Usage {
    /// User and kernel time together.
    pub cpu: Duration,
    pub working_set: u64,
    /// Memory only this process uses: Windows' private bytes, Linux's anonymous resident and
    /// swapped pages.
    pub private: u64,
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

/// FXServer's process tree: the main process, and the processes it started kept apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeUsage {
    /// `None` once the main process is gone and only children are left.
    pub main: Option<Usage>,
    pub children: Vec<Member>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub pid: u32,
    pub name: String,
    pub usage: Usage,
}

/// The machine. Each part is `None` when the system would not say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostUsage {
    pub cpu: Option<CpuTimes>,
    pub memory: Option<Memory>,
    pub interfaces: Option<Vec<Interface>>,
}

/// Processor time of every core since boot, in the system's own unit: only the ratio of two
/// deltas means something.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuTimes {
    pub busy: u64,
    pub total: u64,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Space {
    /// What this user may still write.
    pub available: u64,
    pub total: u64,
}
