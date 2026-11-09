//! Bans over HTTP: who sees them, who makes and lifts them, and the refusal message, which takes
//! the settings' permission and the password again.

use axum::http::{Method, StatusCode};
use jiff::SignedDuration;
use observer_domain::players::IdentifierKind;
use observer_store::players::{self, Seen};
use serde_json::json;

use super::{OWNER_PASSWORD, Server};

const DASHBOARD_VIEW: &str = "1";
const BANS_VIEW: &str = "400000";
const PLAYERS_BAN: &str = "800000";
const BANS_LIFT: &str = "1000000";
const SETTINGS_MANAGE: &str = "400";
const LICENSE: &str = "license:00000000000000000000000000000000000000aa";
const BANS: &str = "/api/servers/default/bans";

impl Server {
    /// A player Observer saw, with a Discord account and a hardware token.
    async fn player_to_ban(&self) -> i64 {
        let at = self.clock.now();
        self.db
            .write(move |tx| {
                players::record_seen(
                    tx,
                    &Seen {
                        license: LICENSE,
                        name: Some("kaito"),
                        others: &[
                            (IdentifierKind::Discord, "123456789012345678".to_owned()),
                            (IdentifierKind::Token, format!("4:{}", "ab".repeat(32))),
                        ],
                        at,
                        pseudonym: [7; 16],
                    },
                )
            })
            .await
            .unwrap()
            .0
    }
}

#[tokio::test]
async fn bans_are_seen_made_and_lifted_under_their_own_permissions() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let player = server.player_to_ban().await;
    let dashboard = server.member(&owner, "grace", DASHBOARD_VIEW).await;
    let viewer = server.member(&owner, "hopper", BANS_VIEW).await;
    let moderator = server.member(&owner, "maxime", PLAYERS_BAN).await;
    let reviewer = server.member(&owner, "sarah", BANS_LIFT).await;
    let ban = json!({ "player": player, "reason": "Aimbot on stream", "minutes": 60 });

    let unseen = server.call(Method::GET, BANS, Some(&dashboard), None).await;
    let not_banned = server
        .call(Method::POST, BANS, Some(&viewer), Some(ban.clone()))
        .await;
    let banned = server.call(Method::POST, BANS, Some(&moderator), Some(ban)).await;
    let code = banned.json()["code"].as_str().unwrap().to_owned();
    let lift = format!("{BANS}/{code}/lift");
    let not_lifted = server
        .call(
            Method::POST,
            &lift,
            Some(&moderator),
            Some(json!({ "reason": null })),
        )
        .await;
    let listed = server.call(Method::GET, BANS, Some(&viewer), None).await;
    let freed = server
        .call(
            Method::POST,
            &lift,
            Some(&reviewer),
            Some(json!({ "reason": "appeal" })),
        )
        .await;
    let twice = server
        .call(
            Method::POST,
            &lift,
            Some(&reviewer),
            Some(json!({ "reason": null })),
        )
        .await;

    assert_eq!(unseen.status, StatusCode::FORBIDDEN);
    assert_eq!(not_banned.status, StatusCode::FORBIDDEN);
    assert_eq!(banned.status, StatusCode::CREATED, "{}", banned.text);
    assert_eq!(banned.json()["by"], "maxime");
    assert_eq!(banned.json()["license"], LICENSE);
    assert_eq!(
        (
            banned.json()["identifiers"].clone(),
            banned.json()["tokens"].clone()
        ),
        (json!(1), json!(1))
    );
    assert_eq!(not_lifted.status, StatusCode::FORBIDDEN);
    assert_eq!(listed.json()["bans"][0]["code"], code.as_str());
    assert_eq!(listed.json()["counts"]["active"], 1);
    assert_eq!(freed.status, StatusCode::NO_CONTENT, "{}", freed.text);
    assert_eq!(twice.status, StatusCode::CONFLICT);
    assert_eq!(twice.code(), "ban_not_active");
    let made = server.audited("player_banned").await;
    assert_eq!(made[0]["target"], code.as_str());
    assert_eq!(made[0]["details"]["tokens"], 1);
    assert_eq!(
        server.audited("ban_lifted").await[0]["details"]["reason"],
        "appeal"
    );
}

#[tokio::test]
async fn a_profile_lists_the_bans_made_from_it() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let player = server.player_to_ban().await;
    let made = server
        .call(
            Method::POST,
            BANS,
            Some(&owner),
            Some(json!({ "player": player, "reason": "Ban evasion", "minutes": null })),
        )
        .await;
    let typed = server
        .call(
            Method::POST,
            BANS,
            Some(&owner),
            Some(json!({ "player": null, "identifiers": ["steam:nope"], "reason": "x", "minutes": null })),
        )
        .await;

    let listed = server
        .call(
            Method::GET,
            &format!("/api/servers/default/players/{player}/bans"),
            Some(&owner),
            None,
        )
        .await;

    assert_eq!(made.status, StatusCode::CREATED, "{}", made.text);
    assert_eq!(typed.status, StatusCode::BAD_REQUEST);
    assert_eq!(typed.json()["params"]["field"], "identifiers");
    let bans = listed.json();
    assert_eq!(bans.as_array().unwrap().len(), 1);
    assert_eq!(bans[0]["endsAt"], json!(null));
    assert_eq!(bans[0]["state"], "active");
}

#[tokio::test]
async fn the_refusal_message_takes_the_settings_permission_and_the_password_again() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let moderator = server.member(&owner, "maxime", PLAYERS_BAN).await;
    let manager = server.member(&owner, "sarah", SETTINGS_MANAGE).await;
    let settings = json!({ "message": "Banned: {reason}", "tokenMatches": 2 });
    let path = format!("{BANS}/settings");

    let refused = server
        .call(Method::PUT, &path, Some(&moderator), Some(settings.clone()))
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
    let changed = server
        .call(Method::PUT, &path, Some(&owner), Some(settings))
        .await;
    let read = server.call(Method::GET, &path, Some(&manager), None).await;

    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    assert_eq!(without_password.code(), "reauth_required");
    assert_eq!(reauth.status, StatusCode::NO_CONTENT, "{}", reauth.text);
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.text);
    assert_eq!(
        read.json(),
        json!({ "message": "Banned: {reason}", "tokenMatches": 2 })
    );
}
