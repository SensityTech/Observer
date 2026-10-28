pub mod authenticode;
pub mod console;
pub mod message_loop;
mod session;
pub(crate) mod usage;

use std::ffi::OsStr;
use std::fs::File;
use std::io;
use std::marker::PhantomData;
use std::mem::{offset_of, size_of, size_of_val};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
use std::path::Path;
use std::{ptr, thread};

use windows_sys::Win32::Foundation::{
    ERROR_MORE_DATA, HANDLE_FLAG_INHERIT, SetHandleInformation, TRUE, WAIT_OBJECT_0,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_BASIC_PROCESS_ID_LIST,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectBasicAccountingInformation, JobObjectBasicProcessIdList,
    JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject,
    TerminateJobObject,
};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessW,
    DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess, INFINITE,
    InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
    PROCESS_INFORMATION, ResumeThread, STARTF_USESTDHANDLES, STARTUPINFOEXW, STARTUPINFOW, TerminateProcess,
    UpdateProcThreadAttribute, WaitForSingleObject,
};

use crate::stream::{Output, Stdin};
use crate::{Exit, ExitWatch, Launch, SpawnError, Spawned};

/// How the C runtime marks an inherited descriptor that is open and backed by a pipe.
const FOPEN: u8 = 0x01;
const FPIPE: u8 = 0x08;

/// The exit code of every process killed with the tree.
const KILLED: u32 = 1;

