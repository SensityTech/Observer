//! The audit log as the panel reads it: a closed catalog of actions, who did them and how it went.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

/// Every action the audit log records. Stored by name, so a variant is never renamed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum AuditAction {
    SetupCompleted,
    SetupLinkCreated,
    LoginSucceeded,
    /// One entry per username, network and 15-minute window, with the count in `details`.
    LoginFailed,
    Logout,
    SessionRevoked,
    PasswordChanged,
    UserCreated,
    UserDeleted,
    UserPermissionsChanged,
    UserPasswordReset,
    UserUnlocked,
    /// Only for actions whose permission is sensitive or critical.
    PermissionDenied,
    AuditExported,
    /// A signed-in user entered their password again, which unlocks sensitive actions for a while.
    /// A wrong one is a failure, as a wrong current password is for `PasswordChanged`.
    Reauthenticated,
    /// The owner changed how FXServer is launched. `details` holds each changed setting with its
    /// value before and after.
    LaunchSettingsChanged,
    /// Someone started FXServer, or Observer did as it started (`automatic`). A start that failed
    /// is a failure, with the step that failed as its `reason`.
    ServerStarted,
    ServerStopped,
    ServerRestarted,
    /// Written before the command reaches FXServer, with the `command` as the console shows it,
    /// secrets masked. `observer_cmd` is denied, with its `refusal`.
    ConsoleCommandSent,
    /// A console log file was downloaded; the target is its name.
    ConsoleLogDownloaded,
    /// FXServer failed too often in a row: Observer stopped restarting it until someone starts it.
    /// `details` holds the `failures` counted.
    ServerCrashLoop,
    /// `details` holds each changed setting with its value before and after, and what the change
    /// `dropped`: a skip or a delay of an occurrence the schedule no longer has.
    ScheduleChanged,
    /// By a user, or by the system when FXServer started shortly before (`recentStart`).
    ScheduledRestartSkipped,
    ScheduledRestartRestored,
    ScheduledRestartDelayed,
    /// A restart that did not happen: its time passed while Observer was held up (`late`), or
    /// FXServer was busy, or its launch settings are refused on this host.
    ScheduledRestartMissed,
    OneOffRestartPlanned,
    /// By a user, or by the system when the run it was planned for ended first.
    OneOffRestartCancelled,
    /// FXServer's tree started a shell or a program it never starts by itself. At most one entry
    /// per program and run every 10 minutes, the starts since the previous one in `repeated`.
    TreeProcessStarted,
    /// Observer pictured a hung server before killing it: a dump, or on Linux a report of its
    /// threads. A failure carries its `reason`.
    HangSnapshotTaken,
    /// The owner downloaded one of a crash's files.
    CrashFileDownloaded,
    /// The owner deleted a crash's files; the analysis stays.
    CrashFilesDeleted,
}

