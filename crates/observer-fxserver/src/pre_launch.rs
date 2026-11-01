//! Work Observer does on a server's files before each spawn of FXServer, while nothing runs from
//! them: the server tools heal and apply their fixes there. The supervisor runs it as a phase of its
//! own, between a start being taken on and the process being spawned, for every cause of a start.
//! What that work is belongs to the application, so the supervisor knows only this trait.
//!
//! The phase never keeps a server down and never runs beside it. FXServer is not spawned while the
//! work runs: a stop, Observer's shutdown and the time limit only tell it to end, and the
//! supervisor waits until it has. Whatever it returns, fails with or panics with, the spawn follows
//! unless a stop came meanwhile.

use std::error::Error;
use std::fmt;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use crate::cfg::Scan;
use crate::launch::ServerLaunch;
use crate::layout::Layout;
use crate::supervisor::Cause;

/// The console's actor for the lines the work hands back.
pub const TOOLS: &str = "tools";

/// How long the work gets before it is told to end, unless the settings say otherwise.
pub const DEFAULT_LIMIT: Duration = Duration::from_secs(60);

/// What the work before a spawn gets: exactly what that spawn uses, so that it acts on the folder,
/// the build and the cfg FXServer is about to run, and never reads the cfg a second time.
#[derive(Debug)]
pub struct PreLaunchRequest {
    /// The launch the spawn starts, as it was when the start was taken on: a launch configured
    /// meanwhile applies from the next start.
    pub launch: ServerLaunch,
    /// Where the spawn's build keeps its system resources and its other files.
    pub layout: Layout,
    /// The scan of the cfg that preparing the launch made.
    pub scan: Scan,
    /// Why the server starts: `Requested`, `Restart`, `Scheduled` or `Recovery`.
    pub cause: Cause,
    /// The generation the start took as the phase began, which its state, the lines handed back
    /// and the spawn carry. A start stopped during the work leaves it unused: the next start
    /// takes the one after.
    pub generation: u32,
    /// Set once the work must end: a stop or Observer's shutdown came, or its time ran out. The
    /// work checks it between steps and returns what it did so far; nothing is spawned before it
    /// has returned.
    pub cancel: Arc<AtomicBool>,
}

/// Why the work failed, as the console and the log show it.
pub type PreLaunchError = Box<dyn Error + Send + Sync>;

/// The lines the work hands back for the console, or why it failed.
pub type PreLaunchResult = Result<Vec<String>, PreLaunchError>;

/// The work's future, sent to a task of its own.
pub type PreLaunchFuture<'a> = Pin<Box<dyn Future<Output = PreLaunchResult> + Send + 'a>>;

/// The work the supervisor runs before each spawn, given through
/// [`Settings::pre_launch`](crate::supervisor::Settings::pre_launch).
pub trait PreLaunch: Send + Sync + fmt::Debug {
    /// Does the work for one spawn. It runs as a task while the server shows as starting and the
    /// supervisor keeps answering requests. The lines it returns go to the console under
    /// [`TOOLS`], once it ended whether or not it was told to; it keeps them few. An error or a
    /// panic is logged and noted, and the spawn goes on: this work never keeps a server down.
    fn run(&self, request: PreLaunchRequest) -> PreLaunchFuture<'_>;
}
