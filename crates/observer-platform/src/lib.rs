//! What Observer needs from the operating system to run FXServer: spawning it with stdin, stdout,
//! stderr and file descriptor 3, owning its whole process tree, and the console and signal plumbing
//! around it. The only crate allowed to use `unsafe` outside test tools.
#![allow(unsafe_code)]

mod stream;
#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use tokio::sync::watch;

pub use stream::{Output, Stdin};
#[cfg(unix)]
pub use unix::ProcessTree;
#[cfg(unix)]
pub use unix::leftovers::{PidFile, Sweep};
#[cfg(windows)]
pub use windows::ProcessTree;
#[cfg(windows)]
pub use windows::console::{ConsoleEvent, ConsoleEvents};

/// What to start: the program, its arguments and the directory it runs in. The environment is
/// inherited.
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
