//! Types shared by Observer's crates and exported to the web panel as TypeScript.
//!
//! Every type the panel reads derives `TS`; `cargo xtask bindings` regenerates
//! `web/src/api/generated` from them.

pub mod api;
