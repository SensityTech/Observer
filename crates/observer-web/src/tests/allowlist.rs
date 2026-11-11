//! The allowlist over HTTP: who reads its lists, who answers a request or approves a licence, its
//! settings, which take the settings' permission and the password again, and its live topic.

use axum::http::{Method, StatusCode};
use jiff::SignedDuration;
use observer_store::allowlist;
use serde_json::json;

use super::live::{receive, send};
use super::{OWNER_PASSWORD, Server};

const DASHBOARD_VIEW: &str = "1";
const PLAYERS_VIEW: &str = "4000";
const ALLOWLIST_MANAGE: &str = "8000000";
const SETTINGS_MANAGE: &str = "400";
const KAITO: &str = "license:00000000000000000000000000000000000000aa";
const MIRA: &str = "license:00000000000000000000000000000000000000bb";
const ALLOWLIST: &str = "/api/servers/default/allowlist";

impl Server {
    /// The request a refused connection leaves, as the admission writes it.
    async fn asked(&self, license: &'static str, name: &'static str, code: &'static str) {
        let at = self.clock.now();
        self.db
            .write(move |tx| allowlist::ask(tx, license, Some(name), code, at, 100))
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn the_lists_are_read_with_the_players_and_answered_under_the_allowlists_permission() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let dashboard = server.member(&owner, "grace", DASHBOARD_VIEW).await;
    let viewer = server.member(&owner, "hopper", PLAYERS_VIEW).await;
    let staff = server.member(&owner, "maxime", ALLOWLIST_MANAGE).await;
    server.asked(KAITO, "kaito", "7KQ-2XP").await;
    server.asked(MIRA, "Mira", "3HD-9QW").await;
    let requests = format!("{ALLOWLIST}/requests");
    let approve = format!("{ALLOWLIST}/requests/7kq2xp/approve");

    let unseen = server.call(Method::GET, &requests, Some(&dashboard), None).await;
    let listed = server.call(Method::GET, &requests, Some(&viewer), None).await;
    let not_approved = server.call(Method::POST, &approve, Some(&viewer), None).await;
    let approved = server.call(Method::POST, &approve, Some(&staff), None).await;
    let twice = server.call(Method::POST, &approve, Some(&staff), None).await;
    let not_denied = server
        .call(
            Method::DELETE,
            &format!("{requests}/3HD-9QW"),
            Some(&viewer),
            None,
        )
        .await;
    let denied = server
        .call(Method::DELETE, &format!("{requests}/3HD-9QW"), Some(&staff), None)
        .await;
    let left = server.call(Method::GET, &requests, Some(&staff), None).await;
    let list = server
        .call(
            Method::GET,
            &format!("{ALLOWLIST}/approved?search=kai"),
            Some(&viewer),
            None,
        )
        .await;

    assert_eq!(unseen.status, StatusCode::FORBIDDEN);
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.text);
    let waiting: Vec<_> = listed.json()["requests"]
        .as_array()
        .unwrap()
        .iter()
        .map(|request| request["code"].clone())
        .collect();
    assert_eq!(waiting.len(), 2);
    assert!(waiting.contains(&json!("7KQ-2XP")), "{waiting:?}");
    assert_eq!(not_approved.status, StatusCode::FORBIDDEN);
    assert_eq!(approved.status, StatusCode::OK, "{}", approved.text);
    assert_eq!(approved.json()["license"], KAITO);
    assert_eq!(approved.json()["by"], "maxime");
    assert_eq!(approved.json()["request"], "7KQ-2XP");
    assert_eq!(twice.status, StatusCode::NOT_FOUND);
    assert_eq!(twice.code(), "allowlist_request_not_found");
    assert_eq!(not_denied.status, StatusCode::FORBIDDEN);
    assert_eq!(denied.status, StatusCode::NO_CONTENT, "{}", denied.text);
    assert_eq!(left.json()["requests"], json!([]));
    assert_eq!(list.json()["approved"][0]["name"], "kaito");
    let made = server.audited("allowlist_request_approved").await;
    assert_eq!(made[0]["target"], "7KQ-2XP");
    assert_eq!(made[0]["details"]["license"], KAITO);
    assert_eq!(
        server.audited("allowlist_request_denied").await[0]["details"]["playerName"],
        "Mira"
    );
}

