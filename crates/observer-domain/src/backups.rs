//! Backups of the game server's database: what the owner sets, where the copies go, the runs and
//! what became of each copy, as the panel shows them; the checks of the settings and of the
//! destinations, the retention rule, and the connection strings resources read.
//!
//! A backup is the whole database, every player in it, so nothing here ever carries a secret back
//! out: passwords and keys come in through [`Secret`], which neither prints nor serialises.

use std::cmp::Reverse;
use std::fmt;
use std::path::{Component, Path};

use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::schedule::{self, ScheduleProblem};

pub const DEFAULT_RESTART_WAIT_SECONDS: u32 = 60;
/// The longest a restart waits for its backup's dump before FXServer starts anyway.
pub const MAX_RESTART_WAIT_SECONDS: u32 = 600;
pub const DEFAULT_KEEP_LAST: u32 = 14;
pub const MAX_KEEP_LAST: u32 = 1000;
pub const MAX_KEEP_DAYS: u32 = 3650;
pub const MAX_NAME_CHARS: usize = 64;
/// The most tables without transactions a run names: its count covers them all.
pub const MAX_LOOSE_TABLES: usize = 50;
/// The longest technical detail kept with a failure.
pub const MAX_DETAIL_CHARS: usize = 500;
pub const MAX_SECRET_BYTES: usize = 8192;
const MAX_PATH_CHARS: usize = 1024;
const MAX_HOST_CHARS: usize = 253;
const MAX_FIELD_CHARS: usize = 128;
const MAX_PREFIX_CHARS: usize = 512;
const MAX_HOST_KEY_CHARS: usize = 1024;

/// A password or a key on its way in. It never prints and never serialises, so that no log line,
/// audit entry or response can carry it.
#[derive(Clone, PartialEq, Eq, Deserialize, TS)]
#[ts(export, type = "string")]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

/// Which restarts a backup goes before.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum BeforeRestarts {
    Off,
    /// The schedule's restarts and the single restarts someone planned.
    #[default]
    Scheduled,
    /// Those, and the restarts asked for in the panel or the host's window.
    AllRestarts,
}

impl BeforeRestarts {
    pub const ALL: [Self; 3] = [Self::Off, Self::Scheduled, Self::AllRestarts];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Scheduled => "scheduled",
            Self::AllRestarts => "allRestarts",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|value| value.as_str() == name)
    }
}

/// The database server the backups dump. Both speak the MySQL protocol and share a driver; what
/// differs is which client tool is preferred and a few of its options.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DatabaseEngine {
    #[default]
    MariaDb,
    MySql,
}

impl DatabaseEngine {
    pub const ALL: [Self; 2] = [Self::MariaDb, Self::MySql];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MariaDb => "mariaDb",
            Self::MySql => "mySql",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|value| value.as_str() == name)
    }
}

/// Where the backups find the database.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum SourceKind {
    /// The `mysql_connection_string` the server's cfg sets, read again at every run.
    #[default]
    Cfg,
    /// The connection below, with the password kept apart.
    Manual,
}

impl SourceKind {
    pub const ALL: [Self; 2] = [Self::Cfg, Self::Manual];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Cfg => "cfg",
            Self::Manual => "manual",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|value| value.as_str() == name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct ManualSource {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub database: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct BackupSettings {
    pub before_restarts: BeforeRestarts,
    /// `HH:MM`, 24-hour, in the restart schedule's zone.
    pub times: Vec<String>,
    /// How long a restart waits for its backup's dump before FXServer starts anyway.
    pub restart_wait_seconds: u32,
    pub engine: DatabaseEngine,
    pub source: SourceKind,
    /// Kept while the source is the cfg, so that switching back loses nothing.
    pub manual: Option<ManualSource>,
    /// The folder of the client tools, or null to look for them.
    pub client_folder: Option<String>,
}

impl Default for BackupSettings {
    fn default() -> Self {
        Self {
            before_restarts: BeforeRestarts::default(),
            times: Vec::new(),
            restart_wait_seconds: DEFAULT_RESTART_WAIT_SECONDS,
            engine: DatabaseEngine::default(),
            source: SourceKind::default(),
            manual: None,
            client_folder: None,
        }
    }
}

/// `PUT /api/servers/default/backups/settings`.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct BackupSettingsChange {
    pub settings: BackupSettings,
    /// The version the change was made from, null before the first save.
    pub version: Option<i64>,
    /// The manual source's password; null keeps the one stored.
    #[serde(default)]
    pub password: Option<Secret>,
}

/// The settings as stored, with the version a change must name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredBackupSettings {
    pub settings: BackupSettings,
    pub version: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum BackupField {
    /// The list of times as a whole.
    Times,
    /// One of the times.
    Time,
    RestartWaitSeconds,
    Host,
    Port,
    User,
    Database,
    Password,
    ClientFolder,
}

/// Why a field is refused, for the settings and the destinations alike.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum FieldProblem {
    Required,
    TooMany,
    TooLong,
    /// Not `HH:MM` on a 24-hour clock.
    InvalidTime,
    Duplicate,
    /// Less than 30 minutes after the time before it, across midnight too.
    TooClose,
    OutOfRange,
    /// A path that does not start from a root or a drive.
    NotAbsolute,
    /// Characters the field cannot hold, or a shape it does not take.
    Invalid,
    /// `http` to anything but this host.
    NotEncrypted,
    /// A secret the destination does not take: Observer makes its own SFTP key.
    NotAccepted,
    /// Inside the server's folder, which its resources read.
    InsideServerFolder,
}

