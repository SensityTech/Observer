//! What Observer sees of FXServer from outside: the processes its tree starts, where it connects,
//! and what its crash and hang dumps say. Addresses, codes and offsets travel as hex strings
//! (`0x1a2b3`, `0xC0000005`), since a 64-bit value does not fit a JavaScript number.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::server::HangCause;

/// What a process FXServer's tree started is. Stored by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ProcessKind {
    /// FXServer started again as its crash dump server.
    DumpServer,
    /// The hidden console host Windows gives FXServer.
    ConsoleHost,
    /// FXServer started again as the Node.js process of a resource.
    Node,
    /// A shell, a script host or a network tool: what a backdoor runs.
    Shell,
    /// Anything else, which FXServer does not start by itself.
    Other,
}

impl ProcessKind {
    pub const ALL: [Self; 5] = [
        Self::DumpServer,
        Self::ConsoleHost,
        Self::Node,
        Self::Shell,
        Self::Other,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DumpServer => "dumpServer",
            Self::ConsoleHost => "consoleHost",
            Self::Node => "node",
            Self::Shell => "shell",
            Self::Other => "other",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == name)
    }

    /// Shells and programs FXServer never starts by itself go to the audit log.
    pub const fn is_unexpected(self) -> bool {
        matches!(self, Self::Shell | Self::Other)
    }
}

