//! What a crash or hang dump says without symbols: the exception, the thread, the module and
//! offset it was in, and an approximate stack from scanning the thread's stack memory. A dump may
//! have been planted by a resource, which runs as the same user: every read is bounded and nothing
//! read from it reaches a shell or a path.

use std::io::Read as _;
use std::path::Path;

use minidump::{
    Minidump, MinidumpException, MinidumpMemoryList, MinidumpMiscInfo, MinidumpModuleList,
    MinidumpSystemInfo, MinidumpThread, MinidumpThreadList, MinidumpThreadNames, Module, UnifiedMemoryList,
};
use observer_domain::diagnostics::{
    AnalysedThread, CrashAnalysis, CrashException, Frame, ThreadRun, ThreadState,
};

use super::report::HangReport;

const MAX_FRAMES: usize = 16;
const MAX_SCAN_BYTES: usize = 64 * 1024;
const MAX_SIGNATURE: usize = 256;
/// A hang report is about 10 KB: past this a file is not one of Observer's.
const MAX_REPORT_BYTES: u64 = 1024 * 1024;
/// FXServer's main thread, whose name holds this.
const SV_MAIN: &str = "svMain";
/// svMain at this share of a core or more was busy on the processor rather than waiting.
const BUSY_PERCENT: f64 = 90.0;
/// Where any thread waits or allocates on Windows: a hang in them says nothing of FXServer.
const SYSTEM_MODULES: [&str; 8] = [
    "ntdll.dll",
    "kernelbase.dll",
    "kernel32.dll",
    "win32u.dll",
    "user32.dll",
    "ucrtbase.dll",
    "msvcp140.dll",
    "vcruntime140.dll",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AnalysisError {
    #[error("the dump cannot be read")]
    Unreadable,
    #[error("the file is not a minidump")]
    NotADump,
}

/// An analysis and the signature that groups crashes alike.
#[derive(Debug, Clone, PartialEq)]
pub struct Analysis {
    pub analysis: CrashAnalysis,
    pub signature: String,
}

/// Reads a minidump. Blocks: run it on a blocking thread.
pub fn analyse(path: &Path) -> Result<Analysis, AnalysisError> {
    let dump = Minidump::read_path(path).map_err(|error| match error {
        minidump::Error::MissingHeader
        | minidump::Error::HeaderMismatch
        | minidump::Error::VersionMismatch
        | minidump::Error::MissingDirectory => AnalysisError::NotADump,
        _ => AnalysisError::Unreadable,
    })?;
    let system = dump
        .get_stream::<MinidumpSystemInfo>()
        .map_err(|_| AnalysisError::Unreadable)?;
    let threads = dump
        .get_stream::<MinidumpThreadList<'_>>()
        .map_err(|_| AnalysisError::Unreadable)?;
    let modules = dump.get_stream::<MinidumpModuleList>().unwrap_or_default();
    let names = dump.get_stream::<MinidumpThreadNames>().ok();
    let misc = dump.get_stream::<MinidumpMiscInfo>().ok();
    let memory = dump
        .get_stream::<MinidumpMemoryList<'_>>()
        .map(UnifiedMemoryList::Memory)
        .unwrap_or_default();
    let exception = dump.get_stream::<MinidumpException<'_>>().ok();
    let name_of = |id: u32| {
        names
            .as_ref()
            .and_then(|names| names.get_name(id))
            .map(std::borrow::Cow::into_owned)
    };

    let thread: Option<&MinidumpThread<'_>> = match &exception {
        Some(exception) => threads.get_thread(exception.thread_id),
        None => threads
            .threads
            .iter()
            .find(|thread| name_of(thread.raw.thread_id).is_some_and(|name| name.contains(SV_MAIN))),
    };
    let crash = exception.as_ref().map(|exception| {
        let reason = exception.get_crash_reason(system.os, system.cpu);
        CrashException {
            code: format!("0x{:08X}", exception.raw.exception_record.exception_code),
            name: reason.to_string(),
            address: Some(format!(
                "{:#x}",
                exception.get_crash_address(system.os, system.cpu)
            )),
        }
    });

    let mut frames = Vec::new();
    if let Some(thread) = thread {
        let context = exception
            .as_ref()
            .and_then(|exception| exception.context(&system, misc.as_ref()))
            .or_else(|| thread.context(&system, misc.as_ref()));
        if let Some(context) = context {
            frames.push(frame(&modules, context.get_instruction_pointer(), false));
            if let Some(stack) = thread.stack_memory(&memory) {
                scan(
                    stack.bytes(),
                    stack.base_address(),
                    context.get_stack_pointer(),
                    &modules,
                    &mut frames,
                );
            }
        }
    }
    let analysed = thread.map(|thread| AnalysedThread {
        id: thread.raw.thread_id,
        name: name_of(thread.raw.thread_id),
    });
    let module = module(crash.is_some(), &frames);
    let signature = signature(crash.as_ref(), analysed.as_ref(), &frames, module.as_deref());
    Ok(Analysis {
        analysis: CrashAnalysis {
            exception: crash,
            thread: analysed,
            frames,
            module,
            threads: Vec::new(),
            syscalls_readable: None,
        },
        signature,
    })
}