/// One thing wrong with backup settings, which the panel shows next to its field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BackupSettingsIssue {
    pub field: BackupField,
    /// The position in its list, for `time`.
    pub index: Option<usize>,
    pub problem: FieldProblem,
}

impl BackupSettings {
    /// Everything that keeps these settings from being stored. `has_password` says whether a
    /// password is stored or comes with the change: a manual source needs one.
    pub fn issues(&self, has_password: bool) -> Vec<BackupSettingsIssue> {
        let mut issues = Vec::new();
        let mut note = |field, index, problem| {
            issues.push(BackupSettingsIssue {
                field,
                index,
                problem,
            });
        };
        schedule::check_times(&self.times, |index, problem| {
            let field = if index.is_some() {
                BackupField::Time
            } else {
                BackupField::Times
            };
            let problem = match problem {
                ScheduleProblem::InvalidTime => FieldProblem::InvalidTime,
                ScheduleProblem::Duplicate => FieldProblem::Duplicate,
                ScheduleProblem::TooClose => FieldProblem::TooClose,
                _ => FieldProblem::TooMany,
            };
            note(field, index, problem);
        });
        if self.restart_wait_seconds > MAX_RESTART_WAIT_SECONDS {
            note(BackupField::RestartWaitSeconds, None, FieldProblem::OutOfRange);
        }
        if let Some(folder) = &self.client_folder
            && let Some(problem) = path_problem(folder)
        {
            note(BackupField::ClientFolder, None, problem);
        }
        match (&self.manual, self.source) {
            (None, SourceKind::Manual) => note(BackupField::Host, None, FieldProblem::Required),
            (Some(manual), _) => {
                if let Some(problem) = host_problem(&manual.host) {
                    note(BackupField::Host, None, problem);
                }
                if manual.port == 0 {
                    note(BackupField::Port, None, FieldProblem::OutOfRange);
                }
                if let Some(problem) = word_problem(&manual.user, MAX_FIELD_CHARS) {
                    note(BackupField::User, None, problem);
                }
                if let Some(problem) = word_problem(&manual.database, MAX_FIELD_CHARS) {
                    note(BackupField::Database, None, problem);
                }
                if self.source == SourceKind::Manual && !has_password {
                    note(BackupField::Password, None, FieldProblem::Required);
                }
            }
            (None, SourceKind::Cfg) => {}
        }
        issues
    }

