//! What the Settings page reads of `observer.toml` and of the statistics' retention: the owner's.

use axum::http::{Method, StatusCode};

use super::Server;

const OBSERVER: &str = "/api/settings/observer";

#[tokio::test]
async fn the_owner_reads_what_observer_toml_said_and_how_long_statistics_stay() {
    let server = Server::behind_proxy().await;
    let owner = server.owner().await;

    let reply = server.call(Method::GET, OBSERVER, Some(&owner), None).await;

    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
    let settings = reply.json();
    assert!(
        settings["configFile"]
            .as_str()
            .unwrap()
            .ends_with("observer.toml"),
        "{settings}"
    );
    assert_eq!(settings["listen"], "0.0.0.0:40125");
    assert_eq!(settings["hosts"], serde_json::json!(["observer.test"]));
    assert_eq!(settings["trustedProxies"], serde_json::json!(["10.0.0.0/8"]));
    assert_eq!(settings["statistics"]["samples"], 72 * 3600);
    assert_eq!(settings["statistics"]["minutes"], 30 * 86_400);
}

#[tokio::test]
async fn a_user_holding_every_permission_is_not_the_owner_here() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let manager = server.member(&owner, "grace", "ffff").await;

    let refused = server.call(Method::GET, OBSERVER, Some(&manager), None).await;

    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.text);
    assert_eq!(refused.code(), "owner_only");
}
