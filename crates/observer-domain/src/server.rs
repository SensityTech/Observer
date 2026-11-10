//! FXServer as the panel sees it: its state under the supervisor, its console, and what the routes
//! that control it take and answer.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The states the supervisor moves FXServer through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ServerState {
    Stopped,
    Starting,
    Online,
    /// Running, with a health problem.
    Degraded,
    /// Not responding; the supervisor is killing it.
    Hung,
    Stopping,
    /// Ended without being asked to; a restart follows unless the crash loop breaker tripped.
    Crashed,
    /// Automatic restarts are off until someone starts the server.
    CrashLoop,
}

/// What made the server enter its state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum StateCause {
    /// A start or a stop. `by` is a username, or `observer` for its own.
    Requested {
        by: String,
    },
    /// A restart someone asked for: its stop, then the start that follows.
    Restart {
        by: String,
    },
    /// An automatic restart after a failure.
    Recovery,
    /// A restart the schedule made: its stop, then the start that follows. `by` is `observer` for
    /// the schedule's own times, or whoever planned a single restart.
    Scheduled {
        by: String,
    },
    Ready,
    Recovered,
    Unhealthy {
        problems: Vec<HealthProblem>,
    },
    Hang {
        cause: HangCause,
    },
    /// The process ended; `description` reads the exit code, such as `0xC0000005 access violation`.
    Exited {
        description: String,
    },
    /// FXServer's own `Fatal Error:` line.
    FatalError {
        message: String,
    },
    BindError {
        address: String,
    },
    LicenceRejected {
        message: String,
    },
    /// An operator typed `quit`.
    ConsoleQuit {
        by: String,
    },
    /// Observer is stopping.
    Shutdown,
    CrashLoop {
        failures: u32,
    },
    /// The process could not be started.
    SpawnFailed {
        reason: SpawnFailure,
    },
}

/// Why FXServer could not be started. The full reason names the owner's folders, which only the
/// owner reads in the launch settings, so it goes to the console and Observer's log; the status
/// and the API only say which step failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum SpawnFailure {
    /// No launch settings were saved yet.
    NotConfigured,
    /// The rules in force on this host refuse the launch settings.
    InvalidSettings,
    /// The build folder holds no FXServer program.
    NotABuild,
    /// The server data folder does not exist.
    MissingData,
    /// The server cfg cannot be read.
    CfgUnreadable,
    /// The observer bridge could not be written into the build.
    Bridge,
    /// The operating system refused to start the process.
    Spawn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum HealthProblem {
    /// `/dynamic.json` has not answered for a while.
    HttpLate,
    /// The bridge's heartbeat is late: the main loop is slow or stuck.
    HeartbeatLate,
    /// The main thread's tick count stopped between two reads of `/perf/`.
    MainThreadStalled,
    /// `/perf/` refuses Observer's credentials.
    ProbeUnauthorized,
    /// FXServer's watchdog reported a stuck thread.
    WatchdogBark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum HangCause {
    NoHeartbeat,
    NoHttp,
}

/// Whether the stored launch settings can start FXServer on this host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum LaunchReadiness {
    Ready,
    /// The owner has not saved any yet.
    NotConfigured,
    /// Saved, but refused by the rules in force on this host: the owner fixes them in the launch settings.
    Invalid,
    /// No build is selected: one is installed or imported on the Artifacts page.
    NoBuild,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct PlayerCount {
    pub online: u64,
    /// `sv_maxclients`, when FXServer reports it.
    pub max: Option<u64>,
}

/// The last resource FXServer said it started.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StartedResource {
    pub name: String,
    pub at: Timestamp,
    /// What FXServer counted after its name as it started.
    pub warnings: u32,
}

/// The command of the cfg a start is at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StartCommand {
    /// The cfg file the command is written in, named from the server folder.
    pub file: String,
    pub line: u32,
    /// Its argument as written: a resource, or a category in brackets.
    pub target: String,
    /// When FXServer came to it.
    pub since: Timestamp,
    /// How many of the resources it starts are up or refused to start, out of `of`.
    pub done: u32,
    pub of: u32,
    /// The resource whose server scripts FXServer is loading, when it said so.
    pub resource: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SlowResource {
    pub name: String,
    /// From the resource started before it to this one.
    pub ms: u32,
}

/// How far the start under way has come, read from what FXServer prints while it runs its cfg:
/// each resource once it is up, and nothing of the one it is starting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StartProgress {
    /// Resources FXServer said it started.
    pub started: u32,
    /// Resources it could not start.
    pub failed: u32,
    /// How many the server folder holds, once FXServer has said.
    pub found: Option<u32>,
    /// How many the cfg's commands start. Null when Observer could not read them all.
    pub planned: Option<u32>,
    pub last: Option<StartedResource>,
    /// Worked out from the resources each command starts. Null before the first command and
    /// after the last, and when Observer could not read every command.
    pub at: Option<StartCommand>,
    /// The five slowest, the slowest first.
    pub slowest: Vec<SlowResource>,
    /// When Observer takes the start for stuck and restarts it, should FXServer start no resource
    /// until then. Null with hang detection off, and once the cfg has run.
    pub stuck_at: Option<Timestamp>,
    /// What `POST /servers/default/start/extend` adds to `stuckAt`, in minutes.
    pub extension_minutes: u32,
}

