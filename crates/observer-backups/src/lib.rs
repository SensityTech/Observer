//! Backups of a game server's database, without Observer's state: the dump through the engine's own
//! tool, the gzip file made of it and checked by reading it back, and the destinations its copies
//! go to (a folder, SFTP, an S3-compatible store). What runs when, and the records, belong to
//! `observer-app`.

pub mod archive;
pub mod destination;
pub mod dump;

use std::io;

use observer_domain::backups::{self, BackupError};

/// Why a step failed: a code the panel turns into a message, and what the tool or the remote end
/// said, cut short and cleared of secrets.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{}{}", .error.as_str(), .detail.as_deref().map(|detail| format!(": {detail}")).unwrap_or_default())]
pub struct Failure {
    pub error: BackupError,
    pub detail: Option<String>,
}

impl Failure {
    pub const fn new(error: BackupError) -> Self {
        Self { error, detail: None }
    }

    pub fn with(error: BackupError, detail: impl AsRef<str>) -> Self {
        let detail = backups::detail(detail.as_ref());
        Self {
            error,
            detail: (!detail.is_empty()).then_some(detail),
        }
    }
}

/// `text` with every occurrence of each secret replaced, before anything cuts it short.
pub fn redact(text: &str, secrets: &[&str]) -> String {
    secrets
        .iter()
        .filter(|secret| !secret.is_empty())
        .fold(text.to_owned(), |text, secret| text.replace(secret, "[REDACTED]"))
}

impl From<io::Error> for Failure {
    fn from(error: io::Error) -> Self {
        let code = if error.kind() == io::ErrorKind::StorageFull {
            BackupError::DiskFull
        } else {
            BackupError::Io
        };
        Self::with(code, error.to_string())
    }
}
