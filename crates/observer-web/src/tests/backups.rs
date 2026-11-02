//! The backups over HTTP: who sees them, who starts one, the owner's password again for what
//! decides where the database goes, and secrets that never come back.

use axum::http::{Method, StatusCode};
use jiff::SignedDuration;
use serde_json::{Value, json};

use super::{OWNER_PASSWORD, Server, Session};

const BACKUPS: &str = "/api/servers/default/backups";
const SETTINGS: &str = "/api/servers/default/backups/settings";
const DESTINATIONS: &str = "/api/servers/default/backups/destinations";
const RUN: &str = "/api/servers/default/backups/run";
/// `BACKUPS_VIEW`, `BACKUPS_RUN` and `BACKUPS_DOWNLOAD`, as the panel sends them.
const VIEW: &str = "80000";
const START: &str = "100000";
const DOWNLOAD: &str = "200000";

impl Server {
    async fn reauthenticate_for_backups(&self, session: &Session, password: &str) {
        let reply = self
            .call(
                Method::POST,
                "/api/auth/reauth",
                Some(session),
                Some(json!({ "password": password })),
            )
            .await;
        assert_eq!(reply.status, StatusCode::NO_CONTENT, "{}", reply.text);
    }
}

fn settings(view: &Value) -> Value {
    view["settings"].clone()
}

#[tokio::test]
async fn a_viewer_reads_the_backups_and_changes_nothing() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let viewer = server.member(&owner, "viewer", VIEW).await;

    let view = server.call(Method::GET, BACKUPS, Some(&viewer), None).await;
    assert_eq!(view.status, StatusCode::OK, "{}", view.text);
    assert_eq!(view.json()["settings"]["beforeRestarts"], "scheduled");
    assert_eq!(view.json()["source"]["problem"], "launchNotConfigured");

    let change = json!({ "settings": settings(&view.json()), "version": null });
    let refused = server
        .call(Method::PUT, SETTINGS, Some(&viewer), Some(change))
        .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    assert_eq!(refused.code(), "owner_only");
    let started = server.call(Method::POST, RUN, Some(&viewer), None).await;
    assert_eq!(started.status, StatusCode::FORBIDDEN);
    assert_eq!(started.json()["params"]["permission"], "backupsRun");
}

#[tokio::test]
async fn a_backup_without_a_destination_goes_nowhere() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let runner = server.member(&owner, "runner", START).await;

    let reply = server.call(Method::POST, RUN, Some(&runner), None).await;

    assert_eq!(reply.status, StatusCode::CONFLICT, "{}", reply.text);
    assert_eq!(reply.code(), "backup_not_configured");
}

#[tokio::test]
async fn the_settings_take_the_owner_s_password_again_and_keep_the_database_s() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let view = server.call(Method::GET, BACKUPS, Some(&owner), None).await.json();
    let mut changed = settings(&view);
    changed["source"] = json!("manual");
    changed["manual"] =
        json!({ "host": "127.0.0.1", "port": 3307, "user": "backup", "database": "es_extended" });
    changed["times"] = json!(["04:00", "16:00"]);
    let change = json!({ "settings": changed, "version": null, "password": "db-secret-1" });
    server.clock.advance(SignedDuration::from_mins(11));

    let early = server
        .call(Method::PUT, SETTINGS, Some(&owner), Some(change.clone()))
        .await;
    assert_eq!(early.status, StatusCode::FORBIDDEN);
    assert_eq!(early.code(), "reauth_required");

    server.reauthenticate_for_backups(&owner, OWNER_PASSWORD).await;
    let saved = server
        .call(Method::PUT, SETTINGS, Some(&owner), Some(change))
        .await;

    assert_eq!(saved.status, StatusCode::OK, "{}", saved.text);
    let saved = saved.json();
    assert_eq!(saved["version"], 1);
    assert_eq!(saved["hasPassword"], true);
    assert_eq!(saved["source"]["database"], "es_extended");
    assert!(saved["nextAt"].is_string());
    assert!(!saved.to_string().contains("db-secret-1"));
    let audit = server.call(Method::GET, "/api/audit", Some(&owner), None).await;
    assert!(!audit.text.contains("db-secret-1"));
    assert!(audit.text.contains("backup_settings_changed"));
}

