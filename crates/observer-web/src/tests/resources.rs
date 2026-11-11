//! The orders of the Resources page and its topic, without an FXServer to run: who they refuse,
//! why, and what they audit. `fake-fxserver/tests/runtime.rs` drives resources through them.

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};

use super::Server;
use super::live::{receive, send};

const START: &str = "/api/servers/default/resources/start";
const STOP: &str = "/api/servers/default/resources/stop";
const RESTART: &str = "/api/servers/default/resources/restart";
const RESCAN: &str = "/api/servers/default/resources/rescan";
/// `RESOURCES_CONTROL` alone.
const KEEPER: &str = "4000000";

#[tokio::test]
async fn orders_on_resources_take_their_own_permission_and_a_server_that_listed_them() {
    let server = Server::new().await;
    let owner = server.owner().await;
    // The dashboard, the console and the server's controls: none of them starts a resource here.
    let operator = server.member(&owner, "grace", "f").await;
    let keeper = server.member(&owner, "kenji", KEEPER).await;
    let order = json!({ "name": "chat" });

    for path in [START, STOP, RESTART, RESCAN] {
        let body = (path != RESCAN).then(|| order.clone());
        let refused = server.call(Method::POST, path, Some(&operator), body).await;
        assert_eq!(refused.status, StatusCode::FORBIDDEN, "{path}");
        assert_eq!(refused.json()["params"]["permission"], "resourcesControl");
    }
    let denied = server.audited("permission_denied").await;
    assert_eq!(
        denied.len(),
        4,
        "a refused order is audited: the permission is sensitive"
    );
    assert_eq!(denied[0]["target"], format!("POST {RESCAN}"));

    for path in [START, STOP, RESTART, RESCAN] {
        let body = (path != RESCAN).then(|| order.clone());
        let reply = server.call(Method::POST, path, Some(&keeper), body).await;
        assert_eq!(
            (reply.status, reply.code()),
            (StatusCode::CONFLICT, json!("server_not_running")),
            "{path}"
        );
    }
    for action in [
        "resource_started",
        "resource_stopped",
        "resource_restarted",
        "resources_rescanned",
    ] {
        assert_eq!(
            server.audited(action).await,
            Vec::<Value>::new(),
            "{action}: nothing was asked of a server"
        );
    }
}

#[tokio::test]
async fn an_order_names_a_resource_and_nothing_else() {
    let server = Server::new().await;
    let owner = server.owner().await;

    for body in [
        json!({}),
        json!({ "name": 3 }),
        json!({ "name": "chat", "command": "quit" }),
        json!({ "name": "chat", "dependants": "yes" }),
    ] {
        let reply = server
            .call(Method::POST, RESTART, Some(&owner), Some(body.clone()))
            .await;
        assert_eq!(
            (reply.status, reply.code()),
            (StatusCode::BAD_REQUEST, json!("invalid_request")),
            "{body}"
        );
    }
}

#[tokio::test]
async fn the_resources_topic_is_open_to_the_dashboard_and_to_who_starts_and_stops_them() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let viewer = server.member(&owner, "grace", "1").await;
    let keeper = server.member(&owner, "kenji", KEEPER).await;
    let auditor = server.member(&owner, "cid", "100").await;
    let sub = json!({ "type": "sub", "topic": "server.resources" });

    for session in [&viewer, &keeper] {
        let mut socket = server.socket(session).await;
        send(&mut socket, sub.clone()).await;
        assert_eq!(
            receive(&mut socket).await,
            json!({
                "type": "snapshot",
                "topic": "server.resources",
                "data": {
                    "stage": "stopped",
                    "generation": 0,
                    "resources": [],
                    "truncated": false,
                    "rescan": null,
                    "rescanning": false
                }
            })
        );
    }
    let mut refused = server.socket(&auditor).await;
    send(&mut refused, sub).await;
    assert_eq!(
        receive(&mut refused).await,
        json!({ "type": "unsubscribed", "topic": "server.resources", "reason": "forbidden" })
    );
}
