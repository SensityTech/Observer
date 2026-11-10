//! The Debug routes: off until the owner turns the feature on, read with the diagnostics
//! permission, and empty while no server counts.

use axum::http::{Method, StatusCode};
use serde_json::json;

use super::{Reply, Server, Session};

const STATUS: &str = "/api/debug/status";

impl Server {
    async fn turn_debug(&self, owner: &Session, enabled: bool) -> Reply {
        self.call(
            Method::PUT,
            "/api/features/debug",
            Some(owner),
            Some(json!({ "enabled": enabled })),
        )
        .await
    }
}

#[tokio::test]
async fn debug_is_off_until_the_owner_turns_it_on_and_then_waits_for_a_start() {
    let server = Server::new().await;
    let owner = server.owner().await;

    let off = server.call(Method::GET, STATUS, Some(&owner), None).await;
    let switches = server
        .call(Method::GET, "/api/features", Some(&owner), None)
        .await;
    let turned_on = server.turn_debug(&owner, true).await;
    let waiting = server.call(Method::GET, STATUS, Some(&owner), None).await;

    assert_eq!(off.status, StatusCode::NOT_FOUND);
    assert_eq!(off.code(), "feature_disabled");
    assert_eq!(off.json()["params"]["feature"], "debug");
    let debug = &switches.json()["features"][8];
    assert_eq!(debug["feature"], "debug");
    assert_eq!(
        (&debug["enabled"], &debug["running"]),
        (&json!(false), &json!(false))
    );
    assert_eq!(debug["turnOn"], "nextServerStart");
    assert_eq!(debug["reach"]["routes"], json!(["/debug"]));
    assert_eq!(turned_on.status, StatusCode::OK, "{}", turned_on.text);
    let debug = &turned_on.json()["features"][8];
    assert_eq!(
        (&debug["enabled"], &debug["running"]),
        (&json!(true), &json!(true))
    );
    assert_eq!(waiting.status, StatusCode::OK, "{}", waiting.text);
    assert_eq!(
        waiting.json(),
        json!({
            "state": "waiting",
            "running": false,
            "since": null,
            "reporting": 0,
            "silent": [],
            "silentCount": 0,
            "refused": null,
        })
    );
}

#[tokio::test]
async fn the_debug_pages_are_read_with_the_diagnostics_permission_over_a_window() {
    let server = Server::new().await;
    let owner = server.owner().await;
    server.turn_debug(&owner, true).await;
    let reader = server.member(&owner, "grace", "1000").await;
    let stranger = server.member(&owner, "kenji", "200").await;

    let events = server
        .call(Method::GET, "/api/debug/events?window=1m", Some(&reader), None)
        .await;
    let bags = server
        .call(Method::GET, "/api/debug/bags", Some(&reader), None)
        .await;
    let http = server
        .call(Method::GET, "/api/debug/http?window=15m", Some(&reader), None)
        .await;
    let odd = server
        .call(Method::GET, "/api/debug/events?window=2h", Some(&reader), None)
        .await;
    let refused = server
        .call(Method::GET, "/api/debug/events", Some(&stranger), None)
        .await;

    assert_eq!(events.status, StatusCode::OK, "{}", events.text);
    let events = events.json();
    assert_eq!(events["seconds"], 60);
    assert_eq!(events["status"]["state"], "waiting");
    assert_eq!(
        events["totals"],
        json!({
            "toClients": { "count": 0, "bytes": 0 },
            "fromClients": { "count": 0, "bytes": 0 },
            "local": 0,
            "handlerUs": 0,
            "refused": 0,
            "refusedPlayers": 0,
        })
    );
    assert_eq!(events["events"], json!([]));
    assert_eq!(events["truncated"], false);
    assert_eq!(bags.json()["seconds"], 300, "five minutes when nothing is asked");
    assert_eq!(bags.json()["totals"]["keys"], 0);
    assert_eq!(http.json()["seconds"], 900);
    assert_eq!(http.json()["learningUntil"], json!(null));
    assert_eq!(odd.status, StatusCode::BAD_REQUEST);
    assert_eq!(odd.code(), "invalid_request");
    assert_eq!(refused.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn only_the_owner_says_a_host_is_known_and_only_of_a_host_a_resource_called() {
    let server = Server::new().await;
    let owner = server.owner().await;
    server.turn_debug(&owner, true).await;
    let reader = server.member(&owner, "grace", "1000").await;
    let known = json!({ "resource": "logs", "host": "discord.com" });

    let by_reader = server
        .call(
            Method::POST,
            "/api/debug/http/known",
            Some(&reader),
            Some(known.clone()),
        )
        .await;
    let never_called = server
        .call(Method::POST, "/api/debug/http/known", Some(&owner), Some(known))
        .await;
    server.turn_debug(&owner, false).await;
    let off = server.call(Method::GET, STATUS, Some(&owner), None).await;

    assert_eq!(by_reader.code(), "owner_only");
    assert_eq!(never_called.status, StatusCode::NOT_FOUND);
    assert_eq!(off.code(), "feature_disabled");
}
