//! The minidump FXServer's dump server writes of a crash, shaped like one of a Lua resource's
//! access violation: a write at a null page in `citizen-scripting-lua.dll`, on svMain.

use minidump_synth::{
    DumpString, Exception, Memory, Module, SynthMinidump, SystemInfo, Thread, ThreadName, amd64_context,
};
use test_assembler::{Endian, Section};

pub const MODULE: &str = r"C:\fx\citizen-scripting-lua.dll";
pub const MODULE_BASE: u64 = 0x7ffd_3a40_0000;
pub const CRASH_OFFSET: u64 = 0x1a2b3;
const THREAD: u32 = 812;
const STACK: u64 = 0x0000_00a2_4f7f_e000;
/// `PROCESSOR_ARCHITECTURE_AMD64` and `VER_PLATFORM_WIN32_NT`.
const AMD64: u16 = 9;
const WINDOWS_NT: u32 = 2;
const ACCESS_VIOLATION: u32 = 0xC000_0005;

pub fn crash() -> Vec<u8> {
    let endian = Endian::Little;
    let module_name = DumpString::new(MODULE, endian);
    let module = Module::new(endian, MODULE_BASE, 0x20_0000, &module_name, 0x6720_1a2b, 0, None);
    let thread_name = DumpString::new(super::SV_MAIN, endian);
    let crashed_at = MODULE_BASE + CRASH_OFFSET;
    // A return address into the module, then words that point nowhere.
    let stack = Memory::with_section(
        Section::with_endian(endian)
            .D64(MODULE_BASE + 0x4410)
            .D64(0x10)
            .append_repeated(0, 0x400),
        STACK,
    );
    let context = amd64_context(endian, crashed_at, STACK);
    let thread = Thread::new(endian, THREAD, &stack, &context);
    let mut exception = Exception::new(endian);
    exception.thread_id = THREAD;
    exception.exception_record.exception_code = ACCESS_VIOLATION;
    exception.exception_record.exception_address = crashed_at;
    exception.exception_record.number_parameters = 2;
    // A write, at address 0x10.
    exception.exception_record.exception_information[0] = 1;
    exception.exception_record.exception_information[1] = 0x10;
    let system_info = SystemInfo::new(endian)
        .set_processor_architecture(AMD64)
        .set_platform_id(WINDOWS_NT);
    SynthMinidump::with_endian(endian)
        .add_system_info(system_info)
        .add_module(module)
        .add(module_name)
        .add_thread(thread)
        .add(context)
        .add_memory(stack)
        .add_thread_name(ThreadName::new(endian, THREAD, Some(&thread_name)))
        .add(thread_name)
        .add_exception(exception)
        .finish()
        .unwrap_or_default()
}
