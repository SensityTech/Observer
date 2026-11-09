//! Observer's updates over HTTP: the owner reads them and checks, and installs with the password
//! again; nobody else reaches them, and they refuse once the feature is off.

use axum::http::{Method, StatusCode};
use jiff::SignedDuration;
use serde_json::json;

use super::{OWNER_PASSWORD, Server};

#[tokio::test]
async fn only_the_owner_reads_the_updates_whatever_permissions_others_hold() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let everything = server.member(&owner, "grace", "3fffff").await;

    let read = server.call(Method::GET, "/api/updates", Some(&owner), None).await;
    let refused = server
        .call(Method::GET, "/api/updates", Some(&everything), None)
        .await;
    let anonymous = server.get("/api/updates").await;

    assert_eq!(read.status, StatusCode::OK, "{}", read.text);
    let view = read.json();
    assert_eq!(view["current"], "0.1.0");
    assert_eq!(view["available"], json!(null));
    assert_eq!(view["blocked"], json!(null));
    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    assert_eq!(refused.code(), "owner_only");
    assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn an_install_takes_the_password_again_and_only_of_a_release_found() {
    let server = Server::new().await;
    let owner = server.owner().await;
    server.clock.advance(SignedDuration::from_mins(11));
    let install = json!({ "version": "0.2.0", "when": "now" });

    let stale = server
        .call(
            Method::POST,
            "/api/updates/install",
            Some(&owner),
            Some(install.clone()),
        )
        .await;
    let reauth = json!({ "password": OWNER_PASSWORD });
    server
        .call(Method::POST, "/api/auth/reauth", Some(&owner), Some(reauth))
        .await;
    let unknown = server
        .call(Method::POST, "/api/updates/install", Some(&owner), Some(install))
        .await;
    let nothing = server
        .call(Method::DELETE, "/api/updates/install", Some(&owner), None)
        .await;

    assert_eq!(stale.status, StatusCode::FORBIDDEN);
    assert_eq!(stale.code(), "reauth_required");
    assert_eq!(unknown.status, StatusCode::CONFLICT, "{}", unknown.text);
    assert_eq!(unknown.code(), "update_not_available");
    assert_eq!(nothing.status, StatusCode::CONFLICT);
    assert_eq!(nothing.code(), "no_update");
}

#[tokio::test]
async fn a_check_that_reaches_nothing_says_so_rather_than_fail() {
    let server = Server::new().await;
    let owner = server.owner().await;

    let checked = server
        .call(Method::POST, "/api/updates/check", Some(&owner), None)
        .await;

    assert_eq!(checked.status, StatusCode::OK, "{}", checked.text);
    assert_eq!(checked.json()["checkFailure"], json!({ "code": "unreachable" }));
}

#[tokio::test]
async fn the_updates_refuse_once_the_feature_is_off() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let reauth = json!({ "password": OWNER_PASSWORD });
    server
        .call(Method::POST, "/api/auth/reauth", Some(&owner), Some(reauth))
        .await;

    let switched = server
        .call(
            Method::PUT,
            "/api/features/updates",
            Some(&owner),
            Some(json!({ "enabled": false })),
        )
        .await;
    let read = server.call(Method::GET, "/api/updates", Some(&owner), None).await;

    assert_eq!(switched.status, StatusCode::OK, "{}", switched.text);
    assert_eq!(read.status, StatusCode::NOT_FOUND);
    assert_eq!(
        read.json(),
        json!({ "code": "feature_disabled", "params": { "feature": "updates" } })
    );
}
