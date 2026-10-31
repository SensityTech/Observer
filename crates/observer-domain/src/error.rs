use serde::Serialize;
use serde_json::{Map, Value};
use ts_rs::TS;

/// Why the API refused a request. The panel turns the code into a message; the API never sends
/// sentences.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ErrorCode {
    /// No valid session: the cookie or the session key is missing, wrong or expired.
    Unauthenticated,
    InvalidCredentials,
    /// The password was right but it was a temporary one, and it has expired.
    TemporaryPasswordExpired,
    /// Params: `retryAfter`, in seconds.
    TooManyAttempts,
    /// Too many passwords are being checked at once. Params: `retryAfter`, in seconds.
    Busy,
    /// The session already holds as many live sockets, or tickets for them, as it may: a few per
    /// browser, one for each tab of the panel.
    TooManySockets,
    PasswordChangeRequired,
    /// The action needs the password again; `POST /api/auth/reauth`, then retry.
    ReauthRequired,
    WrongPassword,
    /// Params: `permission`, the key of the missing permission.
    Forbidden,
    /// Only the owner may do this, whatever permissions anyone else holds.
    OwnerOnly,
    SelfEdit,
    OwnerProtected,
    /// Params: `permissions`, the bits the editor would grant or revoke without holding them.
    Escalation,
    Hierarchy,
    /// The account changed since it was read; reload it and try again.
    Conflict,
    NotFound,
    NameTaken,
    InvalidUsername,
    /// Params: `min`, in characters.
    PasswordTooShort,
    /// Params: `max`, in bytes.
    PasswordTooLong,
    PasswordUnchanged,
    /// Params: `permissions`, the bits the catalog does not know.
    UnknownPermissions,
    /// Params: `issues`, the `LaunchIssue`s found, one per field at most.
    InvalidLaunchSettings,
    /// FXServer cannot start before the owner saves its launch settings.
    LaunchNotConfigured,
    /// FXServer could not be started. Params: `reason`, the step that failed as a
    /// [`SpawnFailure`](crate::server::SpawnFailure); the console has the full reason.
    LaunchFailed,
    ServerRunning,
    ServerNotRunning,
    /// FXServer is stopping; it can be started again once it has.
    ServerStopping,
    /// Observer is stopping, and FXServer with it.
    ShuttingDown,
    /// Params: `reason`, a `CommandRefusal`.
    CommandRefused,
    /// Params: `retryAfter`, in seconds.
    TooManyCommands,
    AlreadySetUp,
    InvalidSetupToken,
    OriginMismatch,
    /// The request named a host Observer was not told it is reached by: an address, `localhost`
    /// and the names in `http.hosts` of `observer.toml` are.
    UnknownHost,
    JsonRequired,
    InvalidRequest,
    Internal,
    /// Params: `issues`, the `ScheduleIssue`s found.
    InvalidSchedule,
    /// The schedule has no restart ahead to skip.
    NothingToSkip,
    /// The next restart of the schedule is skipped already.
    AlreadySkipped,
    /// No skipped restart is still ahead.
    NothingToRestore,
    /// The next restart is not the schedule's.
    NothingToDelay,
    /// The delay would take the restart more than 12 hours past its time, or past the next one.
    /// Params: `maxMinutes`, how much longer it can still be delayed.
    DelayTooLong,
    /// No single restart is planned.
    NoOneOff,
    /// An install or an import runs already; one at a time.
    JobRunning,
    /// No install or import runs.
    NoJob,
    /// The job is past its validation: the build is being installed and can no longer be stopped.
    JobCommitting,
    /// No source of the catalog answered, and none answered in the last day.
    CatalogUnavailable,
    /// The catalog does not list the build, even after a refresh.
    BuildNotListed,
    /// Params: `reason`, `noCommit` when only the support policy names the build, `noArchive` when
    /// there is no archive of it for this platform.
    NotDownloadable,
    AlreadyInstalled,
    /// The build is past its end of life; the request may allow it.
    EndOfLife,
    BuildNotInstalled,
    /// Params: `reason`, a `BrokenReason`.
    BuildBroken,
    /// Going below build 6449 after a newer build ran needs a confirmation. Params: `highestRun`.
    KvpRollback,
    /// The schedule has no restart ahead to apply the build at.
    NoScheduledRestart,
    /// The build is the selected one already.
    AlreadySelected,
    NoPending,
    /// Params: `reasons`, the `Protection`s of the build.
    BuildProtected,
    /// A process still runs from the build's folder.
    BuildInUse,
    /// Params: `issues`, the `ArtifactSettingsIssue`s found.
    InvalidArtifactSettings,
    /// FXServer cannot start before a build is installed or imported and selected.
    NoBuildSelected,
    /// The folder of an import is not a path FXServer can run a build from. Params: `problem`, a
    /// `LaunchProblem`.
    InvalidImportFolder,
    CrashNotFound,
    /// The name is not one of the crash's files, or the file is gone.
    CrashFileNotFound,
    /// The file is too large to go through the panel: it stays on the host. Params: `maxBytes`.
    CrashFileTooLarge,
    /// The server folder or its cfg keeps FXServer from starting under Observer. Params: `folder`,
    /// a `FolderProblem`; or `cfg`, a `CfgFileProblem`; or `findings`, the blocking `FindingKind`s.
    SetupBlocked,
    /// A rule of a file group is unusable. Params: `index`, the rule's position; `field`, `folder`,
    /// `names`, `rights` or `maxFileSize`.
    InvalidRule,
    /// A pasted public key is unusable. Params: `reason`, `format`, `algorithm` or `certificate`.
    InvalidPublicKey,
    /// The access holds this key already.
    KeyInUse,
    /// Params: `max`, the most keys an access holds.
    TooManyKeys,
    /// A pipeline's settings are unusable. Params: `field`, `name`, `networks` or `expiry`.
    InvalidAccess,
}

/// The body of every error response.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct ApiError {
    pub code: ErrorCode,
    #[ts(type = "Record<string, unknown>")]
    pub params: Map<String, Value>,
}

impl ApiError {
    pub fn new(code: ErrorCode) -> Self {
        Self {
            code,
            params: Map::new(),
        }
    }

    #[must_use]
    pub fn with(mut self, name: &str, value: impl Into<Value>) -> Self {
        self.params.insert(name.to_owned(), value.into());
        self
    }
}

impl From<ErrorCode> for ApiError {
    fn from(code: ErrorCode) -> Self {
        Self::new(code)
    }
}
