//! Observer's own updates: the releases published on GitHub, the check that finds a newer one, the
//! install that downloads, verifies and swaps the binary, and how the last one ended.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::artifacts::DownloadProgress;

/// The repository whose releases Observer installs.
pub const REPOSITORY: &str = "SensityTech/Observer";

/// The release notes the panel gets, at most: GitHub caps a release's body at 125,000 characters.
pub const MAX_NOTES_BYTES: usize = 64 * 1024;

/// The checksums of a release's archives, as `sha256sum` writes them.
pub const SUMS: &str = "SHA256SUMS";
/// The Ed25519 signature of [`SUMS`] by the release key, 64 bytes.
pub const SIGNATURE: &str = "SHA256SUMS.sig";

/// What a release names its archive for a system (`windows`, `linux`) and an architecture
/// (`x86_64`): `cargo xtask dist` writes it, the updater looks for it.
pub fn archive_name(version: &str, os: &str, arch: &str) -> String {
    let extension = if os == "windows" { "zip" } else { "tar.gz" };
    format!("observer-{version}-{os}-{arch}.{extension}")
}

/// The executable an archive holds for a system.
pub fn executable_name(os: &str) -> &'static str {
    if os == "windows" {
        "observer.exe"
    } else {
        "observer"
    }
}

/// `GET /api/updates`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpdatesView {
    /// The version this Observer runs.
    pub current: String,
    /// When GitHub last answered a check.
    pub checked_at: Option<Timestamp>,
    pub checking: bool,
    /// Why the last check got no answer, until one does.
    pub check_failure: Option<CheckFailure>,
    /// The newest stable release, when it is newer than this Observer.
    pub available: Option<Release>,
    /// The install under way, or the last one since Observer started.
    pub job: Option<UpdateJob>,
    /// How the last install that restarted Observer ended.
    pub last: Option<UpdateOutcome>,
    /// What keeps this Observer from installing a release, whatever the release.
    pub blocked: Option<UpdateBlock>,
    /// The next restart an install can wait for: the schedule's, or a single one planned, while
    /// FXServer runs.
    pub next_restart_at: Option<Timestamp>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Release {
    /// `0.2.0`, from the tag `v0.2.0`.
    pub version: String,
    pub name: String,
    pub published_at: Option<Timestamp>,
    /// Markdown, as the release says it, cut at [`MAX_NOTES_BYTES`].
    pub notes: String,
    pub notes_cut: bool,
    /// The release's page on GitHub.
    pub url: String,
    /// The archive for this platform, when the release has one.
    pub asset: Option<ReleaseAsset>,
    /// Whether the release carries a signature of its checksums.
    pub signed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct ReleaseAsset {
    pub name: String,
    pub size: u64,
}

/// Why a check got no answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "code", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum CheckFailure {
    /// GitHub could not be reached.
    Unreachable,
    HttpStatus {
        status: u16,
    },
    /// GitHub lists no release: none was published yet, or the repository is private.
    NoRelease,
    /// GitHub allows 60 checks an hour to an address without an account.
    RateLimited {
        until: Option<Timestamp>,
    },
    /// The answer was not the shape Observer reads.
    Malformed,
}

/// What keeps this Observer from installing any release.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum UpdateBlock {
    /// Built from source rather than a release: it is updated by building it again.
    DevelopmentBuild,
    /// Built without the key releases are signed with, so it cannot tell a release is genuine.
    NoReleaseKey,
    /// The account running Observer cannot write next to its executable.
    ReadOnly,
}

/// When an install takes over: never while players are told nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum UpdateWhen {
    /// Once verified: FXServer stops with the usual notice, Observer restarts, FXServer comes back.
    Now,
    /// At the next restart the schedule makes, one-offs included: resources hear its warnings.
    ScheduledRestart,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpdateJob {
    pub version: String,
    pub when: UpdateWhen,
    pub step: UpdateStep,
    pub state: UpdateJobState,
    pub started_by: String,
    pub started_at: Timestamp,
    pub finished_at: Option<Timestamp>,
    pub download: Option<DownloadProgress>,
    /// The restart it waits for, once verified.
    pub restart_at: Option<Timestamp>,
    pub failure: Option<UpdateFailure>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum UpdateStep {
    Download,
    /// The signature, the checksums, the archive and a run of the new executable.
    Verify,
    /// Verified, until the restart it waits for.
    Wait,
    /// Observer stops and swaps its executable: nothing can stop it any more.
    Restart,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum UpdateJobState {
    Running,
    Failed,
    Cancelled,
}

/// Why an install stopped before Observer restarted, as a code; the log has the full text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "code", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum UpdateFailure {
    Unreachable,
    HttpStatus {
        status: u16,
    },
    /// The archive's length was not announced, or not the one GitHub lists.
    BadLength,
    Truncated,
    TooLarge,
    /// The release carries no signature of its checksums.
    Unsigned,
    /// The checksums are not signed with the key this Observer knows.
    BadSignature,
    /// The signed checksums do not name the archive.
    NotListed,
    /// The archive is not the one the signed checksums describe.
    ChecksumMismatch,
    /// The archive is not the one GitHub says it stores.
    DigestMismatch,
    /// The archive is damaged or holds something else than Observer.
    Damaged,
    /// The new executable did not run, or named another version than the release.
    WontRun,
    /// The running executable is signed and the new one is not, or by someone else.
    Signature,
    DiskSpace {
        needed_mib: u64,
        available_mib: u64,
    },
    /// A file could not be read or written.
    Io,
    /// Observer stopped while it was under way.
    Interrupted,
    /// The restart it waited for was missed, or FXServer stopped before it.
    RestartMissed,
}

/// How an install that restarted Observer ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpdateOutcome {
    pub from: String,
    pub to: String,
    pub at: Timestamp,
    pub result: UpdateResult,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[ts(export)]
pub enum UpdateResult {
    Installed,
    /// The new version did not finish starting: the previous one is back, with its database.
    RolledBack {
        reason: RollbackReason,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum RollbackReason {
    /// It stopped with an error as it started.
    Failed,
    /// It ended without a word, or was started again before it finished starting.
    Crashed,
    /// Observer could not put the new executable in place: the previous one never stopped being.
    Swap,
}

/// `POST /api/updates/install`.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct InstallUpdate {
    /// The version the owner saw: an install of another one is refused.
    pub version: String,
    pub when: UpdateWhen,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_archive_is_named_by_its_version_system_and_architecture() {
        assert_eq!(
            archive_name("0.2.0", "windows", "x86_64"),
            "observer-0.2.0-windows-x86_64.zip"
        );
        assert_eq!(
            archive_name("0.2.0", "linux", "x86_64"),
            "observer-0.2.0-linux-x86_64.tar.gz"
        );
        assert_eq!(executable_name("windows"), "observer.exe");
        assert_eq!(executable_name("linux"), "observer");
    }
}