    /// The settings as they are stored: times ascending.
    #[must_use]
    pub fn normalized(mut self) -> Self {
        self.times.sort_by_key(|time| schedule::parse_time(time));
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DestinationKind {
    Local,
    Sftp,
    S3,
}

impl DestinationKind {
    pub const ALL: [Self; 3] = [Self::Local, Self::Sftp, Self::S3];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Sftp => "sftp",
            Self::S3 => "s3",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|value| value.as_str() == name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum SftpAuth {
    /// An Ed25519 key Observer makes for the destination; its public half goes in the remote
    /// account's `authorized_keys`.
    Key,
    Password,
}

/// Where a destination puts its copies, without its secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum DestinationTarget {
    /// A folder of this host, or a share mounted on it.
    Local { folder: String },
    Sftp {
        host: String,
        port: u16,
        user: String,
        /// Created if it is missing; empty for the account's home.
        folder: String,
        auth: SftpAuth,
        /// The host key the owner accepted, as an OpenSSH public key line; null until then, and
        /// nothing is sent.
        host_key: Option<String>,
    },
    /// An S3-compatible object store.
    S3 {
        /// `https://host[:port]`.
        endpoint: String,
        region: String,
        bucket: String,
        /// Put before every object's name; empty for none.
        prefix: String,
        access_key_id: String,
        /// `https://endpoint/bucket/key` rather than `https://bucket.endpoint/key`.
        path_style: bool,
    },
}

impl DestinationTarget {
    pub const fn kind(&self) -> DestinationKind {
        match self {
            Self::Local { .. } => DestinationKind::Local,
            Self::Sftp { .. } => DestinationKind::Sftp,
            Self::S3 { .. } => DestinationKind::S3,
        }
    }

    /// Whether the destination needs a secret given by the owner: an SFTP password or an S3
    /// secret key. An SFTP key is Observer's own.
    pub const fn takes_secret(&self) -> bool {
        matches!(
            self,
            Self::Sftp {
                auth: SftpAuth::Password,
                ..
            } | Self::S3 { .. }
        )
    }
}

/// How many copies a destination keeps. A copy stays while either rule keeps it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct Retention {
    /// The most recent copies kept.
    pub keep_last: Option<u32>,
    /// Copies younger than this many days are kept.
    pub keep_days: Option<u32>,
}

impl Default for Retention {
    fn default() -> Self {
        Self {
            keep_last: Some(DEFAULT_KEEP_LAST),
            keep_days: None,
        }
    }
}

/// A destination as the owner writes it: `POST …/backups/destinations`.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct DestinationDraft {
    pub name: String,
    pub enabled: bool,
    pub target: DestinationTarget,
    pub retention: Retention,
    /// The SFTP password or the S3 secret key; null keeps the one stored.
    #[serde(default)]
    pub secret: Option<Secret>,
}

/// `PUT …/backups/destinations/{id}`.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct DestinationChange {
    pub destination: DestinationDraft,
    pub version: i64,
}