/// The module a crash faulted in; for a hang, the first outside the system libraries, or the
/// first at all when every frame is in them.
fn module(crashed: bool, frames: &[Frame]) -> Option<String> {
    let mut modules = frames.iter().filter_map(|frame| frame.module.as_deref());
    let first = modules.clone().next();
    let own = modules.find(|module| {
        !SYSTEM_MODULES
            .iter()
            .any(|system| system.eq_ignore_ascii_case(module))
    });
    if crashed { first } else { own.or(first) }.map(str::to_owned)
}

/// Reads one of Observer's Linux hang reports: svMain and the five busiest threads. A file over
/// 1 MiB is unreadable, read no further: any file named like a report reaches here.
pub fn analyse_report(path: &Path) -> Result<Analysis, AnalysisError> {
    let mut text = Vec::new();
    std::fs::File::open(path)
        .and_then(|file| file.take(MAX_REPORT_BYTES + 1).read_to_end(&mut text))
        .map_err(|_| AnalysisError::Unreadable)?;
    if text.len() as u64 > MAX_REPORT_BYTES {
        return Err(AnalysisError::Unreadable);
    }
    let report: HangReport = serde_json::from_slice(&text).map_err(|_| AnalysisError::Unreadable)?;
    let (main, mut others): (Vec<ThreadState>, Vec<ThreadState>) = report
        .threads
        .into_iter()
        .partition(|thread| thread.name.contains(SV_MAIN));
    others.sort_by(|a, b| b.cpu.unwrap_or(0.0).total_cmp(&a.cpu.unwrap_or(0.0)));
    let main = main.into_iter().next();
    let signature = match &main {
        Some(main) => {
            let busy = main.state == ThreadRun::Running && main.cpu.is_some_and(|cpu| cpu >= BUSY_PERCENT);
            match (&main.syscall, busy) {
                (_, true) => format!("hang {} busy", main.name),
                (Some(syscall), false) => format!("hang {} {syscall}", main.name),
                (None, false) => format!("hang {}", main.name),
            }
        }
        None => "hang".to_owned(),
    };
    let thread = main.as_ref().map(|main| AnalysedThread {
        id: main.tid,
        name: Some(main.name.clone()),
    });
    Ok(Analysis {
        analysis: CrashAnalysis {
            exception: None,
            thread,
            frames: Vec::new(),
            module: None,
            threads: main.into_iter().chain(others.into_iter().take(5)).collect(),
            syscalls_readable: Some(report.syscalls_readable),
        },
        signature: bounded(signature),
    })
}

