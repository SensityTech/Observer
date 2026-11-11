//! Maintenance mode over HTTP: who closes the server and opens it again, who writes its message
//! and its list, which take the settings' permission and the password again, and its live topic.

use axum::http::{Method, StatusCode};
use jiff::SignedDuration;
use serde_json::json;

use super::live::{receive, send};
use super::{OWNER_PASSWORD, Server};

const SERVER_CONTROL: &str = "8";
const PLAYERS_VIEW: &str = "4000";
const SETTINGS_MANAGE: &str = "400";
const KAITO: &str = "license:00000000000000000000000000000000000000aa";
const MAINTENANCE: &str = "/api/servers/default/maintenance";

#[tokio::test]
async fn whoever_controls_the_server_reads_the_message_and_closes_it_once() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let viewer = server.member(&owner, "hopper", PLAYERS_VIEW).await;
    let operator = server.member(&owner, "maxime", SERVER_CONTROL).await;
    let on = json!({ "on": true });

    let unseen = server.call(Method::GET, MAINTENANCE, Some(&viewer), None).await;
    let refused = server
        .call(Method::PUT, MAINTENANCE, Some(&viewer), Some(on.clone()))
        .await;
    let preview = server.call(Method::GET, MAINTENANCE, Some(&operator), None).await;
    let mut closed = Vec::new();
    for _ in 0..2 {
        closed.push(
            server
                .call(Method::PUT, MAINTENANCE, Some(&operator), Some(on.clone()))
                .await,
        );
    }
    let unreadable = server
        .call(Method::PUT, MAINTENANCE, Some(&operator), Some(json!({})))
        .await;
    let opened = server
        .call(
            Method::PUT,
            MAINTENANCE,
            Some(&operator),
            Some(json!({ "on": false })),
        )
        .await;
    let settings = server
        .call(
            Method::GET,
            &format!("{MAINTENANCE}/settings"),
            Some(&operator),
            None,
        )
        .await;

    assert_eq!(unseen.status, StatusCode::FORBIDDEN);
    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    assert_eq!(preview.status, StatusCode::OK, "{}", preview.text);
    assert!(
        preview.json()["message"]
            .as_str()
            .is_some_and(|message| !message.is_empty())
    );
    assert_eq!(preview.json()["serverName"], json!(null));
    for closed in closed {
        assert_eq!(closed.status, StatusCode::NO_CONTENT, "{}", closed.text);
    }
    assert_eq!(unreadable.status, StatusCode::BAD_REQUEST);
    assert_eq!(opened.status, StatusCode::NO_CONTENT, "{}", opened.text);
    assert_eq!(settings.status, StatusCode::FORBIDDEN);
    let started = server.audited("maintenance_started").await;
    assert_eq!(started.len(), 1);
    assert_eq!(started[0]["actorName"], "maxime");
    assert_eq!(started[0]["target"], "default");
    assert_eq!(server.audited("maintenance_ended").await.len(), 1);
}