pub(crate) fn spawn(launch: &Launch) -> Result<Spawned, SpawnError> {
    let (child_stdin, stdin) = pipe().map_err(SpawnError::Pipes)?;
    let (stdout, child_stdout) = pipe().map_err(SpawnError::Pipes)?;
    let (stderr, child_stderr) = pipe().map_err(SpawnError::Pipes)?;
    let (traces, child_traces) = pipe().map_err(SpawnError::Pipes)?;
    let job = Job::new().map_err(SpawnError::Tree)?;

    let inherited =
        [&child_stdin, &child_stdout, &child_stderr, &child_traces].map(AsRawHandle::as_raw_handle);
    let created = create_suspended(launch, inherited);
    // The parent must not keep the child's ends, or the pipes never report end of file.
    drop((child_stdin, child_stdout, child_stderr, child_traces));
    let Created {
        process,
        main_thread,
        pid,
    } = created?;

    if let Err(error) = job.assign(&process) {
        // SAFETY: `process` is an open handle with every access right; the process never ran.
        unsafe { TerminateProcess(process.as_raw_handle(), KILLED) };
        return Err(SpawnError::Tree(error));
    }
    // The original goes to the thread that waits for the exit; the tree measures through a copy.
    let main = process.try_clone().map_err(SpawnError::Tree)?;
    let tree = ProcessTree { job, main, pid };
    // SAFETY: `main_thread` is the open handle of the process's suspended main thread.
    if unsafe { ResumeThread(main_thread.as_raw_handle()) } == u32::MAX {
        return Err(SpawnError::Tree(io::Error::last_os_error()));
    }

    let stdin = Stdin::write_to(File::from(stdin)).map_err(SpawnError::Threads)?;
    let stdout = Output::read("process-stdout", File::from(stdout)).map_err(SpawnError::Threads)?;
    let stderr = Output::read("process-stderr", File::from(stderr)).map_err(SpawnError::Threads)?;
    let traces = Output::read("process-fd3", File::from(traces)).map_err(SpawnError::Threads)?;
    let (exited, exit) = ExitWatch::new();
    thread::Builder::new()
        .name("process-exit".to_owned())
        .spawn(move || {
            let _ = exited.send(Some(wait_for_exit(&process)));
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

/// FXServer and every process it starts (its console host, its crash dump server, the Node
/// processes of resources): a Job Object that kills them all when its last handle closes, so even
/// a crash of Observer leaves nothing behind.
pub struct ProcessTree {
    job: Job,
    main: OwnedHandle,
    pid: u32,
}

impl ProcessTree {
    /// Kills every process in the tree at once.
    pub fn kill(&self) -> io::Result<()> {
        // SAFETY: the job handle stays open for as long as `self`.
        check(unsafe { TerminateJobObject(self.job.raw(), KILLED) })
    }

    pub fn is_empty(&self) -> io::Result<bool> {
        let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        // SAFETY: the buffer is the structure this information class fills, and outlives the call.
        check(unsafe {
            QueryInformationJobObject(
                self.job.raw(),
                JobObjectBasicAccountingInformation,
                (&raw mut accounting).cast(),
                size_u32::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>(),
                ptr::null_mut(),
            )
        })?;
        Ok(accounting.ActiveProcesses == 0)
    }

    /// The ids of the processes in the tree right now.
    pub fn members(&self) -> io::Result<Vec<u32>> {
        let header = offset_of!(JOBOBJECT_BASIC_PROCESS_ID_LIST, ProcessIdList) / size_of::<usize>();
        let mut capacity = 16;
        loop {
            // The list ends in a variable-length array of process ids, each a `usize`.
            let mut buffer = vec![0usize; header + capacity];
            let bytes = u32::try_from(size_of_val(buffer.as_slice())).map_err(io::Error::other)?;
            let list = buffer.as_mut_ptr().cast::<JOBOBJECT_BASIC_PROCESS_ID_LIST>();
            // SAFETY: the buffer is `bytes` long and aligned for the list, which starts with its
            // fixed header; the job handle stays open for as long as `self`.
            let ok = unsafe {
                QueryInformationJobObject(
                    self.job.raw(),
                    JobObjectBasicProcessIdList,
                    list.cast(),
                    bytes,
                    ptr::null_mut(),
                )
            };
            // SAFETY: the header fields are written on success and on ERROR_MORE_DATA alike.
            let (assigned, listed) = unsafe {
                (
                    (*list).NumberOfAssignedProcesses,
                    (*list).NumberOfProcessIdsInList,
                )
            };
            if ok == 0 {
                let error = io::Error::last_os_error();
                if error.raw_os_error() == Some(ERROR_MORE_DATA.cast_signed()) {
                    capacity = assigned as usize + 16;
                    continue;
                }
                return Err(error);
            }
            return Ok(buffer[header..]
                .iter()
                .take(listed as usize)
                .filter_map(|&id| u32::try_from(id).ok())
                .collect());
        }
    }
}

struct Job(OwnedHandle);

impl Job {
    fn new() -> io::Result<Self> {
        // SAFETY: null attributes and name create an anonymous job whose handle children cannot
        // inherit.
        let handle = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: CreateJobObjectW returned an open handle nothing else owns.
        let job = Self(unsafe { OwnedHandle::from_raw_handle(handle) });

        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: the structure is the one this information class takes, and outlives the call.
        check(unsafe {
            SetInformationJobObject(
                job.raw(),
                JobObjectExtendedLimitInformation,
                (&raw const limits).cast(),
                size_u32::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>(),
            )
        })?;
        Ok(job)
    }

    fn assign(&self, process: &OwnedHandle) -> io::Result<()> {
        // SAFETY: both handles are open for the duration of the call.
        check(unsafe { AssignProcessToJobObject(self.raw(), process.as_raw_handle()) })
    }

    fn raw(&self) -> RawHandle {
        self.0.as_raw_handle()
    }
}

struct Created {
    process: OwnedHandle,
    main_thread: OwnedHandle,
    pid: u32,
}

/// Starts the process suspended, so that it joins the job before it runs a single instruction and
/// anything it starts lands in the job too.
fn create_suspended(launch: &Launch, inherited: [RawHandle; 4]) -> Result<Created, SpawnError> {
    let application = wide(launch.program.as_os_str())?;
    let mut command_line = command_line(&launch.program, &launch.args)?;
    let cwd = wide(launch.cwd.as_os_str())?;
    let start = |source| SpawnError::Start {
        program: launch.program.clone(),
        source,
    };

    for handle in inherited {
        // SAFETY: the handle is one of the child's pipe ends, open until the caller drops it.
        check(unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) })
            .map_err(start)?;
    }
    let attributes = HandleList::new(&inherited).map_err(start)?;
    let descriptors = crt_descriptors(inherited.map(|handle| handle as usize));

    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo.cb = size_u32::<STARTUPINFOEXW>();
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    [
        startup.StartupInfo.hStdInput,
        startup.StartupInfo.hStdOutput,
        startup.StartupInfo.hStdError,
    ] = [inherited[0], inherited[1], inherited[2]];
    startup.StartupInfo.cbReserved2 = u16::try_from(descriptors.len())
        .map_err(io::Error::other)
        .map_err(start)?;
    startup.StartupInfo.lpReserved2 = descriptors.as_ptr().cast_mut();
    startup.lpAttributeList = attributes.as_ptr();

    let flags = CREATE_SUSPENDED
        | CREATE_NO_WINDOW
        | CREATE_NEW_PROCESS_GROUP
        | EXTENDED_STARTUPINFO_PRESENT
        | CREATE_UNICODE_ENVIRONMENT;
    let mut created = PROCESS_INFORMATION::default();
    // SAFETY: every string is NUL-terminated and outlives the call, the command line is a mutable
    // buffer as CreateProcessW requires, `descriptors` and `attributes` (and the handles the list
    // points to) are alive, and a null environment means the parent's.
    let ok = unsafe {
        CreateProcessW(
            application.as_ptr(),
            command_line.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            TRUE,
            flags,
            ptr::null(),
            cwd.as_ptr(),
            (&raw const startup).cast::<STARTUPINFOW>(),
            &raw mut created,
        )
    };
    if ok == 0 {
        return Err(start(io::Error::last_os_error()));
    }
    // SAFETY: CreateProcessW succeeded, so both handles are open and belong to us alone.
    let (process, main_thread) = unsafe {
        (
            OwnedHandle::from_raw_handle(created.hProcess),
            OwnedHandle::from_raw_handle(created.hThread),
        )
    };
    Ok(Created {
        process,
        main_thread,
        pid: created.dwProcessId,
    })
}

/// The block the C runtime reads from `lpReserved2` to rebuild its descriptor table in the child:
/// the descriptor count, one flag byte per descriptor, then one handle per descriptor, unaligned.
/// It is the only way to hand a child a descriptor above 2, and what FXServer's `write(3, ...)`
/// needs.
fn crt_descriptors(handles: [usize; 4]) -> Vec<u8> {
    let count = i32::try_from(handles.len()).unwrap_or(i32::MAX);
    let mut block = Vec::with_capacity(size_of::<i32>() + handles.len() * (1 + size_of::<usize>()));
    block.extend_from_slice(&count.to_ne_bytes());
    block.extend(handles.iter().map(|_| FOPEN | FPIPE));
    for handle in handles {
        block.extend_from_slice(&handle.to_ne_bytes());
    }
    block
}

/// Only the four pipe ends may be inherited, whatever else in Observer is inheritable at the time.
struct HandleList<'a> {
    buffer: Vec<usize>,
    handles: PhantomData<&'a [RawHandle; 4]>,
}

