//! File access over HTTP: who reads and writes the groups, and who gives them to whom.

use axum::http::{Method, StatusCode};
use serde_json::json;

use super::Server;

fn textures() -> serde_json::Value {
    json!({
        "name": "Mappers",
        "rules": [{
            "effect": "allow",
            "folder": "resources/[maps]",
            "depth": "tree",
            "names": ["*.ytd"],
            "rights": ["list", "read", "create", "modify"],
        }],
    })
}

#[tokio::test]
async fn groups_are_the_owners_to_write_and_a_manager_gives_them_within_reach() {
    let server = Server::new().await;
    let owner = server.owner().await;
    // FILES_MANAGE (0x2000) and DASHBOARD_VIEW: within reach of linus, not of ken's SERVER_CONTROL.
    let manager = server.member(&owner, "grace", "2001").await;
    server.member(&owner, "linus", "1").await;
    server.member(&owner, "ken", "8").await;
    let linus = server.user_id(&owner, "linus").await;
    let ken = server.user_id(&owner, "ken").await;

    let refused = server
        .call(
            Method::POST,
            "/api/servers/default/files/groups",
            Some(&manager),
            Some(textures()),
        )
        .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.text);
    let created = server
        .call(
            Method::POST,
            "/api/servers/default/files/groups",
            Some(&owner),
            Some(textures()),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.text);
    let group = created.json();
    assert_eq!(group["reach"], "assets");
    assert_eq!(group["rules"][0]["folder"], "resources/[maps]");

    let listed = server
        .call(
            Method::GET,
            "/api/servers/default/files/groups",
            Some(&manager),
            None,
        )
        .await;
    assert_eq!(listed.status, StatusCode::OK);
    assert_eq!(listed.json().as_array().unwrap().len(), 1);

    let body = json!({ "groups": [group["id"]] });
    let given = server
        .call(
            Method::PUT,
            &format!("/api/files/accesses/users/{linus}"),
            Some(&manager),
            Some(body.clone()),
        )
        .await;
    assert_eq!(given.status, StatusCode::NO_CONTENT, "{}", given.text);
    let stronger = server
        .call(
            Method::PUT,
            &format!("/api/files/accesses/users/{ken}"),
            Some(&manager),
            Some(body),
        )
        .await;
    assert_eq!(stronger.status, StatusCode::FORBIDDEN);
    assert_eq!(stronger.json()["code"], "hierarchy");

    let broken = json!({
        "name": "Broken",
        "rules": [{ "effect": "allow", "folder": "resources/**", "depth": "tree", "rights": ["list"] }],
    });
    let invalid = server
        .call(
            Method::POST,
            "/api/servers/default/files/groups",
            Some(&owner),
            Some(broken),
        )
        .await;
    assert_eq!(invalid.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(invalid.json()["params"], json!({ "index": 0, "field": "folder" }));
}

/// A throwaway public key, made for these tests.
const KEY: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIHVxIho0YkjaiYCfuVODbF9qWydNFowI8MproaBig3sF linus@laptop";

#[tokio::test]
async fn an_account_adds_and_removes_its_own_keys_with_the_password_again() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let linus = server.member(&owner, "linus", "1").await;

    let added = server
        .call(
            Method::POST,
            "/api/account/files/keys",
            Some(&linus),
            Some(json!({ "publicKey": KEY })),
        )
        .await;
    assert_eq!(added.status, StatusCode::CREATED, "{}", added.text);
    let key = added.json();
    assert_eq!(key["algorithm"], "ssh-ed25519");
    assert_eq!(key["comment"], "linus@laptop");
    let again = server
        .call(
            Method::POST,
            "/api/account/files/keys",
            Some(&linus),
            Some(json!({ "publicKey": KEY })),
        )
        .await;
    assert_eq!(again.status, StatusCode::CONFLICT);
    assert_eq!(again.json()["code"], "key_in_use");
    let rsa = server
        .call(
            Method::POST,
            "/api/account/files/keys",
            Some(&linus),
            Some(json!({ "publicKey": "ssh-rsa AAAAB3NzaC1yc2E= old" })),
        )
        .await;
    assert_eq!(rsa.json()["params"]["reason"], "algorithm");

    let mine = server
        .call(Method::GET, "/api/account/files", Some(&linus), None)
        .await;
    assert_eq!(mine.json()["login"], "linus");
    assert_eq!(mine.json()["keys"].as_array().unwrap().len(), 1);
    assert_eq!(mine.json()["groups"], json!([]));

    let removed = server
        .call(
            Method::DELETE,
            &format!("/api/account/files/keys/{}", key["id"]),
            Some(&linus),
            None,
        )
        .await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT);
    let mine = server
        .call(Method::GET, "/api/account/files", Some(&linus), None)
        .await;
    assert_eq!(mine.json()["keys"], json!([]));
}