impl AuditAction {
    pub const ALL: [Self; 33] = [
        Self::SetupCompleted,
        Self::SetupLinkCreated,
        Self::LoginSucceeded,
        Self::LoginFailed,
        Self::Logout,
        Self::SessionRevoked,
        Self::PasswordChanged,
        Self::UserCreated,
        Self::UserDeleted,
        Self::UserPermissionsChanged,
        Self::UserPasswordReset,
        Self::UserUnlocked,
        Self::PermissionDenied,
        Self::AuditExported,
        Self::Reauthenticated,
        Self::LaunchSettingsChanged,
        Self::ServerStarted,
        Self::ServerStopped,
        Self::ServerRestarted,
        Self::ConsoleCommandSent,
        Self::ConsoleLogDownloaded,
        Self::ServerCrashLoop,
        Self::ScheduleChanged,
        Self::ScheduledRestartSkipped,
        Self::ScheduledRestartRestored,
        Self::ScheduledRestartDelayed,
        Self::ScheduledRestartMissed,
        Self::OneOffRestartPlanned,
        Self::OneOffRestartCancelled,
        Self::TreeProcessStarted,
        Self::HangSnapshotTaken,
        Self::CrashFileDownloaded,
        Self::CrashFilesDeleted,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SetupCompleted => "setup_completed",
            Self::SetupLinkCreated => "setup_link_created",
            Self::LoginSucceeded => "login_succeeded",
            Self::LoginFailed => "login_failed",
            Self::Logout => "logout",
            Self::SessionRevoked => "session_revoked",
            Self::PasswordChanged => "password_changed",
            Self::UserCreated => "user_created",
            Self::UserDeleted => "user_deleted",
            Self::UserPermissionsChanged => "user_permissions_changed",
            Self::UserPasswordReset => "user_password_reset",
            Self::UserUnlocked => "user_unlocked",
            Self::PermissionDenied => "permission_denied",
            Self::AuditExported => "audit_exported",
            Self::Reauthenticated => "reauthenticated",
            Self::LaunchSettingsChanged => "launch_settings_changed",
            Self::ServerStarted => "server_started",
            Self::ServerStopped => "server_stopped",
            Self::ServerRestarted => "server_restarted",
            Self::ConsoleCommandSent => "console_command_sent",
            Self::ConsoleLogDownloaded => "console_log_downloaded",
            Self::ServerCrashLoop => "server_crash_loop",
            Self::ScheduleChanged => "schedule_changed",
            Self::ScheduledRestartSkipped => "scheduled_restart_skipped",
            Self::ScheduledRestartRestored => "scheduled_restart_restored",
            Self::ScheduledRestartDelayed => "scheduled_restart_delayed",
            Self::ScheduledRestartMissed => "scheduled_restart_missed",
            Self::OneOffRestartPlanned => "one_off_restart_planned",
            Self::OneOffRestartCancelled => "one_off_restart_cancelled",
            Self::TreeProcessStarted => "tree_process_started",
            Self::HangSnapshotTaken => "hang_snapshot_taken",
            Self::CrashFileDownloaded => "crash_file_downloaded",
            Self::CrashFilesDeleted => "crash_files_deleted",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|action| action.as_str() == name)
    }
}

/// Who acted: a signed-in user, someone at the host's own window, the command line, Observer
/// itself, or an anonymous client such as a failed login.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ActorKind {
    User,
    Local,
    Cli,
    System,
    Anonymous,
}

impl ActorKind {
    pub const ALL: [Self; 5] = [Self::User, Self::Local, Self::Cli, Self::System, Self::Anonymous];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Local => "local",
            Self::Cli => "cli",
            Self::System => "system",
            Self::Anonymous => "anonymous",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Outcome {
    Success,
    Failure,
    /// Refused for lack of permission.
    Denied,
}

impl Outcome {
    pub const ALL: [Self; 3] = [Self::Success, Self::Failure, Self::Denied];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
            Self::Denied => "denied",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|outcome| outcome.as_str() == name)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AuditEntry {
    pub id: i64,
    pub at: Timestamp,
    pub actor_kind: ActorKind,
    /// The user's id when the actor was a user. It outlives the account, unlike a foreign key.
    pub actor_id: Option<i64>,
    pub actor_name: Option<String>,
    pub ip: Option<String>,
    pub user_agent: Option<String>,
    pub action: AuditAction,
    pub target: Option<String>,
    pub outcome: Outcome,
    #[ts(type = "Record<string, unknown>")]
    pub details: Value,
}

/// `GET /api/audit`: newest first.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct AuditPage {
    pub entries: Vec<AuditEntry>,
    /// The `before` of the next page, or null on the last one.
    pub next: Option<i64>,
}

/// The query of `GET /api/audit` and `GET /api/audit/export.csv`. `from` is inclusive, `to`
/// exclusive; `before` and `limit` only apply to the page.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export, optional_fields)]
pub struct AuditFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<AuditAction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<Outcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<Timestamp>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<Timestamp>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_names_match_the_wire_names() {
        for action in AuditAction::ALL {
            assert_eq!(serde_json::to_value(action).unwrap(), action.as_str());
            assert_eq!(AuditAction::from_name(action.as_str()), Some(action));
        }
        for kind in ActorKind::ALL {
            assert_eq!(serde_json::to_value(kind).unwrap(), kind.as_str());
            assert_eq!(ActorKind::from_name(kind.as_str()), Some(kind));
        }
        for outcome in Outcome::ALL {
            assert_eq!(serde_json::to_value(outcome).unwrap(), outcome.as_str());
            assert_eq!(Outcome::from_name(outcome.as_str()), Some(outcome));
        }
        assert_eq!(AuditAction::from_name("user_renamed"), None);
    }
}