impl<'a> HandleList<'a> {
    fn new(handles: &'a [RawHandle; 4]) -> io::Result<Self> {
        let mut bytes = 0;
        // SAFETY: a null list only asks for the size one attribute needs; the call reports that
        // size and fails by design.
        unsafe { InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &raw mut bytes) };
        let mut buffer = vec![0usize; bytes.div_ceil(size_of::<usize>())];
        // SAFETY: the buffer is at least `bytes` long and pointer-aligned.
        check(unsafe {
            InitializeProcThreadAttributeList(buffer.as_mut_ptr().cast(), 1, 0, &raw mut bytes)
        })?;
        let list = Self {
            buffer,
            handles: PhantomData,
        };
        // SAFETY: the list was initialised for one attribute; it keeps a pointer to `handles`,
        // which the lifetime ties to the list.
        check(unsafe {
            UpdateProcThreadAttribute(
                list.as_ptr(),
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                handles.as_ptr().cast(),
                size_of_val(handles),
                ptr::null_mut(),
                ptr::null(),
            )
        })?;
        Ok(list)
    }

    fn as_ptr(&self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.buffer.as_ptr().cast_mut().cast()
    }
}

impl Drop for HandleList<'_> {
    fn drop(&mut self) {
        // SAFETY: the list was initialised in `new` and is deleted exactly once.
        unsafe { DeleteProcThreadAttributeList(self.as_ptr()) };
    }
}

