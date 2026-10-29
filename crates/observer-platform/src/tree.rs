//! The processes FXServer's tree starts, as the operating system reports them.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::mpsc;

/// How many changes wait for the reader before new ones are dropped and counted.
const QUEUED_CHANGES: usize = 512;

/// A process of the tree as it was read when it started. `None` image or command line: it ended
/// before it could be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessInfo {
    pub pid: u32,
    pub parent: Option<u32>,
    pub image: Option<PathBuf>,
    /// Linux's argv, or Windows' command line split the way the C runtime splits it.
    pub args: Vec<String>,
    /// Windows' command line as given, or Linux's argv joined with blanks.
    pub command_line: Option<String>,
    /// When the process started, in milliseconds since the Unix epoch.
    pub created_ms: Option<i64>,
    /// The start in the system's own unit: with the pid it tells a process from a later one that
    /// reused the pid.
    pub stamp: Option<u64>,
    /// The entries of its environment (`NAME=value`) that make a program run code its arguments do
    /// not name: a shell's startup file and options, the functions bash imports, the dynamic
    /// loaders' variables. Read on Linux only; `None` there when the environment could not be read.
    pub startup_variables: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeChange {
    Started(ProcessInfo),
    Ended { pid: u32 },
}

/// The tree's processes as they start and end, the main process left out. The operating system
/// does not guarantee every notification and a reader that falls behind loses some: the caller
/// reconciles with [`ProcessTree::all_members`](crate::ProcessTree::all_members) now and then.
pub struct TreeChanges {
    changes: mpsc::Receiver<TreeChange>,
    dropped: Arc<AtomicU64>,
}

impl TreeChanges {
    pub(crate) fn channel() -> (ChangeSender, Self) {
        let (sender, changes) = mpsc::channel(QUEUED_CHANGES);
        let dropped = Arc::new(AtomicU64::new(0));
        (
            ChangeSender {
                sender,
                dropped: Arc::clone(&dropped),
            },
            Self { changes, dropped },
        )
    }

    /// The next change, or `None` once the tree is gone.
    pub async fn next(&mut self) -> Option<TreeChange> {
        self.changes.recv().await
    }

    /// How many changes were dropped because the reader was behind.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

pub(crate) struct ChangeSender {
    sender: mpsc::Sender<TreeChange>,
    dropped: Arc<AtomicU64>,
}

impl ChangeSender {
    /// Hands a change on without waiting. `false` once nobody reads them any more.
    pub(crate) fn send(&self, change: TreeChange) -> bool {
        match self.sender.try_send(change) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                true
            }
            Err(mpsc::error::TrySendError::Closed(_)) => false,
        }
    }
}
