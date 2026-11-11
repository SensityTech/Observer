//! Observer's state database: SQLite with versioned migrations, backups, and the repositories the
//! services build their transactions from. Repositories hold SQL only; the rules live in
//! `observer-app`.

pub mod allowlist;
pub mod artifacts;
pub mod audit;
mod backup;
pub mod backups;
pub mod bans;
pub mod cfg_versions;
pub mod changes;
mod database;
pub mod debug;
pub mod diagnostics;
pub mod entities;
pub mod features;
pub mod files;
pub mod launch;
pub mod login_failures;
pub mod maintenance;
pub mod map;
pub mod metrics;
mod migrations;
pub mod players;
pub mod schedule;
pub mod sessions;
pub mod settings;
pub mod setup_tokens;
mod side;
mod time;
pub mod tools;
pub mod users;

pub use backup::{BackupKind, restore as restore_backup};
pub use changes::ChangesDatabase;
pub use database::{Database, Inspection, StoreError};
pub use metrics::MetricsDatabase;
pub use rusqlite::{Connection, Error as RusqliteError, Transaction};
