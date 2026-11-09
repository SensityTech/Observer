use std::fmt::{self, Write};
use std::io;
use std::path::{Path, PathBuf};

use crate::Platform;
use crate::catalog::Source;
use crate::extract::Refusal;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A source answered, but not in the shape its parser knows. Observer needs an update.
    #[error("{origin} changed format: {reason}")]
    FormatChanged { origin: Source, reason: String },
    #[error("the HTTP client cannot start: {0}")]
    Client(reqwest::Error),
    #[error("{url} is not on a host Observer downloads from, over HTTPS")]
    UrlRefused { url: String },
    #[error("{url} redirects more than {} times", crate::http::MAX_REDIRECTS)]
    TooManyRedirects { url: String },
    #[error("{url} answered HTTP {status}")]
    Status { url: String, status: u16 },
    #[error("cannot reach {url}: {reason}")]
    Network { url: String, reason: String },
    #[error("{url} is larger than the {limit_mib} MiB Observer accepts")]
    TooLarge { url: String, limit_mib: u64 },
    #[error("{url} does not announce its size, so a truncated download could not be told apart")]
    LengthMissing { url: String },
    #[error("{url} sent more bytes than it announced")]
    Overlong { url: String },
    #[error("{url} announced {announced} bytes, then sent {sent}")]
    LengthChanged { url: String, announced: u64, sent: u64 },
    #[error("the download of {url} stopped at {received} of {expected} bytes")]
    Truncated {
        url: String,
        received: u64,
        expected: u64,
    },
    #[error("{url} resumed the download at the wrong offset")]
    BadRange { url: String },
    #[error(
        "the archive of build {build} changed since it was first installed (sha256 {first}, now {now}); \
         Cfx.re publishes no checksum, so the new one is not trusted"
    )]
    ArchiveChanged { build: u32, first: String, now: String },
    #[error("{} has {available_mib} MiB free, {needed_mib} MiB are needed", .path.display())]
    DiskSpace {
        path: PathBuf,
        needed_mib: u64,
        available_mib: u64,
    },
    #[error("the archive entry `{}` {refusal}", Untrusted(.entry))]
    Unsafe { entry: String, refusal: Refusal },
    #[error("the archive is damaged, download it again: {}", Untrusted(.0))]
    Damaged(String),
    #[error("the build is incomplete: {path} {problem}")]
    Layout {
        path: &'static str,
        problem: LayoutProblem,
    },
    #[error("FXServer.exe is not signed by Rockstar Games: {0}")]
    Signature(String),
    #[error("build {0} is already installed")]
    AlreadyInstalled(u32),
    #[error("build {0} is not installed")]
    NotInstalled(u32),
    #[error("a {0} build can only be installed on {0}")]
    WrongPlatform(Platform),
    #[error("{} is a link; it must be a plain folder on the same volume as the builds", .0.display())]
    StagingLink(PathBuf),
    #[error("{}: {reason}", .path.display())]
    Manifest { path: PathBuf, reason: String },
    #[error("{}: {source}", .path.display())]
    Io { path: PathBuf, source: io::Error },
    #[error("the operating system gave no random bytes: {0}")]
    Random(getrandom::Error),
    #[error("the installation was interrupted by a shutdown")]
    Interrupted,
    #[error("the installation was cancelled")]
    Cancelled,
    #[error("{}: {refusal}", .path.display())]
    Import { path: PathBuf, refusal: ImportRefusal },
    #[error("{} stays busy: another program holds a file in it", .0.display())]
    Busy(PathBuf),
    #[error("GitHub refused more requests from this address for now")]
    RateLimited { until: Option<jiff::Timestamp> },
    #[error("GitHub's answer about the release is not one Observer reads: {}", Untrusted(.0))]
    ReleaseFormat(String),
    #[error("this build of Observer has no release key, so it cannot tell a genuine release")]
    NoReleaseKey,
    #[error("the release has no {name}")]
    NoArchive { name: String },
    #[error("the release carries no signature of its checksums")]
    Unsigned,
    #[error("the release's checksums are not signed with Observer's release key")]
    BadSignature,
    #[error("the release's signed checksums do not list {name}")]
    NotListed { name: String },
    #[error("{name} is not the archive the signed checksums describe (sha256 {downloaded}, listed {listed})")]
    ChecksumMismatch {
        name: String,
        listed: String,
        downloaded: String,
    },
    #[error("{name} is not the archive GitHub stores (sha256 {downloaded}, GitHub says {github})")]
    DigestMismatch {
        name: String,
        github: String,
        downloaded: String,
    },
    #[error("the new executable does not run as a release of Observer: {}", Untrusted(.0))]
    WontRun(String),
    #[error("the new executable is not signed as the running one is: {0}")]
    ExecutableSignature(String),
}

/// What is wrong with a file a build must have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutProblem {
    Missing,
    /// A link, or a folder where a file belongs and the other way round.
    NotPlain,
    NotExecutable,
}

impl fmt::Display for LayoutProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Missing => "is missing",
            Self::NotPlain => "is not a plain file or folder",
            Self::NotExecutable => "is not executable",
        })
    }
}

/// Why a folder cannot be imported as a build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportRefusal {
    NotAbsolute,
    NotAFolder,
    /// The folder is a link, or holds one that leaves it (a reparse point on Windows).
    Link,
    /// The folder is the artifacts folder or inside it.
    InsideArtifacts,
    TooManyFiles,
    TooLarge,
}

impl fmt::Display for ImportRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotAbsolute => "is not a full path",
            Self::NotAFolder => "is not a folder",
            Self::Link => "is or holds a link Observer does not copy",
            Self::InsideArtifacts => "is inside the artifacts folder already",
            Self::TooManyFiles => "has more than 20000 entries",
            Self::TooLarge => "is larger than 2 GiB",
        })
    }
}

impl Error {
    /// Whether another attempt at the same download may succeed.
    pub(crate) fn is_transient(&self) -> bool {
        match self {
            Self::Network { .. } | Self::Truncated { .. } | Self::BadRange { .. } => true,
            Self::Status { status, .. } => *status >= 500,
            _ => false,
        }
    }
}

/// Text an archive chose, such as an entry name, with its control characters escaped: printed as
/// it is, an escape sequence in it would drive the operator's terminal.
struct Untrusted<'a>(&'a str);

impl fmt::Display for Untrusted<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for character in self.0.chars() {
            if character.is_control() {
                write!(f, "{}", character.escape_default())?;
            } else {
                f.write_char(character)?;
            }
        }
        Ok(())
    }
}

/// Attaches the path an I/O error happened on, for `map_err`.
pub(crate) fn io(path: &Path) -> impl FnOnce(io::Error) -> Error + '_ {
    move |source| Error::Io {
        path: path.to_path_buf(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_damaged_archive_is_reported_with_the_control_characters_it_wrote_escaped() {
        // `tar` quotes a header's name when one of its numbers does not parse.
        let error =
            Error::Damaged("numeric field was not a number:  when getting size for \u{1b}[2Kx".to_owned());

        assert!(!error.to_string().chars().any(char::is_control), "{error:?}");
        assert!(error.to_string().ends_with(r"for \u{1b}[2Kx"), "{error}");
    }
}
