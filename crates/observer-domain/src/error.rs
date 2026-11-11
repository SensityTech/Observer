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
    /// Only a start whose cfg still runs can be given more time.
    ServerNotStarting,
    /// The launch's cfg is not edited from the panel. Params: `reason`, a
    /// [`CfgRefusal`](crate::cfg::CfgRefusal).
    CfgNotEditable,
    /// The cfg is no longer the file the edit started from: it is read again before a save.
    CfgChanged,
    /// The text is not written as a cfg. Params: `reason`, `unknownMark` for a secret's mark that
    /// stands for nothing, `control` for a NUL, `tooLarge`.
    InvalidCfgText,
    CfgVersionNotFound,
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
    /// No player has this id: never seen, forgotten, or past the profiles' retention.
    PlayerNotFound,
    /// A players search never takes an address: the panel must not tell which licences used an
    /// address someone already knows.
    AddressSearch,
    /// The search names a kind of identifier with a value that is not one of that kind.
    InvalidSearch,
    /// A retention is out of its range, or would outlive what it hangs on.
    InvalidPlayersSettings,
    /// txAdmin's players database was refused, whole. Params: `problem`, a `PlayersFileProblem`.
    InvalidPlayersFile,
    /// The drop retention is out of its range. Params: `min`, `max`.
    InvalidDropsSettings,
    /// No ban has this code.
    BanNotFound,
    /// The ban already ended or was lifted.
    BanNotActive,
    /// A ban cannot be made or edited as asked. Params: `field`, `reason`, `minutes` or
    /// `identifiers` (none left once checked, or too many).
    InvalidBan,
    /// The refusal message is empty or too long, the tokens needed are out of range, a template
    /// has no reason or a length no ban takes, or there are too many templates. Params:
    /// `maxMessage`, `maxTokens`, `maxTemplates`.
    InvalidBansSettings,
    /// No player holds this server id now, or another player than the one named does.
    PlayerNotConnected,
    /// A kick needs a reason of 1 to `max` characters. Params: `max`.
    InvalidKick,
    /// The kick message is empty or too long. Params: `maxMessage`.
    InvalidKickSettings,
    /// An announcement or a direct message needs a text of 1 to `max` characters. Params: `max`.
    InvalidMessage,
    /// No request waits under this code: answered already, or never made.
    AllowlistRequestNotFound,
    /// No licence is approved under this id.
    ApprovalNotFound,
    /// The licence is approved already.
    AlreadyApproved,
    /// A licence cannot be approved as asked. Params: `field`, `license` or `note`; `maxNote`.
    InvalidApproval,
    /// The allowlist's message is empty or too long. Params: `maxMessage`.
    InvalidAllowlistSettings,
    /// No one is on the maintenance list under this id.
    PassNotFound,
    /// The licence is on the maintenance list already.
    PassExists,
    /// A licence cannot go on the maintenance list as asked. Params: `field`, `license` or
    /// `note`; `maxNote`.
    InvalidPass,
    /// The maintenance list is full. Params: `max`.
    TooManyPasses,
    /// The maintenance message is empty or too long, or the server's maintenance name too long.
    /// Params: `maxMessage`, `maxName`.
    InvalidMaintenanceSettings,
    /// The live map's image was refused. Params: `problem`, `notLocal`, `missing`, `unreadable`,
    /// `tooLarge` or `notAnImage`.
    InvalidMapImage,
    /// The live map has no image.
    NoMapImage,
    /// The thresholds are not ordered, the cadence is not 1, 2 or 5 seconds, or the edges enclose
    /// no area.
    InvalidMapSettings,
    /// The server's entities are not listed yet: the first sweep comes a moment after someone
    /// starts looking at them.
    EntitiesNotListed,
    /// The server or its bridge started again since the list was read: its net ids name other
    /// entities now.
    EntitiesChanged,
    /// The list is too old to delete from: the server stopped answering.
    EntitiesOutOfDate,
    /// Another deletion is still under way.
    EntitiesDeleting,
    /// Params: `max`, the most entities one deletion takes.
    TooManyEntities,
    /// The cadence is not one the panel offers.
    InvalidEntitiesSettings,
    /// The file changes' database could not be opened: it was set aside, or Observer is stopping.
    ChangesUnavailable,
    /// No start or check of the server has this id.
    StartNotFound,
    /// The file's contents are never shown. Params: `glob`, the never-show name or the owner's glob.
    ContentHidden,
    /// The contents of this version are no longer kept: the budget or a purge dropped them.
    ContentNotKept,
    /// The file changed in more lines than a diff shows. Params: `lines`, `maxLines`.
    DiffTooLarge,
    /// A check of the disk, or a snapshot's job, is running.
    CheckRunning,
    /// A check of the disk ran moments ago. Params: `retryAfter`, in seconds.
    CheckTooSoon,
    /// A glob of the settings is unusable. Params: `field`, `excluded` or `hidden`; `index`.
    InvalidGlob,
    /// A fresh scan plans another fix than the one the owner confirmed, so nothing was applied.
    /// Params: `run`, the fresh run (a `ToolRun`) whose report and plan to review.
    PlanChanged,
    /// The server tools change files only while FXServer is stopped.
    ServerNotStopped,
    /// The tool does not run on this server. Params: `game`, the game the cfg sets (`rdr3`).
    ToolNotAvailable,
    /// The route belongs to a feature that does not run. Params: `feature`, a `Feature`; `held`,
    /// a `FeatureHold`, when something keeps it off whatever the owner chooses.
    FeatureDisabled,
    /// `observer.toml` turns the feature off, which only the host can undo. Params: `feature`.
    FeatureLocked,
    /// `observer.toml` names where SFTP listens, which only the host changes.
    SftpListenPinned,
    /// Not an address SFTP can listen on. Params: `reason`, `address` when it is not an IP address
    /// with a port, `port` when the port is 0.
    InvalidSftpListen,
    /// A backup of the server's database is under way: one at a time.
    BackupRunning,
    /// Params: `issues`, a list of `BackupSettingsIssue`.
    InvalidBackupSettings,
    /// Params: `issues`, a list of `DestinationIssue`.
    InvalidBackupDestination,
    /// No destination is enabled, so a backup would go nowhere.
    BackupNotConfigured,
    /// The copy is not stored: it failed, was deleted, or its destination was.
    BackupCopyUnavailable,
    /// The version asked is not the newer release Observer found: check again.
    UpdateNotAvailable,
    /// An install of a release is under way or waits for a restart: one at a time.
    UpdateRunning,
    /// No install of a release is under way.
    NoUpdate,
    /// Observer is restarting into the new release: it can no longer be stopped.
    UpdateRestarting,
    /// Params: `reason`, an `UpdateBlock`.
    UpdateBlocked,
    /// The server has not listed its resources in this run: it does once its cfg has run.
    ResourcesNotListed,
    /// FXServer listed no resource of this name in this run.
    ResourceNotFound,
    /// The resource's name holds what a console command cannot carry: Observer names a resource
    /// in a command only by ASCII letters, digits, `_`, `.` and `-`, 64 at most.
    ResourceNameRefused,
    /// FXServer has not answered the last order of the Resources page on this resource yet.
    ResourceBusy,
    /// The resource is not in the state the order needs: a start needs it stopped, a stop and a
    /// restart need it started.
    ResourceChanged,
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
