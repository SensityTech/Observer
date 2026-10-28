//! FXServer builds: the catalog Cfx.re publishes, and the download, verification and extraction
//! that install one under `<data>/artifacts/<build>/`. Only the Cfx.re hosts are ever contacted,
//! and nothing appears under a build's final name until it is complete and valid.

mod archive;
pub mod catalog;
mod disk;
mod download;
mod error;
mod extract;
mod http;
mod install;
mod layout;
#[cfg(test)]
mod samples;
#[cfg(test)]
mod test_server;
#[cfg(windows)]
mod windows;

pub use archive::{ArchiveKind, Branch, CommitSha, Platform};
pub use error::Error;
pub use extract::Refusal;
pub use http::Http;
pub use install::{InstallRequest, Installed, Installer, Progress};
pub use layout::Manifest;
