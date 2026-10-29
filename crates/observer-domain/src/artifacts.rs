//! FXServer builds as the panel sees them: what is installed, which build runs and which runs
//! next, the install job, the probation of a new build, the catalog, the cleanup and the settings.
//! The `server.artifacts` topic carries [`ArtifactsStatus`], and every change answers it.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The build below which FXServer stores the server's KVP data in another format: going back
/// under it after a newer build ran may lose that data.
pub const KVP_BUILD: u32 = 6449;
/// Build numbers have 4 to 7 digits.
pub const BUILDS: std::ops::RangeInclusive<u32> = 1000..=9_999_999;
pub const MAX_KEEP_BUILDS: u8 = 20;
pub const MAX_BRANCHES: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum BuildPlatform {
    Windows,
    Linux,
}

impl BuildPlatform {
    pub const fn host() -> Self {
        if cfg!(windows) { Self::Windows } else { Self::Linux }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::Linux => "linux",
        }
    }
}

/// The `server.artifacts` topic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ArtifactsStatus {
    pub platform: BuildPlatform,
    /// Newest number first.
    pub installed: Vec<InstalledBuild>,
    /// What the next spawn of FXServer runs.
    pub selected: Option<u32>,
    /// Waiting for the next scheduled restart, which makes it the selected build.
    pub pending: Option<PendingBuild>,
    /// What the current run of FXServer was started from.
    pub running: Option<u32>,
    pub last_healthy: Option<HealthyBuild>,
    pub probation: Option<Probation>,
    /// The highest build FXServer ever ran here, which the KVP guard goes by.
    pub highest_run: Option<u32>,
    pub notice: Option<ArtifactNotice>,
    /// The job running, or the last one until the next starts.
    pub job: Option<Job>,
    pub settings: ArtifactSettings,
    /// The settings' version, null until they are first saved.
    pub version: Option<i64>,
    /// FXServer ran from a folder of the owner's before builds lived in the data folder; the
    /// owner reads its path in the launch settings.
    pub legacy_folder: bool,
    /// On Windows, the length of the artifacts folder when it leaves FXServer too little room.
    pub long_path: Option<u32>,
    pub disk: Option<DiskSpace>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct DiskSpace {
    pub free: u64,
    pub total: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InstalledBuild {
    pub build: u32,
    /// Null for a folder without a readable manifest.
    pub origin: Option<BuildOrigin>,
    pub commit: Option<String>,
    pub branch: Option<String>,
    pub installed_at: Option<Timestamp>,
    pub installed_by: Option<String>,
    pub bytes: u64,
    pub files: Option<u64>,
    pub health: BuildHealth,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum BuildOrigin {
    Download,
    Import,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum BuildHealth {
    Valid,
    Broken { reason: BrokenReason },
}

/// Why FXServer cannot start from an installed build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum BrokenReason {
    NoManifest,
    /// The manifest cannot be read, or names another build or platform.
    BadManifest,
    /// The build's folder is a link.
    Link,
    /// A file FXServer needs is missing or changed kind.
    Layout,
    /// It came from an archive other than the one first installed for this build.
    ArchiveChanged,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct PendingBuild {
    pub build: u32,
    pub by: String,
    pub at: Timestamp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct HealthyBuild {
    pub build: u32,
    /// When its probation passed, or when Observer adopted it.
    pub since: Timestamp,
}

/// A build on trial after a switch: it must get ready, then stay up for a while.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Probation {
    pub build: u32,
    /// The last healthy build when it opened, which a failure goes back to.
    pub from: Option<u32>,
    /// Null until the run on this build is ready.
    pub ready_at: Option<Timestamp>,
    pub passes_at: Option<Timestamp>,
}

/// The latest thing Observer did by itself to the builds, which stays until a user changes the
/// selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct ArtifactNotice {
    pub kind: NoticeKind,
    pub build: u32,
    /// The build FXServer went back to, or the one chosen since that starts next.
    pub other: Option<u32>,
    pub reason: NoticeReason,
    pub at: Timestamp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum NoticeKind {
    /// A build failed its probation and FXServer went back to the last healthy one.
    RolledBack,
    /// A build failed its probation and FXServer kept it.
    ProbationFailed,
    /// The pending build could not be applied at the scheduled restart.
    PendingDropped,
}

impl NoticeKind {
    pub const ALL: [Self; 3] = [Self::RolledBack, Self::ProbationFailed, Self::PendingDropped];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RolledBack => "rolledBack",
            Self::ProbationFailed => "probationFailed",
            Self::PendingDropped => "pendingDropped",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum NoticeReason {
    Crashed,
    Hung,
    /// The automatic return is off.
    Disabled,
    NoHealthyBuild,
    Broken,
    /// Going back would cross the KVP build.
    Kvp,
    NotInstalled,
    /// Another build was chosen while the failed one was on probation: that one starts next.
    Replaced,
}

impl NoticeReason {
    pub const ALL: [Self; 8] = [
        Self::Crashed,
        Self::Hung,
        Self::Disabled,
        Self::NoHealthyBuild,
        Self::Broken,
        Self::Kvp,
        Self::NotInstalled,
        Self::Replaced,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Crashed => "crashed",
            Self::Hung => "hung",
            Self::Disabled => "disabled",
            Self::NoHealthyBuild => "noHealthyBuild",
            Self::Broken => "broken",
            Self::Kvp => "kvp",
            Self::NotInstalled => "notInstalled",
            Self::Replaced => "replaced",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|reason| reason.as_str() == name)
    }
}

/// An install or an import, from its start until the next job starts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Job {
    pub id: u32,
    pub kind: JobKind,
    pub build: u32,
    pub step: JobStep,
    pub state: JobState,
    pub started_by: String,
    pub started_at: Timestamp,
    pub finished_at: Option<Timestamp>,
    pub download: Option<DownloadProgress>,
    pub entries: Option<EntryProgress>,
    pub cancel_requested: bool,
    pub failure: Option<InstallFailure>,
    pub warnings: Vec<JobWarning>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum JobKind {
    Install,
    Import,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum JobStep {
    PreCheck,
    Download,
    Verify,
    Extract,
    Copy,
    Validate,
    Commit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum JobState {
    Running,
    Installed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct DownloadProgress {
    pub received: u64,
    pub total: u64,
    /// Bytes per second over the last five seconds.
    pub rate: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct EntryProgress {
    pub done: u64,
    /// Unknown for an xz archive, which has no index.
    pub total: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum JobWarning {
    LongPath,
    FreeSpaceUnknown,
}

/// Why a job failed, as a code with its parameters; Observer's log has the full text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "code", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum InstallFailure {
    /// Cfx.re could not be reached.
    Unreachable,
    HttpStatus {
        status: u16,
    },
    ArchiveTooLarge,
    /// The archive's length was not announced, or changed.
    BadLength,
    Truncated,
    /// The archive differs from the one first installed for this build.
    ArchiveChanged,
    DiskSpace {
        needed_mib: u64,
        available_mib: u64,
    },
    UnsafeEntry {
        refusal: String,
    },
    Damaged,
    Layout {
        path: String,
        problem: String,
    },
    /// FXServer.exe is not signed by Rockstar Games.
    Signature,
    AlreadyInstalled,
    Cancelled,
    /// Observer stopped during the job.
    Interrupted,
    /// A file could not be read or written.
    Io,
    NotAFolder,
    Link,
    InsideArtifacts,
    TooManyFiles,
    TooLarge,
}

/// `GET /api/servers/default/artifacts/catalog`: the builds of the host's platform.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CatalogView {
    pub platform: BuildPlatform,
    pub fetched_at: Timestamp,
    /// Newest first.
    pub builds: Vec<CatalogEntry>,
    pub warnings: Vec<CatalogWarning>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "separate facts the catalog states about a build"
)]
pub struct CatalogEntry {
    pub build: u32,
    pub commit: Option<String>,
    pub branch: Option<String>,
    pub recommended: bool,
    pub latest: bool,
    pub published_at: Option<Timestamp>,
    pub end_of_support: Option<PolicyDay>,
    pub end_of_life: Option<PolicyDay>,
    pub support: SupportLevel,
    pub availability: Availability,
    /// A source lists it now; otherwise Observer saw it listed before.
    pub listed: bool,
    pub installed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum PolicyDay {
    At { at: Timestamp },
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum SupportLevel {
    Supported,
    EndOfSupport,
    /// Players can no longer join a server running it.
    EndOfLife,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum Availability {
    Downloadable {
        size: Option<u64>,
    },
    /// Only the support policy names it.
    NoCommit,
    /// No archive of it for this platform.
    NoArchive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum CatalogWarning {
    /// The source failed; its answer from `since` stands in for it.
    Stale { source: CatalogSource, since: Timestamp },
    /// The source failed and had no answer younger than a day.
    Missing { source: CatalogSource },
    /// The docs page and the changelogs API name different commits for the build.
    CommitMismatch { build: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum CatalogSource {
    DocsPage,
    Changelog,
    Branch { name: String },
}

/// `GET /api/servers/default/artifacts/catalog`.
#[derive(Debug, Clone, Copy, Default, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct CatalogQuery {
    /// Fetch again rather than answer from a listing younger than ten minutes.
    #[serde(default)]
    #[ts(optional)]
    pub refresh: Option<bool>,
}

/// `POST /api/servers/default/artifacts/install`.
#[derive(Debug, Clone, Copy, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct InstallBuild {
    pub build: u32,
    /// Install a build past its end of life anyway.
    #[serde(default)]
    pub allow_end_of_life: bool,
}

/// `POST /api/servers/default/artifacts/import`.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct ImportBuild {
    /// The full path of a build folder on this host.
    pub folder: String,
    /// A folder says nothing reliable about its build: the owner names it.
    pub build: u32,
}

/// `PUT /api/servers/default/artifacts/selection`.
#[derive(Debug, Clone, Copy, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct SelectBuild {
    pub build: u32,
    pub apply: ApplyMode,
    #[serde(default)]
    pub confirm_kvp_rollback: bool,
    #[serde(default)]
    pub allow_end_of_life: bool,
}

/// When a build replaces the one FXServer runs: never while it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ApplyMode {
    /// A restart now, which also takes the right to control the server.
    Now,
    /// Whatever starts FXServer next: a start, a restart, a recovery, Observer's own start.
    NextStart,
    /// The next restart the schedule makes, one-offs included.
    ScheduledRestart,
}

/// `GET /api/servers/default/artifacts/cleanup`: what a cleanup would remove now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct CleanupPreview {
    pub remove: Vec<CleanupBuild>,
    pub keep: Vec<KeptBuild>,
    pub freed: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct CleanupBuild {
    pub build: u32,
    pub bytes: u64,
    /// The catalog still offers it, so it can be installed again.
    pub reinstallable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct KeptBuild {
    pub build: u32,
    /// Empty for a build kept within the quota.
    pub protected: Vec<Protection>,
}

/// Why a build may not be deleted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Protection {
    Selected,
    Pending,
    Running,
    LastHealthy,
    Probation,
}

/// `POST /api/servers/default/artifacts/cleanup`: the builds of the preview the user saw.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct CleanupRequest {
    pub builds: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct CleanupResult {
    pub deleted: Vec<u32>,
    pub skipped: Vec<SkippedBuild>,
    pub freed: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct SkippedBuild {
    pub build: u32,
    pub reason: SkipReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum SkipReason {
    /// A process still runs from it.
    InUse,
    /// Another program kept holding one of its files.
    Busy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct ArtifactSettings {
    /// How many builds a cleanup keeps besides the ones in use.
    pub keep_builds: u8,
    /// Go back to the last healthy build when a new one fails its probation.
    pub auto_rollback: bool,
    /// Test branches of runtime.fivem.net the catalog also lists, `master` aside.
    pub branches: Vec<String>,
}

impl Default for ArtifactSettings {
    fn default() -> Self {
        Self {
            keep_builds: 3,
            auto_rollback: true,
            branches: Vec::new(),
        }
    }
}

/// `PUT /api/servers/default/artifacts/settings`.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct ArtifactSettingsChange {
    pub settings: ArtifactSettings,
    /// The version the change was made from, null before the first save.
    pub version: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct ArtifactSettingsIssue {
    pub field: ArtifactSettingsField,
    /// The branch's position in the list.
    pub index: Option<usize>,
    pub problem: ArtifactSettingsProblem,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ArtifactSettingsField {
    KeepBuilds,
    Branches,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ArtifactSettingsProblem {
    /// `keepBuilds` is not 1 to 20.
    OutOfRange,
    /// More than ten branches.
    TooMany,
    /// Not a branch name runtime.fivem.net could have.
    Invalid,
    /// `master` is always listed.
    Master,
    Duplicate,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn stored_names_match_the_wire_names() {
        for kind in NoticeKind::ALL {
            assert_eq!(serde_json::to_value(kind).unwrap(), kind.as_str());
            assert_eq!(NoticeKind::from_name(kind.as_str()), Some(kind));
        }
        for reason in NoticeReason::ALL {
            assert_eq!(serde_json::to_value(reason).unwrap(), reason.as_str());
            assert_eq!(NoticeReason::from_name(reason.as_str()), Some(reason));
        }
    }

    #[test]
    fn a_failure_is_a_code_with_its_parameters() {
        let failures = [
            (InstallFailure::Signature, json!({ "code": "signature" })),
            (
                InstallFailure::HttpStatus { status: 503 },
                json!({ "code": "httpStatus", "status": 503 }),
            ),
            (
                InstallFailure::DiskSpace {
                    needed_mib: 450,
                    available_mib: 12,
                },
                json!({ "code": "diskSpace", "neededMib": 450, "availableMib": 12 }),
            ),
        ];
        for (failure, wire) in failures {
            assert_eq!(serde_json::to_value(failure).unwrap(), wire);
        }
    }

    #[test]
    fn a_selection_names_how_it_applies_and_refuses_what_it_does_not_know() {
        let selection: SelectBuild =
            serde_json::from_value(json!({ "build": 37900, "apply": "scheduledRestart" })).unwrap();

        assert_eq!(selection.apply, ApplyMode::ScheduledRestart);
        assert!(!selection.confirm_kvp_rollback);
        assert!(serde_json::from_value::<SelectBuild>(json!({ "build": 37900, "apply": "later" })).is_err());
        assert!(
            serde_json::from_value::<SelectBuild>(json!({ "build": 37900, "apply": "now", "hot": true }))
                .is_err()
        );
    }

    #[test]
    fn the_catalog_dates_say_when_or_never() {
        let entry = json!(PolicyDay::At {
            at: "2027-01-26T12:08:14Z".parse().unwrap()
        });

        assert_eq!(entry, json!({ "kind": "at", "at": "2027-01-26T12:08:14Z" }));
        assert_eq!(json!(PolicyDay::Never), json!({ "kind": "never" }));
    }
}