/// Return addresses found on the stack: each aligned word from the stack pointer upward that falls
/// inside a module, within 64 KiB and up to 16 frames in all.
fn scan(stack: &[u8], base: u64, pointer: u64, modules: &MinidumpModuleList, frames: &mut Vec<Frame>) {
    let start = pointer.saturating_sub(base);
    let Ok(start) = usize::try_from(start) else {
        return;
    };
    let start = if start < stack.len() {
        start.next_multiple_of(8)
    } else {
        0
    };
    let words = stack.get(start..).unwrap_or_default();
    let words = &words[..words.len().min(MAX_SCAN_BYTES)];
    for word in words.as_chunks::<8>().0 {
        if frames.len() >= MAX_FRAMES {
            return;
        }
        let address = u64::from_le_bytes(*word);
        if modules.module_at_address(address).is_some() {
            frames.push(frame(modules, address, true));
        }
    }
}

fn frame(modules: &MinidumpModuleList, address: u64, scanned: bool) -> Frame {
    match modules.module_at_address(address) {
        Some(module) => Frame {
            module: Some(file_name(&module.code_file())),
            offset: format!("{:#x}", address - module.base_address()),
            scanned,
        },
        None => Frame {
            module: None,
            offset: format!("{address:#x}"),
            scanned,
        },
    }
}

/// The name after the last separator of either system.
fn file_name(path: &str) -> String {
    path.rsplit(['\\', '/']).next().unwrap_or(path).to_owned()
}

/// A crash by its code and the first frame inside a module; a hang or a live dump by its thread and
/// the module it is placed in, as its sentence names them, since a busy loop's offset moves every
/// time; the code or the thread alone when no frame is inside a module.
fn signature(
    exception: Option<&CrashException>,
    thread: Option<&AnalysedThread>,
    frames: &[Frame],
    module: Option<&str>,
) -> String {
    let thread = thread.and_then(|thread| thread.name.clone());
    let first = frames
        .iter()
        .find_map(|frame| Some((frame.module.as_ref()?, &frame.offset)));
    let signature = match (exception, first) {
        (Some(exception), Some((module, offset))) => format!("{} {module}+{offset}", exception.code),
        (Some(exception), None) => match thread {
            Some(thread) => format!("{} {thread}", exception.code),
            None => exception.code.clone(),
        },
        (None, _) => {
            let mut signature = String::from("hang");
            if let Some(thread) = thread {
                signature.push(' ');
                signature.push_str(&thread);
            }
            if let Some(module) = module {
                signature.push(' ');
                signature.push_str(module);
            }
            signature
        }
    };
    bounded(signature)
}

