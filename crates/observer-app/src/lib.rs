//! The services that own Observer's rules: accounts and sessions, first-run setup, login
//! throttling, the audit log, how servers are launched, their supervision, their statistics,
//! their restart schedule, their FXServer builds and their diagnostics. They know nothing of HTTP;
//! the web layer and the command line both call them.

mod accounts;
mod artifacts;
mod audit;
mod changes;
mod clock;
mod diagnostics;
mod error;
mod files;
mod metrics;
mod passwords;
mod players;
mod schedule;
mod secrets;
mod servers;
mod sessions;
mod setup;
mod supervision;
#[cfg(test)]
mod tests;
mod throttle;
mod tickets;
mod users;

pub use accounts::Accounts;
pub use artifacts::{ArtifactTimings, Artifacts, ArtifactsSetup, ArtifactsTask};
pub use audit::AuditLog;
pub use changes::{Changes, ChangesSetup, ChangesTask, ComparedTo, DiffOptions};
pub use clock::Clock;
pub use diagnostics::{Diagnostics, DiagnosticsSetup, DiagnosticsTask, MAX_DOWNLOAD};
pub use error::Error;
pub use files::{Files, Grant, Group, Login, SERVER as FILES_SERVER, file_group};
pub use metrics::{DatabaseFiles, Metrics, MetricsSetup, Sampling, Sources};
pub use passwords::PasswordCost;
pub use players::{Players, PlayersSetup, PlayersTask};
pub use schedule::{Schedule, ScheduleSetup, ScheduleTask, ScheduleTimings};
pub use servers::Servers;
pub use sessions::{Caller, Client, SignedIn};
pub use setup::SetupToken;
pub use supervision::{ConsoleLines, LinesMissed, Running, Setup, Supervision};
