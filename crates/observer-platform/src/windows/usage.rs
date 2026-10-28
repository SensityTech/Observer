use std::collections::HashMap;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
use std::path::Path;
use std::ptr;
use std::time::Duration;

use windows_sys::Win32::Foundation::{FALSE, FILETIME, INVALID_HANDLE_VALUE, NO_ERROR, WAIT_OBJECT_0};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    FreeMibTable, GetIfTable2, MIB_IF_ROW2, MIB_IF_TABLE2,
};
use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::JobObjects::IsProcessInJob;
use windows_sys::Win32::System::ProcessStatus::{
    GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
};
use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows_sys::Win32::System::Threading::{
    ALL_PROCESSOR_GROUPS, GetActiveProcessorCount, GetCurrentProcess, GetProcessHandleCount,
    GetProcessIoCounters, GetProcessTimes, GetSystemTimes, IO_COUNTERS, OpenProcess,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ, WaitForSingleObject,
};

use super::{ProcessTree, check, size_u32};
use crate::usage::{CpuTimes, HostUsage, Interface, Member, Memory, Space, TreeUsage, Usage};

/// `MIB_IF_ROW2.InterfaceAndOperStatusFlags`: a network card or a USB adapter, rather than a
/// loopback, a tunnel or a virtual switch's port.
const HARDWARE_INTERFACE: u8 = 0x01;
/// A filter driver's view of an adapter that is also listed on its own.
const FILTER_INTERFACE: u8 = 0x02;

impl ProcessTree {
    /// What the main process and each of its children use now. A child that exits meanwhile is
    /// left out.
    pub fn usage(&self) -> io::Result<TreeUsage> {
        let members = self.members()?;
        let listed = snapshot()?;
        // SAFETY: the handle stays open for as long as `self`; a zero wait only reads its state.
        let exited = unsafe { WaitForSingleObject(self.main.as_raw_handle(), 0) } == WAIT_OBJECT_0;
        let main = if exited {
            None
        } else {
            let threads = listed.get(&self.pid).map(|entry| entry.threads);
            Some(measure(self.main.as_raw_handle(), threads)?)
        };
        let mut children = Vec::new();
        for pid in members.into_iter().filter(|&pid| pid != self.pid) {
            let Some(process) = open_member(pid, self.job.raw()) else {
                continue;
            };
            let entry = listed.get(&pid);
            let Ok(usage) = measure(process.as_raw_handle(), entry.map(|entry| entry.threads)) else {
                continue;
            };
            children.push(Member {
                pid,
                name: entry.map(|entry| entry.name.clone()).unwrap_or_default(),
                usage,
            });
        }
        Ok(TreeUsage { main, children })
    }
}

pub(crate) fn own_usage() -> io::Result<Usage> {
    // SAFETY: the pseudo handle of the current process needs no closing and is always valid.
    measure(unsafe { GetCurrentProcess() }, None)
}

pub(crate) fn host_usage() -> HostUsage {
    HostUsage {
        cpu: cpu_times(),
        processors: processors(),
        memory: memory(),
        interfaces: interfaces(),
    }
}

pub(crate) fn volume_space(dir: &Path) -> io::Result<Space> {
    let path: Vec<u16> = dir.as_os_str().encode_wide().chain([0]).collect();
    let (mut available, mut total) = (0, 0);
    // SAFETY: `path` is NUL-terminated and outlives the call, both outputs are writable u64s, and
    // the total free bytes we do not want may be null.
    check(unsafe {
        GetDiskFreeSpaceExW(path.as_ptr(), &raw mut available, &raw mut total, ptr::null_mut())
    })?;
    Ok(Space { available, total })
}

/// Opens a member of the job to read its counters, unless it exited since the job listed it or its
/// pid now belongs to a process outside the job.
fn open_member(pid: u32, job: RawHandle) -> Option<OwnedHandle> {
    // SAFETY: OpenProcess takes any pid and returns null when it cannot open it.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ, FALSE, pid) };
    if handle.is_null() {
        return None;
    }
    // SAFETY: OpenProcess returned an open handle nothing else owns.
    let process = unsafe { OwnedHandle::from_raw_handle(handle) };
    let mut inside = FALSE;
    // SAFETY: both handles are open for the call and `inside` is a writable BOOL.
    let ok = unsafe { IsProcessInJob(process.as_raw_handle(), job, &raw mut inside) };
    (ok != 0 && inside != 0).then_some(process)
}

