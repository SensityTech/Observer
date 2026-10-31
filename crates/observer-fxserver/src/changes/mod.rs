//! What a server runs from, read from the host at each start: the resources under `resources/` the
//! way FXServer finds them, every file's keyed digest, the text of what gets line diffs, and each
//! resource's manifest, git head and npm packages. A server that never boots still gets its record.
//! Nothing here writes to the server folder or runs anything from it.

mod content;
pub mod diff;
mod git;
mod manifest;
pub mod mask;
mod packages;
pub mod tree;
mod walk;

use std::path::PathBuf;

use observer_domain::changes::DEFAULT_EXTENSIONS;

pub use content::{FileRead, Rate, Treatment, decode, read};
pub use walk::{Entry, Found, ListError, Listing, ResourceListing, Shortened, Stat, list};

/// The work was cancelled: a newer start, or Observer stopping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("cancelled")]
pub struct Cancelled;

/// The two keys of an instance, random and never sent: one keys the digests of text and hidden
/// files, so that a stored digest lets no one check a guess at a short secret, and one keys the
/// tags that tell a hidden secret changed.
#[derive(Clone)]
pub struct Keys {
    pub content: [u8; 32],
    pub tag: [u8; 32],
}

impl std::fmt::Debug for Keys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Keys")
    }
}

#[cfg(test)]
impl Keys {
    pub(crate) fn fixed() -> Self {
        Self {
            content: [1; 32],
            tag: [2; 32],
        }
    }
}

/// What a scan reads and how far it goes.
#[derive(Debug, Clone)]
pub struct ScanSettings {
    /// Extensions whose files may be text, lower case and without the dot.
    pub extensions: Vec<String>,
    /// The owner's globs of what is never walked.
    pub excluded: Vec<String>,
    /// The owner's globs of files whose contents are never stored.
    pub hidden: Vec<String>,
    /// Observer's own folders, never read even through a link: its data folder and the builds.
    pub never_read: Vec<PathBuf>,
    pub limits: Limits,
}

impl Default for ScanSettings {
    fn default() -> Self {
        Self {
            extensions: DEFAULT_EXTENSIONS
                .iter()
                .map(|&extension| extension.to_owned())
                .collect(),
            excluded: Vec::new(),
            hidden: Vec::new(),
            never_read: Vec::new(),
            limits: Limits::default(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Files listed in one snapshot, `node_modules` aside.
    pub files: usize,
    pub resources: usize,
    /// Folders below `resources/`, categories included.
    pub depth: usize,
    /// Bytes of a path from the data folder.
    pub path: usize,
    /// The largest text whose contents are kept.
    pub max_text: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            files: 300_000,
            resources: 5000,
            depth: 32,
            path: 1024,
            max_text: 1024 * 1024,
        }
    }
}
