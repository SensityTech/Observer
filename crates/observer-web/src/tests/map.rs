//! The live map over HTTP: who sees it, the owner's image from a host path with the password again,
//! the image served privately, and the settings, which take the settings' permission.

use axum::http::{Method, StatusCode, header};
use serde_json::json;

use super::{OWNER_PASSWORD, Server};

const PLAYERS_VIEW: &str = "4000";
const PLAYERS_MAP: &str = "2004000";
const SETTINGS_MANAGE: &str = "600";
const MAP: &str = "/api/servers/default/players/map";

/// A PNG of 800 by 600 on the host, as far as its header says.
fn png(dir: &std::path::Path) -> String {
    let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    png.extend_from_slice(&800u32.to_be_bytes());
    png.extend_from_slice(&600u32.to_be_bytes());
    png.extend_from_slice(&[1; 100]);
    let path = dir.join("atlas.png");
    std::fs::write(&path, png).unwrap();
    path.to_string_lossy().into_owned()
}

#[tokio::test]
async fn the_map_opens_to_its_own_permission_and_its_image_to_the_owner() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let moderator = server.member(&owner, "grace", PLAYERS_VIEW).await;
    let watcher = server.member(&owner, "hopper", PLAYERS_MAP).await;
    let path = png(server.dir.path());

    let refused = server.call(Method::GET, MAP, Some(&moderator), None).await;
    let seen = server.call(Method::GET, MAP, Some(&watcher), None).await;
    let not_owner = server
        .call(
            Method::PUT,
            &format!("{MAP}/image"),
            Some(&watcher),
            Some(json!({ "path": path })),
        )
        .await;
    let reauth = server
        .call(
            Method::POST,
            "/api/auth/reauth",
            Some(&owner),
            Some(json!({ "password": OWNER_PASSWORD })),
        )
        .await;
    let added = server
        .call(
            Method::PUT,
            &format!("{MAP}/image"),
            Some(&owner),
            Some(json!({ "path": path })),
        )
        .await;
    let image = server
        .call(Method::GET, &format!("{MAP}/image"), Some(&watcher), None)
        .await;

    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    assert_eq!(seen.status, StatusCode::OK, "{}", seen.text);
    assert_eq!(seen.json()["image"], json!(null));
    assert_eq!(seen.json()["settings"]["everySeconds"], 2);
    assert_eq!(not_owner.status, StatusCode::FORBIDDEN);
    assert_eq!(reauth.status, StatusCode::NO_CONTENT, "{}", reauth.text);
    assert_eq!(added.status, StatusCode::OK, "{}", added.text);
    assert_eq!(
        (added.json()["width"].clone(), added.json()["height"].clone()),
        (json!(800), json!(600))
    );
    assert_eq!(image.status, StatusCode::OK);
    assert_eq!(image.headers[header::CONTENT_TYPE], "image/png");
    assert_eq!(
        image.headers[header::CACHE_CONTROL],
        "private, max-age=31536000, immutable"
    );
    let audited = server.audited("map_image_changed").await;
    assert_eq!(audited[0]["details"]["width"], 800);
    assert!(!audited[0].to_string().contains("atlas.png"));
}

#[tokio::test]
async fn the_settings_take_the_settings_permission_and_are_checked() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let watcher = server.member(&owner, "hopper", PLAYERS_MAP).await;
    let manager = server.member(&owner, "sarah", SETTINGS_MANAGE).await;
    let settings = json!({
        "bounds": { "west": -5640.0, "east": 6470.0, "south": -4140.0, "north": 7970.0 },
        "newHours": 5,
        "veteranHours": 50,
        "everySeconds": 1,
    });
    let path = format!("{MAP}/settings");

    let refused = server
        .call(Method::PUT, &path, Some(&watcher), Some(settings.clone()))
        .await;
    let wrong = server
        .call(
            Method::PUT,
            &path,
            Some(&manager),
            Some(json!({ "bounds": null, "newHours": 9, "veteranHours": 3, "everySeconds": 2 })),
        )
        .await;
    let changed = server
        .call(Method::PUT, &path, Some(&manager), Some(settings.clone()))
        .await;
    let no_image = server
        .call(Method::GET, &format!("{MAP}/image"), Some(&owner), None)
        .await;

    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    assert_eq!(wrong.code(), "invalid_map_settings");
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.text);
    assert_eq!(changed.json(), settings);
    assert_eq!(no_image.code(), "no_map_image");
    assert_eq!(server.audited("map_settings_changed").await.len(), 1);
}