fn measure(process: RawHandle, threads: Option<u32>) -> io::Result<Usage> {
    let (mut created, mut exited, mut kernel, mut user) = Default::default();
    // SAFETY: the handle is open with query rights for the call and the four outputs are writable.
    check(unsafe {
        GetProcessTimes(
            process,
            &raw mut created,
            &raw mut exited,
            &raw mut kernel,
            &raw mut user,
        )
    })?;
    let mut memory = PROCESS_MEMORY_COUNTERS_EX {
        cb: size_u32::<PROCESS_MEMORY_COUNTERS_EX>(),
        ..Default::default()
    };
    // SAFETY: the extended structure starts with the basic one and `cb` gives its full size, which
    // is how the call is told it may fill the private bytes too.
    check(unsafe {
        GetProcessMemoryInfo(
            process,
            (&raw mut memory).cast::<PROCESS_MEMORY_COUNTERS>(),
            memory.cb,
        )
    })?;
    let mut handles = 0;
    // SAFETY: same open handle, and `handles` is a writable u32.
    let handles = (unsafe { GetProcessHandleCount(process, &raw mut handles) } != 0).then_some(handles);
    let mut io = IO_COUNTERS::default();
    // SAFETY: same open handle, and the structure is the one the call fills.
    let io = (unsafe { GetProcessIoCounters(process, &raw mut io) } != 0).then_some(io);
    Ok(Usage {
        cpu: hundreds_of_ns(filetime(kernel) + filetime(user)),
        working_set: memory.WorkingSetSize as u64,
        private: memory.PrivateUsage as u64,
        threads,
        handles,
        read: io.map(|io| io.ReadTransferCount),
        written: io.map(|io| io.WriteTransferCount),
        started: filetime(created),
    })
}

pub(super) struct Listed {
    threads: u32,
    name: String,
}

/// Every process's thread count and executable name, from one snapshot of the system.
pub(super) fn snapshot() -> io::Result<HashMap<u32, Listed>> {
    // SAFETY: a process snapshot takes no pid; the handle is checked before use.
    let handle = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: CreateToolhelp32Snapshot returned an open handle nothing else owns.
    let snapshot = unsafe { OwnedHandle::from_raw_handle(handle) };
    let mut listed = HashMap::new();
    let mut entry = PROCESSENTRY32W {
        dwSize: size_u32::<PROCESSENTRY32W>(),
        ..Default::default()
    };
    // SAFETY: the snapshot is open and `entry` is a writable structure whose `dwSize` is set.
    let mut more = unsafe { Process32FirstW(snapshot.as_raw_handle(), &raw mut entry) } != 0;
    while more {
        let length = entry
            .szExeFile
            .iter()
            .position(|&unit| unit == 0)
            .unwrap_or(entry.szExeFile.len());
        listed.insert(
            entry.th32ProcessID,
            Listed {
                threads: entry.cntThreads,
                name: String::from_utf16_lossy(&entry.szExeFile[..length]),
            },
        );
        // SAFETY: as for the first entry.
        more = unsafe { Process32NextW(snapshot.as_raw_handle(), &raw mut entry) } != 0;
    }
    Ok(listed)
}

fn cpu_times() -> Option<CpuTimes> {
    let (mut idle, mut kernel, mut user) = Default::default();
    // SAFETY: the three outputs are writable FILETIMEs.
    check(unsafe { GetSystemTimes(&raw mut idle, &raw mut kernel, &raw mut user) }).ok()?;
    // Kernel time includes the idle time.
    let total = filetime(kernel) + filetime(user);
    Some(CpuTimes {
        busy: total.saturating_sub(filetime(idle)),
        total,
    })
}

/// Every processor group's, as `GetSystemTimes` counts them, not only the ones this process may
/// run on.
fn processors() -> Option<u32> {
    // SAFETY: the call takes a group number and nothing else; it returns 0 when it fails.
    let count = unsafe { GetActiveProcessorCount(ALL_PROCESSOR_GROUPS) };
    (count > 0).then_some(count)
}

fn memory() -> Option<Memory> {
    let mut status = MEMORYSTATUSEX {
        dwLength: size_u32::<MEMORYSTATUSEX>(),
        ..Default::default()
    };
    // SAFETY: the structure is writable and its length is set, as the call requires.
    check(unsafe { GlobalMemoryStatusEx(&raw mut status) }).ok()?;
    Some(Memory {
        used: status.ullTotalPhys.saturating_sub(status.ullAvailPhys),
        total: status.ullTotalPhys,
    })
}

