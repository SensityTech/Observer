use std::ffi::OsString;
use std::fs::File;
use std::io;
use std::mem::size_of;
use std::os::windows::ffi::OsStringExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::{ptr, thread};

use windows_sys::Wdk::System::Threading::{
    NtQueryInformationProcess, ProcessBasicInformation, ProcessCommandLineInformation,
};
use windows_sys::Win32::Foundation::{
    FILETIME, INVALID_HANDLE_VALUE, STATUS_INFO_LENGTH_MISMATCH, UNICODE_STRING,
};
use windows_sys::Win32::System::Diagnostics::Debug::{
    MiniDumpNormal, MiniDumpWithThreadInfo, MiniDumpWithUnloadedModules, MiniDumpWriteDump,
};
use windows_sys::Win32::System::IO::{
    CreateIoCompletionPort, GetQueuedCompletionStatus, OVERLAPPED, PostQueuedCompletionStatus,
};
use windows_sys::Win32::System::JobObjects::{
    JOBOBJECT_ASSOCIATE_COMPLETION_PORT, JobObjectAssociateCompletionPortInformation, SetInformationJobObject,
};
use windows_sys::Win32::System::Threading::{
    GetProcessTimes, INFINITE, PROCESS_BASIC_INFORMATION, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};

use super::usage::{filetime, open_member};
use super::{Job, ProcessTree, check, size_u32};
use crate::processes::{ChangeSender, ProcessInfo, TreeChange};

/// The job's messages, from `winnt.h`; windows-sys files them under a module of thousands of
/// unrelated items.
const JOB_OBJECT_MSG_NEW_PROCESS: u32 = 6;
const JOB_OBJECT_MSG_EXIT_PROCESS: u32 = 7;
const JOB_OBJECT_MSG_ABNORMAL_EXIT_PROCESS: u32 = 8;
/// The completion keys of the job's messages and of the packet that stops the watcher.
const JOB_KEY: usize = 1;
const STOP_KEY: usize = 2;
/// A command line is at most 32,767 characters; anything longer is not one.
const MAX_COMMAND_LINE_BYTES: usize = 64 * 1024 + size_of::<UNICODE_STRING>();
/// 100 ns intervals between 1601 and 1970.
const UNIX_EPOCH_IN_FILETIME: u64 = 116_444_736_000_000_000;

/// dbghelp is single-threaded.
static DBGHELP: Mutex<()> = Mutex::new(());

/// The completion port the job posts its messages to: a process joined, a process ended.
pub(crate) struct Port(OwnedHandle);