#[tokio::test]
async fn a_licence_is_approved_ahead_once_and_its_approval_taken_back_by_its_id() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let viewer = server.member(&owner, "hopper", PLAYERS_VIEW).await;
    let approved = format!("{ALLOWLIST}/approved");
    let ahead = json!({ "license": KAITO, "note": "Sarah's brother" });

    let refused = server
        .call(Method::POST, &approved, Some(&viewer), Some(ahead.clone()))
        .await;
    let added = server
        .call(Method::POST, &approved, Some(&owner), Some(ahead.clone()))
        .await;
    let twice = server
        .call(Method::POST, &approved, Some(&owner), Some(ahead))
        .await;
    let unreadable = server
        .call(
            Method::POST,
            &approved,
            Some(&owner),
            Some(json!({ "license": "kaito", "note": null })),
        )
        .await;
    let id = added.json()["id"].as_i64().unwrap();
    let not_removed = server
        .call(Method::DELETE, &format!("{approved}/{id}"), Some(&viewer), None)
        .await;
    let removed = server
        .call(Method::DELETE, &format!("{approved}/{id}"), Some(&owner), None)
        .await;
    let gone = server
        .call(Method::DELETE, &format!("{approved}/{id}"), Some(&owner), None)
        .await;
    let by_licence = server
        .call(Method::DELETE, &format!("{approved}/{KAITO}"), Some(&owner), None)
        .await;

    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    assert_eq!(added.status, StatusCode::CREATED, "{}", added.text);
    assert_eq!(added.json()["note"], "Sarah's brother");
    assert_eq!(twice.status, StatusCode::CONFLICT);
    assert_eq!(twice.code(), "already_approved");
    assert_eq!(unreadable.status, StatusCode::BAD_REQUEST);
    assert_eq!(unreadable.json()["params"]["field"], "license");
    assert_eq!(not_removed.status, StatusCode::FORBIDDEN);
    assert_eq!(removed.status, StatusCode::NO_CONTENT, "{}", removed.text);
    assert_eq!(gone.code(), "approval_not_found");
    assert_eq!(by_licence.status, StatusCode::NOT_FOUND);
    assert_eq!(
        server.audited("allowlist_licence_approved").await[0]["details"]["note"],
        "Sarah's brother"
    );
    assert_eq!(
        server.audited("allowlist_approval_removed").await[0]["details"]["license"],
        KAITO
    );
}

#[tokio::test]
async fn denying_all_takes_the_time_the_staff_last_saw() {
    let server = Server::new().await;
    let owner = server.owner().await;
    server.asked(KAITO, "kaito", "7KQ-2XP").await;
    let seen = server.clock.now();
    server.clock.advance(SignedDuration::from_secs(30));
    server.asked(MIRA, "Mira", "3HD-9QW").await;
    let deny = format!("{ALLOWLIST}/requests/deny");

    let denied = server
        .call(Method::POST, &deny, Some(&owner), Some(json!({ "until": seen })))
        .await;
    let unbounded = server
        .call(Method::POST, &deny, Some(&owner), Some(json!({})))
        .await;
    let left = server
        .call(Method::GET, &format!("{ALLOWLIST}/requests"), Some(&owner), None)
        .await;

    assert_eq!(denied.status, StatusCode::OK, "{}", denied.text);
    assert_eq!(denied.json(), json!({ "denied": 1 }));
    assert_eq!(unbounded.status, StatusCode::BAD_REQUEST);
    assert_eq!(left.json()["requests"][0]["code"], "3HD-9QW");
    assert_eq!(
        server.audited("allowlist_requests_denied").await[0]["details"]["denied"],
        1
    );
}

#[tokio::test]
async fn the_switch_and_the_message_take_the_settings_permission_and_the_password_again() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let staff = server.member(&owner, "maxime", ALLOWLIST_MANAGE).await;
    let manager = server.member(&owner, "sarah", SETTINGS_MANAGE).await;
    let settings = json!({ "enabled": true, "message": "Ask the staff: {id}" });
    let path = format!("{ALLOWLIST}/settings");

    let refused = server
        .call(Method::PUT, &path, Some(&staff), Some(settings.clone()))
        .await;
    server.clock.advance(SignedDuration::from_mins(11));
    let without_password = server
        .call(Method::PUT, &path, Some(&manager), Some(settings.clone()))
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
            &path,
            Some(&owner),
            Some(json!({ "enabled": true, "message": " " })),
        )
        .await;
    let changed = server
        .call(Method::PUT, &path, Some(&owner), Some(settings.clone()))
        .await;
    let read = server.call(Method::GET, &path, Some(&manager), None).await;

    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    assert_eq!(without_password.code(), "reauth_required");
    assert_eq!(reauth.status, StatusCode::NO_CONTENT, "{}", reauth.text);
    assert_eq!(empty.status, StatusCode::BAD_REQUEST);
    assert_eq!(empty.code(), "invalid_allowlist_settings");
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.text);
    assert_eq!(read.json(), settings);
    assert_eq!(
        server.audited("allowlist_settings_changed").await[0]["details"]["to"]["enabled"],
        true
    );
}

#[tokio::test]
async fn the_admission_topic_follows_the_allowlist_for_whoever_sees_players() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let viewer = server.member(&owner, "hopper", PLAYERS_VIEW).await;
    let auditor = server.member(&owner, "kenji", "100").await;
    let mut socket = server.socket(&viewer).await;
    let mut refused = server.socket(&auditor).await;
    for socket in [&mut socket, &mut refused] {
        send(socket, json!({ "type": "sub", "topic": "server.admission" })).await;
    }

    let snapshot = receive(&mut socket).await;
    let forbidden = receive(&mut refused).await;
    let added = server
        .call(
            Method::POST,
            &format!("{ALLOWLIST}/approved"),
            Some(&owner),
            Some(json!({ "license": KAITO, "note": null })),
        )
        .await;
    let event = receive(&mut socket).await;

    assert_eq!(
        (&snapshot["type"], &snapshot["topic"]),
        (&json!("snapshot"), &json!("server.admission"))
    );
    assert_eq!(
        snapshot["data"]["allowlist"],
        json!({ "enabled": false, "waiting": 0, "approved": 0 })
    );
    assert_eq!(forbidden["type"], "unsubscribed");
    assert_eq!(added.status, StatusCode::CREATED, "{}", added.text);
    assert_eq!(
        (&event["type"], &event["topic"]),
        (&json!("event"), &json!("server.admission"))
    );
    assert_eq!(
        event["data"]["allowlist"],
        json!({ "enabled": false, "waiting": 0, "approved": 1 })
    );
}
