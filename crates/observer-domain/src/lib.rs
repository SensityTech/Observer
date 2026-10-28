//! Types shared by Observer's crates and exported to the web panel as TypeScript, and the rules
//! that only depend on them.
//!
//! Every type the panel reads derives `TS`; `cargo xtask bindings` regenerates
//! `web/src/api/generated` from them.

pub mod api;
pub mod delegation;
pub mod permissions;