/// `POST …/backups/destinations/test`.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct DestinationTestRequest {
    pub destination: DestinationDraft,
    /// A stored destination whose secret and key the test uses when the draft brings none.
    #[serde(default)]
    pub id: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DestinationField {
    Name,
    Folder,
    Host,
    Port,
    User,
    HostKey,
    Endpoint,
    Region,
    Bucket,
    Prefix,
    AccessKeyId,
    Secret,
    Retention,
    KeepLast,
    KeepDays,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DestinationIssue {
    pub field: DestinationField,
    pub problem: FieldProblem,
}

impl DestinationDraft {
    /// Everything that keeps this destination from being stored. `has_secret` says whether one is
    /// stored already. The host key's own format and the server folder are the service's checks.
    pub fn issues(&self, has_secret: bool) -> Vec<DestinationIssue> {
        let mut issues = Vec::new();
        let mut note = |field, problem| issues.push(DestinationIssue { field, problem });
        if let Some(problem) = text_problem(&self.name, MAX_NAME_CHARS) {
            note(DestinationField::Name, problem);
        }
        match &self.target {
            DestinationTarget::Local { folder } => {
                if let Some(problem) = path_problem(folder) {
                    note(DestinationField::Folder, problem);
                }
            }
            DestinationTarget::Sftp {
                host,
                port,
                user,
                folder,
                host_key,
                ..
            } => {
                if let Some(problem) = host_problem(host) {
                    note(DestinationField::Host, problem);
                }
                if *port == 0 {
                    note(DestinationField::Port, FieldProblem::OutOfRange);
                }
                if let Some(problem) = word_problem(user, MAX_FIELD_CHARS) {
                    note(DestinationField::User, problem);
                }
                if folder.chars().count() > MAX_PATH_CHARS {
                    note(DestinationField::Folder, FieldProblem::TooLong);
                } else if folder.chars().any(char::is_control) {
                    note(DestinationField::Folder, FieldProblem::Invalid);
                }
                if let Some(key) = host_key
                    && (key.len() > MAX_HOST_KEY_CHARS || key.chars().any(char::is_control))
                {
                    note(DestinationField::HostKey, FieldProblem::Invalid);
                }
            }
            DestinationTarget::S3 {
                endpoint,
                region,
                bucket,
                prefix,
                access_key_id,
                ..
            } => {
                if let Some(problem) = endpoint_problem(endpoint) {
                    note(DestinationField::Endpoint, problem);
                }
                let region_fits = !region.is_empty()
                    && region.len() <= 64
                    && region
                        .bytes()
                        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
                if !region_fits {
                    note(DestinationField::Region, FieldProblem::Invalid);
                }
                if !bucket_fits(bucket) {
                    note(DestinationField::Bucket, FieldProblem::Invalid);
                }
                if prefix.chars().count() > MAX_PREFIX_CHARS {
                    note(DestinationField::Prefix, FieldProblem::TooLong);
                } else if prefix.starts_with('/') || prefix.chars().any(char::is_control) {
                    note(DestinationField::Prefix, FieldProblem::Invalid);
                }
                if let Some(problem) = word_problem(access_key_id, MAX_FIELD_CHARS) {
                    note(DestinationField::AccessKeyId, problem);
                }
            }
        }
        match &self.secret {
            Some(_) if !self.target.takes_secret() => {
                note(DestinationField::Secret, FieldProblem::NotAccepted);
            }
            Some(secret) if secret.expose().is_empty() => {
                note(DestinationField::Secret, FieldProblem::Required);
            }
            Some(secret) if secret.expose().len() > MAX_SECRET_BYTES => {
                note(DestinationField::Secret, FieldProblem::TooLong);
            }
            None if self.target.takes_secret() && !has_secret => {
                note(DestinationField::Secret, FieldProblem::Required);
            }
            _ => {}
        }
        let Retention { keep_last, keep_days } = self.retention;
        if keep_last.is_none() && keep_days.is_none() {
            note(DestinationField::Retention, FieldProblem::Required);
        }
        if keep_last.is_some_and(|last| !(1..=MAX_KEEP_LAST).contains(&last)) {
            note(DestinationField::KeepLast, FieldProblem::OutOfRange);
        }
        if keep_days.is_some_and(|days| !(1..=MAX_KEEP_DAYS).contains(&days)) {
            note(DestinationField::KeepDays, FieldProblem::OutOfRange);
        }
        issues
    }
}

/// A destination as the panel shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BackupDestination {
    pub id: i64,
    pub name: String,
    pub enabled: bool,
    pub target: DestinationTarget,
    pub retention: Retention,
    /// The public half of Observer's SFTP key, as an `authorized_keys` line.
    pub public_key: Option<String>,
    /// Whether a password or a secret key is stored.
    pub has_secret: bool,
    pub version: i64,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    /// The copies stored there now, and what they weigh.
    pub stored: u32,
    #[ts(type = "number")]
    pub stored_bytes: u64,
    /// The latest copy made there, null before the first.
    pub last: Option<LastCopy>,
}

/// How the latest copy to a destination went.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LastCopy {
    pub run: i64,
    pub state: CopyState,
    /// When it was stored, or when it began for one under way or failed.
    pub at: Timestamp,
    pub error: Option<BackupError>,
    pub error_detail: Option<String>,
}

/// What started a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum BackupTrigger {
    /// One of the settings' times.
    Schedule,
    /// A restart, before FXServer started again.
    Restart,
    /// Someone asked in the panel.
    Manual,
}

impl BackupTrigger {
    pub const ALL: [Self; 3] = [Self::Schedule, Self::Restart, Self::Manual];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Schedule => "schedule",
            Self::Restart => "restart",
            Self::Manual => "manual",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|value| value.as_str() == name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum BackupOutcome {
    /// Every destination holds its copy.
    Succeeded,
    /// Some destinations hold their copy, others failed.
    Partial,
    /// The dump failed, or no destination holds a copy.
    Failed,
    Cancelled,
    /// Observer stopped without ending the run.
    Interrupted,
}

impl BackupOutcome {
    pub const ALL: [Self; 5] = [
        Self::Succeeded,
        Self::Partial,
        Self::Failed,
        Self::Cancelled,
        Self::Interrupted,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Partial => "partial",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|value| value.as_str() == name)
    }
}

