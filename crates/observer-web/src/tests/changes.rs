//! What changed between starts, over HTTP: who reads the history, who reads contents, what a path
//! may be, and the owner's settings.

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};

use super::{OWNER_PASSWORD, Server, Session};
use crate::api::routes;
use crate::gate::Gate;
use observer_domain::permissions::Permissions;

const CHANGES_VIEW: &str = "4000";
const DASHBOARD_VIEW: &str = "1";
const STARTS: &str = "/api/servers/default/changes/starts";
const DIFF: &str = "/api/servers/default/changes/diff";
const SETTINGS: &str = "/api/servers/default/changes/settings";

impl Server {
    async fn changes_entries(&self, action: &str) -> Vec<Value> {
        self.audit.publish_new().await.unwrap();
        let owner = self.login("ada", OWNER_PASSWORD).await.session();
        let page = self
            .call(
                Method::GET,
                &format!("/api/audit?action={action}"),
                Some(&owner),
                None,
            )
            .await;
        page.json()["entries"].as_array().unwrap().clone()
    }

    async fn get_as(&self, path: &str, session: &Session) -> super::Reply {
        self.call(Method::GET, path, Some(session), None).await
    }
}

#[test]
fn the_route_table_keeps_contents_behind_the_critical_bit_and_settings_for_the_owner() {
    let gates: Vec<_> = routes()
        .into_iter()
        .filter(|route| route.path.starts_with("/servers/default/changes"))
        .map(|route| (route.method, route.path, route.gate))
        .collect();
    let view = Gate::Permission(Permissions::CHANGES_VIEW);
    let contents = Gate::PermissionReadsAudited(Permissions::CHANGES_CONTENT);

    assert_eq!(
        gates,
        [
            (Method::GET, "/servers/default/changes", view),
            (Method::GET, "/servers/default/changes/starts", view),
            (Method::GET, "/servers/default/changes/resource", view),
            (Method::GET, "/servers/default/changes/history", view),
            (Method::POST, "/servers/default/changes/check", view),
            (Method::GET, "/servers/default/changes/diff", contents),
            (Method::GET, "/servers/default/changes/file", contents),
            (Method::GET, "/servers/default/changes/settings", Gate::Owner),
            (
                Method::PUT,
                "/servers/default/changes/settings",
                Gate::ReauthenticatedOwner
            ),
            (
                Method::POST,
                "/servers/default/changes/verify",
                Gate::ReauthenticatedOwner
            ),
            (
                Method::DELETE,
                "/servers/default/changes/contents",
                Gate::ReauthenticatedOwner
            ),
        ]
    );
}

#[tokio::test]
async fn the_history_is_read_with_its_bit_and_a_refused_read_of_contents_is_audited() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let viewer = server.member(&owner, "vera", CHANGES_VIEW).await;
    let stranger = server.member(&owner, "sam", DASHBOARD_VIEW).await;

    let starts = server.get_as(STARTS, &viewer).await;
    let refused = server.get_as(STARTS, &stranger).await;
    let diff = server
        .get_as(&format!("{DIFF}?path=resources/chat/client.lua"), &viewer)
        .await;
    let nothing_yet = server.get_as("/api/servers/default/changes", &owner).await;

    assert_eq!(starts.status, StatusCode::OK, "{}", starts.text);
    assert_eq!(starts.json()["starts"], json!([]));
    assert_eq!(refused.code(), "forbidden");
    assert_eq!(refused.json()["params"]["permission"], "changesView");
    assert_eq!(diff.code(), "forbidden");
    assert_eq!(diff.json()["params"]["permission"], "changesContent");
    assert_eq!(nothing_yet.status, StatusCode::NOT_FOUND);
    assert_eq!(nothing_yet.code(), "start_not_found");
    let denied: Vec<Value> = server
        .changes_entries("permission_denied")
        .await
        .into_iter()
        .map(|entry| entry["target"].clone())
        .collect();
    assert_eq!(denied, [json!("GET /api/servers/default/changes/diff")]);
}

#[tokio::test]
async fn a_path_that_climbs_is_refused_before_any_lookup() {
    let server = Server::new().await;
    let owner = server.owner().await;

    for path in ["resources/../server.cfg", "..\\observer.sqlite3", ""] {
        let reply = server
            .get_as(&format!("{DIFF}?path={}", path.replace('\\', "%5C")), &owner)
            .await;
        assert_eq!(reply.code(), "invalid_request", "{path}: {}", reply.text);
    }
    let file = server
        .get_as(
            "/api/servers/default/changes/file?start=1&path=resources/../server.cfg",
            &owner,
        )
        .await;
    assert_eq!(file.code(), "invalid_request");
}

#[tokio::test]
async fn the_owner_alone_reads_and_changes_the_settings() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let viewer = server.member(&owner, "vera", CHANGES_VIEW).await;

    let defaults = server.get_as(SETTINGS, &owner).await;
    let refused = server.get_as(SETTINGS, &viewer).await;
    let mut settings = defaults.json()["settings"].clone();
    settings["hidden"] = json!(["resources/[core]/my_bot/config", "*.secret"]);
    settings["excluded"] = json!(["a/../b"]);
    let invalid = server
        .call(Method::PUT, SETTINGS, Some(&owner), Some(settings.clone()))
        .await;
    settings["excluded"] = json!(["dist"]);
    let saved = server
        .call(Method::PUT, SETTINGS, Some(&owner), Some(settings))
        .await;

    assert_eq!(defaults.status, StatusCode::OK, "{}", defaults.text);
    assert_eq!(defaults.json()["settings"]["retentionDays"], 180);
    assert_eq!(refused.code(), "owner_only");
    assert_eq!(invalid.code(), "invalid_glob");
    assert_eq!(
        invalid.json()["params"],
        json!({ "field": "excluded", "index": 0 })
    );
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.text);
    assert_eq!(saved.json()["settings"]["hidden"][1], "*.secret");
    assert_eq!(saved.json()["hiddenMatches"], json!([0, 0]));
    let changed = server.changes_entries("change_settings_changed").await;
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0]["details"]["after"]["excluded"], json!(["dist"]));
}
