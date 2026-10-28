//! FXServer builds: the catalog Cfx.re publishes, the download, verification and extraction that
//! install one under `<data>/artifacts/<build>/`, the import of a build folder by copy, and the
//! scan and deletion of what is installed. Only the Cfx.re hosts are ever contacted, and nothing
//! appears under a build's final name until it is complete and valid.

mod archive;
pub mod catalog;
mod disk;
mod download;
mod error;
mod extract;
mod folder;
mod http;
mod import;
mod install;
mod layout;
#[cfg(any(test, feature = "test-support"))]
pub mod samples;
#[cfg(any(test, feature = "test-support"))]
pub mod test_server;

pub use archive::{ArchiveKind, Branch, CommitSha, Platform};
pub use error::{Error, ImportRefusal, LayoutProblem};
pub use extract::Refusal;
pub use folder::{Broken, Health, Scanned, inspect};
pub use http::Http;
pub use install::{InstallRequest, Installed, Installer, Progress};
pub use layout::{Manifest, Origin, RUNTIME_PATHS};