/// Why a run, a copy or a test failed. The panel turns it into a message; the detail beside it is
/// what the tool or the remote end said, for whoever fixes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum BackupError {
    /// The source is the cfg and no server is set up yet.
    LaunchNotConfigured,
    /// The cfg sets no `mysql_connection_string`.
    SourceMissing,
    /// The connection string or the stored source cannot be used.
    SourceUnreadable,
    /// No dump tool was found for the engine.
    ClientMissing,
    /// The tool failed: wrong password, unknown database, missing privilege.
    DumpFailed,
    /// The tool ended without writing the end of a complete dump.
    DumpIncomplete,
    /// The file read back is not the one written.
    VerifyFailed,
    DiskFull,
    Io,
    /// The host does not answer, or its name does not resolve.
    Unreachable,
    /// The remote end refused the password or the key.
    Authentication,
    /// No host key is pinned for the destination.
    HostKeyUnknown,
    /// The host showed another key than the pinned one.
    HostKeyMismatch,
    /// The host offers only host keys Observer does not check, such as RSA.
    HostKeyAlgorithm,
    PermissionDenied,
    /// The remote end refused the request; the detail holds its status and code.
    Remote,
    /// Larger than the destination takes in one upload.
    TooLarge,
    TimedOut,
    /// A local destination inside the server's folder, which its resources read.
    InsideServerFolder,
    Cancelled,
    Interrupted,
}

impl BackupError {
    pub const ALL: [Self; 21] = [
        Self::LaunchNotConfigured,
        Self::SourceMissing,
        Self::SourceUnreadable,
        Self::ClientMissing,
        Self::DumpFailed,
        Self::DumpIncomplete,
        Self::VerifyFailed,
        Self::DiskFull,
        Self::Io,
        Self::Unreachable,
        Self::Authentication,
        Self::HostKeyUnknown,
        Self::HostKeyMismatch,
        Self::HostKeyAlgorithm,
        Self::PermissionDenied,
        Self::Remote,
        Self::TooLarge,
        Self::TimedOut,
        Self::InsideServerFolder,
        Self::Cancelled,
        Self::Interrupted,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LaunchNotConfigured => "launchNotConfigured",
            Self::SourceMissing => "sourceMissing",
            Self::SourceUnreadable => "sourceUnreadable",
            Self::ClientMissing => "clientMissing",
            Self::DumpFailed => "dumpFailed",
            Self::DumpIncomplete => "dumpIncomplete",
            Self::VerifyFailed => "verifyFailed",
            Self::DiskFull => "diskFull",
            Self::Io => "io",
            Self::Unreachable => "unreachable",
            Self::Authentication => "authentication",
            Self::HostKeyUnknown => "hostKeyUnknown",
            Self::HostKeyMismatch => "hostKeyMismatch",
            Self::HostKeyAlgorithm => "hostKeyAlgorithm",
            Self::PermissionDenied => "permissionDenied",
            Self::Remote => "remote",
            Self::TooLarge => "tooLarge",
            Self::TimedOut => "timedOut",
            Self::InsideServerFolder => "insideServerFolder",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|value| value.as_str() == name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum CopyState {
    Uploading,
    Stored,
    Failed,
    Deleted,
}

impl CopyState {
    pub const ALL: [Self; 4] = [Self::Uploading, Self::Stored, Self::Failed, Self::Deleted];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Uploading => "uploading",
            Self::Stored => "stored",
            Self::Failed => "failed",
            Self::Deleted => "deleted",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|value| value.as_str() == name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DeleteReason {
    /// The destination's retention let it go.
    Retention,
    /// The owner deleted the backup.
    Manual,
}

impl DeleteReason {
    pub const ALL: [Self; 2] = [Self::Retention, Self::Manual];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Retention => "retention",
            Self::Manual => "manual",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|value| value.as_str() == name)
    }
}

/// One copy of a run's file, at one destination.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BackupCopy {
    pub id: i64,
    pub destination_id: i64,
    pub destination: String,
    pub kind: DestinationKind,
    pub state: CopyState,
    /// The file's path in the folder, or the object's key.
    pub location: String,
    #[ts(type = "number | null")]
    pub bytes: Option<u64>,
    pub started_at: Timestamp,
    pub stored_at: Option<Timestamp>,
    pub deleted_at: Option<Timestamp>,
    pub delete_reason: Option<DeleteReason>,
    pub error: Option<BackupError>,
    pub error_detail: Option<String>,
}

