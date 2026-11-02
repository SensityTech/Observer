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
    assert_eq!(sftp["turnOn"], "observerRestart");
    assert_eq!(sftp["turnOff"], "now");
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