impl Port {
    /// Creates the port and has `job` post to it. It must be done before the main process joins
    /// the job, or its first children go unreported.
    pub(crate) fn for_job(job: &Job) -> io::Result<Self> {
        // SAFETY: a new port that no file is associated with; null is the documented failure.
        let handle = unsafe { CreateIoCompletionPort(INVALID_HANDLE_VALUE, ptr::null_mut(), 0, 1) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: CreateIoCompletionPort returned an open handle nothing else owns.
        let port = Self(unsafe { OwnedHandle::from_raw_handle(handle) });
        let association = JOBOBJECT_ASSOCIATE_COMPLETION_PORT {
            CompletionKey: JOB_KEY as *mut core::ffi::c_void,
            CompletionPort: port.raw(),
        };
        // SAFETY: the structure is the one this information class takes and outlives the call;
        // both handles are open.
        check(unsafe {
            SetInformationJobObject(
                job.raw(),
                JobObjectAssociateCompletionPortInformation,
                (&raw const association).cast(),
                size_u32::<JOBOBJECT_ASSOCIATE_COMPLETION_PORT>(),
            )
        })?;
        Ok(port)
    }

    fn raw(&self) -> RawHandle {
        self.0.as_raw_handle()
    }

    /// Wakes the watcher for good.
    pub(crate) fn stop(&self) {
        // SAFETY: the port is open; a null overlapped pointer is allowed for a posted packet.
        unsafe { PostQueuedCompletionStatus(self.raw(), 0, STOP_KEY, ptr::null()) };
    }

    /// The next packet: the message, its key and the pid it is about. `None` once the port fails.
    fn next(&self) -> Option<(u32, usize, u32)> {
        let (mut message, mut key, mut overlapped) = (0u32, 0usize, ptr::null_mut::<OVERLAPPED>());
        // SAFETY: the port is open for as long as `self`, and the three outputs are writable.
        let ok = unsafe {
            GetQueuedCompletionStatus(
                self.raw(),
                &raw mut message,
                &raw mut key,
                &raw mut overlapped,
                INFINITE,
            )
        };
        // A job message carries its pid in place of the overlapped pointer.
        let pid = u32::try_from(overlapped as usize).unwrap_or(0);
        (ok != 0).then_some((message, key, pid))
    }
}

/// The thread that turns the job's messages into changes, until the tree is dropped or nobody
/// reads them. It only holds the job weakly, so that the job still closes with the tree.
pub(crate) fn watch(port: Arc<Port>, job: Weak<Job>, main: u32, changes: ChangeSender) -> io::Result<()> {
    thread::Builder::new()
        .name("process-tree".to_owned())
        .spawn(move || {
            while let Some((message, key, pid)) = port.next() {
                if key == STOP_KEY {
                    break;
                }
                if key != JOB_KEY || pid == main {
                    continue;
                }
                let change = match message {
                    JOB_OBJECT_MSG_NEW_PROCESS => {
                        let Some(job) = job.upgrade() else {
                            break;
                        };
                        TreeChange::Started(inspect(pid, &job).unwrap_or_else(|| gone(pid)))
                    }
                    JOB_OBJECT_MSG_EXIT_PROCESS | JOB_OBJECT_MSG_ABNORMAL_EXIT_PROCESS => {
                        TreeChange::Ended { pid }
                    }
                    _ => continue,
                };
                if !changes.send(change) {
                    break;
                }
            }
        })?;
    Ok(())
}

impl ProcessTree {
    /// What the tree's process `pid` is, or `None` when it is gone or not in the tree.
    pub fn describe(&self, pid: u32) -> Option<ProcessInfo> {
        inspect(pid, &self.job)
    }

    /// Every process in the tree: the job's members.
    pub fn all_members(&self) -> io::Result<Vec<u32>> {
        self.members()
    }

    /// Whether `pid`, which the tree reported, still runs. A process never leaves the job, so the
    /// job's list answers; `stamp` matters on Linux only.
    pub fn still_runs(&self, pid: u32, _stamp: Option<u64>) -> bool {
        self.members().is_ok_and(|members| members.contains(&pid))
    }

    /// Writes a minidump of the main process into `file`: the threads' stacks and contexts, their
    /// names and times, and the modules. No heap and no environment block, which hold FXServer's
    /// secrets, so it stays a few megabytes.
    pub fn write_dump(&self, file: &File) -> io::Result<()> {
        let _single = DBGHELP.lock().unwrap_or_else(PoisonError::into_inner);
        let kind = MiniDumpNormal | MiniDumpWithThreadInfo | MiniDumpWithUnloadedModules;
        // SAFETY: the process handle has every access right (it came from CreateProcessW) and the
        // file handle is open for writing for the call; the three optional pointers may be null.
        check(unsafe {
            MiniDumpWriteDump(
                self.main.as_raw_handle(),
                self.pid,
                file.as_raw_handle(),
                kind,
                ptr::null(),
                ptr::null(),
                ptr::null(),
            )
        })
    }
}

/// Reads what a member of the job is. A process that already ended usually keeps its image and
/// parent but loses its command line with its memory.
fn inspect(pid: u32, job: &Job) -> Option<ProcessInfo> {
    let process = open_member(pid, job.raw(), PROCESS_QUERY_LIMITED_INFORMATION)?;
    let handle = process.as_raw_handle();
    let command_line = command_line(handle);
    let created = creation(handle);
    Some(ProcessInfo {
        pid,
        parent: parent(handle),
        image: image(handle),
        args: command_line
            .as_deref()
            .map(split_command_line)
            .unwrap_or_default(),
        command_line,
        created_ms: created.and_then(unix_ms),
        stamp: created,
        startup_variables: None,
    })
}

/// A process that ended before it could be read.
fn gone(pid: u32) -> ProcessInfo {
    ProcessInfo {
        pid,
        parent: None,
        image: None,
        args: Vec::new(),
        command_line: None,
        created_ms: None,
        stamp: None,
        startup_variables: None,
    }
}

fn image(process: RawHandle) -> Option<PathBuf> {
    let mut buffer = vec![0u16; 32 * 1024];
    let mut length = u32::try_from(buffer.len()).ok()?;
    // SAFETY: the handle is open with query rights, the buffer holds `length` characters and the
    // call writes back how many it used.
    let ok = unsafe {
        QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, buffer.as_mut_ptr(), &raw mut length)
    };
    (ok != 0).then(|| PathBuf::from(OsString::from_wide(&buffer[..length as usize])))
}

