//! The routes and topic of FXServer's builds, without a build to install: who may do what, and
//! where the password is asked again. `observer-app` and `fake-fxserver/tests/artifacts.rs` drive
//! the jobs, the switches and the probation themselves.

use axum::http::{Method, StatusCode};
use jiff::SignedDuration;
use serde_json::{Value, json};

use super::live::{receive, send};
use super::{OWNER_PASSWORD, Server, Session};

const BASE: &str = "/api/servers/default/artifacts";

impl Server {
    async fn reauthenticated(&self, session: &Session, password: &str) {
        let reauth = self
            .call(
                Method::POST,
                "/api/auth/reauth",
                Some(session),
                Some(json!({ "password": password })),
            )
            .await;
        assert_eq!(reauth.status, StatusCode::NO_CONTENT, "{}", reauth.text);
    }

    async fn artifacts_entries(&self, action: &str) -> Vec<Value> {
        self.audit.publish_new().await.unwrap();
        let owner = self.login("ada", OWNER_PASSWORD).await.session();
        let page = self
            .call(
                Method::GET,
                &format!("/api/audit?action={action}"),
                Some(&owner),
                None,
            )
            .await;
        page.json()["entries"].as_array().unwrap().clone()
    }
}

fn selection(apply: &str) -> Value {
    json!({ "build": 37872, "apply": apply })
}

#[tokio::test]
async fn installing_choosing_deleting_and_the_settings_ask_for_the_password_again() {
    let server = Server::new().await;
    let owner = server.owner().await;
    server.clock.advance(SignedDuration::from_mins(11));

    let changes = [
        (
            Method::POST,
            format!("{BASE}/install"),
            Some(json!({ "build": 37872 })),
        ),
        (
            Method::PUT,
            format!("{BASE}/selection"),
            Some(selection("nextStart")),
        ),
        (Method::DELETE, format!("{BASE}/builds/37872"), None),
        (
            Method::POST,
            format!("{BASE}/cleanup"),
            Some(json!({ "builds": [] })),
        ),
        (
            Method::PUT,
            format!("{BASE}/settings"),
            Some(
                json!({ "settings": { "keepBuilds": 3, "autoRollback": true, "branches": [] }, "version": null }),
            ),
        ),
        (
            Method::POST,
            format!("{BASE}/import"),
            Some(json!({ "folder": "/srv/fxserver", "build": 37872 })),
        ),
    ];
    for (method, path, body) in changes {
        let reply = server.call(method.clone(), &path, Some(&owner), body).await;
        assert_eq!(reply.code(), "reauth_required", "{method} {path}");
    }
    // Going back takes no password, and the preview changes nothing.
    let job = server
        .call(Method::DELETE, &format!("{BASE}/job"), Some(&owner), None)
        .await;
    let pending = server
        .call(Method::DELETE, &format!("{BASE}/pending"), Some(&owner), None)
        .await;
    let preview = server
        .call(Method::GET, &format!("{BASE}/cleanup"), Some(&owner), None)
        .await;
    assert_eq!(job.code(), "no_job");
    assert_eq!(pending.code(), "no_pending");
    assert_eq!(preview.status, StatusCode::OK, "{}", preview.text);
    assert_eq!(preview.json(), json!({ "remove": [], "keep": [], "freed": 0 }));
}

#[tokio::test]
async fn the_artifacts_view_reads_the_catalog_but_cannot_install() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let viewer = server.member(&owner, "grace", "20").await;
    server.reauthenticated(&viewer, "grace chose this one").await;

    let catalog = server
        .call(Method::GET, &format!("{BASE}/catalog"), Some(&viewer), None)
        .await;
    let install = server
        .call(
            Method::POST,
            &format!("{BASE}/install"),
            Some(&viewer),
            Some(json!({ "build": 37872 })),
        )
        .await;
    let malformed = server
        .call(
            Method::GET,
            &format!("{BASE}/catalog?refresh=later"),
            Some(&viewer),
            None,
        )
        .await;

    assert_eq!(catalog.status, StatusCode::OK, "{}", catalog.text);
    assert_eq!(catalog.json()["builds"], json!([]));
    assert_eq!(install.status, StatusCode::FORBIDDEN);
    assert_eq!(install.json()["params"]["permission"], "artifactsManage");
    assert_eq!(malformed.code(), "invalid_request");
    let denied = server.artifacts_entries("permission_denied").await;
    assert_eq!(denied[0]["target"], format!("POST {BASE}/install"));
}