/// One backup: the dump, its file and its copies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BackupRun {
    pub id: i64,
    pub trigger: BackupTrigger,
    /// Who asked, or for a restart who made it; null for what Observer did by itself.
    pub actor: Option<String>,
    pub started_at: Timestamp,
    pub finished_at: Option<Timestamp>,
    /// Null while the run is under way.
    pub outcome: Option<BackupOutcome>,
    pub engine: DatabaseEngine,
    pub database: Option<String>,
    pub host: Option<String>,
    /// The dump tool and its version.
    pub client: Option<String>,
    pub file_name: Option<String>,
    #[ts(type = "number | null")]
    pub sql_bytes: Option<u64>,
    #[ts(type = "number | null")]
    pub file_bytes: Option<u64>,
    pub file_sha256: Option<String>,
    /// FXServer started again before the dump ended: tables with transactions are as they were
    /// when it began, the others as the dump reached them.
    pub server_started: bool,
    /// Tables whose engine has no transactions, which the dump does not freeze: the first 50.
    pub loose_tables: Vec<String>,
    pub loose_table_count: u32,
    pub error: Option<BackupError>,
    pub error_detail: Option<String>,
    pub copies: Vec<BackupCopy>,
}

/// What a run is doing now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum BackupPhase {
    Preparing,
    Dumping,
    Verifying,
    Copying,
    Pruning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BackupProgress {
    pub run: i64,
    pub phase: BackupPhase,
    /// The dump's bytes read so far.
    #[ts(type = "number")]
    pub sql_bytes: u64,
    /// The destination being copied to, and how far.
    pub destination: Option<i64>,
    #[ts(type = "number")]
    pub copied_bytes: u64,
}

/// The database the backups would dump now, without its password.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SourceView {
    pub kind: SourceKind,
    pub host: Option<String>,
    pub port: Option<u16>,
    /// A Unix socket the connection goes through instead of TCP.
    pub socket: Option<String>,
    pub user: Option<String>,
    pub database: Option<String>,
    pub problem: Option<BackupError>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ClientFlavour {
    MariaDb,
    MySql,
}

/// The dump tool the next run would use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ClientView {
    pub path: Option<String>,
    pub flavour: Option<ClientFlavour>,
    /// As the tool prints it.
    pub version: Option<String>,
    pub problem: Option<BackupError>,
}

/// `GET /api/servers/default/backups`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BackupsView {
    pub settings: BackupSettings,
    /// Null until the settings are first saved.
    pub version: Option<i64>,
    /// Whether the manual source's password is stored.
    pub has_password: bool,
    /// The IANA name of the zone the times are in: the restart schedule's.
    pub zone: String,
    /// When the next timed run is due.
    pub next_at: Option<Timestamp>,
    /// The next restart of the schedule, when the settings back up before it.
    pub next_restart_at: Option<Timestamp>,
    /// The runs started in the last 30 days.
    pub runs_last_30_days: u32,
    pub source: SourceView,
    pub client: ClientView,
    pub destinations: Vec<BackupDestination>,
    pub running: Option<BackupProgress>,
    /// The latest runs, newest first, the one under way included.
    pub recent: Vec<BackupRun>,
}

/// `GET /api/servers/default/backups/runs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BackupRunsPage {
    pub runs: Vec<BackupRun>,
    /// The `before` that reads the next page, null on the last.
    pub next_before: Option<i64>,
}

/// `POST …/backups/source/test`: the stored settings, or a draft with its password.
#[derive(Debug, Clone, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct SourceTestRequest {
    #[serde(default)]
    pub settings: Option<BackupSettings>,
    #[serde(default)]
    pub password: Option<Secret>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SourceTestResult {
    pub source: SourceView,
    pub client: ClientView,
    /// As the database server reports it.
    pub server_version: Option<String>,
    pub tables: Option<u32>,
    /// Data and indexes, as the server estimates them.
    #[ts(type = "number | null")]
    pub bytes: Option<u64>,
    /// Tables whose engine has no transactions.
    pub loose_tables: Option<u32>,
    pub error: Option<BackupError>,
    pub error_detail: Option<String>,
}

/// A host key as SSH shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HostKey {
    /// The OpenSSH public key line, which a destination pins.
    pub key: String,
    /// `SHA256:…`, as `ssh-keygen -l` prints it.
    pub fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DestinationTestResult {
    pub error: Option<BackupError>,
    pub error_detail: Option<String>,
    /// The key an SFTP host showed, pinned or not.
    pub host_key: Option<HostKey>,
}

/// A stored copy as the retention sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoredCopy {
    pub id: i64,
    pub stored_at: Timestamp,
}