/// The `server.status` topic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ServerStatus {
    pub state: ServerState,
    /// Null until the first change of state.
    pub cause: Option<StateCause>,
    pub since: Timestamp,
    /// When the server that runs now started, which its uptime counts from.
    pub started_at: Option<Timestamp>,
    /// How many times Observer started FXServer since it started itself.
    pub generation: u32,
    pub problems: Vec<HealthProblem>,
    /// The build the running server reports.
    pub build: Option<u32>,
    pub players: Option<PlayerCount>,
    /// When the restart after a failure is due.
    pub restart_at: Option<Timestamp>,
    pub launch: LaunchReadiness,
    /// `sv_projectName` without its colour codes, as of the last run that set it.
    pub name: Option<String>,
    /// The game port, as of the last run.
    pub port: Option<u16>,
    /// Only while FXServer is starting, from its first second.
    pub start: Option<StartProgress>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum LineKind {
    Stdout,
    Stderr,
    /// A command an operator sent, echoed once FXServer has it.
    Command,
    /// A note from Observer itself.
    Observer,
}

/// The line was cut at 16 KiB.
pub const LINE_TRUNCATED: u8 = 1;
/// FXServer wrote no line end after it for a while: the rest may follow on the next line.
pub const LINE_NO_EOL: u8 = 2;
/// A secret was masked in it.
pub const LINE_MASKED: u8 = 4;

/// One console line, as a tuple to keep thousands of them light: `[seq, ts, generation, kind,
/// actor, text, flags]`. `ts` is in milliseconds since the epoch; `actor` names who sent a command
/// or which part of Observer wrote a note; `text` holds printable characters and SGR colour
/// sequences only; `flags` is a set of the `LINE_*` bits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct ConsoleLine(
    pub u64,
    pub i64,
    pub u32,
    pub LineKind,
    pub Option<String>,
    pub String,
    pub u8,
);

/// The console's snapshot: the lines after the `since` of the subscription, or the latest ones.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct ConsoleSnapshot {
    pub lines: Vec<ConsoleLine>,
    /// Lines of this Observer after `since` are missing: they left its memory.
    pub gap: bool,
    /// `since` came from an earlier Observer: what that one printed after it is in the log files.
    pub restarted: bool,
    /// The number of the newest line this Observer gave, sent or not, which the next `sub`
    /// resumes after.
    pub last: u64,
}

/// New console lines, in order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct ConsoleBatch {
    pub lines: Vec<ConsoleLine>,
}

/// `POST /api/servers/default/console`.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct ConsoleCommand {
    pub line: String,
}

/// What became of a command, once it was audited.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Delivery {
    /// FXServer has it; the console echoes it.
    Written,
    /// Handed to FXServer, which is not reading its console: it runs, and is echoed, when the
    /// server reads again.
    Pending,
    /// Not written: FXServer has not read the command before this one yet.
    Busy,
}

/// The answer to a console command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct CommandAck {
    pub delivery: Delivery,
    /// A secret was masked in the command as the console and the audit log show it: the panel
    /// keeps such a line out of the history it stores in the browser.
    pub masked: bool,
}

/// Why a console command was refused before anything was written. `observer_cmd` is Observer's
/// own channel to the bridge, refused to everyone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum CommandRefusal {
    Empty,
    /// Longer than 1,024 bytes.
    TooLong,
    ControlCharacter,
    Reserved,
}

/// One console log file, as `GET /api/servers/default/console/logs` lists them, newest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LogFile {
    /// `console-2026-10-28.log`, which the download route takes back.
    pub name: String,
    pub bytes: u64,
    pub modified_at: Timestamp,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_console_line_travels_as_a_tuple() {
        let line = ConsoleLine(
            1_761_642_000_000_001,
            1_761_642_000_123,
            3,
            LineKind::Command,
            Some("ada".to_owned()),
            "ensure chat".to_owned(),
            LINE_MASKED,
        );

        assert_eq!(
            serde_json::to_value(line).unwrap(),
            json!([
                1_761_642_000_000_001_u64,
                1_761_642_000_123_i64,
                3,
                "command",
                "ada",
                "ensure chat",
                4
            ])
        );
    }

    #[test]
    fn a_cause_names_its_kind_and_carries_its_detail() {
        let causes = [
            (
                StateCause::Requested { by: "ada".to_owned() },
                json!({ "kind": "requested", "by": "ada" }),
            ),
            (
                StateCause::Hang {
                    cause: HangCause::NoHeartbeat,
                },
                json!({ "kind": "hang", "cause": "noHeartbeat" }),
            ),
            (
                StateCause::CrashLoop { failures: 5 },
                json!({ "kind": "crashLoop", "failures": 5 }),
            ),
            (StateCause::Shutdown, json!({ "kind": "shutdown" })),
            (
                StateCause::SpawnFailed {
                    reason: SpawnFailure::NotABuild,
                },
                json!({ "kind": "spawnFailed", "reason": "notABuild" }),
            ),
        ];

        for (cause, expected) in causes {
            assert_eq!(serde_json::to_value(cause).unwrap(), expected);
        }
    }
}
