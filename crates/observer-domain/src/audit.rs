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
    /// An install or an import began, with the build and its `kind`.
    BuildInstallStarted,
    /// Someone cancelled the job, at the `step` it was at.
    BuildInstallCancelled,
    /// By whoever started the job; a failure when the build was not installed, with the `step`
    /// and the `reason`.
    BuildInstallFinished,
    /// By a user, with how the build `apply`s; by the system for the first build installed or a
    /// build folder adopted at start.
    BuildSelected,
    /// A build waits for the next scheduled restart.
    PendingBuildSet,
    PendingBuildCancelled,
    /// The scheduled restart made the pending build the selected one.
    PendingBuildApplied,
    /// The pending build was deleted or broken by the scheduled restart: it was not applied.
    PendingBuildDropped,
    /// A new build stayed up long enough after it got ready: it is the last healthy one.
    ProbationPassed,
    /// A new build crashed or hung in its probation and FXServer kept it, for the `why` given.
    ProbationFailed,
    /// A new build failed its probation: FXServer went back to the last healthy one.
    BuildRolledBack,
    /// By hand, or by a `cleanup`.
    BuildDeleted,
    /// `details` holds each changed setting with its value before and after.
    ArtifactSettingsChanged,
    /// Observer began to stop, and FXServer with it. `details` says where the request came
    /// `from`: someone at the host (its window or its console) or a signal.
    ObserverStopped,
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
    /// A run's tree started more programs and command lines than its rows and audit entries keep:
    /// once when it happens, then at the run's end with the starts whose command line was not kept
    /// in `unrecorded`.
    TreeRecordsFull,
    FileSessionOpened,
    /// With its reads counted: files and bytes.
    FileSessionClosed,
    /// One entry per login, network and 15-minute window.
    FileLoginFailed,
    FileWritten,
    FileDeleted,
    FolderCreated,
    FolderRemoved,
    FileRenamed,
    FileRefused,
    /// A session reached its cap of change entries.
    FileRecordsFull,
    FileGroupCreated,
    FileGroupChanged,
    FileGroupDeleted,
    FileAccessChanged,
    ServiceAccessCreated,
    ServiceAccessChanged,
    ServiceAccessDeleted,
    FileKeyAdded,
    FileKeyRemoved,
    FileSessionDisconnected,
    FileSettingsChanged,
    FileHostKeyCreated,
    /// Every trace of a player was erased on request. The target is the licence's keyed
    /// fingerprint, never the licence.
    PlayerForgotten,
    /// txAdmin's players database was taken in. The details hold the file's size and counts only.
    PlayersImported,
    /// The retentions of the players' identities, play time per day and profiles changed.
    PlayersRetentionChanged,
    /// The drop log went out as CSV. The details hold the kinds of its filters, never their
    /// values, and the row count.
    PlayerDropsExported,
    /// The daily drop rates went out as CSV.
    DropRatesExported,
    /// How long players' sessions and their texts are kept changed.
    DropsRetentionChanged,
    /// A diff or a full file of the server's contents, once an hour per account, file and pair
    /// of starts.
    ChangeContentsViewed,
    ChangeSettingsChanged,
    ChangeContentsPurged,
    ChangeVerifyRequested,
    /// A check of the disk, which reads the server folder at the capped rate.
    ChangeCheckRequested,
    /// The owner changed a tool's settings. The target is the tool; `details` holds each changed
    /// setting with its value before and after.
    ToolSettingsChanged,
    /// A tool's run changed files in the server folder. The target is the tool; `details` holds
    /// the `run`, its `trigger`, the `files` changed (a capped list) and their `count`.
    ToolFilesChanged,
    /// A tool gave files back entries it had taken out, or their original. The target is the
    /// tool; `details` holds the `run`, the `reason` (`heal` or `undo`), the `files` (a capped
    /// list) and their `count`.
    ToolFilesRestored,
    /// `details` holds each changed setting with its value before and after, and
    /// `passwordChanged` when the manual source's password was replaced, never the password.
    BackupSettingsChanged,
    /// The target is the destination's id; `details` hold its name, kind and fields, never its
    /// secret, and `secretSet` when one was given.
    BackupDestinationCreated,
    /// `details` hold each changed field with its value before and after, and `secretChanged`.
    BackupDestinationChanged,
    /// `details` hold its name, kind, and the copies left where it pointed (`stored`).
    BackupDestinationDeleted,
    /// A run ended, by whoever asked for it or the system. The target is the run; `details` hold
    /// its `trigger`, `outcome`, `database`, `fileBytes`, the copies `stored` and `failed`,
    /// `serverStarted`, and the `error` it failed with.
    BackupFinished,
    /// Someone cancelled the run under way, the target.
    BackupCancelled,
    /// A copy of a backup went out of Observer: the target is the run, `details` hold the `copy` and
    /// its `destination`.
    BackupDownloaded,
    /// The owner deleted a backup's copies everywhere: the target is the run, `details` hold the
    /// copies `deleted` and those that could not be (`failed`).
    BackupDeleted,
    /// A destination's retention let copies go after a run. The target is the destination;
    /// `details` hold the `run` and the copies `deleted`.
    BackupsExpired,
}

