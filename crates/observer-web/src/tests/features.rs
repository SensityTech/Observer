//! The switches of the optional features over HTTP: who reads them, who turns them on or off, and
//! what a feature turned off still answers: its routes and topics refuse, the rest does not move.

use axum::http::{Method, StatusCode};
use jiff::SignedDuration;
use observer_domain::features::Feature;
use serde_json::json;

use super::live::{receive, send};
use super::{OWNER_PASSWORD, Reply, Server, Session};
use crate::api::routes;

const FEATURES: &str = "/api/features";
const SFTP: &str = "/api/settings/sftp";
const GROUPS: &str = "/api/servers/default/files/groups";

impl Server {
    async fn switch(&self, session: &Session, feature: &str, enabled: bool) -> Reply {
        self.call(
            Method::PUT,
            &format!("{FEATURES}/{feature}"),
            Some(session),
            Some(json!({ "enabled": enabled })),
        )
        .await
    }
}

#[test]
fn every_prefix_of_a_feature_holds_routes_and_the_switches_belong_to_no_feature() {
    let routes = routes();
    for feature in Feature::ALL {
        for prefix in feature.routes() {
            let gated = routes
                .iter()
                .filter(|route| route.path.starts_with(prefix))
                .all(|route| Feature::of_route(route.path) == Some(feature));
            let any = routes.iter().any(|route| route.path.starts_with(prefix));
            assert!(any && gated, "{prefix} must gate routes of {feature:?} only");
        }
    }
    for route in routes
        .iter()
        .filter(|route| route.path.starts_with(FEATURES.trim_start_matches("/api")))
    {
        assert_eq!(Feature::of_route(route.path), None, "{}", route.path);
    }
}

#[tokio::test]
async fn the_switches_are_read_with_the_settings_permission_and_say_what_each_feature_opens() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let reader = server.member(&owner, "grace", "200").await;
    let nobody = server.member(&owner, "alan", "1").await;

    let read = server.call(Method::GET, FEATURES, Some(&reader), None).await;
    let refused = server.call(Method::GET, FEATURES, Some(&nobody), None).await;

    assert_eq!(read.status, StatusCode::OK, "{}", read.text);
    let list = read.json();
    let features = list["features"].as_array().unwrap();
    assert_eq!(features.len(), Feature::ALL.len());
    let sftp = &features[0];
    assert_eq!(sftp["feature"], "sftp");
    assert_eq!((&sftp["enabled"], &sftp["running"]), (&json!(true), &json!(true)));
    assert_eq!(sftp["turnOn"], "now");
    assert_eq!(sftp["turnOff"], "now");
    assert_eq!(features[3]["turnOn"], "observerRestart");
    assert_eq!(sftp["reach"]["listens"], json!(["127.0.0.1:41976"]));
    assert_eq!(sftp["reach"]["serverFolder"], "writes");
    assert_eq!(
        sftp["reach"]["tasks"],
        json!([
            { "task": "sftpServer", "cadence": { "kind": "continuous" } },
            { "task": "sftpUpkeep", "cadence": { "kind": "every", "seconds": 60 } },
            { "task": "uploadsSweep", "cadence": { "kind": "observerStart" } },
        ])
    );
    assert_eq!(features[2]["turnOn"], "nextServerStart");
    let drops = &features[6];
    assert_eq!(drops["requires"], json!(["players", "metrics"]));
    assert_eq!(
        drops["reach"]["files"],
        json!([{ "path": "metrics.sqlite3", "bytes": null }])
    );
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.text);
    assert_eq!(refused.code(), "forbidden");
}

