use std::collections::HashMap;
use std::io;
use std::mem::size_of;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
use std::path::Path;
use std::ptr;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use windows_sys::Wdk::System::SystemInformation::{NtQuerySystemInformation, SystemProcessInformation};
use windows_sys::Win32::Foundation::{
    ERROR_SUCCESS, FALSE, FILETIME, INVALID_HANDLE_VALUE, LocalFree, NO_ERROR, STATUS_INFO_LENGTH_MISMATCH,
    WAIT_OBJECT_0,
};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    FreeMibTable, GetIfTable2, MIB_IF_ROW2, MIB_IF_TABLE2,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, GetDiskFreeSpaceExW, GetVolumeNameForVolumeMountPointW,
    GetVolumePathNameW, OPEN_EXISTING,
};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Ioctl::{DISK_PERFORMANCE, IOCTL_DISK_PERFORMANCE};
use windows_sys::Win32::System::JobObjects::{
    IsProcessInJob, JOBOBJECT_BASIC_AND_IO_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectBasicAndIoAccountingInformation, JobObjectExtendedLimitInformation, QueryInformationJobObject,
};
use windows_sys::Win32::System::Performance::{
    PDH_CSTATUS_NEW_DATA, PDH_CSTATUS_VALID_DATA, PDH_FMT_COUNTERVALUE, PDH_FMT_DOUBLE, PDH_HCOUNTER,
    PDH_HQUERY, PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterValue,
    PdhOpenQueryW,
};
use windows_sys::Win32::System::ProcessStatus::{
    GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
};
use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows_sys::Win32::System::Threading::{
    ALL_PROCESSOR_GROUPS, GetActiveProcessorCount, GetCurrentProcess, GetProcessHandleCount,
    GetProcessIdOfThread, GetProcessIoCounters, GetProcessTimes, GetSystemTimes, GetThreadDescription,
    GetThreadTimes, IO_COUNTERS, OpenProcess, OpenThread, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ,
    THREAD_QUERY_LIMITED_INFORMATION, WaitForSingleObject,
};
use windows_sys::Win32::System::WindowsProgramming::{SYSTEM_PROCESS_INFORMATION, SYSTEM_THREAD_INFORMATION};

use super::{ProcessTree, check, size_u32};
use crate::usage::{
    Clock, CpuTimes, DiskCounters, HostUsage, Interface, Member, Memory, Space, ThreadUsage, TreeTotals,
    TreeUsage, Usage,
};

/// `MIB_IF_ROW2.InterfaceAndOperStatusFlags`: a network card or a USB adapter, rather than a
/// loopback, a tunnel or a virtual switch's port.
const HARDWARE_INTERFACE: u8 = 0x01;
/// A filter driver's view of an adapter that is also listed on its own.
const FILTER_INTERFACE: u8 = 0x02;

/// `PDH_FMT_NOCAP100`, which the bindings lack: a boosting processor's performance passes 100%.
const PDH_FMT_NOCAP100: u32 = 0x0000_8000;
/// English names, which `PdhAddEnglishCounterW` takes whatever the system's language.
const FREQUENCY: &str = r"\Processor Information(_Total)\Processor Frequency";
const PERFORMANCE: &str = r"\Processor Information(_Total)\% Processor Performance";
const LIMIT: &str = r"\Processor Information(_Total)\% Performance Limit";

