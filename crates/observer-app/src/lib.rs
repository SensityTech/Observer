//! The services that own Observer's rules: accounts and sessions, first-run setup, login
//! throttling, the audit log, how servers are launched, their supervision and their restart
//! schedule. They know nothing of HTTP; the web layer and the command line both call them.

mod accounts;
mod audit;
mod clock;
mod error;
mod passwords;
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
pub use audit::AuditLog;
pub use clock::Clock;
pub use error::Error;
pub use passwords::PasswordCost;
pub use schedule::{Schedule, ScheduleSetup, ScheduleTask, ScheduleTimings};
pub use servers::Servers;
pub use sessions::{Caller, Client, SignedIn};
pub use supervision::{ConsoleLines, LinesMissed, Running, Setup, Supervision};
