pub mod leftovers;

use std::fs::{self, File};
use std::io;
use std::mem::ManuallyDrop;
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Child, Command, Stdio};
use std::sync::{OnceLock, mpsc};
use std::thread;

use rustix::io::{Errno, FdFlags};
use rustix::pipe::PipeFlags;
use rustix::process::{Pid, Signal};

use crate::stream::{Output, Stdin};
use crate::{Exit, ExitWatch, Launch, SpawnError, Spawned};

/// The descriptor FXServer writes its structured traces to.
const TRACES_FD: i32 = 3;

pub(crate) fn spawn(launch: &Launch) -> Result<Spawned, SpawnError> {
    let (traces, child_traces) =
        rustix::pipe::pipe_with(PipeFlags::CLOEXEC).map_err(|error| SpawnError::Pipes(error.into()))?;
    // Keep the source above the standard descriptors, which the spawn rewires before `pre_exec`.
    let child_traces = rustix::io::fcntl_dupfd_cloexec(&child_traces, TRACES_FD)
        .map_err(|error| SpawnError::Pipes(error.into()))?;

    let mut command = Command::new(&launch.program);
    command
        .args(&launch.args)
        .current_dir(&launch.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let source = child_traces.as_raw_fd();
    let parent = rustix::process::getpid();
    // SAFETY: the closure runs in the forked child before exec and only makes async-signal-safe
    // system calls through rustix; it allocates nothing (errors convert from raw errno values).
    unsafe {
        command.pre_exec(move || {
            // A session of its own: a new process group to kill as a whole, and no terminal
            // whose Ctrl+C would reach it.
            rustix::process::setsid()?;
            if source == TRACES_FD {
                // dup2 onto itself is a no-op that would leave FD_CLOEXEC set.
                rustix::io::fcntl_setfd(BorrowedFd::borrow_raw(TRACES_FD), FdFlags::empty())?;
            } else {
                let mut target = ManuallyDrop::new(OwnedFd::from_raw_fd(TRACES_FD));
                rustix::io::dup2(BorrowedFd::borrow_raw(source), &mut target)?;
            }
            rustix::process::set_parent_process_death_signal(Some(Signal::KILL))?;
            // Observer may have died between the fork and the line above.
            if rustix::process::getppid() != Some(parent) {
                return Err(Errno::SRCH.into());
            }
            Ok(())
        });
    }

    let started = spawner()
        .and_then(|spawner| spawner.spawn(command))
        .map_err(|source| SpawnError::Start {
            program: launch.program.clone(),
            source,
        });
    // The parent must not keep the child's end, or the pipe never reports end of file.
    drop(child_traces);
    let mut child = started?;

    let pid = child.id();
    let tree = ProcessTree {
        group: pid_of(pid).map_err(SpawnError::Tree)?,
    };
    let (Some(stdin), Some(stdout), Some(stderr)) =
        (child.stdin.take(), child.stdout.take(), child.stderr.take())
    else {
        return Err(SpawnError::Pipes(io::Error::other(
            "the spawned process has no pipes",
        )));
    };

    let stdin = Stdin::write_to(File::from(OwnedFd::from(stdin))).map_err(SpawnError::Threads)?;
    let stdout =
        Output::read("process-stdout", File::from(OwnedFd::from(stdout))).map_err(SpawnError::Threads)?;
    let stderr =
        Output::read("process-stderr", File::from(OwnedFd::from(stderr))).map_err(SpawnError::Threads)?;
    let traces = Output::read("process-fd3", File::from(traces)).map_err(SpawnError::Threads)?;
    let (exited, exit) = ExitWatch::new();
    thread::Builder::new()
        .name("process-exit".to_owned())
        .spawn(move || {
            let _ = exited.send(Some(wait_for_exit(&mut child)));
        })
        .map_err(SpawnError::Threads)?;

    Ok(Spawned {
        pid,
        stdin,
        stdout,
        stderr,
        traces,
        exit,
        tree,
    })
}

/// The process group FXServer leads. Dropping it kills whatever is left in the group.
pub struct ProcessTree {
    group: Pid,
}

impl ProcessTree {
    /// Kills every process in the group at once.
    pub fn kill(&self) -> io::Result<()> {
        signal_group(self.group, Signal::KILL)
    }

    /// Sends SIGINT to the whole group: FXServer runs `quit` on it.
    pub fn interrupt(&self) -> io::Result<()> {
        signal_group(self.group, Signal::INT)
    }

    pub fn is_empty(&self) -> io::Result<bool> {
        match rustix::process::test_kill_process_group(self.group) {
            Ok(()) => Ok(false),
            Err(Errno::SRCH) => Ok(true),
            Err(error) => Err(error.into()),
        }
    }

    /// The ids of the processes in the group right now.
    pub fn members(&self) -> io::Result<Vec<u32>> {
        let group = self.group.as_raw_pid().cast_unsigned();
        let mut members = Vec::new();
        for entry in fs::read_dir("/proc")? {
            let Some(pid) = entry?
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<u32>().ok())
            else {
                continue;
            };
            if leftovers::stat(pid).is_some_and(|stat| stat.group == group) {
                members.push(pid);
            }
        }
        Ok(members)
    }
}