/// A process FXServer's tree started, once per run for each program and command line: a repeat
/// counts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TreeProcess {
    pub id: i64,
    /// When the run it belongs to started.
    pub run_started: Timestamp,
    pub generation: u32,
    /// The latest start's.
    pub pid: u32,
    pub parent: Option<ParentProcess>,
    /// The full path, or `None` when the process ended before it could be read.
    pub image: Option<String>,
    pub name: Option<String>,
    /// Secrets masked, control characters removed, at most 4 KiB.
    pub command_line: Option<String>,
    pub truncated: bool,
    pub kind: ProcessKind,
    pub started_at: Timestamp,
    pub last_started_at: Timestamp,
    /// When the latest start ended; `None` while it runs.
    pub ended_at: Option<Timestamp>,
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ParentProcess {
    pub pid: u32,
    /// The file name only.
    pub name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Protocol {
    Tcp,
    Udp,
}

impl Protocol {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Tcp => "tcp",
            Self::Udp => "udp",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        [Self::Tcp, Self::Udp]
            .into_iter()
            .find(|protocol| protocol.as_str() == name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Scope {
    Loopback,
    /// A private, shared or link-local network.
    Private,
    Public,
}

/// What usually answers on a well-known port. A guess from the port alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Service {
    Mysql,
    Postgresql,
    Redis,
    Mongodb,
    Https,
    Http,
    Dns,
}

/// Somewhere a process of FXServer's tree connects to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Destination {
    pub protocol: Protocol,
    pub address: String,
    pub port: u16,
    /// The file name of the process holding the connections.
    pub process: String,
    pub scope: Scope,
    pub service: Option<Service>,
    /// Connections open at the last look.
    pub open: u32,
    /// The most seen open at once.
    pub peak: u32,
    pub first_seen: Timestamp,
    pub last_seen: Timestamp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ListenerRole {
    /// The endpoint players and Observer's probes reach.
    Game,
}

/// An endpoint a process of FXServer's tree listens on, or an unconnected UDP socket.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Listener {
    pub protocol: Protocol,
    pub address: String,
    pub port: u16,
    pub process: String,
    pub role: Option<ListenerRole>,
    /// Connections accepted and still open.
    pub inbound: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum CrashKind {
    /// FXServer ended without being asked to.
    Crash,
    /// FXServer stopped responding and Observer killed it.
    Hang,
    /// Dumps found while no crash or hang was seen: an earlier Observer's, or a stall the server
    /// recovered from.
    Found,
}

impl CrashKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Crash => "crash",
            Self::Hang => "hang",
            Self::Found => "found",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        [Self::Crash, Self::Hang, Self::Found]
            .into_iter()
            .find(|kind| kind.as_str() == name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum AnalysisState {
    Pending,
    Done,
    /// Nothing to read: Linux leaves no dump, and a Windows crash may not either.
    NoDump,
    Unreadable,
}

impl AnalysisState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Done => "done",
            Self::NoDump => "noDump",
            Self::Unreadable => "unreadable",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        [Self::Pending, Self::Done, Self::NoDump, Self::Unreadable]
            .into_iter()
            .find(|state| state.as_str() == name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum CrashFileKind {
    /// FXServer's minidump of a crash.
    Minidump,
    /// FXServer's dump of its whole memory: every secret it holds is in it.
    FullDump,
    /// FXServer's own dump of a stuck main thread.
    LiveDump,
    /// Observer's stack dump of a hung server, before it killed it.
    HangDump,
    /// Observer's report of a hung server's threads, on Linux.
    HangReport,
    Other,
}

impl CrashFileKind {
    /// What a file of Observer's crash folder is, by its name.
    pub fn of(name: &str) -> Self {
        let lower = name.to_ascii_lowercase();
        let (stem, extension) = lower.rsplit_once('.').unwrap_or((&lower, ""));
        match extension {
            "dmp" if stem.starts_with("hang-") => Self::HangDump,
            "json" if stem.starts_with("hang-") => Self::HangReport,
            "dmp" if stem.starts_with("livedump") => Self::LiveDump,
            // A second dump of the same name gets a number: `<id>-full-1.dmp`.
            "dmp" if stem.ends_with("-full") || stem.contains("-full-") => Self::FullDump,
            "dmp" => Self::Minidump,
            _ => Self::Other,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CrashFile {
    pub name: String,
    pub bytes: u64,
    pub kind: CrashFileKind,
    /// `false` once the dump policy or the owner removed it.
    pub kept: bool,
}

/// A crash, a hang or dumps found, with what their analysis read.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Crash {
    pub id: i64,
    pub at: Timestamp,
    pub kind: CrashKind,
    pub run_started: Option<Timestamp>,
    pub generation: Option<u32>,
    pub build: Option<u32>,
    /// How FXServer ended, in words: `exit code 0xC0000005`, `signal 11 (SIGSEGV)`.
    pub exit: Option<String>,
    pub exit_code: Option<String>,
    pub signal: Option<i32>,
    pub hang_cause: Option<HangCause>,
    /// What groups crashes that look alike: the code and where it happened.
    pub signature: Option<String>,
    pub analysis_state: AnalysisState,
    pub analysis: Option<CrashAnalysis>,
    /// FXServer's watchdog report of the stuck thread's stack, masked.
    pub watchdog_stack: Option<String>,
    pub files: Vec<CrashFile>,
    /// Crashes with the same signature over the last 7 days, this one included.
    pub group: Option<CrashGroup>,
    pub updated_at: Timestamp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CrashGroup {
    pub count: u32,
    pub first_at: Timestamp,
}

/// What a dump or a hang report says, without symbols.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CrashAnalysis {
    pub exception: Option<CrashException>,
    pub thread: Option<AnalysedThread>,
    /// The thread's instruction pointer, then return addresses found by scanning its stack:
    /// approximate without symbols.
    pub frames: Vec<Frame>,
    /// A Linux hang report's threads.
    pub threads: Vec<ThreadState>,
    /// `false` when Linux would not show the system calls the threads wait in.
    pub syscalls_readable: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CrashException {
    /// `0xC0000005`.
    pub code: String,
    /// The crate's name for it, such as `EXCEPTION_ACCESS_VIOLATION_WRITE`.
    pub name: String,
    /// The address an access violation touched, or where the exception happened.
    pub address: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AnalysedThread {
    pub id: u32,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Frame {
    /// The module's file name, without its folder; `None` outside every module.
    pub module: Option<String>,
    /// From the module's base, or the address itself outside every module.
    pub offset: String,
    /// Found by scanning the stack rather than read from the thread's context.
    pub scanned: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ThreadRun {
    Running,
    Sleeping,
    DiskWait,
    Stopped,
    Zombie,
    Other,
}

/// The kind of wait a system call is, for the panel's words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum SyscallGroup {
    /// Waiting for a lock another thread holds.
    Lock,
    Io,
    Wait,
    Sleep,
}

/// One thread of a hung server, from a Linux hang report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ThreadState {
    pub tid: u32,
    pub name: String,
    pub state: ThreadRun,
    /// Percent of one core over the report's window.
    pub cpu: Option<f64>,
    /// The system call it waits in, by name, or by number when the name is not known.
    pub syscall: Option<String>,
    pub syscall_group: Option<SyscallGroup>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum AlertKind {
    /// FXServer's tree started a shell.
    Shell,
    Crash,
    Hang,
}

/// Something of the last 24 hours that someone should look at. It names files, never paths,
/// command lines or addresses: the dashboard sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Alert {
    /// `process:<id>` or `crash:<id>`.
    pub key: String,
    pub kind: AlertKind,
    pub at: Timestamp,
    pub last_at: Timestamp,
    pub count: u32,
    /// The program's file name, for a shell.
    pub name: Option<String>,
    /// Its parent's file name.
    pub parent: Option<String>,
    pub crash: Option<AlertCrash>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AlertCrash {
    pub exception: Option<AlertException>,
    /// The module the crash or the hang was in.
    pub module: Option<String>,
    pub thread: Option<String>,
    /// How FXServer ended when no dump says more: `0xC0000005`, `SIGSEGV`.
    pub exit: Option<String>,
    /// A Linux hang: svMain was running on the processor.
    pub busy: bool,
    /// A Linux hang: the system call svMain waited in.
    pub waiting_in: Option<String>,
    /// Observer took a dump or a report of the hung server before it killed it.
    pub snapshot: bool,
    /// Crashes alike over the last 7 days, this one included.
    pub group_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AlertException {
    pub code: String,
    pub name: String,
}

/// The `server.alerts` topic, whole on every change: newest first, at most 10.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AlertList {
    pub alerts: Vec<Alert>,
    /// The start of the window the list covers.
    pub since: Timestamp,
}

/// Observer's crash folder against the dump policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CrashFolder {
    pub files: u32,
    pub bytes: u64,
    pub max_files: u32,
    pub max_bytes: u64,
    /// What this user may still write on its volume.
    pub free: Option<u64>,
}

/// The `server.diagnostics` topic's snapshot: the newest 200 processes, the destinations seen in
/// the last 24 hours, the listeners now and the newest 50 crashes.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DiagnosticsSnapshot {
    pub processes: Vec<TreeProcess>,
    pub destinations: Vec<Destination>,
    pub listeners: Vec<Listener>,
    pub crashes: Vec<Crash>,
    pub crash_folder: CrashFolder,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum DiagnosticsEvent {
    /// A process recorded, or its record updated.
    Process {
        process: Box<TreeProcess>,
    },
    /// The destinations and listeners again, whenever one changed.
    Connections {
        destinations: Vec<Destination>,
        listeners: Vec<Listener>,
    },
    /// A crash recorded, or its record updated.
    Crash {
        crash: Box<Crash>,
    },
    CrashFolder {
        folder: CrashFolder,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_crash_folder_s_files_are_told_apart_by_their_names() {
        let names = [
            (
                "6c1f0b2e-3e5d-4b1a-9d2c-1f0a5e7b9c3d.dmp",
                CrashFileKind::Minidump,
            ),
            (
                "6c1f0b2e-3e5d-4b1a-9d2c-1f0a5e7b9c3d-full.dmp",
                CrashFileKind::FullDump,
            ),
            ("6c1f0b2e-full-1.dmp", CrashFileKind::FullDump),
            ("livedump-3.dmp", CrashFileKind::LiveDump),
            ("hang-4-20261029T101500Z.dmp", CrashFileKind::HangDump),
            ("hang-4-20261029T101500Z.json", CrashFileKind::HangReport),
            ("notes.txt", CrashFileKind::Other),
        ];

        for (name, kind) in names {
            assert_eq!(CrashFileKind::of(name), kind, "{name}");
        }
    }

    #[test]
    fn stored_names_read_back() {
        for kind in ProcessKind::ALL {
            assert_eq!(ProcessKind::from_name(kind.as_str()), Some(kind));
        }
        assert_eq!(ProcessKind::from_name("dump_server"), None);
        assert_eq!(CrashKind::from_name("hang"), Some(CrashKind::Hang));
        assert_eq!(AnalysisState::from_name("noDump"), Some(AnalysisState::NoDump));
        assert_eq!(Protocol::from_name("udp"), Some(Protocol::Udp));
    }

    #[test]
    fn events_are_tagged_by_their_kind() {
        let event = DiagnosticsEvent::CrashFolder {
            folder: CrashFolder {
                files: 2,
                bytes: 515,
                max_files: 10,
                max_bytes: 20,
                free: None,
            },
        };

        assert_eq!(
            serde_json::to_value(&event).unwrap(),
            serde_json::json!({ "kind": "crashFolder", "folder": { "files": 2, "bytes": 515, "maxFiles": 10, "maxBytes": 20, "free": null } })
        );
    }
}
