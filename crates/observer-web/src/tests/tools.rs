//! The server tools over HTTP: who reads the Tools page and scans, the password again for what
//! writes, the plan a fix confirms, the settings' version, and RedM.

use std::fs;
use std::path::Path;

use axum::http::{Method, StatusCode};
use jiff::SignedDuration;
use observer_domain::launch::{LaunchSettings, OneSync, StoredLaunch};
use observer_store::launch;
use serde_json::{Value, json};

use super::{OWNER_PASSWORD, Server, Session};

const TOOLS: &str = "/api/tools";
const SCAN: &str = "/api/tools/duplicate-archetypes/scan";
const FIX: &str = "/api/tools/duplicate-archetypes/fix";
const UNDO: &str = "/api/tools/duplicate-archetypes/undo";
const SETTINGS: &str = "/api/tools/duplicate-archetypes/settings";

impl Server {
    /// A server folder whose `props` resource holds a marshmallow twice in one `.ytyp`, with the
    /// cfg `cfg`, saved as the launch settings' folder.
    async fn props_folder(&self, cfg: &str) -> std::path::PathBuf {
        let folder = self.dir.path().join("server-data");
        let write = |path: &str, bytes: &[u8]| {
            let full = folder.join(path);
            fs::create_dir_all(full.parent().unwrap()).unwrap();
            fs::write(full, bytes).unwrap();
        };
        write(
            "resources/props/fxmanifest.lua",
            b"fx_version 'cerulean'\ngame 'gta5'\n",
        );
        write(
            "resources/props/stream/camp.ytyp",
            &observer_gamefiles::testing::ytyp("camp", &["marshmallow", "stick", "marshmallow"]),
        );
        write("server.cfg", cfg.as_bytes());
        let stored = StoredLaunch {
            settings: LaunchSettings {
                data_path: folder.to_string_lossy().into_owned(),
                cfg_file: "server.cfg".to_owned(),
                onesync: OneSync::On,
                auto_start: false,
                txadmin_events: false,
                extra_arguments: Vec::new(),
            },
            version: 1,
            legacy_build_path: None,
        };
        let now = self.clock.now();
        self.db
            .write(move |tx| launch::save(tx, "default", &stored, now))
            .await
            .unwrap();
        folder
    }

    async fn reauthenticate_for_tools(&self, session: &Session) {
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

fn tent(folder: &Path) {
    fs::write(
        folder.join("resources/props/stream/tent.ytyp"),
        observer_gamefiles::testing::ytyp("tent", &["tent", "tent"]),
    )
    .unwrap();
}

#[tokio::test]
async fn the_tools_are_the_owner_s_and_what_writes_takes_the_password_again() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let manager = server.member(&owner, "grace", "ffff").await;
    server.props_folder("ensure props\n").await;
    server.clock.advance(SignedDuration::from_mins(11));

    for (method, path, body) in [
        (Method::GET, TOOLS, None),
        (Method::POST, SCAN, None),
        (Method::POST, FIX, Some(json!({ "plan": "0".repeat(64) }))),
        (Method::POST, UNDO, Some(json!({ "scope": "all" }))),
    ] {
        let refused = server.call(method, path, Some(&manager), body).await;
        assert_eq!(refused.status, StatusCode::FORBIDDEN, "{path}");
        assert_eq!(refused.code(), "owner_only", "{path}");
    }
    let page = server.call(Method::GET, TOOLS, Some(&owner), None).await;
    let scanned = server.call(Method::POST, SCAN, Some(&owner), None).await;
    let plan = scanned.json()["plan"].clone();
    let without_password = [
        server
            .call(Method::POST, FIX, Some(&owner), Some(json!({ "plan": plan })))
            .await,
        server
            .call(Method::POST, UNDO, Some(&owner), Some(json!({ "scope": "all" })))
            .await,
        server
            .call(
                Method::PUT,
                SETTINGS,
                Some(&owner),
                Some(json!({ "settings": settings("fix"), "version": null })),
            )
            .await,
    ];

    assert_eq!(page.status, StatusCode::OK, "{}", page.text);
    assert_eq!(page.json()["tools"][0]["tool"], "duplicateArchetypes");
    assert_eq!(page.json()["tools"][0]["settings"]["atStart"], "off");
    assert_eq!(scanned.status, StatusCode::OK, "{}", scanned.text);
    assert_eq!(scanned.json()["mode"], "report");
    assert_eq!(scanned.json()["summary"]["fixable"], 1);
    for refused in without_password {
        assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.text);
        assert_eq!(refused.code(), "reauth_required");
    }
}

fn settings(at_start: &str) -> Value {
    json!({ "atStart": at_start, "differences": "identical", "escrowed": "skip", "excluded": [] })
}