#[tokio::test]
async fn only_the_owner_turns_a_feature_on_or_off_and_with_the_password_again() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let manager = server.member(&owner, "grace", "400").await;
    let by_manager = server.switch(&manager, "tools", false).await;
    server.clock.advance(SignedDuration::from_mins(11));

    let stale = server.switch(&owner, "tools", false).await;
    let reauth = json!({ "password": OWNER_PASSWORD });
    let reauthenticated = server
        .call(Method::POST, "/api/auth/reauth", Some(&owner), Some(reauth))
        .await;
    let switched = server.switch(&owner, "tools", false).await;
    let unknown = server.switch(&owner, "backups", false).await;

    assert_eq!(by_manager.status, StatusCode::FORBIDDEN);
    assert_eq!(by_manager.code(), "owner_only");
    assert_eq!(stale.status, StatusCode::FORBIDDEN);
    assert_eq!(stale.code(), "reauth_required");
    assert_eq!(reauthenticated.status, StatusCode::NO_CONTENT);
    assert_eq!(switched.status, StatusCode::OK, "{}", switched.text);
    let tools = &switched.json()["features"][2];
    assert_eq!(tools["feature"], "tools");
    assert_eq!(
        (&tools["enabled"], &tools["running"]),
        (&json!(false), &json!(false))
    );
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
    let audit = server
        .call(
            Method::GET,
            "/api/audit?action=feature_changed",
            Some(&owner),
            None,
        )
        .await;
    let entries = audit.json()["entries"].clone();
    assert_eq!(entries.as_array().unwrap().len(), 1, "{entries}");
    assert_eq!(entries[0]["target"], "tools");
    assert_eq!(entries[0]["actorName"], "ada");
    assert_eq!(
        entries[0]["details"],
        json!({ "enabled": false, "applies": "now" })
    );
}

#[tokio::test]
async fn a_feature_turned_off_refuses_its_routes_to_whoever_may_use_them_and_nothing_else() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let viewer = server.member(&owner, "grace", "1").await;

    let switched = server.switch(&owner, "players", false).await;
    let players = server
        .call(
            Method::GET,
            "/api/servers/default/players/online",
            Some(&owner),
            None,
        )
        .await;
    let drops = server
        .call(
            Method::GET,
            "/api/servers/default/drops/settings",
            Some(&owner),
            None,
        )
        .await;
    let without_permission = server
        .call(
            Method::GET,
            "/api/servers/default/players/online",
            Some(&viewer),
            None,
        )
        .await;
    let statistics = server
        .call(Method::GET, "/api/metrics/growth", Some(&owner), None)
        .await;

    assert_eq!(switched.status, StatusCode::OK, "{}", switched.text);
    assert_eq!(players.status, StatusCode::NOT_FOUND, "{}", players.text);
    assert_eq!(
        players.json(),
        json!({ "code": "feature_disabled", "params": { "feature": "players" } })
    );
    assert_eq!(
        drops.json(),
        json!({ "code": "feature_disabled", "params": { "feature": "drops", "held": "requires" } })
    );
    assert_eq!(without_permission.status, StatusCode::FORBIDDEN);
    assert_eq!(without_permission.code(), "forbidden");
    assert_eq!(statistics.status, StatusCode::OK, "{}", statistics.text);
}

#[tokio::test]
async fn the_topics_of_a_feature_turned_off_are_withdrawn_and_refused_while_the_others_go_on() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let mut socket = server.socket(&owner).await;
    send(&mut socket, json!({ "type": "sub", "topic": "metrics.live" })).await;
    assert_eq!(receive(&mut socket).await["type"], "snapshot");

    let switched = server.switch(&owner, "metrics", false).await;

    assert_eq!(switched.status, StatusCode::OK, "{}", switched.text);
    let withdrawn = json!({ "type": "unsubscribed", "topic": "metrics.live", "reason": "disabled" });
    assert_eq!(receive(&mut socket).await, withdrawn);
    send(&mut socket, json!({ "type": "sub", "topic": "metrics.live" })).await;
    assert_eq!(receive(&mut socket).await, withdrawn);
    send(&mut socket, json!({ "type": "sub", "topic": "server.status" })).await;
    assert_eq!(receive(&mut socket).await["type"], "snapshot");
}

