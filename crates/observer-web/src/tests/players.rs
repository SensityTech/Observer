//! The players over HTTP: who may read them, what a search may ask for, and the erasures and
//! retentions only the owner may change, with the password again.

use axum::http::{Method, StatusCode};
use jiff::SignedDuration;
use observer_domain::players::IdentifierKind;
use observer_store::players::{self, Seen};
use serde_json::{Value, json};

use super::{OWNER_PASSWORD, Server, Session};

const DASHBOARD_VIEW: &str = "1";
const PLAYERS_VIEW: &str = "2000";
const PLAYER_IDENTIFIERS_VIEW: &str = "4000";
const LICENSE: &str = "license:0123456789abcdef0123456789abcdef01234567";
const DISCORD: &str = "123456789012345678";

impl Server {
    /// A player Observer saw, as a confirmed join writes them.
    async fn seen_player(&self) -> i64 {
        let at = self.clock.now();
        self.db
            .write(move |tx| {
                players::record_seen(
                    tx,
                    &Seen {
                        license: LICENSE,
                        name: Some("Ada"),
                        others: &[(IdentifierKind::Discord, DISCORD.to_owned())],
                        at,
                        pseudonym: [7; 16],
                    },
                )
            })
            .await
            .unwrap()
            .0
    }

    async fn reauthenticate_owner(&self, session: &Session) {
        let reply = self
            .call(
                Method::POST,
                "/api/auth/reauth",
                Some(session),
                Some(json!({ "password": OWNER_PASSWORD })),
            )
            .await;
        assert_eq!(reply.status, StatusCode::NO_CONTENT, "{}", reply.text);
    }
}

fn search(text: &str) -> String {
    format!("/api/servers/default/players?search={text}")
}

#[tokio::test]
async fn players_open_to_their_own_permission() {
    let server = Server::new().await;
    let owner = server.owner().await;
    server.seen_player().await;
    let dashboard = server.member(&owner, "grace", DASHBOARD_VIEW).await;
    let moderator = server.member(&owner, "hopper", PLAYERS_VIEW).await;

    let refused = server
        .call(
            Method::GET,
            "/api/servers/default/players",
            Some(&dashboard),
            None,
        )
        .await;
    let listed = server
        .call(
            Method::GET,
            "/api/servers/default/players",
            Some(&moderator),
            None,
        )
        .await;
    let online = server
        .call(
            Method::GET,
            "/api/servers/default/players/online",
            Some(&moderator),
            None,
        )
        .await;

    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    assert_eq!(refused.json()["params"]["permission"], "playersView");
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.text);
    assert_eq!(listed.json()["players"][0]["license"], LICENSE);
    assert_eq!(listed.json()["players"][0]["online"], false);
    assert_eq!(online.json(), json!({ "players": [] }));
}

#[tokio::test]
async fn a_search_by_another_identifier_than_the_licence_needs_its_own_permission() {
    let server = Server::new().await;
    let owner = server.owner().await;
    server.seen_player().await;
    let moderator = server.member(&owner, "hopper", PLAYERS_VIEW).await;
    let investigator = server.member(&owner, "lovelace", PLAYER_IDENTIFIERS_VIEW).await;

    let refused = server
        .call(
            Method::GET,
            &search(&format!("discord:{DISCORD}")),
            Some(&moderator),
            None,
        )
        .await;
    let bare = server
        .call(Method::GET, &search(DISCORD), Some(&moderator), None)
        .await;
    let by_licence = server
        .call(Method::GET, &search(&LICENSE[8..]), Some(&moderator), None)
        .await;
    let found = server
        .call(
            Method::GET,
            &search(&format!("discord:{DISCORD}")),
            Some(&investigator),
            None,
        )
        .await;

    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    assert_eq!(refused.json()["params"]["permission"], "playerIdentifiersView");
    assert_eq!(bare.status, StatusCode::OK);
    assert_eq!(bare.json()["players"], json!([]));
    assert_eq!(by_licence.json()["players"][0]["name"], "Ada");
    assert_eq!(found.json()["players"][0]["name"], "Ada");
}