impl Drop for ProcessTree {
    fn drop(&mut self) {
        if !self.is_empty().unwrap_or(true) {
            let _ = self.kill();
        }
    }
}

fn signal_group(group: Pid, signal: Signal) -> io::Result<()> {
    match rustix::process::kill_process_group(group, signal) {
        Ok(()) | Err(Errno::SRCH) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn pid_of(pid: u32) -> io::Result<Pid> {
    i32::try_from(pid)
        .ok()
        .and_then(Pid::from_raw)
        .ok_or_else(|| io::Error::other(format!("{pid} is not a process id")))
}

fn wait_for_exit(child: &mut Child) -> Exit {
    match child.wait() {
        Ok(status) => match (status.code(), status.signal()) {
            (Some(code), _) => Exit::Code(code.cast_unsigned()),
            (None, Some(signal)) => Exit::Signal(signal),
            (None, None) => Exit::Unknown,
        },
        Err(_) => Exit::Unknown,
    }
}

type SpawnRequest = (Command, mpsc::SyncSender<io::Result<Child>>);

/// The thread every child is forked from. `PR_SET_PDEATHSIG` fires when the thread that forked the
/// child exits, not the process, so it must live as long as Observer.
struct Spawner(mpsc::Sender<SpawnRequest>);

impl Spawner {
    fn spawn(&self, command: Command) -> io::Result<Child> {
        let (reply, child) = mpsc::sync_channel(1);
        let gone = || io::Error::other("the spawner thread is gone");
        self.0.send((command, reply)).map_err(|_| gone())?;
        child.recv().map_err(|_| gone())?
    }
}

fn spawner() -> io::Result<&'static Spawner> {
    static SPAWNER: OnceLock<Spawner> = OnceLock::new();
    if let Some(spawner) = SPAWNER.get() {
        return Ok(spawner);
    }
    let (requests, queue) = mpsc::channel::<SpawnRequest>();
    thread::Builder::new()
        .name("process-spawner".to_owned())
        .spawn(move || {
            for (mut command, reply) in queue {
                let _ = reply.send(command.spawn());
            }
        })?;
    // Losing a race to another first spawn only leaves an idle thread that ends with its queue.
    Ok(SPAWNER.get_or_init(|| Spawner(requests)))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use super::*;
    use crate::Launch;

    fn shell(script: &str) -> Launch {
        Launch {
            program: PathBuf::from("/bin/sh"),
            args: vec!["-c".into(), script.into()],
            cwd: std::env::temp_dir(),
        }
    }

    async fn read_all(output: &mut Output) -> Vec<u8> {
        let mut bytes = Vec::new();
        while let Some(chunk) = output.next().await {
            bytes.extend(chunk);
        }
        bytes
    }

    #[tokio::test]
    async fn descriptor_3_reaches_the_child_and_the_streams_stay_apart() {
        let mut spawned = spawn(&shell("echo out; echo err >&2; printf '{\"test\":\"test\"}' >&3")).unwrap();

        assert_eq!(read_all(&mut spawned.stdout).await, b"out\n");
        assert_eq!(read_all(&mut spawned.stderr).await, b"err\n");
        assert_eq!(read_all(&mut spawned.traces).await, br#"{"test":"test"}"#);
        assert_eq!(spawned.exit.wait().await, Exit::Code(0));
    }

    #[tokio::test]
    async fn the_child_leads_a_session_of_its_own() {
        let mut spawned = spawn(&shell("ps -o pid=,pgid=,sid= -p $$")).unwrap();

        let line = String::from_utf8(read_all(&mut spawned.stdout).await).unwrap();
        let ids: Vec<&str> = line.split_whitespace().collect();
        assert_eq!(ids.len(), 3, "unexpected ps output: {line:?}");
        assert!(
            ids.iter().all(|id| *id == ids[0]),
            "pid, pgid and sid differ: {line:?}"
        );
        assert_eq!(ids[0], spawned.pid.to_string());
    }

    #[tokio::test]
    async fn interrupt_reaches_the_whole_group() {
        let mut spawned = spawn(&shell(
            "trap 'echo interrupted; exit 7' INT; echo ready; while :; do sleep 0.05; done",
        ))
        .unwrap();
        assert_eq!(spawned.stdout.next().await.as_deref(), Some(&b"ready\n"[..]));

        spawned.tree.interrupt().unwrap();

        assert_eq!(spawned.exit.wait().await, Exit::Code(7));
        assert!(spawned.tree.wait_empty(Duration::from_secs(5)).await.unwrap());
    }

    #[tokio::test]
    async fn a_killed_child_reports_its_signal() {
        let mut spawned = spawn(&shell("echo ready; exec sleep 30")).unwrap();
        assert_eq!(spawned.stdout.next().await.as_deref(), Some(&b"ready\n"[..]));

        spawned.tree.kill().unwrap();

        assert_eq!(spawned.exit.wait().await, Exit::Signal(9));
    }

    #[tokio::test]
    async fn a_missing_program_is_reported_as_a_start_error() {
        let launch = Launch {
            program: PathBuf::from("/nonexistent/FXServer"),
            args: Vec::new(),
            cwd: std::env::temp_dir(),
        };

        assert!(matches!(spawn(&launch), Err(SpawnError::Start { .. })));
    }
}