#[tokio::test]
async fn the_message_and_the_list_take_the_settings_permission_and_the_password_again() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let operator = server.member(&owner, "maxime", SERVER_CONTROL).await;
    let manager = server.member(&owner, "sarah", SETTINGS_MANAGE).await;
    let settings = format!("{MAINTENANCE}/settings");
    let passes = format!("{MAINTENANCE}/passes");
    let message = json!({ "message": "Back at 8, {name}.", "serverName": "Aperture RP | maintenance" });
    let pass = json!({ "license": KAITO, "note": "Tester from the dev team" });

    let refused = server
        .call(Method::POST, &passes, Some(&operator), Some(pass.clone()))
        .await;
    server.clock.advance(SignedDuration::from_mins(11));
    let without_password = server
        .call(Method::POST, &passes, Some(&manager), Some(pass.clone()))
        .await;
    let reauth = server
        .call(
            Method::POST,
            "/api/auth/reauth",
            Some(&owner),
            Some(json!({ "password": OWNER_PASSWORD })),
        )
        .await;
    let empty = server
        .call(
            Method::PUT,
            &settings,
            Some(&owner),
            Some(json!({ "message": " " })),
        )
        .await;
    let long_name = server
        .call(
            Method::PUT,
            &settings,
            Some(&owner),
            Some(json!({ "message": "Back at 8.", "serverName": "n".repeat(41) })),
        )
        .await;
    let changed = server
        .call(Method::PUT, &settings, Some(&owner), Some(message.clone()))
        .await;
    let added = server
        .call(Method::POST, &passes, Some(&owner), Some(pass.clone()))
        .await;
    let twice = server.call(Method::POST, &passes, Some(&owner), Some(pass)).await;
    let unreadable = server
        .call(
            Method::POST,
            &passes,
            Some(&owner),
            Some(json!({ "license": "kaito", "note": null })),
        )
        .await;
    let listed = server.call(Method::GET, &passes, Some(&manager), None).await;
    let read = server.call(Method::GET, &settings, Some(&manager), None).await;
    let id = added.json()["id"].as_i64().unwrap();
    let removed = server
        .call(Method::DELETE, &format!("{passes}/{id}"), Some(&owner), None)
        .await;
    let gone = server
        .call(Method::DELETE, &format!("{passes}/{id}"), Some(&owner), None)
        .await;

    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    assert_eq!(without_password.code(), "reauth_required");
    assert_eq!(reauth.status, StatusCode::NO_CONTENT, "{}", reauth.text);
    assert_eq!(empty.status, StatusCode::BAD_REQUEST);
    assert_eq!(empty.code(), "invalid_maintenance_settings");
    assert_eq!(long_name.status, StatusCode::BAD_REQUEST);
    assert_eq!(long_name.json()["params"]["maxName"], 40);
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.text);
    assert_eq!(read.json(), message);
    assert_eq!(added.status, StatusCode::CREATED, "{}", added.text);
    assert_eq!(added.json()["by"], "ada");
    assert_eq!(twice.status, StatusCode::CONFLICT);
    assert_eq!(twice.code(), "pass_exists");
    assert_eq!(unreadable.status, StatusCode::BAD_REQUEST);
    assert_eq!(unreadable.json()["params"]["field"], "license");
    assert_eq!(listed.json()["passes"][0]["license"], KAITO);
    assert_eq!(listed.json()["passes"][0]["note"], "Tester from the dev team");
    assert_eq!(removed.status, StatusCode::NO_CONTENT, "{}", removed.text);
    assert_eq!(gone.code(), "pass_not_found");
    assert_eq!(
        server.audited("maintenance_pass_added").await[0]["details"]["license"],
        KAITO
    );
    assert_eq!(server.audited("maintenance_pass_removed").await.len(), 1);
    assert_eq!(
        server.audited("maintenance_settings_changed").await[0]["details"]["to"]["message"],
        "Back at 8, {name}."
    );
}

#[tokio::test]
async fn the_admission_topic_says_since_when_the_server_is_closed_and_by_whom() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let operator = server.member(&owner, "maxime", SERVER_CONTROL).await;
    let mut socket = server.socket(&operator).await;
    send(&mut socket, json!({ "type": "sub", "topic": "server.admission" })).await;

    let snapshot = receive(&mut socket).await;
    let since = server.clock.now();
    let closed = server
        .call(
            Method::PUT,
            MAINTENANCE,
            Some(&operator),
            Some(json!({ "on": true })),
        )
        .await;
    let event = receive(&mut socket).await;

    assert_eq!(
        snapshot["data"]["maintenance"],
        json!({ "on": false, "since": null, "by": null, "passes": 0 })
    );
    assert_eq!(closed.status, StatusCode::NO_CONTENT, "{}", closed.text);
    assert_eq!(
        event["data"]["maintenance"],
        json!({ "on": true, "since": since, "by": "maxime", "passes": 0 })
    );
}
