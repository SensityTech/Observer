//! What the panel's Documentation page is built from: the integration contract as this build
//! ships it, and what the server Observer runs does of it.

use serde::Serialize;
use ts_rs::TS;

/// `GET /api/docs/integration`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct IntegrationDocs {
    /// The contract in Markdown, as this build ships it: the page is drawn from it.
    pub contract: String,
    pub contract_version: u32,
    /// Every server event the bridge raises, as resources hear it: `observer:shuttingDown`.
    pub events: Vec<String>,
    pub exports: Vec<BridgeExport>,
    /// The example resource, file by file.
    pub example: Vec<ExampleFile>,
    /// Null for an account that does not see the server's state, which takes the dashboard's
    /// permission: the contract is anyone's to read, what the server does now is not.
    pub server: Option<IntegrationServer>,
}

/// An export of the bridge, as its script declares it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct BridgeExport {
    pub name: String,
    pub parameters: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExampleFile {
    /// From the contract's own folder, as its links name it: `observer_example/server.lua`.
    pub name: String,
    pub text: String,
}

/// What the server Observer runs does of the contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct IntegrationServer {
    /// The bridge's name in the server that runs now, drawn for its start. Null while no server
    /// runs.
    pub bridge: Option<String>,
    /// That server is still starting: its bridge may not have reported yet.
    pub starting: bool,
    /// Whether the server that runs now raises txAdmin's events too. Null while no server runs.
    pub txadmin_events: Option<bool>,
    /// Whether the next start will, as the launch settings have it. Null until the server is set
    /// up.
    pub txadmin_events_next: Option<bool>,
}
