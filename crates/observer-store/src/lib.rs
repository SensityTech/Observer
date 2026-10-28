//! Observer's state database: SQLite with versioned migrations and backups.

mod backup;
mod database;
mod migrations;

pub use backup::BackupKind;
pub use database::{Database, StoreError};