fn command_line(process: RawHandle) -> Option<String> {
    // `u64`s, so that the UNICODE_STRING at the start is aligned.
    let mut buffer = vec![0u64; 512];
    loop {
        let bytes = buffer.len() * size_of::<u64>();
        let mut needed = 0u32;
        // SAFETY: the buffer is `bytes` long and aligned; the handle is open with query rights.
        let status = unsafe {
            NtQueryInformationProcess(
                process,
                ProcessCommandLineInformation,
                buffer.as_mut_ptr().cast(),
                u32::try_from(bytes).ok()?,
                &raw mut needed,
            )
        };
        if status == STATUS_INFO_LENGTH_MISMATCH && (needed as usize) <= MAX_COMMAND_LINE_BYTES {
            let grown = (needed as usize).div_ceil(size_of::<u64>());
            if grown <= buffer.len() {
                return None;
            }
            buffer.resize(grown, 0);
            continue;
        }
        if status < 0 {
            return None;
        }
        let start = buffer.as_ptr() as usize;
        // SAFETY: on success the buffer starts with the UNICODE_STRING the class returns.
        let text = unsafe { buffer.as_ptr().cast::<UNICODE_STRING>().read() };
        let (address, length) = (text.Buffer as usize, usize::from(text.Length));
        // The text follows the header inside the buffer; anything else is not trusted.
        if address < start + size_of::<UNICODE_STRING>()
            || address + length > start + bytes
            || address % 2 != 0
        {
            return None;
        }
        // SAFETY: the characters lie inside the buffer, checked above, and are aligned.
        let characters = unsafe { std::slice::from_raw_parts(text.Buffer, length / 2) };
        return Some(String::from_utf16_lossy(characters));
    }
}

fn parent(process: RawHandle) -> Option<u32> {
    let mut basic = PROCESS_BASIC_INFORMATION::default();
    // SAFETY: the structure is the one this class fills and outlives the call; the handle is open
    // with query rights.
    let status = unsafe {
        NtQueryInformationProcess(
            process,
            ProcessBasicInformation,
            (&raw mut basic).cast(),
            size_u32::<PROCESS_BASIC_INFORMATION>(),
            ptr::null_mut(),
        )
    };
    (status >= 0)
        .then(|| u32::try_from(basic.InheritedFromUniqueProcessId).ok())
        .flatten()
}

fn creation(process: RawHandle) -> Option<u64> {
    let (mut created, mut exited, mut kernel, mut user) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    // SAFETY: the handle is open with query rights and the four outputs are writable.
    check(unsafe {
        GetProcessTimes(
            process,
            &raw mut created,
            &raw mut exited,
            &raw mut kernel,
            &raw mut user,
        )
    })
    .ok()?;
    Some(filetime(created))
}

fn unix_ms(filetime: u64) -> Option<i64> {
    i64::try_from(filetime.checked_sub(UNIX_EPOCH_IN_FILETIME)? / 10_000).ok()
}

