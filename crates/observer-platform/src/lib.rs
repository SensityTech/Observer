//! What Observer needs from the operating system to run FXServer: spawning it with stdin, stdout,
//! stderr and file descriptor 3, owning its whole process tree, and the console and signal plumbing
//! around it; and for installing builds, the free space of a volume and the signature check of
//! FXServer.exe, the processes a build runs, files opened where they were listed, and folders held
//! while they are swept; for the statistics, what the tree, Observer and the host use. The only
//! crate allowed to use `unsafe` outside test tools.
#![allow(unsafe_code)]

mod stream;
#[cfg(unix)]
mod unix;
mod usage;
#[cfg(windows)]
mod windows;

use std::ffi::OsString;
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tokio::sync::watch;

pub use stream::{Output, Stdin};
#[cfg(unix)]
pub use unix::ProcessTree;
#[cfg(unix)]
pub use unix::leftovers::{PidFile, Sweep};
pub use usage::{
    Clock, CpuTimes, DiskCounters, HostUsage, Interface, Member, Memory, Pressure, Space, ThreadUsage,
    TreeTotals, TreeUsage, Usage,
};
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

/// The processes whose executable lies under `dir`, by pid: what keeps a build from being deleted.
pub fn processes_in(dir: &Path) -> io::Result<Vec<u32>> {
    #[cfg(windows)]
    {
        windows::processes::processes_in(dir)
    }
    #[cfg(unix)]
    {
        unix::processes::processes_in(dir)
    }
}

/// Opens a file to read, with the path the file system took to it, every link and junction on the
/// way resolved: a folder swapped for a link since it was listed shows there. A link in the file's
/// own place is not followed, and a named pipe does not make the open wait.
pub fn open_resolved(path: &Path) -> io::Result<(File, PathBuf)> {
    #[cfg(windows)]
    {
        windows::files::open_resolved(path)
    }
    #[cfg(unix)]
    {
        unix::files::open_resolved(path)
    }
}

/// A folder held open, so that what is removed through [`HeldFolder::path`] is removed from it
/// even once its own path was swapped for a link. A link, a junction or a file in its place is
/// refused as `NotADirectory`.
#[derive(Debug)]
pub struct HeldFolder {
    _folder: File,
    path: PathBuf,
}

impl HeldFolder {
    pub fn open(path: &Path) -> io::Result<Self> {
        #[cfg(windows)]
        let (folder, path) = windows::files::hold_folder(path)?;
        #[cfg(unix)]
        let (folder, path) = unix::files::hold_folder(path)?;
        Ok(Self {
            _folder: folder,
            path,
        })
    }

    /// A path that leads into the folder: its entry in `/proc/self/fd` on Linux; its own path on
    /// Windows, where nobody can rename or remove it while it is held.
    pub fn path(&self) -> &Path {
        &self.path
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

/// The reads and writes completed on the volume that holds `dir`, and the time they took.
pub fn disk_counters(dir: &Path) -> io::Result<DiskCounters> {
    #[cfg(windows)]
    {
        windows::usage::disk_counters(dir)
    }
    #[cfg(unix)]
    {
        unix::usage::disk_counters(dir)
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

/// The machine's processors, memory, physical network adapters, clock and pressure.
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

    #[test]
    fn a_file_opened_through_a_linked_folder_shows_where_it_really_lies() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        std::fs::create_dir(&real).unwrap();
        std::fs::write(real.join("scheduler.lua"), b"--").unwrap();
        let linked = dir.path().join("linked");
        link_folder(&real, &linked);

        let (_, direct) = open_resolved(&real.join("scheduler.lua")).unwrap();
        let (_, through) = open_resolved(&linked.join("scheduler.lua")).unwrap();

        let canonical = std::fs::canonicalize(real.join("scheduler.lua")).unwrap();
        assert_eq!(direct, canonical);
        assert_eq!(through, canonical);
    }

    /// A link to a folder any account may make: a symbolic link on Unix, a junction on Windows.
    fn link_folder(target: &Path, link: &Path) {
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, link).unwrap();
        #[cfg(windows)]
        {
            let made = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(link)
                .arg(target)
                .output()
                .unwrap();
            assert!(made.status.success(), "{made:?}");
        }
    }

    #[test]
    fn a_link_or_a_file_in_the_place_of_a_folder_to_hold_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("photos")).unwrap();
        link_folder(&dir.path().join("photos"), &dir.path().join("trash"));
        std::fs::write(dir.path().join("notes.txt"), b"notes").unwrap();

        for refused in ["trash", "notes.txt"] {
            let error = HeldFolder::open(&dir.path().join(refused)).unwrap_err();

            assert_eq!(error.kind(), io::ErrorKind::NotADirectory, "{refused}: {error}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_held_folder_is_still_reached_once_its_path_leads_elsewhere() {
        let dir = tempfile::tempdir().unwrap();
        let trash = dir.path().join("trash");
        std::fs::create_dir(&trash).unwrap();
        std::fs::create_dir(dir.path().join("photos")).unwrap();
        let held = HeldFolder::open(&trash).unwrap();

        std::fs::rename(&trash, dir.path().join("moved")).unwrap();
        link_folder(&dir.path().join("photos"), &trash);
        std::fs::write(held.path().join("left.txt"), b"left").unwrap();

        assert!(dir.path().join("moved/left.txt").exists());
        assert!(!dir.path().join("photos/left.txt").exists());
    }

    #[cfg(windows)]
    #[test]
    fn a_held_folder_can_be_neither_moved_nor_removed() {
        let dir = tempfile::tempdir().unwrap();
        let trash = dir.path().join("trash");
        std::fs::create_dir(&trash).unwrap();
        let held = HeldFolder::open(&trash).unwrap();

        assert!(std::fs::rename(&trash, dir.path().join("moved")).is_err());
        assert!(std::fs::remove_dir(&trash).is_err());
        std::fs::write(held.path().join("left.txt"), b"left").unwrap();
        std::fs::remove_file(held.path().join("left.txt")).unwrap();
        drop(held);
        std::fs::rename(&trash, dir.path().join("moved")).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_link_in_the_place_of_a_file_is_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("secret"), b"secret").unwrap();
        std::os::unix::fs::symlink(dir.path().join("secret"), dir.path().join("link")).unwrap();

        assert!(open_resolved(&dir.path().join("link")).is_err());
    }
}