#[tokio::test]
async fn the_owner_gives_sftp_its_address_from_the_panel_and_takes_it_back() {
    let server = Server::unpinned().await;
    let owner = server.owner().await;
    let manager = server.member(&owner, "grace", "400").await;
    let give = |listen: &str| Some(json!({ "listen": listen }));

    let before = server.call(Method::GET, SFTP, Some(&manager), None).await;
    let waiting = server.call(Method::GET, GROUPS, Some(&owner), None).await;
    let by_manager = server
        .call(Method::PUT, SFTP, Some(&manager), give("127.0.0.1:41977"))
        .await;
    let a_name = server
        .call(Method::PUT, SFTP, Some(&owner), give("sftp.example.com:22"))
        .await;
    let no_port = server
        .call(Method::PUT, SFTP, Some(&owner), give("127.0.0.1:0"))
        .await;
    let given = server
        .call(Method::PUT, SFTP, Some(&owner), give(" 127.0.0.1:41977 "))
        .await;
    let switches = server.call(Method::GET, FEATURES, Some(&owner), None).await;
    let answering = server.call(Method::GET, GROUPS, Some(&owner), None).await;
    let taken_back = server
        .call(Method::PUT, SFTP, Some(&owner), Some(json!({ "listen": null })))
        .await;
    let waiting_again = server.call(Method::GET, GROUPS, Some(&owner), None).await;

    let nothing = json!({
        "listen": null, "pinned": false, "open": false, "fingerprint": null, "refused": null,
    });
    assert_eq!(before.status, StatusCode::OK, "{}", before.text);
    assert_eq!(before.json(), nothing);
    assert_eq!(waiting.code(), "feature_disabled");
    assert_eq!(waiting.json()["params"]["held"], "notConfigured");
    assert_eq!(by_manager.code(), "owner_only");
    assert_eq!(a_name.status, StatusCode::BAD_REQUEST);
    assert_eq!(a_name.code(), "invalid_sftp_listen");
    assert_eq!(a_name.json()["params"]["reason"], "address");
    assert_eq!(no_port.json()["params"]["reason"], "port");
    assert_eq!(given.status, StatusCode::OK, "{}", given.text);
    // Nothing keeps the port in these tests, so it is given and not open.
    assert_eq!(
        given.json(),
        json!({
            "listen": "127.0.0.1:41977", "pinned": false, "open": false, "fingerprint": null,
            "refused": null,
        })
    );
    let sftp = &switches.json()["features"][0];
    assert_eq!(sftp["feature"], "sftp");
    assert_eq!((&sftp["running"], &sftp["held"]), (&json!(true), &json!(null)));
    assert_eq!(sftp["turnOn"], "now");
    assert_eq!(sftp["reach"]["listens"], json!(["127.0.0.1:41977"]));
    assert_eq!(answering.status, StatusCode::OK, "{}", answering.text);
    assert_eq!(taken_back.json(), nothing);
    assert_eq!(waiting_again.code(), "feature_disabled");
    let audit = server
        .call(
            Method::GET,
            "/api/audit?action=sftp_listen_changed",
            Some(&owner),
            None,
        )
        .await;
    let entries = audit.json()["entries"].clone();
    let details: Vec<_> = entries
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| (entry["target"].clone(), entry["details"].clone()))
        .collect();
    assert_eq!(details.len(), 2, "{entries}");
    assert!(details.contains(&(
        json!("sftp"),
        json!({ "listen": "127.0.0.1:41977", "previous": null })
    )));
    assert!(details.contains(&(
        json!("sftp"),
        json!({ "listen": null, "previous": "127.0.0.1:41977" })
    )));
}

#[tokio::test]
async fn an_address_observer_toml_names_is_not_the_panels_to_change() {
    let server = Server::new().await;
    let owner = server.owner().await;

    let shown = server.call(Method::GET, SFTP, Some(&owner), None).await;
    let refused = server
        .call(
            Method::PUT,
            SFTP,
            Some(&owner),
            Some(json!({ "listen": "127.0.0.1:41977" })),
        )
        .await;
    let after = server.call(Method::GET, SFTP, Some(&owner), None).await;

    assert_eq!(
        shown.json(),
        json!({
            "listen": "127.0.0.1:41976", "pinned": true, "open": false, "fingerprint": null,
            "refused": null,
        })
    );
    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    assert_eq!(refused.code(), "sftp_listen_pinned");
    assert_eq!(after.json(), shown.json());
}