/// Builds the command line the child's C runtime splits back into the same arguments, with the
/// rules `std` uses: the program is always quoted, an argument only when it holds a space or a
/// tab, or is empty.
fn command_line(program: &Path, args: &[std::ffi::OsString]) -> Result<Vec<u16>, SpawnError> {
    let mut line = Vec::new();
    push_arg(&mut line, program.as_os_str(), true)?;
    for arg in args {
        line.push(u16::from(b' '));
        let quote = arg.is_empty() || arg.encode_wide().any(|unit| unit == SPACE || unit == TAB);
        push_arg(&mut line, arg, quote)?;
    }
    line.push(0);
    Ok(line)
}

const SPACE: u16 = b' ' as u16;
const TAB: u16 = b'\t' as u16;
const QUOTE: u16 = b'"' as u16;
const BACKSLASH: u16 = b'\\' as u16;

fn push_arg(line: &mut Vec<u16>, arg: &OsStr, quote: bool) -> Result<(), SpawnError> {
    if quote {
        line.push(QUOTE);
    }
    let mut backslashes = 0;
    for unit in arg.encode_wide() {
        match unit {
            0 => return Err(SpawnError::Unrepresentable(arg.to_owned())),
            BACKSLASH => backslashes += 1,
            QUOTE => {
                // Backslashes before a quote are halved by the parser, and one more escapes it.
                line.extend(std::iter::repeat_n(BACKSLASH, backslashes + 1));
                backslashes = 0;
            }
            _ => backslashes = 0,
        }
        line.push(unit);
    }
    if quote {
        // Trailing backslashes would otherwise escape the closing quote.
        line.extend(std::iter::repeat_n(BACKSLASH, backslashes));
        line.push(QUOTE);
    }
    Ok(())
}

fn wide(value: &OsStr) -> Result<Vec<u16>, SpawnError> {
    let mut wide: Vec<u16> = value.encode_wide().collect();
    if wide.contains(&0) {
        return Err(SpawnError::Unrepresentable(value.to_owned()));
    }
    wide.push(0);
    Ok(wide)
}

/// An anonymous pipe as (read end, write end). Neither end is inheritable until the spawn marks
/// the child's.
fn pipe() -> io::Result<(OwnedHandle, OwnedHandle)> {
    let (mut read, mut write) = (ptr::null_mut(), ptr::null_mut());
    // SAFETY: both out pointers are valid; null attributes make the handles non-inheritable.
    check(unsafe { CreatePipe(&raw mut read, &raw mut write, ptr::null(), 0) })?;
    // SAFETY: CreatePipe succeeded, so both handles are open and belong to us alone.
    Ok(unsafe {
        (
            OwnedHandle::from_raw_handle(read),
            OwnedHandle::from_raw_handle(write),
        )
    })
}

fn wait_for_exit(process: &OwnedHandle) -> Exit {
    // SAFETY: the handle stays open for the whole call.
    if unsafe { WaitForSingleObject(process.as_raw_handle(), INFINITE) } != WAIT_OBJECT_0 {
        return Exit::Unknown;
    }
    let mut code = 0;
    // SAFETY: same open handle, and `code` is a valid out pointer.
    if unsafe { GetExitCodeProcess(process.as_raw_handle(), &raw mut code) } == 0 {
        return Exit::Unknown;
    }
    Exit::Code(code)
}