impl ProcessTree {
    /// What the main process, its threads and each of its children use now, and the tree's totals.
    /// A child or a thread that exits meanwhile is left out.
    pub fn usage(&self) -> io::Result<TreeUsage> {
        let members = self.members()?;
        let listed = snapshot(self.pid)?;
        // SAFETY: the handle stays open for as long as `self`; a zero wait only reads its state.
        let exited = unsafe { WaitForSingleObject(self.main.as_raw_handle(), 0) } == WAIT_OBJECT_0;
        let (main, threads) = if exited {
            (None, None)
        } else {
            let threads = listed.processes.get(&self.pid).map(|entry| entry.threads);
            (
                Some(measure(self.main.as_raw_handle(), threads)?),
                Some(measure_threads(self.pid, &listed.threads)),
            )
        };
        let mut children = Vec::new();
        for pid in members.into_iter().filter(|&pid| pid != self.pid) {
            let Some(process) = open_member(pid, self.job.raw()) else {
                continue;
            };
            let entry = listed.processes.get(&pid);
            let Ok(usage) = measure(process.as_raw_handle(), entry.map(|entry| entry.threads)) else {
                continue;
            };
            children.push(Member {
                pid,
                name: entry.map(|entry| entry.name.clone()).unwrap_or_default(),
                usage,
            });
        }
        Ok(TreeUsage {
            main,
            children,
            threads,
            totals: self.totals(),
        })
    }

