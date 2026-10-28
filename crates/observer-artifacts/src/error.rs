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
    #[error("{url} is not on a Cfx.re artifact host over HTTPS")]
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
    #[error("the download of {url} stopped at {received} of {expected} bytes")]
    Truncated {
        url: String,
        received: u64,
        expected: u64,
    },
    #[error("{url} resumed the download at the wrong offset")]
    BadRange { url: String },
    #[error(
        "the archive of build {build} changed since it was first downloaded (sha256 {first}, now {now}); \
         Cfx.re publishes no checksum, so the new one is not trusted"
    )]
    ArchiveChanged { build: u32, first: String, now: String },
    #[error("{} has {available_mib} MiB free, {needed_mib} MiB are needed", .path.display())]
    DiskSpace {
        path: PathBuf,
        needed_mib: u64,
        available_mib: u64,
    },
    #[error("the archive entry `{entry}` {refusal}")]
    Unsafe { entry: String, refusal: Refusal },
    #[error("the archive is damaged, download it again: {0}")]
    Damaged(String),
    #[error("the extracted build is incomplete: {0}")]
    Layout(String),
    #[error("FXServer.exe is not signed by Rockstar Games: {0}")]
    Signature(String),
    #[error("build {0} is already installed")]
    AlreadyInstalled(u32),
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

/// Attaches the path an I/O error happened on, for `map_err`.
pub(crate) fn io(path: &Path) -> impl FnOnce(io::Error) -> Error + '_ {
    move |source| Error::Io {
        path: path.to_path_buf(),
        source,
    }
}