#[tokio::test]
async fn settings_with_issues_are_refused_with_each_one() {
    let server = Server::new().await;
    let owner = server.owner().await;
    server.reauthenticate_for_backups(&owner, OWNER_PASSWORD).await;
    let view = server.call(Method::GET, BACKUPS, Some(&owner), None).await.json();
    let mut changed = settings(&view);
    changed["times"] = json!(["25:00"]);
    changed["restartWaitSeconds"] = json!(9000);

    let reply = server
        .call(
            Method::PUT,
            SETTINGS,
            Some(&owner),
            Some(json!({ "settings": changed, "version": null })),
        )
        .await;

    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(reply.code(), "invalid_backup_settings");
    assert_eq!(
        reply.json()["params"]["issues"],
        json!([
            { "field": "time", "index": 0, "problem": "invalidTime" },
            { "field": "restartWaitSeconds", "index": null, "problem": "outOfRange" },
        ])
    );
}

#[tokio::test]
async fn a_destination_comes_back_without_its_secret_and_with_observer_s_key() {
    let server = Server::new().await;
    let owner = server.owner().await;
    server.reauthenticate_for_backups(&owner, OWNER_PASSWORD).await;
    let bucket = json!({
        "name": "Offsite bucket",
        "enabled": true,
        "target": {
            "kind": "s3", "endpoint": "https://s3.example.net", "region": "auto", "bucket": "sensity-backups",
            "prefix": "db/", "accessKeyId": "GKKEYID", "pathStyle": true,
        },
        "retention": { "keepLast": 14, "keepDays": null },
        "secret": "s3-secret-key",
    });
    let sftp = json!({
        "name": "Offsite host",
        "enabled": true,
        "target": {
            "kind": "sftp", "host": "backup.example.net", "port": 22, "user": "fivem", "folder": "dumps",
            "auth": "key", "hostKey": null,
        },
        "retention": { "keepLast": null, "keepDays": 30 },
    });

    let bucket = server
        .call(Method::POST, DESTINATIONS, Some(&owner), Some(bucket))
        .await;
    let sftp = server
        .call(Method::POST, DESTINATIONS, Some(&owner), Some(sftp))
        .await;

    assert_eq!(bucket.status, StatusCode::CREATED, "{}", bucket.text);
    assert_eq!(bucket.json()["hasSecret"], true);
    assert!(!bucket.text.contains("s3-secret-key"));
    assert_eq!(sftp.status, StatusCode::CREATED, "{}", sftp.text);
    let key = sftp.json()["publicKey"].as_str().unwrap().to_owned();
    assert!(key.starts_with("ssh-ed25519 "), "{key}");
    assert!(!sftp.text.contains("PRIVATE KEY"));
    let view = server.call(Method::GET, BACKUPS, Some(&owner), None).await;
    assert_eq!(view.json()["destinations"].as_array().unwrap().len(), 2);
    assert!(!view.text.contains("s3-secret-key") && !view.text.contains("PRIVATE KEY"));
}

#[tokio::test]
async fn a_destination_with_issues_is_refused_with_each_one() {
    let server = Server::new().await;
    let owner = server.owner().await;
    server.reauthenticate_for_backups(&owner, OWNER_PASSWORD).await;

    let reply = server
        .call(
            Method::POST,
            DESTINATIONS,
            Some(&owner),
            Some(json!({
                "name": "",
                "enabled": true,
                "target": { "kind": "local", "folder": "backups" },
                "retention": { "keepLast": null, "keepDays": null },
            })),
        )
        .await;

    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(reply.code(), "invalid_backup_destination");
    assert_eq!(
        reply.json()["params"]["issues"],
        json!([
            { "field": "name", "problem": "required" },
            { "field": "folder", "problem": "notAbsolute" },
            { "field": "retention", "problem": "required" },
        ])
    );
}

#[tokio::test]
async fn a_download_takes_its_permission_and_the_password_again() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let downloader = server.member(&owner, "downloader", DOWNLOAD).await;
    let path = "/api/servers/default/backups/copies/1/download";
    server.clock.advance(SignedDuration::from_mins(11));

    let early = server.call(Method::GET, path, Some(&downloader), None).await;
    assert_eq!(early.status, StatusCode::FORBIDDEN);
    assert_eq!(early.code(), "reauth_required");

    server
        .reauthenticate_for_backups(&downloader, "downloader chose this one")
        .await;
    let missing = server.call(Method::GET, path, Some(&downloader), None).await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND, "{}", missing.text);
    let history = server
        .call(
            Method::GET,
            "/api/servers/default/backups/runs",
            Some(&downloader),
            None,
        )
        .await;
    assert_eq!(history.status, StatusCode::OK, "{}", history.text);
    assert_eq!(history.json(), json!({ "runs": [], "nextBefore": null }));
}
