//! Kicks, announcements and messages over HTTP: each under its own permission, a kick of everyone
//! under the server control's, and the kick message under the settings' with the password again.
//! No server runs here: an account that may act is told so, one that may not is refused first.

use axum::http::{Method, StatusCode};
use jiff::SignedDuration;
use serde_json::{Value, json};

use super::{OWNER_PASSWORD, Reply, Server, Session};

const SERVER_CONTROL: &str = "8";
const SETTINGS_MANAGE: &str = "400";
const PLAYERS_KICK: &str = "8000000";
const PLAYERS_ANNOUNCE: &str = "10000000";
const PLAYERS_MESSAGE: &str = "20000000";
const KICKS: &str = "/api/servers/default/kicks";
const KICK_ALL: &str = "/api/servers/default/kicks/all";
const ANNOUNCEMENTS: &str = "/api/servers/default/announcements";
const MESSAGES: &str = "/api/servers/default/messages";

fn kick() -> Value {
    json!({ "player": 3, "license": null, "reason": "RDM" })
}

fn kick_all() -> Value {
    json!({ "reason": "Maintenance" })
}

fn announcement() -> Value {
    json!({ "message": "Restart in five minutes" })
}

fn message() -> Value {
    json!({ "player": 3, "license": null, "message": "Come to the staff room" })
}

async fn post(server: &Server, path: &str, session: &Session, body: Value) -> Reply {
    server.call(Method::POST, path, Some(session), Some(body)).await
}

#[tokio::test]
async fn each_action_on_connected_players_takes_its_own_permission() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let kicker = server.member(&owner, "maxime", PLAYERS_KICK).await;
    let announcer = server.member(&owner, "sarah", PLAYERS_ANNOUNCE).await;
    let messenger = server.member(&owner, "grace", PLAYERS_MESSAGE).await;
    let operator = server.member(&owner, "hopper", SERVER_CONTROL).await;

    let allowed = [
        post(&server, KICKS, &kicker, kick()).await,
        post(&server, KICK_ALL, &operator, kick_all()).await,
        post(&server, ANNOUNCEMENTS, &announcer, announcement()).await,
        post(&server, MESSAGES, &messenger, message()).await,
    ];
    let refused = [
        (post(&server, KICKS, &operator, kick()).await, "playersKick"),
        (
            post(&server, KICK_ALL, &kicker, kick_all()).await,
            "serverControl",
        ),
        (
            post(&server, ANNOUNCEMENTS, &kicker, announcement()).await,
            "playersAnnounce",
        ),
        (
            post(&server, MESSAGES, &announcer, message()).await,
            "playersMessage",
        ),
    ];

    for reply in allowed {
        assert_eq!(reply.status, StatusCode::CONFLICT, "{}", reply.text);
        assert_eq!(reply.code(), "server_not_running");
    }
    for (reply, permission) in refused {
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{}", reply.text);
        assert_eq!(reply.json()["params"]["permission"], permission);
    }
    let denied = server.audited("permission_denied").await;
    assert_eq!(denied.len(), 4);
    for action in [
        "player_kicked",
        "everyone_kicked",
        "announcement_sent",
        "direct_message_sent",
    ] {
        assert_eq!(server.audited(action).await.len(), 0, "{action}");
    }
}

#[tokio::test]
async fn a_kick_without_a_reason_and_a_message_without_a_text_are_refused() {
    let server = Server::new().await;
    let owner = server.owner().await;

    let kick = server
        .call(
            Method::POST,
            KICKS,
            Some(&owner),
            Some(json!({ "player": 3, "license": null, "reason": " " })),
        )
        .await;
    let everyone = server
        .call(
            Method::POST,
            KICK_ALL,
            Some(&owner),
            Some(json!({ "reason": "" })),
        )
        .await;
    let announcement = server
        .call(
            Method::POST,
            ANNOUNCEMENTS,
            Some(&owner),
            Some(json!({ "message": "\n" })),
        )
        .await;
    let unknown_field = server
        .call(
            Method::POST,
            MESSAGES,
            Some(&owner),
            Some(json!({ "player": 3, "license": null, "message": "hi", "everyone": true })),
        )
        .await;

    assert_eq!(kick.status, StatusCode::BAD_REQUEST, "{}", kick.text);
    assert_eq!(kick.code(), "invalid_kick");
    assert_eq!(kick.json()["params"]["max"], 200);
    assert_eq!(everyone.code(), "invalid_kick");
    assert_eq!(announcement.code(), "invalid_message");
    assert_eq!(announcement.json()["params"]["max"], 400);
    assert_eq!(unknown_field.code(), "invalid_request");
}

#[tokio::test]
async fn the_kick_message_takes_the_settings_permission_and_the_password_again() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let kicker = server.member(&owner, "maxime", PLAYERS_KICK).await;
    let manager = server.member(&owner, "sarah", SETTINGS_MANAGE).await;
    let settings = json!({ "message": "Kicked by {by}: {reason}" });
    let path = format!("{KICKS}/settings");

    let unread = server.call(Method::GET, &path, Some(&kicker), None).await;
    let refused = server
        .call(Method::PUT, &path, Some(&kicker), Some(settings.clone()))
        .await;
    let default = server.call(Method::GET, &path, Some(&manager), None).await;
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
        .call(Method::PUT, &path, Some(&owner), Some(json!({ "message": " " })))
        .await;
    let changed = server
        .call(Method::PUT, &path, Some(&owner), Some(settings))
        .await;
    let read = server.call(Method::GET, &path, Some(&manager), None).await;

    assert_eq!(unread.status, StatusCode::FORBIDDEN);
    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    assert_eq!(
        default.json(),
        json!({ "message": "You were kicked from this server.\nReason: {reason}" })
    );
    assert_eq!(without_password.code(), "reauth_required");
    assert_eq!(reauth.status, StatusCode::NO_CONTENT, "{}", reauth.text);
    assert_eq!(empty.status, StatusCode::BAD_REQUEST);
    assert_eq!(empty.code(), "invalid_kick_settings");
    assert_eq!(empty.json()["params"]["maxMessage"], 600);
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.text);
    assert_eq!(read.json(), json!({ "message": "Kicked by {by}: {reason}" }));
    let audited = server.audited("kick_settings_changed").await;
    assert_eq!(audited.len(), 1);
    assert_eq!(audited[0]["details"]["to"]["message"], "Kicked by {by}: {reason}");
}