impl AuditAction {
    pub const ALL: [Self; 93] = [
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
        Self::BuildInstallStarted,
        Self::BuildInstallCancelled,
        Self::BuildInstallFinished,
        Self::BuildSelected,
        Self::PendingBuildSet,
        Self::PendingBuildCancelled,
        Self::PendingBuildApplied,
        Self::PendingBuildDropped,
        Self::ProbationPassed,
        Self::ProbationFailed,
        Self::BuildRolledBack,
        Self::BuildDeleted,
        Self::ArtifactSettingsChanged,
        Self::ObserverStopped,
        Self::TreeProcessStarted,
        Self::HangSnapshotTaken,
        Self::CrashFileDownloaded,
        Self::CrashFilesDeleted,
        Self::TreeRecordsFull,
        Self::FileSessionOpened,
        Self::FileSessionClosed,
        Self::FileLoginFailed,
        Self::FileWritten,
        Self::FileDeleted,
        Self::FolderCreated,
        Self::FolderRemoved,
        Self::FileRenamed,
        Self::FileRefused,
        Self::FileRecordsFull,
        Self::FileGroupCreated,
        Self::FileGroupChanged,
        Self::FileGroupDeleted,
        Self::FileAccessChanged,
        Self::ServiceAccessCreated,
        Self::ServiceAccessChanged,
        Self::ServiceAccessDeleted,
        Self::FileKeyAdded,
        Self::FileKeyRemoved,
        Self::FileSessionDisconnected,
        Self::FileSettingsChanged,
        Self::FileHostKeyCreated,
        Self::PlayerForgotten,
        Self::PlayersImported,
        Self::PlayersRetentionChanged,
        Self::PlayerDropsExported,
        Self::DropRatesExported,
        Self::DropsRetentionChanged,
        Self::ChangeContentsViewed,
        Self::ChangeSettingsChanged,
        Self::ChangeContentsPurged,
        Self::ChangeVerifyRequested,
        Self::ChangeCheckRequested,
        Self::ToolSettingsChanged,
        Self::ToolFilesChanged,
        Self::ToolFilesRestored,
        Self::BackupSettingsChanged,
        Self::BackupDestinationCreated,
        Self::BackupDestinationChanged,
        Self::BackupDestinationDeleted,
        Self::BackupFinished,
        Self::BackupCancelled,
        Self::BackupDownloaded,
        Self::BackupDeleted,
        Self::BackupsExpired,
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
            Self::BuildInstallStarted => "build_install_started",
            Self::BuildInstallCancelled => "build_install_cancelled",
            Self::BuildInstallFinished => "build_install_finished",
            Self::BuildSelected => "build_selected",
            Self::PendingBuildSet => "pending_build_set",
            Self::PendingBuildCancelled => "pending_build_cancelled",
            Self::PendingBuildApplied => "pending_build_applied",
            Self::PendingBuildDropped => "pending_build_dropped",
            Self::ProbationPassed => "probation_passed",
            Self::ProbationFailed => "probation_failed",
            Self::BuildRolledBack => "build_rolled_back",
            Self::BuildDeleted => "build_deleted",
            Self::ArtifactSettingsChanged => "artifact_settings_changed",
            Self::ObserverStopped => "observer_stopped",
            Self::TreeProcessStarted => "tree_process_started",
            Self::HangSnapshotTaken => "hang_snapshot_taken",
            Self::CrashFileDownloaded => "crash_file_downloaded",
            Self::CrashFilesDeleted => "crash_files_deleted",
            Self::TreeRecordsFull => "tree_records_full",
            Self::FileSessionOpened => "file_session_opened",
            Self::FileSessionClosed => "file_session_closed",
            Self::FileLoginFailed => "file_login_failed",
            Self::FileWritten => "file_written",
            Self::FileDeleted => "file_deleted",
            Self::FolderCreated => "folder_created",
            Self::FolderRemoved => "folder_removed",
            Self::FileRenamed => "file_renamed",
            Self::FileRefused => "file_refused",
            Self::FileRecordsFull => "file_records_full",
            Self::FileGroupCreated => "file_group_created",
            Self::FileGroupChanged => "file_group_changed",
            Self::FileGroupDeleted => "file_group_deleted",
            Self::FileAccessChanged => "file_access_changed",
            Self::ServiceAccessCreated => "service_access_created",
            Self::ServiceAccessChanged => "service_access_changed",
            Self::ServiceAccessDeleted => "service_access_deleted",
            Self::FileKeyAdded => "file_key_added",
            Self::FileKeyRemoved => "file_key_removed",
            Self::FileSessionDisconnected => "file_session_disconnected",
            Self::FileSettingsChanged => "file_settings_changed",
            Self::FileHostKeyCreated => "file_host_key_created",
            Self::PlayerForgotten => "player_forgotten",
            Self::PlayersImported => "players_imported",
            Self::PlayersRetentionChanged => "players_retention_changed",
            Self::PlayerDropsExported => "player_drops_exported",
            Self::DropRatesExported => "drop_rates_exported",
            Self::DropsRetentionChanged => "drops_retention_changed",
            Self::ChangeContentsViewed => "change_contents_viewed",
            Self::ChangeSettingsChanged => "change_settings_changed",
            Self::ChangeContentsPurged => "change_contents_purged",
            Self::ChangeVerifyRequested => "change_verify_requested",
            Self::ChangeCheckRequested => "change_check_requested",
            Self::ToolSettingsChanged => "tool_settings_changed",
            Self::ToolFilesChanged => "tool_files_changed",
            Self::ToolFilesRestored => "tool_files_restored",
            Self::BackupSettingsChanged => "backup_settings_changed",
            Self::BackupDestinationCreated => "backup_destination_created",
            Self::BackupDestinationChanged => "backup_destination_changed",
            Self::BackupDestinationDeleted => "backup_destination_deleted",
            Self::BackupFinished => "backup_finished",
            Self::BackupCancelled => "backup_cancelled",
            Self::BackupDownloaded => "backup_downloaded",
            Self::BackupDeleted => "backup_deleted",
            Self::BackupsExpired => "backups_expired",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|action| action.as_str() == name)
    }
}

/// Who acted: a signed-in user, someone at the host's own window, the command line, Observer
/// itself, an anonymous client such as a failed login, or a pipeline's file access.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ActorKind {
    User,
    Local,
    Cli,
    System,
    Anonymous,
    Service,
}

impl ActorKind {
    pub const ALL: [Self; 6] = [
        Self::User,
        Self::Local,
        Self::Cli,
        Self::System,
        Self::Anonymous,
        Self::Service,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Local => "local",
            Self::Cli => "cli",
            Self::System => "system",
            Self::Anonymous => "anonymous",
            Self::Service => "service",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == name)
    }
}

/// Where the request to stop Observer came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum StopSource {
    /// Quit, in the status window on the host.
    Window,
    /// Ctrl+C or a closed console window, on the host.
    Console,
    /// A signal from a service manager or a closed terminal session.
    Signal,
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
    /// The user's id when the actor was a user, or the file access's id for a service. It outlives
    /// the account, unlike a foreign key.
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
