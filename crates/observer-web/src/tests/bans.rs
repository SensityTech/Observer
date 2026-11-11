//! Bans over HTTP: who sees them, who makes, edits and lifts them, and the refusal message and the
//! templates, which take the settings' permission and the password again.

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
const BANS_EDIT: &str = "100000000";
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
async fn an_active_ban_is_edited_under_a_permission_of_its_own() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let player = server.player_to_ban().await;
    let viewer = server.member(&owner, "hopper", BANS_VIEW).await;
    let reviewer = server.member(&owner, "sarah", BANS_LIFT).await;
    let moderator = server.member(&owner, "maxime", PLAYERS_BAN).await;
    let editor = server.member(&owner, "grace", BANS_EDIT).await;
    let ban = json!({ "player": player, "reason": "Aimbot on stream", "minutes": 60 });
    let banned = server.call(Method::POST, BANS, Some(&moderator), Some(ban)).await;
    let made = banned.json();
    let path = format!("{BANS}/{}", made["code"].as_str().unwrap());
    let week = json!({ "reason": "Aimbot, two clips", "ends": { "kind": "in", "minutes": 10080 } });
    let minute = json!({ "reason": null, "ends": { "kind": "in", "minutes": 1 } });

    let mut refused = Vec::new();
    for member in [&viewer, &reviewer, &moderator] {
        let reply = server
            .call(Method::PATCH, &path, Some(member), Some(week.clone()))
            .await;
        refused.push((reply.status, reply.json()["params"]["permission"].clone()));
    }
    let extended = server.call(Method::PATCH, &path, Some(&editor), Some(week)).await;
    let sooner = server
        .call(Method::PATCH, &path, Some(&editor), Some(minute))
        .await;
    let unknown = server
        .call(
            Method::PATCH,
            &format!("{BANS}/ZZZZ-ZZZZ"),
            Some(&editor),
            Some(json!({ "reason": "x", "ends": null })),
        )
        .await;
    let listed = server.call(Method::GET, BANS, Some(&viewer), None).await;

    assert_eq!(refused, vec![(StatusCode::FORBIDDEN, json!("bansEdit")); 3]);
    assert_eq!(extended.status, StatusCode::OK, "{}", extended.text);
    let edited = extended.json();
    assert_eq!(edited["reason"], "Aimbot, two clips");
    assert_eq!(
        (&edited["by"], &edited["createdAt"]),
        (&made["by"], &made["createdAt"])
    );
    assert_eq!(edited["edits"].as_array().unwrap().len(), 1);
    assert_eq!(edited["edits"][0]["by"], "grace");
    assert_eq!(
        edited["edits"][0]["reason"],
        json!({ "before": "Aimbot on stream", "after": "Aimbot, two clips" })
    );
    assert_eq!(edited["edits"][0]["ends"]["before"], made["endsAt"]);
    assert_eq!(edited["edits"][0]["ends"]["after"], edited["endsAt"]);
    assert_eq!(sooner.status, StatusCode::OK, "{}", sooner.text);
    assert_eq!(sooner.json()["edits"][0]["reason"], json!(null));
    assert_eq!(sooner.json()["edits"][0]["ends"]["before"], edited["endsAt"]);
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
    assert_eq!(unknown.code(), "ban_not_found");
    assert_eq!(listed.json()["bans"][0]["edits"].as_array().unwrap().len(), 2);
    let audited = server.audited("ban_edited").await;
    assert_eq!(audited.len(), 2);
    assert_eq!(audited[1]["target"], made["code"]);
    assert_eq!(audited[1]["actorName"], "grace");
    assert_eq!(
        audited[1]["details"]["reason"],
        json!({ "before": "Aimbot on stream", "after": "Aimbot, two clips" })
    );
    assert_eq!(audited[1]["details"]["endsAt"]["before"], made["endsAt"]);
    assert_eq!(audited[0]["details"].get("reason"), None);
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
    let settings = json!({ "message": "Banned: {reason}", "tokenMatches": 2, "templates": [] });
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
        json!({ "message": "Banned: {reason}", "tokenMatches": 2, "templates": [] })
    );
}

#[tokio::test]
async fn templates_are_saved_with_the_settings_and_read_by_who_bans() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let viewer = server.member(&owner, "hopper", BANS_VIEW).await;
    let moderator = server.member(&owner, "maxime", PLAYERS_BAN).await;
    let settings = format!("{BANS}/settings");
    let templates = format!("{BANS}/templates");
    let stored = server.call(Method::GET, &settings, Some(&owner), None).await;
    let with = |templates: serde_json::Value| {
        let mut settings = stored.json();
        settings["templates"] = templates;
        settings
    };

    let none_yet = server.call(Method::GET, &templates, Some(&moderator), None).await;
    let unusable = server
        .call(
            Method::PUT,
            &settings,
            Some(&owner),
            Some(with(json!([{ "reason": " ", "minutes": null }]))),
        )
        .await;
    let without = server
        .call(
            Method::PUT,
            &settings,
            Some(&owner),
            Some(json!({ "message": "Banned: {reason}", "tokenMatches": 1 })),
        )
        .await;
    let saved = server
        .call(
            Method::PUT,
            &settings,
            Some(&owner),
            Some(with(json!([
                { "reason": "Cheating", "minutes": null },
                { "reason": " Insults ", "minutes": 4320 },
            ]))),
        )
        .await;
    let unseen = server.call(Method::GET, &templates, Some(&viewer), None).await;
    let offered = server.call(Method::GET, &templates, Some(&moderator), None).await;

    assert_eq!(none_yet.json(), json!([]));
    assert_eq!(unusable.status, StatusCode::BAD_REQUEST);
    assert_eq!(unusable.code(), "invalid_bans_settings");
    assert_eq!(without.status, StatusCode::BAD_REQUEST);
    assert_eq!(without.code(), "invalid_request");
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.text);
    assert_eq!(unseen.status, StatusCode::FORBIDDEN);
    assert_eq!(
        offered.json(),
        json!([
            { "reason": "Cheating", "minutes": null },
            { "reason": "Insults", "minutes": 4320 },
        ])
    );
    let changed = server.audited("bans_settings_changed").await;
    assert_eq!(changed.len(), 1);
    assert_eq!(
        changed[0]["details"],
        json!({ "templates": { "before": [], "after": offered.json() } })
    );
}
