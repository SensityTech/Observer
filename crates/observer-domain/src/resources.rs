//! The server's resources as the panel follows them: every resource FXServer knows in the run that
//! is up, as it stands, and what the panel may ask of one.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::changes::LiveChangeKind;
use crate::server::Delivery;

/// The resources a rescan found named at most: the count says the rest.
pub const MAX_ADDED_NAMED: usize = 20;

/// The `server.resources` topic: the resources of the run that is up, whole whenever any of it
/// changes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ResourcesView {
    pub stage: ResourcesStage,
    /// The start the list is of.
    pub generation: u32,
    /// In name order, without regard to case.
    pub resources: Vec<ResourceRow>,
    /// FXServer knows more resources than the list holds.
    pub truncated: bool,
    /// The latest rescan of this run.
    pub rescan: Option<ResourcesRescan>,
    /// A rescan asked on the page has not shown yet.
    pub rescanning: bool,
}

/// Whether there is a list to show.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ResourcesStage {
    /// No server runs.
    #[default]
    Stopped,
    /// The server runs its cfg: it lists its resources once that is done.
    Starting,
    Listed,
    /// The server runs, and its bridge listed nothing.
    Unreported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ResourceRow {
    /// As FXServer names it.
    pub name: String,
    /// The category folders it lies under, outermost first, as `[core]/[ox]`: empty at the root.
    pub folder: String,
    pub origin: ResourceOrigin,
    /// What its manifest gives as its version.
    pub version: Option<String>,
    pub state: ResourceState,
    /// When FXServer said it could not start it, for one it has not started since.
    pub failed_at: Option<Timestamp>,
    /// What FXServer warned of as it last started it.
    pub warnings: Option<ResourceWarnings>,
    /// The started resources that depend on it, the nearest first: FXServer stops them with it and
    /// starts none of them again.
    pub dependants: Vec<String>,
    /// Whether Observer can name it in a console command: it starts and stops no other.
    pub controllable: bool,
    /// What last happened to it in this run.
    pub last: Option<ResourceTurn>,
    /// An order of the page FXServer has not answered yet: no other is taken meanwhile.
    pub asked: Option<ResourceAsked>,
}

/// An order of the Resources page, by what it makes of the resource.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ResourceAsked {
    pub kind: LiveChangeKind,
    pub by: String,
}

/// Which of FXServer's two roots a resource lies under.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ResourceOrigin {
    /// The server folder's `resources`.
    Server,
    /// The build's own `system_resources`.
    Build,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ResourceState {
    Started,
    Starting,
    Stopped,
    Stopping,
    /// A state FXServer gave that is none of these.
    Unknown,
}

/// What FXServer said as it started a resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ResourceWarnings {
    /// How many warnings FXServer counted on the line that says it started.
    pub count: u32,
    /// Streamed files it finds heavy, and how many of them it calls oversized.
    pub heavy: u32,
    pub oversized: u32,
    /// Files the manifest names that the resource does not hold.
    pub missing: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ResourceTurn {
    pub at: Timestamp,
    pub kind: LiveChangeKind,
    /// Who asked for it on this page.
    pub by: Option<String>,
    /// The resource it stopped with, being one that depends on it.
    pub with: Option<String>,
}

/// What a rescan found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ResourcesRescan {
    pub at: Timestamp,
    /// Who asked for it on this page: none for a `refresh` typed in the console.
    pub by: Option<String>,
    /// The resources FXServer did not know before, by name: the first of them, when there are
    /// more than a line can say.
    pub added: Vec<String>,
    /// How many there are in all.
    pub added_count: u32,
    /// The manifests that changed among those FXServer read again, when it said.
    pub changed: Option<u32>,
}

/// `POST /api/servers/default/resources/start`, `/stop` and `/restart`.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct ResourceOrder {
    pub name: String,
    /// For a restart: also start again the resources that depend on this one, which FXServer
    /// stops with it and leaves stopped.
    #[serde(default)]
    pub dependants: bool,
}

/// What became of an order, once it was audited.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ResourceAck {
    pub delivery: Delivery,
    /// The resources that stop with this one, as known when the order was made.
    pub dependants: Vec<String>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn an_order_names_a_resource_and_restarts_what_depends_on_it_only_when_asked() {
        let order: ResourceOrder = serde_json::from_value(json!({ "name": "ox_lib" })).unwrap();
        assert_eq!((order.name.as_str(), order.dependants), ("ox_lib", false));

        let order: ResourceOrder =
            serde_json::from_value(json!({ "name": "ox_lib", "dependants": true })).unwrap();
        assert!(order.dependants);

        assert!(serde_json::from_value::<ResourceOrder>(json!({ "name": "a", "command": "quit" })).is_err());
    }

    #[test]
    fn the_view_of_a_stopped_server_is_empty() {
        assert_eq!(
            serde_json::to_value(ResourcesView::default()).unwrap(),
            json!({
                "stage": "stopped",
                "generation": 0,
                "resources": [],
                "truncated": false,
                "rescan": null,
                "rescanning": false
            })
        );
    }
}