    /// The Job Object's own accounting, which keeps the processes that left it.
    fn totals(&self) -> Option<TreeTotals> {
        let mut accounting = JOBOBJECT_BASIC_AND_IO_ACCOUNTING_INFORMATION::default();
        // SAFETY: the buffer is the structure this information class fills, and outlives the call;
        // the job handle stays open for as long as `self`.
        check(unsafe {
            QueryInformationJobObject(
                self.job.raw(),
                JobObjectBasicAndIoAccountingInformation,
                (&raw mut accounting).cast(),
                size_u32::<JOBOBJECT_BASIC_AND_IO_ACCOUNTING_INFORMATION>(),
                ptr::null_mut(),
            )
        })
        .ok()?;
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        // SAFETY: as above, with the structure of the extended limits.
        check(unsafe {
            QueryInformationJobObject(
                self.job.raw(),
                JobObjectExtendedLimitInformation,
                (&raw mut limits).cast(),
                size_u32::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>(),
                ptr::null_mut(),
            )
        })
        .ok()?;
        let basic = accounting.BasicInfo;
        let ticks = |time: i64| u64::try_from(time).unwrap_or(0);
        Some(TreeTotals {
            cpu: hundreds_of_ns(ticks(basic.TotalUserTime) + ticks(basic.TotalKernelTime)),
            processes_started: Some(u64::from(basic.TotalProcesses)),
            private_peak: Some(limits.PeakJobMemoryUsed as u64).filter(|&peak| peak > 0),
        })
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
        clock: clock(),
        pressure: None,
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

/// The volume's counters through `IOCTL_DISK_PERFORMANCE`, which a handle opened without any
/// access right may ask for: no elevation needed.
pub(crate) fn disk_counters(dir: &Path) -> io::Result<DiskCounters> {
    let path: Vec<u16> = dir.as_os_str().encode_wide().chain([0]).collect();
    let mut mount = vec![0u16; path.len().max(261)];
    // SAFETY: `path` is NUL-terminated, and `mount` is writable for the length given.
    check(unsafe { GetVolumePathNameW(path.as_ptr(), mount.as_mut_ptr(), length_u32(&mount)) })?;
    // `\\?\Volume{GUID}\` is 49 characters.
    let mut volume = [0u16; 64];
    // SAFETY: `mount` holds the NUL-terminated mount point the call above wrote, and `volume` is
    // writable for the length given.
    check(unsafe {
        GetVolumeNameForVolumeMountPointW(mount.as_ptr(), volume.as_mut_ptr(), length_u32(&volume))
    })?;
    let end = volume.iter().position(|&unit| unit == 0).unwrap_or(volume.len());
    // Without its trailing backslash the name opens the volume's device, not its root folder.
    let mut device = volume[..end].to_vec();
    if device.last() == Some(&u16::from(b'\\')) {
        device.pop();
    }
    device.push(0);
    // SAFETY: the name is NUL-terminated and outlives the call; no access right, null security
    // attributes and no template are valid arguments.
    let handle = unsafe {
        CreateFileW(
            device.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            ptr::null(),
            OPEN_EXISTING,
            0,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: CreateFileW returned an open handle nothing else owns.
    let device = unsafe { OwnedHandle::from_raw_handle(handle) };
    let mut performance = DISK_PERFORMANCE::default();
    let mut returned = 0;
    // SAFETY: the handle is open, this control code takes no input, and the output buffer is the
    // structure it fills, writable for its whole size.
    check(unsafe {
        DeviceIoControl(
            device.as_raw_handle(),
            IOCTL_DISK_PERFORMANCE,
            ptr::null(),
            0,
            (&raw mut performance).cast(),
            size_u32::<DISK_PERFORMANCE>(),
            &raw mut returned,
            ptr::null_mut(),
        )
    })?;
    let ticks = |time: i64| u64::try_from(time).unwrap_or(0);
    Ok(DiskCounters {
        transfers: u64::from(performance.ReadCount) + u64::from(performance.WriteCount),
        busy: hundreds_of_ns(ticks(performance.ReadTime) + ticks(performance.WriteTime)),
    })
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
        user: hundreds_of_ns(filetime(user)),
        kernel: hundreds_of_ns(filetime(kernel)),
        working_set: memory.WorkingSetSize as u64,
        working_set_peak: Some(memory.PeakWorkingSetSize as u64),
        private: memory.PrivateUsage as u64,
        faults: Some(u64::from(memory.PageFaultCount)),
        // Only an event trace tells a hard fault from a soft one.
        hard_faults: None,
        threads,
        handles,
        read: io.map(|io| io.ReadTransferCount),
        written: io.map(|io| io.WriteTransferCount),
        started: filetime(created),
    })
}

/// The threads of process `pid` among `ids`. One that exited since they were listed, or whose id
/// a thread of another process took meanwhile, is left out.
fn measure_threads(pid: u32, ids: &[u32]) -> Vec<ThreadUsage> {
    ids.iter().filter_map(|&id| measure_thread(pid, id)).collect()
}

fn measure_thread(pid: u32, id: u32) -> Option<ThreadUsage> {
    // SAFETY: OpenThread takes any id and returns null when it cannot open it.
    let handle = unsafe { OpenThread(THREAD_QUERY_LIMITED_INFORMATION, FALSE, id) };
    if handle.is_null() {
        return None;
    }
    // SAFETY: OpenThread returned an open handle nothing else owns.
    let thread = unsafe { OwnedHandle::from_raw_handle(handle) };
    // SAFETY: the handle is open with the right the call needs; it returns 0 when it fails.
    if unsafe { GetProcessIdOfThread(thread.as_raw_handle()) } != pid {
        return None;
    }
    let (mut created, mut exited, mut kernel, mut user) = Default::default();
    // SAFETY: same open handle, and the four outputs are writable.
    check(unsafe {
        GetThreadTimes(
            thread.as_raw_handle(),
            &raw mut created,
            &raw mut exited,
            &raw mut kernel,
            &raw mut user,
        )
    })
    .ok()?;
    Some(ThreadUsage {
        id,
        started: filetime(created),
        name: description(&thread),
        cpu: hundreds_of_ns(filetime(kernel) + filetime(user)),
    })
}

/// The name `SetThreadDescription` gave the thread, empty when it has none.
fn description(thread: &OwnedHandle) -> String {
    let mut text = ptr::null_mut::<u16>();
    // SAFETY: the handle is open with the right the call needs, and on success `text` receives a
    // buffer the system allocated for us.
    let result = unsafe { GetThreadDescription(thread.as_raw_handle(), &raw mut text) };
    if result < 0 || text.is_null() {
        return String::new();
    }
    // SAFETY: on success the buffer holds a NUL-terminated string and stays valid until it is
    // freed below.
    let name = unsafe {
        let mut length = 0;
        while *text.add(length) != 0 {
            length += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(text, length))
    };
    // SAFETY: the buffer came from GetThreadDescription and is freed once, after its last use.
    unsafe { LocalFree(text.cast()) };
    name
}

struct Listed {
    threads: u32,
    name: String,
}

struct Snapshot {
    processes: HashMap<u32, Listed>,
    /// The ids of the threads of the process the snapshot was taken for.
    threads: Vec<u32>,
}

/// Every process's thread count and executable name, and the threads of `owner`, from the system's
/// list of processes and their threads. A Toolhelp snapshot holds the same list, but walking its
/// threads one call at a time costs some 15 µs for each thread of the machine: 60 to 110 ms on a
/// desktop, against a few milliseconds for the one call here.
fn snapshot(owner: u32) -> io::Result<Snapshot> {
    let (words, length) = process_list()?;
    let list = words.as_ptr().cast::<u8>();
    let entry_size = size_of::<SYSTEM_PROCESS_INFORMATION>();
    let thread_size = size_of::<SYSTEM_THREAD_INFORMATION>();
    let mut processes = HashMap::new();
    let mut threads = Vec::new();
    let mut offset = 0;
    while offset + entry_size <= length {
        // SAFETY: the system wrote a chain of entries in the buffer, each at its predecessor's
        // `NextEntryOffset`, and this one lies inside the `length` bytes it wrote.
        let entry = unsafe {
            list.add(offset)
                .cast::<SYSTEM_PROCESS_INFORMATION>()
                .read_unaligned()
        };
        let pid = u32::try_from(entry.UniqueProcessId as usize).unwrap_or(u32::MAX);
        let name = if entry.ImageName.Buffer.is_null() {
            String::new()
        } else {
            // SAFETY: a non-null name points into the same buffer, which stays alive meanwhile:
            // `Length` bytes of UTF-16, 2-aligned like the 8-aligned buffer the system fills.
            String::from_utf16_lossy(unsafe {
                std::slice::from_raw_parts(entry.ImageName.Buffer, usize::from(entry.ImageName.Length) / 2)
            })
        };
        processes.insert(
            pid,
            Listed {
                threads: entry.NumberOfThreads,
                name,
            },
        );
        if pid == owner {
            // The entry's threads follow it.
            let first = offset + entry_size;
            for index in 0..entry.NumberOfThreads as usize {
                let at = first + index * thread_size;
                if at + thread_size > length {
                    break;
                }
                // SAFETY: the thread entry lies inside the bytes the system wrote, as checked.
                let thread = unsafe { list.add(at).cast::<SYSTEM_THREAD_INFORMATION>().read_unaligned() };
                threads.push(u32::try_from(thread.ClientId.UniqueThread as usize).unwrap_or(u32::MAX));
            }
        }
        if entry.NextEntryOffset == 0 {
            break;
        }
        offset += entry.NextEntryOffset as usize;
    }
    Ok(Snapshot { processes, threads })
}

/// `SystemProcessInformation` in a buffer grown until the list fits, and the bytes it takes. The
/// buffer is of `u64` so that the entries in it are aligned.
fn process_list() -> io::Result<(Vec<u64>, usize)> {
    let mut capacity = 1 << 20;
    loop {
        let mut list = vec![0u64; capacity / 8];
        let mut written = 0;
        let length = u32::try_from(capacity).map_err(io::Error::other)?;
        // SAFETY: the buffer is writable for `length` bytes and `written` receives the size used.
        let status = unsafe {
            NtQuerySystemInformation(
                SystemProcessInformation,
                list.as_mut_ptr().cast(),
                length,
                &raw mut written,
            )
        };
        if status == STATUS_INFO_LENGTH_MISMATCH {
            // Processes may start between this call and the next.
            capacity = (written as usize).max(capacity).next_multiple_of(8) + (1 << 16);
            continue;
        }
        if status < 0 {
            return Err(io::Error::other(format!(
                "the system would not list its processes ({status:#x})"
            )));
        }
        return Ok((list, (written as usize).min(capacity)));
    }
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
        steal: None,
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

/// The processors' clock from the performance counters. `CallNtPowerInformation` and WMI report
/// the base clock whatever the boost; only `% Processor Performance` follows it, as Task Manager
/// does.
fn clock() -> Option<Clock> {
    static PDH: Mutex<Pdh> = Mutex::new(Pdh::Unopened);
    let mut pdh = PDH.lock().unwrap_or_else(PoisonError::into_inner);
    if matches!(*pdh, Pdh::Unopened) {
        *pdh = Counters::open().map_or(Pdh::Failed, Pdh::Open);
    }
    match &*pdh {
        Pdh::Open(counters) => counters.read(),
        Pdh::Unopened | Pdh::Failed => None,
    }
}

/// The clock's query, opened once for Observer's life: a rate needs the collection before it.
enum Pdh {
    Unopened,
    Open(Counters),
    Failed,
}

// SAFETY: PDH handles are not tied to the thread that opened them, and the mutex that holds the
// only `Pdh` serialises every call made with them.
unsafe impl Send for Pdh {}

struct Counters {
    query: PDH_HQUERY,
    frequency: PDH_HCOUNTER,
    performance: PDH_HCOUNTER,
    limit: PDH_HCOUNTER,
}

impl Counters {
    fn open() -> Option<Self> {
        let mut query = ptr::null_mut();
        // SAFETY: a null data source means live counters, and `query` receives the handle.
        if unsafe { PdhOpenQueryW(ptr::null(), 0, &raw mut query) } != ERROR_SUCCESS {
            return None;
        }
        let add = |path: &str| {
            let path: Vec<u16> = path.encode_utf16().chain([0]).collect();
            let mut counter = ptr::null_mut();
            // SAFETY: the query is open, the path is NUL-terminated and outlives the call, and
            // `counter` receives the handle.
            let status = unsafe { PdhAddEnglishCounterW(query, path.as_ptr(), 0, &raw mut counter) };
            (status == ERROR_SUCCESS).then_some(counter)
        };
        let counters = match (add(FREQUENCY), add(PERFORMANCE), add(LIMIT)) {
            (Some(frequency), Some(performance), Some(limit)) => Some(Self {
                query,
                frequency,
                performance,
                limit,
            }),
            _ => None,
        };
        if counters.is_none() {
            // SAFETY: the query is open and closed once, with the counters added to it.
            unsafe { PdhCloseQuery(query) };
        }
        counters
    }

    /// `None` after the query's first collection, which has nothing to compute a rate from.
    fn read(&self) -> Option<Clock> {
        // SAFETY: the query stays open for the process's life.
        if unsafe { PdhCollectQueryData(self.query) } != ERROR_SUCCESS {
            return None;
        }
        let frequency = counter_value(self.frequency).filter(|&mhz| mhz > 0.0)?;
        let performance = counter_value(self.performance)?;
        Some(Clock {
            mhz: frequency * performance / 100.0,
            max_mhz: frequency,
            limit: counter_value(self.limit),
        })
    }
}

fn counter_value(counter: PDH_HCOUNTER) -> Option<f64> {
    let mut value = PDH_FMT_COUNTERVALUE::default();
    // SAFETY: the counter belongs to the open query, the counter type output is optional, and
    // `value` is writable.
    let status = unsafe {
        PdhGetFormattedCounterValue(
            counter,
            PDH_FMT_DOUBLE | PDH_FMT_NOCAP100,
            ptr::null_mut(),
            &raw mut value,
        )
    };
    if status != ERROR_SUCCESS || !matches!(value.CStatus, PDH_CSTATUS_VALID_DATA | PDH_CSTATUS_NEW_DATA) {
        return None;
    }
    // SAFETY: `PDH_FMT_DOUBLE` asked for the double of the union.
    let value = unsafe { value.Anonymous.doubleValue };
    (value.is_finite() && value >= 0.0).then_some(value)
}

fn filetime(time: FILETIME) -> u64 {
    u64::from(time.dwHighDateTime) << 32 | u64::from(time.dwLowDateTime)
}

fn hundreds_of_ns(ticks: u64) -> Duration {
    Duration::from_nanos(ticks.saturating_mul(100))
}

/// A buffer's length in UTF-16 units, as Win32 takes it; these hold a path or a volume name.
fn length_u32(buffer: &[u16]) -> u32 {
    u32::try_from(buffer.len()).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;
    use std::sync::mpsc;
    use std::time::Instant;

    use windows_sys::Win32::System::Threading::GetCurrentThread;

    use super::*;
    use crate::{Launch, spawn};

    /// The calling thread's own CPU time, as Windows counts it.
    fn own_thread_cpu() -> Duration {
        let (mut created, mut exited, mut kernel, mut user) = Default::default();
        // SAFETY: GetCurrentThread returns a pseudo-handle valid on this thread, and the four
        // outputs are writable locals.
        check(unsafe {
            GetThreadTimes(
                GetCurrentThread(),
                &raw mut created,
                &raw mut exited,
                &raw mut kernel,
                &raw mut user,
            )
        })
        .unwrap();
        hundreds_of_ns(filetime(kernel) + filetime(user))
    }

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
        let gone = spawned.tree.usage().unwrap();
        assert_eq!((gone.main, gone.children, gone.threads), (None, Vec::new(), None));
        assert!(gone.totals.is_some(), "the job keeps its accounting");
    }

    #[tokio::test]
    async fn the_main_process_s_threads_keep_their_ids_and_starts() {
        let mut spawned = spawn(&cmd("echo ready& ping -n 30 127.0.0.1 >nul")).unwrap();
        assert!(spawned.stdout.next().await.is_some());

        let first = spawned.tree.usage().unwrap().threads.expect("cmd runs");
        let second = spawned.tree.usage().unwrap().threads.expect("cmd runs");

        assert_ne!(first.len(), 0);
        let mut kept = 0;
        for thread in &first {
            if let Some(again) = second.iter().find(|again| again.id == thread.id) {
                assert_eq!(again.started, thread.started);
                assert!(again.cpu >= thread.cpu);
                kept += 1;
            }
        }
        assert!(kept > 0, "{first:?} then {second:?}");
        spawned.tree.kill().unwrap();
        spawned.exit.wait().await;
    }

    #[tokio::test]
    async fn the_tree_totals_keep_a_child_that_ended() {
        let mut spawned = spawn(&cmd("cmd /d /c exit 0& echo ready& ping -n 30 127.0.0.1 >nul")).unwrap();
        assert!(spawned.stdout.next().await.is_some());

        let usage = spawned.tree.usage().unwrap();

        let totals = usage.totals.expect("the job answers");
        // cmd, the cmd that ended and the console host at least.
        assert!(
            totals.processes_started.is_some_and(|started| started >= 2),
            "{totals:?}"
        );
        assert!(totals.private_peak.is_some_and(|peak| peak > 0));
        assert!(totals.cpu >= usage.main.expect("cmd runs").cpu());
        spawned.tree.kill().unwrap();
        spawned.exit.wait().await;
    }

    #[test]
    fn observer_measures_itself() {
        let before = own_usage().unwrap();
        let spin = Instant::now();
        while spin.elapsed() < Duration::from_millis(50) {
            std::hint::black_box(spin.elapsed());
        }
        let after = own_usage().unwrap();

        assert!(after.cpu() > before.cpu());
        assert!(after.working_set > 1024 * 1024);
        assert!(after.private > 0);
        assert_eq!(after.threads, None);
        assert_eq!(after.started, before.started);
    }

    #[test]
    fn user_and_kernel_time_are_measured_apart() {
        let before = own_usage().unwrap();
        // Spin until Windows has counted the time rather than for a stretch of wall time, which a
        // busy machine shares out among more threads than it has CPUs.
        let started = Instant::now();
        let mut spun = before;
        while spun.user < before.user + Duration::from_millis(100)
            && started.elapsed() < Duration::from_secs(10)
        {
            std::hint::black_box(started.elapsed());
            spun = own_usage().unwrap();
        }
        // File system calls spend their time in the kernel.
        let dir = std::env::temp_dir();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut after = spun;
        while after.kernel < spun.kernel + Duration::from_millis(20) && Instant::now() < deadline {
            for _ in 0..200 {
                let _ = std::fs::metadata(&dir);
            }
            after = own_usage().unwrap();
        }

        assert!(
            spun.user >= before.user + Duration::from_millis(100),
            "{before:?} {spun:?}"
        );
        assert!(
            after.kernel >= spun.kernel + Duration::from_millis(20),
            "{spun:?} {after:?}"
        );
        assert_eq!(after.cpu(), after.user + after.kernel);
    }

    #[test]
    fn touching_64_mb_raises_the_page_faults_and_the_peak_follows() {
        let before = own_usage().unwrap();
        let mut memory = vec![0u8; 64 << 20];
        for page in memory.chunks_mut(4096) {
            page[0] = 1;
        }
        std::hint::black_box(&memory);
        let after = own_usage().unwrap();

        let faults = after.faults.unwrap() - before.faults.unwrap();
        assert!(faults >= 16_000, "{faults} faults");
        let peak = after.working_set_peak.unwrap();
        assert!(peak >= after.working_set && peak >= 64 << 20, "{after:?}");
        assert_eq!(after.hard_faults, None);
    }

    #[test]
    fn a_named_thread_of_this_process_is_found_with_its_cpu() {
        let (spun, spinning) = mpsc::channel();
        let (release, held) = mpsc::channel::<()>();
        let worker = std::thread::Builder::new()
            .name("UV loop: svNetwork".to_owned())
            .spawn(move || {
                // Spin until Windows has counted the thread's own time rather than for a stretch
                // of wall time, which a busy machine shares out among more threads than it has CPUs.
                let started = Instant::now();
                while own_thread_cpu() < Duration::from_millis(60)
                    && started.elapsed() < Duration::from_secs(10)
                {
                    std::hint::black_box(started.elapsed());
                }
                spun.send(()).unwrap();
                let _ = held.recv();
            })
            .unwrap();
        spinning.recv().unwrap();

        let pid = std::process::id();
        let threads = measure_threads(pid, &snapshot(pid).unwrap().threads);
        release.send(()).unwrap();
        worker.join().unwrap();

        let named = threads
            .iter()
            .find(|thread| thread.name == "UV loop: svNetwork")
            .expect("the thread is listed by its name");
        assert!(named.cpu >= Duration::from_millis(50), "{named:?}");
        assert!(threads.iter().all(|thread| thread.started > 0));
    }

    #[test]
    fn the_host_reports_its_processors_memory_and_adapters() {
        let host = host_usage();

        let cpu = host.cpu.unwrap();
        assert!(cpu.total > cpu.busy && cpu.busy > 0);
        assert_eq!(cpu.steal, None);
        assert_eq!(host.pressure, None);
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
    fn the_clock_is_known_from_the_second_reading() {
        let _ = host_usage();
        std::thread::sleep(Duration::from_millis(250));

        let clock = host_usage().clock.expect("the performance counters answer");

        assert!(clock.mhz > 0.0 && clock.max_mhz > 0.0, "{clock:?}");
        assert!(
            clock.limit.is_some_and(|limit| limit > 0.0 && limit <= 100.0),
            "{clock:?}"
        );
    }

    #[test]
    fn a_volume_reports_its_free_and_total_bytes() {
        let space = volume_space(&std::env::temp_dir()).unwrap();

        assert!(space.total >= space.available && space.available > 0);
    }

    #[test]
    fn the_disk_counters_grow_after_a_synced_write() {
        let dir = tempfile::tempdir().unwrap();
        let before = disk_counters(dir.path()).unwrap();

        let mut file = std::fs::File::create(dir.path().join("written")).unwrap();
        file.write_all(&vec![0x5a; 1 << 20]).unwrap();
        file.sync_all().unwrap();

        let after = disk_counters(dir.path()).unwrap();
        assert!(after.transfers > before.transfers, "{before:?} {after:?}");
        assert!(after.busy >= before.busy);
    }
}