fn check(result: windows_sys::core::BOOL) -> io::Result<()> {
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Win32 takes structure sizes as `u32`; the structures passed here are a few hundred bytes.
#[allow(clippy::cast_possible_truncation)]
const fn size_u32<T>() -> u32 {
    size_of::<T>() as u32
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::*;

    fn line(program: &str, args: &[&str]) -> String {
        let args: Vec<OsString> = args.iter().map(OsString::from).collect();
        let mut line = command_line(Path::new(program), &args).unwrap();
        assert_eq!(line.pop(), Some(0));
        String::from_utf16(&line).unwrap()
    }

    #[test]
    fn the_crt_block_lists_four_pipes_then_their_handles_unaligned() {
        let block = crt_descriptors([0x11, 0x22, 0x33, 0x44]);

        assert_eq!(block.len(), 40);
        assert_eq!(block[..4], 4i32.to_ne_bytes());
        assert_eq!(block[4..8], [0x09; 4]);
        for (index, handle) in [0x11usize, 0x22, 0x33, 0x44].into_iter().enumerate() {
            let start = 8 + index * 8;
            assert_eq!(block[start..start + 8], handle.to_ne_bytes());
        }
    }

    #[test]
    fn only_the_program_and_arguments_with_blanks_are_quoted() {
        assert_eq!(
            line(r"C:\fx\FXServer.exe", &["+exec", "server.cfg"]),
            r#""C:\fx\FXServer.exe" +exec server.cfg"#
        );
        assert_eq!(line("a.exe", &[""]), r#""a.exe" """#);
        assert_eq!(
            line("a.exe", &["two words", "tab\there"]),
            "\"a.exe\" \"two words\" \"tab\there\""
        );
    }

    #[test]
    fn quotes_and_trailing_backslashes_survive_the_round_trip() {
        assert_eq!(line("a.exe", &[r#"say "hi""#]), r#""a.exe" "say \"hi\"""#);
        assert_eq!(line("a.exe", &[r#"a\"b"#]), r#""a.exe" a\\\"b"#);
        assert_eq!(line("a.exe", &[r"C:\my data\"]), r#""a.exe" "C:\my data\\""#);
        assert_eq!(line("a.exe", &[r"C:\data\"]), r#""a.exe" C:\data\"#);
    }

    fn cmd(script: &str) -> Launch {
        Launch {
            program: std::env::var_os("ComSpec")
                .map_or_else(|| r"C:\Windows\System32\cmd.exe".into(), Into::into),
            args: vec!["/d".into(), "/c".into(), script.into()],
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
        let mut spawned = spawn(&cmd("echo out& echo err 1>&2& echo trace 1>&3& exit 7")).unwrap();

        assert_eq!(read_all(&mut spawned.stdout).await, b"out\r\n");
        assert_eq!(read_all(&mut spawned.stderr).await, b"err \r\n");
        assert_eq!(read_all(&mut spawned.traces).await, b"trace \r\n");
        assert_eq!(spawned.exit.wait().await, Exit::Code(7));
    }

    #[tokio::test]
    async fn killing_the_tree_ends_every_process_in_it() {
        let mut spawned = spawn(&cmd("echo ready& ping -n 30 127.0.0.1 >nul")).unwrap();
        assert!(spawned.stdout.next().await.is_some());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while spawned.tree.members().unwrap().len() < 2 && std::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert_eq!(
            spawned.tree.members().unwrap().len(),
            2,
            "cmd and ping should both be in the job"
        );

        spawned.tree.kill().unwrap();

        assert_eq!(spawned.exit.wait().await, Exit::Code(KILLED));
        assert!(
            spawned
                .tree
                .wait_empty(std::time::Duration::from_secs(5))
                .await
                .unwrap()
        );
        assert_eq!(spawned.tree.members().unwrap(), Vec::<u32>::new());
    }

    #[tokio::test]
    async fn stdin_reaches_the_child() {
        let mut spawned = spawn(&cmd("set /p line=& call echo got %line%")).unwrap();

        spawned.stdin.write(b"quit\r\n".to_vec()).await.unwrap();

        assert_eq!(read_all(&mut spawned.stdout).await, b"got quit\r\n");
    }

    #[test]
    fn a_missing_program_is_reported_as_a_start_error() {
        let launch = Launch {
            program: r"C:\nonexistent\FXServer.exe".into(),
            args: Vec::new(),
            cwd: std::env::temp_dir(),
        };

        assert!(matches!(spawn(&launch), Err(SpawnError::Start { .. })));
    }

    #[test]
    fn a_nul_character_is_refused() {
        let error = command_line(Path::new("a.exe"), &[OsString::from("a\0b")]).unwrap_err();
        assert!(matches!(error, SpawnError::Unrepresentable(_)));
    }
}
