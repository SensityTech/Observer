//! Observer's report of a hung server's threads on Linux, where a dump would take `ptrace`: each
//! thread's state, its processor use over half a second, and the system call it waits in.

use jiff::Timestamp;
use observer_domain::diagnostics::{SyscallGroup, ThreadState};
use serde::{Deserialize, Serialize};

pub const VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HangReport {
    pub version: u32,
    pub taken_at: Timestamp,
    pub pid: u32,
    pub syscalls_readable: bool,
    pub threads: Vec<ThreadState>,
}

/// The `x86_64` system calls a stuck thread usually waits in, with the kind of wait each is.
const SYSCALLS: [(u32, &str, SyscallGroup); 25] = [
    (0, "read", SyscallGroup::Io),
    (1, "write", SyscallGroup::Io),
    (7, "poll", SyscallGroup::Wait),
    (23, "select", SyscallGroup::Wait),
    (24, "sched_yield", SyscallGroup::Sleep),
    (35, "nanosleep", SyscallGroup::Sleep),
    (42, "connect", SyscallGroup::Io),
    (43, "accept", SyscallGroup::Io),
    (44, "sendto", SyscallGroup::Io),
    (45, "recvfrom", SyscallGroup::Io),
    (46, "sendmsg", SyscallGroup::Io),
    (47, "recvmsg", SyscallGroup::Io),
    (61, "wait4", SyscallGroup::Wait),
    (74, "fsync", SyscallGroup::Io),
    (75, "fdatasync", SyscallGroup::Io),
    (202, "futex", SyscallGroup::Lock),
    (230, "clock_nanosleep", SyscallGroup::Sleep),
    (232, "epoll_wait", SyscallGroup::Wait),
    (257, "openat", SyscallGroup::Io),
    (270, "pselect6", SyscallGroup::Wait),
    (271, "ppoll", SyscallGroup::Wait),
    (281, "epoll_pwait", SyscallGroup::Wait),
    (288, "accept4", SyscallGroup::Io),
    (426, "io_uring_enter", SyscallGroup::Io),
    (441, "epoll_pwait2", SyscallGroup::Wait),
];

/// A system call's name and kind of wait, or its number alone when it is not one of the table's.
pub fn syscall(number: u32) -> (String, Option<SyscallGroup>) {
    SYSCALLS
        .iter()
        .find(|(known, _, _)| *known == number)
        .map_or_else(
            || (format!("syscall {number}"), None),
            |(_, name, group)| ((*name).to_owned(), Some(*group)),
        )
}

#[cfg(unix)]
pub(crate) fn report(pid: u32, states: observer_platform::ThreadStates) -> HangReport {
    use observer_domain::diagnostics::ThreadRun;
    use observer_platform::RunState;

    let threads = states
        .threads
        .into_iter()
        .map(|thread| {
            let (syscall, syscall_group) = match thread.syscall {
                Some(number) => {
                    let (name, group) = syscall(number);
                    (Some(name), group)
                }
                None => (None, None),
            };
            ThreadState {
                tid: thread.tid,
                name: thread.name,
                state: match thread.state {
                    RunState::Running => ThreadRun::Running,
                    RunState::Sleeping => ThreadRun::Sleeping,
                    RunState::DiskWait => ThreadRun::DiskWait,
                    RunState::Stopped => ThreadRun::Stopped,
                    RunState::Zombie => ThreadRun::Zombie,
                    RunState::Other => ThreadRun::Other,
                },
                cpu: thread.cpu,
                syscall,
                syscall_group,
            }
        })
        .collect();
    HangReport {
        version: VERSION,
        taken_at: Timestamp::now(),
        pid,
        syscalls_readable: states.syscalls_readable,
        threads,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_system_calls_are_named_and_grouped_and_the_rest_keep_their_number() {
        assert_eq!(syscall(202), ("futex".to_owned(), Some(SyscallGroup::Lock)));
        assert_eq!(
            syscall(230),
            ("clock_nanosleep".to_owned(), Some(SyscallGroup::Sleep))
        );
        assert_eq!(syscall(232).1, Some(SyscallGroup::Wait));
        assert_eq!(syscall(999), ("syscall 999".to_owned(), None));
    }
}
