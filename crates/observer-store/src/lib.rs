//! Observer's state database: SQLite with versioned migrations, backups, and the repositories the
//! services build their transactions from. Repositories hold SQL only; the rules live in
//! `observer-app`.

pub mod audit;
mod backup;
mod database;
pub mod launch;
pub mod login_failures;
pub mod metrics;
mod migrations;
pub mod sessions;
pub mod settings;
pub mod setup_tokens;
mod time;
pub mod users;

pub use backup::BackupKind;
pub use database::{Database, Inspection, StoreError};
pub use metrics::MetricsDatabase;
pub use rusqlite::{Connection, Transaction};