/// The copies the retention lets go, by id: those neither among the `keep_last` most recent nor
/// younger than `keep_days`. The most recent copy always stays.
pub fn expired(copies: &[StoredCopy], retention: Retention, now: Timestamp) -> Vec<i64> {
    let mut newest_first = copies.to_vec();
    newest_first.sort_by_key(|copy| Reverse((copy.stored_at, copy.id)));
    newest_first
        .iter()
        .enumerate()
        .skip(1)
        .filter(|(position, copy)| {
            let by_count = retention
                .keep_last
                .is_some_and(|last| *position < usize::try_from(last).unwrap_or(usize::MAX));
            let by_age = retention.keep_days.is_some_and(|days| {
                now.duration_since(copy.stored_at) < SignedDuration::from_hours(i64::from(days) * 24)
            });
            !by_count && !by_age
        })
        .map(|(_, copy)| copy.id)
        .collect()
}

/// A connection to the database, as a connection string or the stored source gives it.
#[derive(Clone, PartialEq, Eq)]
pub struct DatabaseConnection {
    pub host: String,
    pub port: u16,
    /// A Unix socket to use instead of TCP.
    pub socket: Option<String>,
    pub user: String,
    pub password: Secret,
    pub database: String,
}

impl fmt::Debug for DatabaseConnection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DatabaseConnection")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("socket", &self.socket)
            .field("user", &self.user)
            .field("database", &self.database)
            .finish_non_exhaustive()
    }
}

/// Why a connection string cannot be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionProblem {
    NoDatabase,
    Port,
    Shape,
}

impl fmt::Display for ConnectionProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NoDatabase => "the connection string names no database",
            Self::Port => "the connection string's port is not a port",
            Self::Shape => "the connection string is neither a mysql:// address nor key=value pairs",
        })
    }
}

/// Reads `mysql_connection_string` the way oxmysql and mysql-async do: a `mysql://` (or
/// `mariadb://`) address with its user and password percent-encoded, or `key=value` pairs split by
/// `;`, keys in any case and with the synonyms they accept. The host defaults to `localhost`, the
/// port to 3306.
pub fn parse_connection_string(text: &str) -> Result<DatabaseConnection, ConnectionProblem> {
    let text = text.trim();
    let lower = text.to_ascii_lowercase();
    if lower.starts_with("mysql://") || lower.starts_with("mariadb://") {
        parse_address(text)
    } else if text.contains('=') {
        parse_pairs(text)
    } else {
        Err(ConnectionProblem::Shape)
    }
}

fn parse_address(text: &str) -> Result<DatabaseConnection, ConnectionProblem> {
    let (_, rest) = text.split_once("://").ok_or(ConnectionProblem::Shape)?;
    let (authority, rest) = rest.split_once('/').unwrap_or((rest, ""));
    let database = rest.split(['?', '#']).next().unwrap_or_default();
    let (credentials, host) = authority.rsplit_once('@').unwrap_or(("", authority));
    let (user, password) = credentials.split_once(':').unwrap_or((credentials, ""));
    let (host, port) = split_port(host)?;
    let database = percent_decoded(database).ok_or(ConnectionProblem::Shape)?;
    if database.is_empty() {
        return Err(ConnectionProblem::NoDatabase);
    }
    Ok(DatabaseConnection {
        host: if host.is_empty() {
            "localhost".to_owned()
        } else {
            host
        },
        port,
        socket: None,
        user: percent_decoded(user).ok_or(ConnectionProblem::Shape)?,
        password: Secret::new(percent_decoded(password).ok_or(ConnectionProblem::Shape)?),
        database,
    })
}

/// `host`, `host:port` or `[v6]:port`.
fn split_port(authority: &str) -> Result<(String, u16), ConnectionProblem> {
    let (host, port) = if let Some(rest) = authority.strip_prefix('[') {
        let (host, after) = rest.split_once(']').ok_or(ConnectionProblem::Shape)?;
        (host, after.strip_prefix(':'))
    } else {
        match authority.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        }
    };
    let port = match port {
        Some(port) => port_number(port)?,
        None => 3306,
    };
    Ok((host.to_owned(), port))
}

fn port_number(text: &str) -> Result<u16, ConnectionProblem> {
    match text.trim().parse::<u16>() {
        Ok(0) | Err(_) => Err(ConnectionProblem::Port),
        Ok(port) => Ok(port),
    }
}

