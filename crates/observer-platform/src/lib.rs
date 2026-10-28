//! What Observer needs from the operating system to run FXServer: spawning it with stdin, stdout,
//! stderr and file descriptor 3, owning its whole process tree, and the console and signal plumbing
//! around it; and for installing builds, the free space of a volume and the signature check of
//! FXServer.exe; for the statistics, what the tree, Observer and the host use; for the diagnostics,
//! the processes the tree starts, the sockets they hold and a picture of a hung server. The only
//! crate allowed to use `unsafe` outside test tools.
#![allow(unsafe_code)]

mod processes;
mod sockets;
mod stream;
#[cfg(unix)]
mod unix;
mod usage;
#[cfg(windows)]
mod windows;

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tokio::sync::watch;

pub use processes::{ProcessInfo, TreeChange, TreeChanges};
pub use sockets::{Protocol, Socket, SocketState, sockets};
pub use stream::{Output, Stdin};
#[cfg(unix)]
pub use unix::ProcessTree;
#[cfg(unix)]
pub use unix::leftovers::{PidFile, Sweep};
#[cfg(unix)]
pub use unix::threads::{RunState, ThreadState, ThreadStates};
pub use usage::{CpuTimes, HostUsage, Interface, Member, Memory, Space, TreeUsage, Usage};
#[cfg(windows)]
pub use windows::ProcessTree;
#[cfg(windows)]
pub use windows::authenticode::verify_signature;
#[cfg(windows)]
pub use windows::console::{ConsoleEvent, ConsoleEvents};

/// What to start: the program, its arguments and the directory it runs in. The environment is
/// inherited; on Linux it also gets the variable that marks the tree.
#[derive(Debug, Clone)]
pub struct Launch {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub cwd: PathBuf,
}

/// A started process. Dropping `tree` kills every process in it.
pub struct Spawned {
    pub pid: u32,
    pub stdin: Stdin,
    pub stdout: Output,
    pub stderr: Output,
    /// File descriptor 3, where FXServer writes its structured traces.
    pub traces: Output,
    pub exit: ExitWatch,
    pub tree: ProcessTree,
    /// The processes the tree starts and ends, the main one left out.
    pub changes: TreeChanges,
}

#[derive(Debug, thiserror::Error)]
pub enum SpawnError {
    #[error("cannot create the pipes for the process: {0}")]
    Pipes(#[source] io::Error),
    #[error("cannot start {}: {source}", program.display())]
    Start {
        program: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("cannot confine the process tree: {0}")]
    Tree(#[source] io::Error),
    #[error("cannot start the threads that read the process output: {0}")]
    Threads(#[source] io::Error),
    #[error("the launch contains a character the operating system cannot pass on: {0:?}")]
    Unrepresentable(OsString),
}

/// Starts `launch` in a process tree of its own, with four pipes: stdin, stdout, stderr and
/// file descriptor 3.
pub fn spawn(launch: &Launch) -> Result<Spawned, SpawnError> {
    #[cfg(windows)]
    {
        windows::spawn(launch)
    }
    #[cfg(unix)]
    {
        unix::spawn(launch)
    }
}

/// The bytes this user may still write on the volume that holds `dir`.
pub fn available_space(dir: &Path) -> io::Result<u64> {
    Ok(volume_space(dir)?.available)
}

/// The free and total bytes of the volume that holds `dir`.
pub fn volume_space(dir: &Path) -> io::Result<Space> {
    #[cfg(windows)]
    {
        windows::usage::volume_space(dir)
    }
    #[cfg(unix)]
    {
        unix::usage::volume_space(dir)
    }
}

/// What Observer's own process uses.
pub fn own_usage() -> io::Result<Usage> {
    #[cfg(windows)]
    {
        windows::usage::own_usage()
    }
    #[cfg(unix)]
    {
        unix::usage::own_usage()
    }
}

/// The machine's processors, memory and physical network adapters.
pub fn host_usage() -> HostUsage {
    #[cfg(windows)]
    {
        windows::usage::host_usage()
    }
    #[cfg(unix)]
    {
        unix::usage::host_usage()
    }
}

impl ProcessTree {
    /// Waits until no process is left in the tree, polling. Returns `false` if some still run
    /// after `timeout`.
    pub async fn wait_empty(&self, timeout: Duration) -> io::Result<bool> {
        let deadline = Instant::now() + timeout;
        loop {
            if self.is_empty()? {
                return Ok(true);
            }
            if Instant::now() >= deadline {
                return Ok(false);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

/// How the main process ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    /// The exit code. On Windows it is the full 32-bit value, such as `0xC0000005` for an access
    /// violation.
    Code(u32),
    /// Unix only: the signal that killed the process.
    Signal(i32),
    /// The process is gone but its status could not be read.
    Unknown,
}

/// Resolves when the main process exits, as soon as the operating system reports it and without
/// waiting for its pipes to close.
#[derive(Debug, Clone)]
pub struct ExitWatch(watch::Receiver<Option<Exit>>);

impl ExitWatch {
    fn new() -> (watch::Sender<Option<Exit>>, Self) {
        let (sender, receiver) = watch::channel(None);
        (sender, Self(receiver))
    }

    pub fn get(&self) -> Option<Exit> {
        *self.0.borrow()
    }

    pub async fn wait(&mut self) -> Exit {
        match self.0.wait_for(Option::is_some).await {
            Ok(exit) => exit.unwrap_or(Exit::Unknown),
            Err(_) => Exit::Unknown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_free_space_of_a_volume_is_known() {
        let dir = tempfile::tempdir().unwrap();

        assert!(available_space(dir.path()).unwrap() > 0);
    }
}
