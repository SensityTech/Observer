//! The services that own Observer's rules: accounts and sessions, first-run setup, login
//! throttling, the audit log, how servers are launched, their supervision, their statistics,
//! their restart schedule, their FXServer builds, their diagnostics and the tools that repair
//! their resources. They know nothing of HTTP; the web layer and the command line both call them.

mod accounts;
mod artifacts;
mod audit;
mod backups;
mod bans;
mod changes;
mod clock;
mod diagnostics;
mod drops;
mod error;
mod features;
mod files;
mod host_file;
mod metrics;
mod own_paths;
mod passwords;
mod players;
mod schedule;
mod secrets;
mod servers;
mod sessions;
mod setup;
mod statistics;
mod supervision;
#[cfg(test)]
mod tests;
mod throttle;
mod tickets;
mod tools;
mod users;

pub use accounts::Accounts;
pub use artifacts::{ArtifactTimings, Artifacts, ArtifactsSetup, ArtifactsTask};
pub use audit::AuditLog;
pub use backups::{
    Backups, BackupsHook, BackupsSetup, BackupsTask, BackupsTimings, BeforeSpawn, Download, PRE_LAUNCH_LIMIT,
    TOOLS_LIMIT,
};
pub use bans::{Bans, BansSetup, BansTask};
pub use changes::{Changes, ChangesSetup, ChangesTask, ComparedTo, DiffOptions};
pub use clock::Clock;
pub use diagnostics::{Diagnostics, DiagnosticsSetup, DiagnosticsTask, MAX_DOWNLOAD};
pub use drops::{DayRates, Drops, DropsSetup, DropsTask, LogQuery};
pub use error::Error;
pub use features::{Features, FeaturesSetup, Switched};
pub use files::{CommitGate, Files, Grant, Group, Login, SERVER as FILES_SERVER, file_group};
pub use metrics::{DatabaseFiles, Metrics, MetricsSetup, Sampling, Sources};
pub use own_paths::{OwnPaths, paths_inside};
pub use passwords::PasswordCost;
pub use players::{Players, PlayersSetup, PlayersTask};
pub use schedule::{Schedule, ScheduleSetup, ScheduleTask, ScheduleTimings};
pub use servers::Servers;
pub use sessions::{Caller, Client, SignedIn};
pub use setup::SetupToken;
pub use statistics::Statistics;
pub use supervision::{ConsoleLines, LinesMissed, Running, Setup, Supervision};
pub use tools::{ReportError, RunResult, Target, Tools, ToolsHook, ToolsSetup, outcome_text, report};