fn parse_pairs(text: &str) -> Result<DatabaseConnection, ConnectionProblem> {
    let mut connection = DatabaseConnection {
        host: "localhost".to_owned(),
        port: 3306,
        socket: None,
        user: String::new(),
        password: Secret::new(""),
        database: String::new(),
    };
    for pair in text.split(';') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        let key: String = key
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
            .to_ascii_lowercase();
        let value = value.trim();
        match key.as_str() {
            "host" | "hostname" | "ip" | "server" | "datasource" | "addr" | "address" => {
                value.clone_into(&mut connection.host);
            }
            "port" => connection.port = port_number(value)?,
            "user" | "userid" | "username" | "uid" => value.clone_into(&mut connection.user),
            "password" | "pwd" | "pass" => connection.password = Secret::new(value),
            "database" | "db" | "initialcatalog" => value.clone_into(&mut connection.database),
            "socketpath" | "socket" => {
                connection.socket = Some(value.to_owned()).filter(|path| !path.is_empty());
            }
            _ => {}
        }
    }
    if connection.database.is_empty() {
        return Err(ConnectionProblem::NoDatabase);
    }
    Ok(connection)
}

fn percent_decoded(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let pair = text.get(index + 1..index + 3)?;
            decoded.push(u8::from_str_radix(pair, 16).ok()?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

/// The file name of a run's backup: the database's name kept to safe characters, the time the
/// run started, the run.
pub fn file_name(database: &str, started_at: Timestamp, run: i64) -> String {
    let safe: String = database
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(MAX_NAME_CHARS)
        .collect();
    format!("{safe}-{}-{run}.sql.gz", started_at.strftime("%Y%m%dT%H%M%SZ"))
}

/// Cuts a detail to what a run keeps.
pub fn detail(text: &str) -> String {
    let text: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(MAX_DETAIL_CHARS)
        .collect();
    text.trim().to_owned()
}

fn text_problem(text: &str, max: usize) -> Option<FieldProblem> {
    if text.trim().is_empty() {
        Some(FieldProblem::Required)
    } else if text.chars().count() > max {
        Some(FieldProblem::TooLong)
    } else if text.chars().any(char::is_control) {
        Some(FieldProblem::Invalid)
    } else {
        None
    }
}

/// A user name, a database or a key id: no spaces, no control characters.
fn word_problem(text: &str, max: usize) -> Option<FieldProblem> {
    text_problem(text, max).or_else(|| {
        text.chars()
            .any(char::is_whitespace)
            .then_some(FieldProblem::Invalid)
    })
}

fn host_problem(host: &str) -> Option<FieldProblem> {
    word_problem(host, MAX_HOST_CHARS).or_else(|| {
        host.contains(['/', '@', '?', '#'])
            .then_some(FieldProblem::Invalid)
    })
}

fn path_problem(path: &str) -> Option<FieldProblem> {
    if path.is_empty() {
        Some(FieldProblem::Required)
    } else if path.chars().count() > MAX_PATH_CHARS {
        Some(FieldProblem::TooLong)
    } else if path.chars().any(char::is_control)
        || Path::new(path)
            .components()
            .any(|component| component == Component::ParentDir)
    {
        Some(FieldProblem::Invalid)
    } else if !Path::new(path).is_absolute() {
        Some(FieldProblem::NotAbsolute)
    } else {
        None
    }
}

/// `https://host[:port]`, a trailing `/` allowed; `http` only to this host.
fn endpoint_problem(endpoint: &str) -> Option<FieldProblem> {
    let (secure, rest) = if let Some(rest) = endpoint.strip_prefix("https://") {
        (true, rest)
    } else if let Some(rest) = endpoint.strip_prefix("http://") {
        (false, rest)
    } else {
        return Some(FieldProblem::Invalid);
    };
    let authority = rest.strip_suffix('/').unwrap_or(rest);
    if authority.is_empty()
        || authority.len() > MAX_HOST_CHARS + 6
        || authority.contains(['/', '?', '#', '@'])
        || authority.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Some(FieldProblem::Invalid);
    }
    let host = match authority.strip_prefix('[') {
        Some(rest) => rest.split(']').next().unwrap_or_default(),
        None => authority.rsplit_once(':').map_or(authority, |(host, _)| host),
    };
    let loopback = host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback());
    (!secure && !loopback).then_some(FieldProblem::NotEncrypted)
}

/// S3's rules for a bucket's name: 3 to 63 lowercase letters, digits, dots and dashes, starting
/// and ending with a letter or a digit.
fn bucket_fits(bucket: &str) -> bool {
    let bytes = bucket.as_bytes();
    (3..=63).contains(&bytes.len())
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'.' || *byte == b'-')
        && bytes.first().is_some_and(u8::is_ascii_alphanumeric)
        && bytes.last().is_some_and(u8::is_ascii_alphanumeric)
}

#[cfg(test)]
mod tests;
