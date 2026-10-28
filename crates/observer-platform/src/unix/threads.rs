use std::collections::HashMap;
use std::fs;
use std::io;
use std::thread;
use std::time::Duration;

use super::ProcessTree;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunState {
    Running,
    Sleeping,
    /// Uninterruptible: waiting on a disk or another device.
    DiskWait,
    Stopped,
    Zombie,
    Other,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ThreadState {
    pub tid: u32,
    pub name: String,
    pub state: RunState,
    /// Percent of one core over the window; `None` for a thread that started during it.
    pub cpu: Option<f64>,
    /// The number of the system call the thread waits in, if it does and it could be read.
    pub syscall: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ThreadStates {
    pub threads: Vec<ThreadState>,
    /// `false` when the system refused to show which system calls the threads wait in.
    pub syscalls_readable: bool,
}

impl ProcessTree {
    /// What each thread of the main process is doing: its state and name, the processor time it
    /// used over `window`, and the system call it waits in. Blocks for `window`. The system calls
    /// need the right to trace the process, which `kernel.yama.ptrace_scope` 2 or 3 takes away.
    pub fn thread_states(&self, window: Duration) -> io::Result<ThreadStates> {
        let pid = self.group();
        let before = tasks(pid)?;
        thread::sleep(window);
        let after = tasks(pid)?;
        let per_second = rustix::param::clock_ticks_per_second().max(1);
        let mut syscalls_readable = true;
        let mut threads = Vec::new();
        for (tid, (name, state, ticks)) in after {
            let cpu = before.get(&tid).map(|(_, _, earlier)| {
                // A few hundred ticks at most over a window of a second.
                let ticks = u32::try_from(ticks.saturating_sub(*earlier)).unwrap_or(u32::MAX);
                let per_second = u32::try_from(per_second).unwrap_or(u32::MAX);
                let used = f64::from(ticks) / f64::from(per_second);
                used / window.as_secs_f64().max(f64::EPSILON) * 100.0
            });
            let syscall = match fs::read_to_string(format!("/proc/{pid}/task/{tid}/syscall")) {
                Ok(text) => parse_syscall(&text),
                Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                    syscalls_readable = false;
                    None
                }
                Err(_) => None,
            };
            threads.push(ThreadState {
                tid,
                name,
                state,
                cpu,
                syscall,
            });
        }
        threads.sort_by_key(|thread| thread.tid);
        Ok(ThreadStates {
            threads,
            syscalls_readable,
        })
    }
}

/// Each thread's name, state and `utime + stime` in clock ticks. A thread that ends meanwhile is
/// left out.
fn tasks(pid: u32) -> io::Result<HashMap<u32, (String, RunState, u64)>> {
    let mut tasks = HashMap::new();
    for entry in fs::read_dir(format!("/proc/{pid}/task"))? {
        let Some(tid) = entry?
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/task/{tid}/stat")) else {
            continue;
        };
        let Some((state, ticks)) = parse_task_stat(&stat) else {
            continue;
        };
        let name = fs::read_to_string(format!("/proc/{pid}/task/{tid}/comm")).unwrap_or_default();
        tasks.insert(tid, (name.trim_end_matches('\n').to_owned(), state, ticks));
    }
    Ok(tasks)
}

/// A task's `stat`, counted from the last `)`: the state letter, then `utime` and `stime`.
fn parse_task_stat(text: &str) -> Option<(RunState, u64)> {
    let fields: Vec<&str> = text.get(text.rfind(')')? + 1..)?.split_whitespace().collect();
    let state = match *fields.first()? {
        "R" => RunState::Running,
        "S" | "I" => RunState::Sleeping,
        "D" => RunState::DiskWait,
        "T" | "t" => RunState::Stopped,
        "Z" | "X" => RunState::Zombie,
        _ => RunState::Other,
    };
    let field = |number: usize| fields.get(number - 3)?.parse::<u64>().ok();
    Some((state, field(14)?.checked_add(field(15)?)?))
}

/// The first field of a task's `syscall`: the number of the call it waits in, `running` while it
/// runs, or `-1` when it is blocked outside a call.
fn parse_syscall(text: &str) -> Option<u32> {
    text.split_whitespace().next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::{Launch, spawn};

    #[test]
    fn a_task_s_state_and_times_are_read_past_its_name() {
        let line =
            "812 (UV loop: svMain) R 1 812 812 0 -1 4194560 100 0 0 0 250 30 0 0 20 0 12 0 98765 1000 77\n";

        assert_eq!(parse_task_stat(line), Some((RunState::Running, 280)));
        assert_eq!(parse_task_stat("812 (x) S 1"), None);
    }

    #[test]
    fn the_system_call_is_the_first_field_when_the_thread_waits_in_one() {
        assert_eq!(
            parse_syscall("230 0x0 0x0 0x7ffc 0x7ffc 0x0 0x0 0x7ffc 0x7fa7\n"),
            Some(230)
        );
        assert_eq!(parse_syscall("running\n"), None);
        assert_eq!(parse_syscall("-1 0x7ffc 0x7fa7\n"), None);
    }

    #[tokio::test]
    async fn a_sleeping_process_waits_in_its_sleep_call_and_uses_no_processor() {
        let mut spawned = spawn(&Launch {
            program: PathBuf::from("/bin/sh"),
            args: vec!["-c".into(), "echo ready; exec sleep 5".into()],
            cwd: std::env::temp_dir(),
        })
        .unwrap();
        assert!(spawned.stdout.next().await.is_some());
        tokio::time::sleep(Duration::from_millis(100)).await;

        let states = spawned.tree.thread_states(Duration::from_millis(200)).unwrap();

        let main = &states.threads[0];
        assert_eq!(main.name, "sleep");
        assert_eq!(main.state, RunState::Sleeping);
        assert!(main.cpu.is_some_and(|cpu| cpu < 10.0), "{main:?}");
        if states.syscalls_readable {
            // clock_nanosleep, or nanosleep on older C libraries.
            assert!(matches!(main.syscall, Some(230 | 35)), "{main:?}");
        }
        spawned.tree.kill().unwrap();
    }
}
