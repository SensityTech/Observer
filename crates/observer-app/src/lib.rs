//! The services that own Observer's rules: accounts and sessions, first-run setup, login
//! throttling, the audit log and how servers are launched. They know nothing of HTTP; the web
//! layer and the command line both call them.

mod accounts;
mod audit;
mod clock;
mod error;
mod passwords;
mod secrets;
mod servers;
mod sessions;
mod setup;
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
pub use servers::Servers;
pub use sessions::{Caller, Client, SignedIn};
