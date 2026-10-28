//! FXServer builds: the catalog Cfx.re publishes for each platform. Only the Cfx.re hosts are ever
//! contacted.

mod archive;
pub mod catalog;
mod error;
mod http;
mod install;
#[cfg(test)]
mod test_server;

pub use archive::{ArchiveKind, Branch, CommitSha, Platform};
pub use error::Error;
pub use http::Http;
pub use install::InstallRequest;