fn bounded(mut text: String) -> String {
    if text.len() > MAX_SIGNATURE {
        let mut end = MAX_SIGNATURE;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    text
}

#[cfg(test)]
mod tests {
    use minidump_synth::{
        DumpString, Exception, Memory, Module as SynthModule, SynthMinidump, SystemInfo, Thread, ThreadName,
        amd64_context,
    };
    use test_assembler::{Endian, Section};

    use super::*;

    const LUA: u64 = 0x7ffd_3a40_0000;
    const NTDLL: u64 = 0x7ffd_5000_0000;
    const STACK: u64 = 0x00a2_4f7f_e000;
    const SV_MAIN_ID: u32 = 812;

    struct Fixture {
        rip: u64,
        stack: Section,
        exception: Option<u32>,
        names: bool,
    }

    impl Fixture {
        fn crash_in_lua() -> Self {
            Self {
                rip: LUA + 0x1a2b3,
                stack: Section::with_endian(Endian::Little).append_repeated(0, 0x100),
                exception: Some(0xC000_0005),
                names: true,
            }
        }

        fn write(self) -> (tempfile::TempDir, std::path::PathBuf) {
            let endian = Endian::Little;
            let lua = DumpString::new(r"C:\fx\citizen-scripting-lua.dll", endian);
            let ntdll = DumpString::new(r"C:\Windows\System32\ntdll.dll", endian);
            let sv_main = DumpString::new("UV loop: svMain", endian);
            let other = DumpString::new("svNetwork", endian);
            let stack = Memory::with_section(self.stack, STACK);
            let idle = Memory::with_section(
                Section::with_endian(endian).append_repeated(0, 0x40),
                STACK + 0x10_0000,
            );
            let context = amd64_context(endian, self.rip, STACK);
            let idle_context = amd64_context(endian, NTDLL + 0x9f0, STACK + 0x10_0000);
            let mut dump = SynthMinidump::with_endian(endian)
                .add_system_info(
                    SystemInfo::new(endian)
                        .set_processor_architecture(9)
                        .set_platform_id(2),
                )
                .add_module(SynthModule::new(endian, LUA, 0x20_0000, &lua, 0, 0, None))
                .add(lua)
                .add_module(SynthModule::new(endian, NTDLL, 0x1f_0000, &ntdll, 0, 0, None))
                .add(ntdll)
                .add_thread(Thread::new(endian, 700, &idle, &idle_context))
                .add(idle_context)
                .add_memory(idle)
                .add_thread(Thread::new(endian, SV_MAIN_ID, &stack, &context))
                .add(context)
                .add_memory(stack);
            if self.names {
                dump = dump
                    .add_thread_name(ThreadName::new(endian, 700, Some(&other)))
                    .add_thread_name(ThreadName::new(endian, SV_MAIN_ID, Some(&sv_main)))
                    .add(other)
                    .add(sv_main);
            }
            if let Some(code) = self.exception {
                let mut exception = Exception::new(endian);
                exception.thread_id = SV_MAIN_ID;
                exception.exception_record.exception_code = code;
                exception.exception_record.exception_address = self.rip;
                exception.exception_record.number_parameters = 2;
                exception.exception_record.exception_information[0] = 1;
                exception.exception_record.exception_information[1] = 0x10;
                dump = dump.add_exception(exception);
            }
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("crash.dmp");
            std::fs::write(&path, dump.finish().unwrap()).unwrap();
            (dir, path)
        }
    }

    #[test]
    fn an_access_violation_in_a_scripting_module_reads_with_its_code_module_offset_and_thread() {
        let (_dir, path) = Fixture::crash_in_lua().write();

        let Analysis { analysis, signature } = analyse(&path).unwrap();

        let exception = analysis.exception.unwrap();
        assert_eq!(exception.code, "0xC0000005");
        assert_eq!(exception.name, "EXCEPTION_ACCESS_VIOLATION_WRITE");
        assert_eq!(exception.address.as_deref(), Some("0x10"));
        assert_eq!(
            analysis.thread,
            Some(AnalysedThread {
                id: SV_MAIN_ID,
                name: Some("UV loop: svMain".to_owned())
            })
        );
        assert_eq!(
            analysis.frames[0],
            Frame {
                module: Some("citizen-scripting-lua.dll".to_owned()),
                offset: "0x1a2b3".to_owned(),
                scanned: false,
            }
        );
        assert_eq!(signature, "0xC0000005 citizen-scripting-lua.dll+0x1a2b3");
    }

    #[test]
    fn a_live_dump_without_an_exception_finds_svmain_by_its_name() {
        let (_dir, path) = Fixture {
            exception: None,
            ..Fixture::crash_in_lua()
        }
        .write();

        let Analysis { analysis, signature } = analyse(&path).unwrap();

        assert_eq!(analysis.exception, None);
        assert_eq!(analysis.thread.unwrap().id, SV_MAIN_ID);
        assert_eq!(signature, "hang UV loop: svMain citizen-scripting-lua.dll");
    }

    #[test]
    fn a_hang_waiting_in_the_system_is_placed_and_grouped_in_the_first_module_of_fxserver_s_own() {
        let stack = Section::with_endian(Endian::Little)
            .D64(NTDLL + 0x4410)
            .D64(LUA + 0x8000);
        let (_dir, path) = Fixture {
            rip: NTDLL + 0x16_1dd4,
            stack,
            exception: None,
            ..Fixture::crash_in_lua()
        }
        .write();

        let Analysis { analysis, signature } = analyse(&path).unwrap();

        assert_eq!(analysis.frames[0].module.as_deref(), Some("ntdll.dll"));
        assert_eq!(analysis.module.as_deref(), Some("citizen-scripting-lua.dll"));
        assert_eq!(
            signature, "hang UV loop: svMain citizen-scripting-lua.dll",
            "the busy hang in Lua's group"
        );
    }

    #[test]
    fn a_hang_waiting_in_the_system_with_nothing_of_fxserver_s_on_the_stack_is_grouped_there() {
        let (_dir, path) = Fixture {
            rip: NTDLL + 0x16_1dd4,
            stack: Section::with_endian(Endian::Little).D64(NTDLL + 0x4410),
            exception: None,
            ..Fixture::crash_in_lua()
        }
        .write();

        let Analysis { analysis, signature } = analyse(&path).unwrap();

        assert_eq!(analysis.module.as_deref(), Some("ntdll.dll"));
        assert_eq!(signature, "hang UV loop: svMain ntdll.dll");
    }

    #[test]
    fn a_crash_is_placed_where_it_faulted_even_in_the_system() {
        let stack = Section::with_endian(Endian::Little).D64(LUA + 0x8000);
        let (_dir, path) = Fixture {
            rip: NTDLL + 0x9f0,
            stack,
            ..Fixture::crash_in_lua()
        }
        .write();

        assert_eq!(
            analyse(&path).unwrap().analysis.module.as_deref(),
            Some("ntdll.dll")
        );
    }

    #[test]
    fn a_crash_outside_every_module_is_signed_by_its_code_and_thread() {
        let (_dir, path) = Fixture {
            rip: 0x1234,
            ..Fixture::crash_in_lua()
        }
        .write();

        let Analysis { analysis, signature } = analyse(&path).unwrap();

        assert_eq!(
            analysis.frames,
            [Frame {
                module: None,
                offset: "0x1234".to_owned(),
                scanned: false
            }]
        );
        assert_eq!(signature, "0xC0000005 UV loop: svMain");
    }

    #[test]
    fn the_stack_scan_keeps_return_addresses_inside_modules_within_its_limits() {
        let stack = Section::with_endian(Endian::Little)
            .D64(0xdead_beef)
            .D64(NTDLL + 0x4410)
            .D64(42)
            .D64(LUA + 0x8000);
        let (_dir, path) = Fixture {
            stack,
            ..Fixture::crash_in_lua()
        }
        .write();

        let frames = analyse(&path).unwrap().analysis.frames;

        let scanned: Vec<(Option<&str>, &str)> = frames[1..]
            .iter()
            .map(|frame| (frame.module.as_deref(), frame.offset.as_str()))
            .collect();
        assert_eq!(
            scanned,
            [
                (Some("ntdll.dll"), "0x4410"),
                (Some("citizen-scripting-lua.dll"), "0x8000")
            ]
        );
        assert!(frames[1..].iter().all(|frame| frame.scanned));

        let mut long = Section::with_endian(Endian::Little);
        for index in 0..40 {
            long = long.D64(LUA + 0x100 * index);
        }
        let (_dir, path) = Fixture {
            stack: long,
            ..Fixture::crash_in_lua()
        }
        .write();
        assert_eq!(analyse(&path).unwrap().analysis.frames.len(), MAX_FRAMES);

        // A return address past the first 64 KiB is not read.
        let far = Section::with_endian(Endian::Little)
            .append_repeated(0, MAX_SCAN_BYTES)
            .D64(LUA + 0x10);
        let (_dir, path) = Fixture {
            stack: far,
            ..Fixture::crash_in_lua()
        }
        .write();
        assert_eq!(analyse(&path).unwrap().analysis.frames.len(), 1);
    }

    #[test]
    fn without_thread_names_a_crash_still_reads() {
        let (_dir, path) = Fixture {
            names: false,
            ..Fixture::crash_in_lua()
        }
        .write();

        let Analysis { analysis, signature } = analyse(&path).unwrap();

        assert_eq!(analysis.thread.unwrap().name, None);
        assert_eq!(signature, "0xC0000005 citizen-scripting-lua.dll+0x1a2b3");
    }

    #[test]
    fn a_file_cut_before_its_directory_or_of_zeros_is_not_a_dump_and_one_without_threads_is_unreadable() {
        let (dir, path) = Fixture::crash_in_lua().write();
        let bytes = std::fs::read(&path).unwrap();
        let truncated = dir.path().join("truncated.dmp");
        std::fs::write(&truncated, &bytes[..bytes.len() / 3]).unwrap();
        let zeros = dir.path().join("zeros.dmp");
        std::fs::write(&zeros, vec![0; 4096]).unwrap();
        let empty = dir.path().join("empty.dmp");
        let system = SystemInfo::new(Endian::Little).set_processor_architecture(9);
        let dump = SynthMinidump::with_endian(Endian::Little).add_system_info(system);
        std::fs::write(&empty, dump.finish().unwrap()).unwrap();

        assert_eq!(analyse(&truncated), Err(AnalysisError::NotADump));
        assert_eq!(analyse(&zeros), Err(AnalysisError::NotADump));
        assert_eq!(analyse(&empty), Err(AnalysisError::Unreadable));
        assert_eq!(
            analyse(&dir.path().join("missing.dmp")),
            Err(AnalysisError::Unreadable)
        );
    }

    #[test]
    fn a_signature_never_grows_past_256_bytes() {
        let thread = AnalysedThread {
            id: 1,
            name: Some("é".repeat(200)),
        };

        let signature = signature(None, Some(&thread), &[], None);

        assert!(signature.len() <= MAX_SIGNATURE);
        assert!(signature.starts_with("hang é"));
    }

    fn thread(tid: u32, name: &str, state: ThreadRun, cpu: f64, syscall: Option<&str>) -> ThreadState {
        ThreadState {
            tid,
            name: name.to_owned(),
            state,
            cpu: Some(cpu),
            syscall: syscall.map(str::to_owned),
            syscall_group: None,
        }
    }

    fn report(threads: Vec<ThreadState>) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hang-3-20261029T101500Z.json");
        let report = HangReport {
            version: super::super::report::VERSION,
            taken_at: "2026-10-29T10:15:00Z".parse().unwrap(),
            pid: 4242,
            syscalls_readable: true,
            threads,
        };
        std::fs::write(&path, serde_json::to_vec(&report).unwrap()).unwrap();
        (dir, path)
    }

    #[test]
    fn a_hang_report_names_svmain_first_then_the_five_busiest_threads() {
        let mut threads: Vec<ThreadState> = (0..8)
            .map(|index| {
                thread(
                    100 + index,
                    &format!("svNet{index}"),
                    ThreadRun::Sleeping,
                    f64::from(index),
                    None,
                )
            })
            .collect();
        threads.push(thread(4242, "UV loop: svMain", ThreadRun::Running, 97.5, None));
        let (_dir, path) = report(threads);

        let Analysis { analysis, signature } = analyse_report(&path).unwrap();

        let names: Vec<&str> = analysis
            .threads
            .iter()
            .map(|thread| thread.name.as_str())
            .collect();
        assert_eq!(
            names,
            [
                "UV loop: svMain",
                "svNet7",
                "svNet6",
                "svNet5",
                "svNet4",
                "svNet3"
            ]
        );
        assert_eq!(analysis.thread.unwrap().id, 4242);
        assert_eq!(analysis.syscalls_readable, Some(true));
        assert_eq!(signature, "hang UV loop: svMain busy");
    }

    #[test]
    fn a_report_past_one_mebibyte_is_not_read() {
        let (_dir, path) = report(vec![thread(
            4242,
            "UV loop: svMain",
            ThreadRun::Running,
            99.0,
            None,
        )]);
        let mut padded = std::fs::read(&path).unwrap();
        padded.resize(2 * 1024 * 1024, b' ');
        std::fs::write(&path, padded).unwrap();

        assert_eq!(analyse_report(&path), Err(AnalysisError::Unreadable));
    }

    #[test]
    fn a_waiting_svmain_is_signed_by_its_system_call() {
        let (_dir, path) = report(vec![thread(
            4242,
            "UV loop: svMain",
            ThreadRun::Sleeping,
            0.0,
            Some("futex"),
        )]);

        assert_eq!(
            analyse_report(&path).unwrap().signature,
            "hang UV loop: svMain futex"
        );
        assert_eq!(
            analyse_report(&path.with_extension("missing")),
            Err(AnalysisError::Unreadable)
        );
    }
}