/// Splits a command line into arguments the way the C runtime does for `main`: the program name
/// ends at the first blank, or the closing quote when it starts with one; then blanks separate
/// arguments outside quotes, `2n` backslashes before a quote give `n` and the quote toggles
/// quoting, `2n + 1` give `n` and a literal quote, `""` inside quotes is a literal quote, and
/// other backslashes are literal.
pub(crate) fn split_command_line(line: &str) -> Vec<String> {
    let mut chars = line.chars().peekable();
    let mut args = Vec::new();
    let mut program = String::new();
    if chars.peek() == Some(&'"') {
        chars.next();
        program.extend(chars.by_ref().take_while(|&c| c != '"'));
    }
    while let Some(&c) = chars.peek() {
        if c == ' ' || c == '\t' {
            break;
        }
        program.push(c);
        chars.next();
    }
    args.push(program);

    loop {
        while chars.next_if(|&c| c == ' ' || c == '\t').is_some() {}
        if chars.peek().is_none() {
            return args;
        }
        let mut arg = String::new();
        let mut quoted = false;
        while let Some(&c) = chars.peek() {
            match c {
                ' ' | '\t' if !quoted => break,
                '\\' => {
                    let mut backslashes = 0;
                    while chars.next_if_eq(&'\\').is_some() {
                        backslashes += 1;
                    }
                    if chars.peek() == Some(&'"') {
                        arg.extend(std::iter::repeat_n('\\', backslashes / 2));
                        if backslashes % 2 == 1 {
                            arg.push('"');
                            chars.next();
                        }
                    } else {
                        arg.extend(std::iter::repeat_n('\\', backslashes));
                    }
                }
                '"' => {
                    chars.next();
                    if quoted && chars.peek() == Some(&'"') {
                        arg.push('"');
                        chars.next();
                    } else {
                        quoted = !quoted;
                    }
                }
                c => {
                    arg.push(c);
                    chars.next();
                }
            }
        }
        args.push(arg);
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;
    use crate::{Launch, spawn};

    fn split(line: &str) -> Vec<String> {
        split_command_line(line)
    }

    #[test]
    fn the_lab_s_command_lines_split_as_fxserver_s_runtime_reads_them() {
        assert_eq!(
            split(r#""C:\lab\build\FXServer.exe" +set citizen_dir "C:\lab\build\citizen" +exec lab.cfg"#),
            [
                r"C:\lab\build\FXServer.exe",
                "+set",
                "citizen_dir",
                r"C:\lab\build\citizen",
                "+exec",
                "lab.cfg"
            ]
        );
        assert_eq!(
            split(r"C:\lab\build\FXServer.exe -dumpserver:408 -parentpid:3312"),
            [r"C:\lab\build\FXServer.exe", "-dumpserver:408", "-parentpid:3312"]
        );
        assert_eq!(
            split(r"\??\C:\WINDOWS\system32\conhost.exe 0x4"),
            [r"\??\C:\WINDOWS\system32\conhost.exe", "0x4"]
        );
    }

    #[test]
    fn backslashes_count_only_before_a_quote() {
        assert_eq!(
            split(r#"a.exe a\b "c d\\" e\"f"#),
            ["a.exe", r"a\b", r"c d\", r#"e"f"#]
        );
        assert_eq!(split(r#"a.exe "say ""hi""" x"#), ["a.exe", r#"say "hi""#, "x"]);
        assert_eq!(split(r#""C:\my dir\a.exe"x y"#), [r"C:\my dir\a.exex", "y"]);
        assert_eq!(split("a.exe   \t "), ["a.exe"]);
        assert_eq!(split(r#"a.exe """#), ["a.exe", ""]);
    }

    fn cmd(script: &str) -> Launch {
        Launch {
            program: std::env::var_os("ComSpec")
                .map_or_else(|| r"C:\Windows\System32\cmd.exe".into(), Into::into),
            args: vec!["/d".into(), "/c".into(), script.into()],
            cwd: std::env::temp_dir(),
        }
    }

    fn file_name(info: &ProcessInfo) -> String {
        info.image
            .as_deref()
            .and_then(|image| image.file_name())
            .map(|name| name.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default()
    }

    /// The changes until `done` says enough was seen, or five seconds.
    async fn changes_until(
        changes: &mut crate::TreeChanges,
        done: impl Fn(&[TreeChange]) -> bool,
    ) -> Vec<TreeChange> {
        let mut seen = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !done(&seen) && Instant::now() < deadline {
            if let Ok(Some(change)) = tokio::time::timeout(Duration::from_millis(100), changes.next()).await {
                seen.push(change);
            }
        }
        seen
    }

    #[tokio::test]
    async fn a_child_is_reported_as_it_starts_with_its_command_line_and_parent_then_as_it_ends() {
        let mut spawned = spawn(&cmd("ping -n 2 127.0.0.1 >nul")).unwrap();
        let ping_ended = |seen: &[TreeChange]| {
            let ping = seen.iter().find_map(|change| match change {
                TreeChange::Started(info) if file_name(info) == "ping.exe" => Some(info.pid),
                _ => None,
            });
            ping.is_some_and(|pid| seen.contains(&TreeChange::Ended { pid }))
        };

        let seen = changes_until(&mut spawned.changes, ping_ended).await;

        let ping = seen
            .iter()
            .find_map(|change| match change {
                TreeChange::Started(info) if file_name(info) == "ping.exe" => Some(info.clone()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no ping in {seen:#?}"));
        assert_eq!(ping.parent, Some(spawned.pid));
        assert!(
            ping.command_line
                .as_deref()
                .is_some_and(|line| line.contains("-n 2")),
            "{ping:?}"
        );
        assert_eq!(ping.args[1..], ["-n", "2", "127.0.0.1"]);
        assert!(ping.created_ms.is_some() && ping.stamp.is_some());
        assert!(ping_ended(&seen), "{seen:#?}");
        assert!(
            seen.iter()
                .all(|change| !matches!(change, TreeChange::Started(info) if info.pid == spawned.pid)),
            "the main process is never reported"
        );
        assert_eq!(spawned.exit.wait().await, crate::Exit::Code(0));
    }

    #[tokio::test]
    async fn a_grandchild_is_reported_with_its_own_parent() {
        let mut spawned = spawn(&cmd("cmd /d /c ping -n 2 127.0.0.1 >nul")).unwrap();

        let seen = changes_until(&mut spawned.changes, |seen| {
            seen.iter()
                .any(|change| matches!(change, TreeChange::Started(info) if file_name(info) == "ping.exe"))
        })
        .await;

        let started: Vec<&ProcessInfo> = seen
            .iter()
            .filter_map(|change| match change {
                TreeChange::Started(info) => Some(info),
                TreeChange::Ended { .. } => None,
            })
            .collect();
        let inner = started
            .iter()
            .find(|info| file_name(info) == "cmd.exe")
            .unwrap_or_else(|| panic!("no inner cmd in {seen:#?}"));
        let ping = started.iter().find(|info| file_name(info) == "ping.exe").unwrap();
        assert_eq!(inner.parent, Some(spawned.pid));
        assert_eq!(ping.parent, Some(inner.pid));
        spawned.tree.kill().unwrap();
    }

    #[tokio::test]
    async fn a_process_outside_the_tree_is_not_described() {
        let spawned = spawn(&cmd("ping -n 3 127.0.0.1 >nul")).unwrap();

        assert_eq!(spawned.tree.describe(std::process::id()), None);
        assert_eq!(
            spawned.tree.describe(spawned.pid).map(|info| info.pid),
            Some(spawned.pid)
        );
        spawned.tree.kill().unwrap();
    }

    /// The stream types a minidump's directory lists.
    fn streams(dump: &[u8]) -> Vec<u32> {
        let word = |at: usize| u32::from_le_bytes(dump[at..at + 4].try_into().unwrap());
        let (count, directory) = (word(8) as usize, word(12) as usize);
        (0..count).map(|index| word(directory + index * 12)).collect()
    }

    #[tokio::test]
    async fn a_dump_of_the_main_process_holds_its_threads_and_modules_but_no_memory_beyond_stacks() {
        let mut spawned = spawn(&cmd("echo ready& ping -n 30 127.0.0.1 >nul")).unwrap();
        assert!(spawned.stdout.next().await.is_some());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hang.dmp");

        spawned.tree.write_dump(&File::create(&path).unwrap()).unwrap();

        let dump = std::fs::read(&path).unwrap();
        assert_eq!(&dump[..4], b"MDMP");
        let streams = streams(&dump);
        // Thread list, module list, thread info list.
        for stream in [3, 4, 17] {
            assert!(
                streams.contains(&stream),
                "stream {stream} missing from {streams:?}"
            );
        }
        // No full memory list.
        assert!(!streams.contains(&9), "{streams:?}");
        assert!(dump.len() < 16 * 1024 * 1024, "{} bytes", dump.len());
        spawned.tree.kill().unwrap();
    }
}