#[tokio::test]
async fn applying_a_build_now_also_takes_server_control_and_its_refusal_is_audited() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let builder = server.member(&owner, "kenji", "40").await;
    server.reauthenticated(&builder, "kenji chose this one").await;

    let now = server
        .call(
            Method::PUT,
            &format!("{BASE}/selection"),
            Some(&builder),
            Some(selection("now")),
        )
        .await;
    let next_start = server
        .call(
            Method::PUT,
            &format!("{BASE}/selection"),
            Some(&builder),
            Some(selection("nextStart")),
        )
        .await;

    assert_eq!(now.status, StatusCode::FORBIDDEN, "{}", now.text);
    assert_eq!(now.json()["params"]["permission"], "serverControl");
    assert_eq!(next_start.code(), "build_not_installed");
    let denied = server.artifacts_entries("permission_denied").await;
    assert_eq!(denied[0]["actorName"], "kenji");
    assert_eq!(denied[0]["target"], format!("PUT {BASE}/selection"));
}

#[tokio::test]
async fn importing_a_folder_is_the_owners_alone() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let builder = server.member(&owner, "kenji", "40").await;
    server.reauthenticated(&builder, "kenji chose this one").await;
    let folder = if cfg!(windows) {
        r"C:\FXServer\build"
    } else {
        "/srv/fxserver/build"
    };

    let by_another = server
        .call(
            Method::POST,
            &format!("{BASE}/import"),
            Some(&builder),
            Some(json!({ "folder": folder, "build": 37872 })),
        )
        .await;
    let relative = server
        .call(
            Method::POST,
            &format!("{BASE}/import"),
            Some(&owner),
            Some(json!({ "folder": "build", "build": 37872 })),
        )
        .await;
    let not_a_build = server
        .call(
            Method::POST,
            &format!("{BASE}/import"),
            Some(&owner),
            Some(json!({ "folder": folder, "build": 12 })),
        )
        .await;

    assert_eq!(by_another.code(), "owner_only");
    assert_eq!(relative.code(), "invalid_request");
    assert_eq!(not_a_build.code(), "invalid_request");
}

#[tokio::test]
async fn the_artifacts_topic_opens_to_the_dashboard_or_the_artifacts_view_only() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let dashboard = server.member(&owner, "grace", "1").await;
    let viewer = server.member(&owner, "kenji", "20").await;
    let console = server.member(&owner, "lin", "2").await;

    for session in [&dashboard, &viewer] {
        let mut socket = server.socket(session).await;
        send(&mut socket, json!({ "type": "sub", "topic": "server.artifacts" })).await;
        let snapshot = receive(&mut socket).await;
        assert_eq!(snapshot["type"], "snapshot");
        assert_eq!(snapshot["data"]["selected"], Value::Null);
        assert_eq!(snapshot["data"]["settings"]["keepBuilds"], 3);
    }
    let mut socket = server.socket(&console).await;
    send(&mut socket, json!({ "type": "sub", "topic": "server.artifacts" })).await;
    assert_eq!(
        receive(&mut socket).await,
        json!({ "type": "unsubscribed", "topic": "server.artifacts", "reason": "forbidden" })
    );
}

#[tokio::test]
async fn settings_are_refused_with_every_issue_and_saved_once_valid() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let settings = |keep: u32, branches: Value| json!({ "settings": { "keepBuilds": keep, "autoRollback": false, "branches": branches }, "version": null });

    let refused = server
        .call(
            Method::PUT,
            &format!("{BASE}/settings"),
            Some(&owner),
            Some(settings(0, json!(["master"]))),
        )
        .await;
    let saved = server
        .call(
            Method::PUT,
            &format!("{BASE}/settings"),
            Some(&owner),
            Some(settings(5, json!(["feature/network-improvements-clean"]))),
        )
        .await;

    assert_eq!(refused.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        refused.json()["params"]["issues"],
        json!([
            { "field": "keepBuilds", "index": null, "problem": "outOfRange" },
            { "field": "branches", "index": 0, "problem": "master" },
        ])
    );
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.text);
    assert_eq!(saved.json()["version"], 1);
    assert_eq!(saved.json()["settings"]["autoRollback"], false);
    let changed = server.artifacts_entries("artifact_settings_changed").await;
    assert_eq!(
        changed[0]["details"],
        json!({
            "keepBuilds": { "before": 3, "after": 5 },
            "autoRollback": { "before": true, "after": false },
            "branches": { "before": [], "after": ["feature/network-improvements-clean"] },
        })
    );
}