#[tokio::test]
async fn a_fix_applies_the_plan_shown_and_answers_a_conflict_with_the_fresh_run_when_it_changed() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let folder = server.props_folder("ensure props\n").await;
    server.reauthenticate_for_tools(&owner).await;
    let shown = server.call(Method::POST, SCAN, Some(&owner), None).await.json();
    tent(&folder);

    let stale = server
        .call(
            Method::POST,
            FIX,
            Some(&owner),
            Some(json!({ "plan": shown["plan"] })),
        )
        .await;
    let fresh = stale.json()["params"]["run"].clone();
    let fixed = server
        .call(
            Method::POST,
            FIX,
            Some(&owner),
            Some(json!({ "plan": fresh["plan"] })),
        )
        .await;
    let malformed = server
        .call(
            Method::POST,
            FIX,
            Some(&owner),
            Some(json!({ "plan": shown["plan"], "extra": 1 })),
        )
        .await;
    let page = server.call(Method::GET, TOOLS, Some(&owner), None).await.json();

    assert_eq!(stale.status, StatusCode::CONFLICT, "{}", stale.text);
    assert_eq!(stale.code(), "plan_changed");
    assert_ne!(fresh["plan"], shown["plan"]);
    assert_eq!(fresh["summary"]["fixable"], 2);
    assert_eq!(fresh["summary"]["filesChanged"], 0);
    assert_eq!(fixed.status, StatusCode::OK, "{}", fixed.text);
    assert_eq!(fixed.json()["summary"]["filesChanged"], 2);
    assert_eq!(malformed.status, StatusCode::BAD_REQUEST);
    assert_eq!(malformed.code(), "invalid_request");
    assert_eq!(page["tools"][0]["open"].as_array().unwrap().len(), 2);
    let entries = server.audited("tool_files_changed").await;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["actorName"], "ada");
    assert_eq!(entries[0]["details"]["count"], 2);
}

#[tokio::test]
async fn an_undo_takes_one_scope_and_gives_the_files_back() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let folder = server.props_folder("ensure props\n").await;
    let camp = fs::read(folder.join("resources/props/stream/camp.ytyp")).unwrap();
    server.reauthenticate_for_tools(&owner).await;
    let shown = server.call(Method::POST, SCAN, Some(&owner), None).await.json();
    let fixed = server
        .call(
            Method::POST,
            FIX,
            Some(&owner),
            Some(json!({ "plan": shown["plan"] })),
        )
        .await
        .json();

    let two_scopes = server
        .call(
            Method::POST,
            UNDO,
            Some(&owner),
            Some(json!({ "scope": { "run": fixed["id"], "changes": [1] } })),
        )
        .await;
    let undone = server
        .call(
            Method::POST,
            UNDO,
            Some(&owner),
            Some(json!({ "scope": { "run": fixed["id"] } })),
        )
        .await;

    assert_eq!(two_scopes.status, StatusCode::BAD_REQUEST);
    assert_eq!(undone.status, StatusCode::OK, "{}", undone.text);
    assert_eq!(undone.json()["mode"], "undo");
    assert_eq!(undone.json()["summary"]["filesRestored"], 1);
    assert_eq!(
        fs::read(folder.join("resources/props/stream/camp.ytyp")).unwrap(),
        camp
    );
    let entries = server.audited("tool_files_restored").await;
    assert_eq!(entries[0]["details"]["reason"], "undo");
}

#[tokio::test]
async fn the_settings_are_saved_over_the_version_read() {
    let server = Server::new().await;
    let owner = server.owner().await;
    server.reauthenticate_for_tools(&owner).await;

    let first = server
        .call(
            Method::PUT,
            SETTINGS,
            Some(&owner),
            Some(json!({ "settings": settings("report"), "version": null })),
        )
        .await;
    let stale = server
        .call(
            Method::PUT,
            SETTINGS,
            Some(&owner),
            Some(json!({ "settings": settings("fix"), "version": null })),
        )
        .await;
    let unknown = server
        .call(
            Method::PUT,
            SETTINGS,
            Some(&owner),
            Some(json!({ "settings": settings("always"), "version": 1 })),
        )
        .await;

    assert_eq!(first.status, StatusCode::OK, "{}", first.text);
    assert_eq!(first.json()["tools"][0]["settings"]["atStart"], "report");
    assert_eq!(first.json()["tools"][0]["version"], 1);
    assert_eq!(stale.status, StatusCode::CONFLICT);
    assert_eq!(stale.code(), "conflict");
    assert_eq!(unknown.status, StatusCode::BAD_REQUEST);
    let entries = server.audited("tool_settings_changed").await;
    assert_eq!(entries[0]["details"]["atStart"]["after"], "report");
}

#[tokio::test]
async fn without_launch_settings_or_on_redm_the_tools_say_why_they_cannot_run() {
    let server = Server::new().await;
    let owner = server.owner().await;

    let unconfigured = server.call(Method::POST, SCAN, Some(&owner), None).await;
    server.props_folder("set gamename rdr3\nensure props\n").await;
    let redm = server.call(Method::POST, SCAN, Some(&owner), None).await;

    assert_eq!(unconfigured.status, StatusCode::CONFLICT);
    assert_eq!(unconfigured.code(), "launch_not_configured");
    assert_eq!(redm.status, StatusCode::CONFLICT);
    assert_eq!(redm.code(), "tool_not_available");
    assert_eq!(redm.json()["params"]["game"], "rdr3");
}