fn interfaces() -> Option<Vec<Interface>> {
    let mut table: *mut MIB_IF_TABLE2 = ptr::null_mut();
    // SAFETY: the call allocates the table and stores its address in `table`.
    if unsafe { GetIfTable2(&raw mut table) } != NO_ERROR || table.is_null() {
        return None;
    }
    // SAFETY: on success the table holds `NumEntries` rows, laid out from `Table` on; it stays
    // valid until FreeMibTable below.
    let rows = unsafe {
        std::slice::from_raw_parts(
            (&raw const (*table).Table).cast::<MIB_IF_ROW2>(),
            (*table).NumEntries as usize,
        )
    };
    let interfaces = rows
        .iter()
        .filter(|row| {
            let flags = row.InterfaceAndOperStatusFlags._bitfield;
            flags & HARDWARE_INTERFACE != 0 && flags & FILTER_INTERFACE == 0
        })
        .map(|row| Interface {
            // SAFETY: every variant of the LUID union is a view of the same 64 bits.
            key: unsafe { row.InterfaceLuid.Value }.to_string(),
            received: row.InOctets,
            sent: row.OutOctets,
        })
        .collect();
    // SAFETY: the table came from GetIfTable2 and is freed once, after its last use.
    unsafe { FreeMibTable(table.cast()) };
    Some(interfaces)
}

fn filetime(time: FILETIME) -> u64 {
    u64::from(time.dwHighDateTime) << 32 | u64::from(time.dwLowDateTime)
}

fn hundreds_of_ns(ticks: u64) -> Duration {
    Duration::from_nanos(ticks.saturating_mul(100))
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;
    use crate::{Launch, spawn};

    fn cmd(script: &str) -> Launch {
        Launch {
            program: std::env::var_os("ComSpec")
                .map_or_else(|| r"C:\Windows\System32\cmd.exe".into(), Into::into),
            args: vec!["/d".into(), "/c".into(), script.into()],
            cwd: std::env::temp_dir(),
        }
    }

    #[tokio::test]
    async fn the_tree_measures_its_main_process_and_its_child_apart() {
        let mut spawned = spawn(&cmd("echo ready& ping -n 30 127.0.0.1 >nul")).unwrap();
        assert!(spawned.stdout.next().await.is_some());
        let is_ping = |child: &Member| child.name.eq_ignore_ascii_case("ping.exe");
        let deadline = Instant::now() + Duration::from_secs(5);
        let usage = loop {
            let usage = spawned.tree.usage().unwrap();
            if usage.children.iter().any(is_ping) || Instant::now() > deadline {
                break usage;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        };

        let main = usage.main.expect("cmd still runs");
        assert!(main.working_set > 0 && main.private > 0);
        assert!(main.threads.is_some_and(|threads| threads >= 1));
        assert!(main.handles.is_some_and(|handles| handles > 0));
        assert!(main.written.is_some(), "cmd wrote `ready` to its pipe");
        // Windows gives a console program without a window a console host of its own, in the job.
        let names: Vec<&str> = usage.children.iter().map(|child| child.name.as_str()).collect();
        assert!(usage.children.len() <= 2, "{names:?}");
        let ping = usage
            .children
            .iter()
            .find(|child| is_ping(child))
            .expect("ping is a child");
        assert!(usage.children.iter().all(|child| child.pid != spawned.pid));
        assert!(ping.usage.working_set > 0);
        assert_ne!(ping.usage.started, main.started);

        spawned.tree.kill().unwrap();
        spawned.exit.wait().await;
        assert!(spawned.tree.wait_empty(Duration::from_secs(5)).await.unwrap());
        assert_eq!(
            spawned.tree.usage().unwrap(),
            TreeUsage {
                main: None,
                children: Vec::new()
            }
        );
    }

    #[test]
    fn observer_measures_itself() {
        let before = own_usage().unwrap();
        let spin = Instant::now();
        while spin.elapsed() < Duration::from_millis(50) {
            std::hint::black_box(spin.elapsed());
        }
        let after = own_usage().unwrap();

        assert!(after.cpu > before.cpu);
        assert!(after.working_set > 1024 * 1024);
        assert!(after.private > 0);
        assert_eq!(after.threads, None);
        assert_eq!(after.started, before.started);
    }

    #[test]
    fn the_host_reports_its_processors_memory_and_adapters() {
        let host = host_usage();

        let cpu = host.cpu.unwrap();
        assert!(cpu.total > cpu.busy && cpu.busy > 0);
        let usable = std::thread::available_parallelism().unwrap().get();
        assert!(host.processors.unwrap() as usize >= usable);
        let memory = host.memory.unwrap();
        assert!(memory.total > memory.used && memory.used > 0);
        let interfaces = host.interfaces.unwrap();
        let mut keys: Vec<&str> = interfaces
            .iter()
            .map(|interface| interface.key.as_str())
            .collect();
        keys.dedup();
        assert_eq!(keys.len(), interfaces.len(), "one row per adapter");
    }

    #[test]
    fn a_volume_reports_its_free_and_total_bytes() {
        let space = volume_space(&std::env::temp_dir()).unwrap();

        assert!(space.total >= space.available && space.available > 0);
    }
}
