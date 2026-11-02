//! Types shared by Observer's crates and exported to the web panel as TypeScript, and the rules
//! that only depend on them.
//!
//! Every type the panel reads derives `TS`; `cargo xtask bindings` regenerates
//! `web/src/api/generated` from them, along with the permission constants of the catalog.

pub mod about;
pub mod accounts;
pub mod api;
pub mod artifacts;
pub mod audit;
pub mod changes;
pub mod delegation;
pub mod diagnostics;
pub mod drops;
pub mod error;
pub mod features;
pub mod files;
pub mod launch;
pub mod live;
pub mod metrics;
pub mod permissions;
pub mod players;
pub mod schedule;
pub mod server;
pub mod server_data;
pub mod statistics;
pub mod tools;