#[tokio::test]
async fn a_search_never_takes_an_address() {
    let server = Server::new().await;
    let owner = server.owner().await;

    for text in ["192.0.2.7", "ip:192.0.2.7"] {
        let refused = server.call(Method::GET, &search(text), Some(&owner), None).await;

        assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{text}");
        assert_eq!(refused.json()["code"], "address_search");
    }
}

#[tokio::test]
async fn a_profile_shows_identifiers_and_links_only_to_who_may_see_them() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let id = server.seen_player().await;
    let moderator = server.member(&owner, "hopper", PLAYERS_VIEW).await;
    let investigator = server.member(&owner, "lovelace", PLAYER_IDENTIFIERS_VIEW).await;
    let path = format!("/api/servers/default/players/{id}");

    let plain = server.call(Method::GET, &path, Some(&moderator), None).await;
    let full = server.call(Method::GET, &path, Some(&investigator), None).await;
    let missing = server
        .call(
            Method::GET,
            "/api/servers/default/players/999",
            Some(&owner),
            None,
        )
        .await;

    assert_eq!(plain.status, StatusCode::OK, "{}", plain.text);
    assert_eq!(plain.json()["name"], "Ada");
    assert_eq!(plain.json()["identifiers"], Value::Null);
    assert_eq!(plain.json()["linked"], Value::Null);
    assert!(!plain.text.contains(DISCORD));
    assert_eq!(full.json()["identifiers"][0]["value"], DISCORD);
    assert_eq!(full.json()["linked"], json!([]));
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    assert_eq!(missing.json()["code"], "player_not_found");
}

#[tokio::test]
async fn forgetting_a_player_is_the_owners_with_the_password_again_and_the_audit_keeps_no_licence() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let id = server.seen_player().await;
    let investigator = server.member(&owner, "lovelace", PLAYER_IDENTIFIERS_VIEW).await;
    let path = format!("/api/servers/default/players/{id}");

    let by_staff = server
        .call(Method::DELETE, &path, Some(&investigator), None)
        .await;
    server.clock.advance(SignedDuration::from_mins(11));
    let without_password = server.call(Method::DELETE, &path, Some(&owner), None).await;
    server.reauthenticate_owner(&owner).await;
    let forgotten = server.call(Method::DELETE, &path, Some(&owner), None).await;
    let again = server.call(Method::DELETE, &path, Some(&owner), None).await;
    let profile = server.call(Method::GET, &path, Some(&owner), None).await;

    assert_eq!(by_staff.status, StatusCode::FORBIDDEN);
    assert_eq!(without_password.json()["code"], "reauth_required");
    assert_eq!(forgotten.status, StatusCode::NO_CONTENT, "{}", forgotten.text);
    assert_eq!(again.status, StatusCode::NOT_FOUND);
    assert_eq!(profile.status, StatusCode::NOT_FOUND);
    let entries = server.audited("player_forgotten").await;
    assert_eq!(entries.len(), 1);
    let target = entries[0]["target"].as_str().unwrap();
    assert_eq!(target.len(), 32);
    assert!(!entries[0].to_string().contains(&LICENSE[8..]));
    let file = std::fs::read_to_string(server.dir.path().join("data/players-forgotten")).unwrap();
    assert!(file.starts_with(target));
}

#[tokio::test]
async fn the_owner_changes_the_retentions_within_their_ranges() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let path = "/api/servers/default/players/settings";
    server.reauthenticate_owner(&owner).await;

    let defaults = server.call(Method::GET, path, Some(&owner), None).await;
    let refused = server
        .call(
            Method::PUT,
            path,
            Some(&owner),
            Some(json!({ "linkDays": 365, "addressDays": 400, "dayDays": 400, "profileDays": 730 })),
        )
        .await;
    let changed = server
        .call(
            Method::PUT,
            path,
            Some(&owner),
            Some(json!({ "linkDays": 180, "addressDays": 30, "dayDays": 400, "profileDays": null })),
        )
        .await;

    assert_eq!(
        defaults.json(),
        json!({ "linkDays": 365, "addressDays": 90, "dayDays": 400, "profileDays": 730 })
    );
    assert_eq!(refused.json()["code"], "invalid_players_settings");
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.text);
    assert_eq!(changed.json()["profileDays"], Value::Null);
    let entries = server.audited("players_retention_changed").await;
    assert_eq!(entries[0]["details"]["to"]["linkDays"], 180);
}
