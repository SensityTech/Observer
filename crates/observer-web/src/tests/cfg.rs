//! The launch's cfg over HTTP: who reads it and what they are shown of its secrets, what a draft
//! is told without anything written, the password again for a save, and the versions kept.

use std::fs;
use std::path::PathBuf;

use axum::http::{Method, StatusCode};
use jiff::SignedDuration;
use observer_domain::launch::{LaunchSettings, OneSync, StoredLaunch};
use observer_store::launch;
use serde_json::{Value, json};

use super::{OWNER_PASSWORD, Server, Session};

const CFG: &str = "/api/servers/default/cfg";
const CHECK: &str = "/api/servers/default/cfg/check";
const DIFF: &str = "/api/servers/default/cfg/diff";
const VERSIONS: &str = "/api/servers/default/cfg/versions";

const KEY: &str = "cfxk_1a2b3c4d5e6f7g8h9i0j_AbC123";
const DATABASE: &str = "mysql://observer:hunter2@localhost/lab";

/// A cfg with Windows line ends, a licence key and a database password.
fn cfg() -> String {
    format!(
        "endpoint_add_tcp \"0.0.0.0:30120\"\r\nendpoint_add_udp \"0.0.0.0:30120\"\r\n\
         sv_hostname \"Lab\"\r\nsv_maxclients 48\r\nsv_licenseKey {KEY}\r\n\
         set mysql_connection_string \"{DATABASE}\"\r\nensure chat\r\n#ensure maps\r\n"
    )
}

impl Server {
    /// A server folder with the resources `chat` and `maps` and the cfg above, saved as the
    /// launch settings' folder. The cfg's path.
    async fn cfg_folder(&self) -> PathBuf {
        let folder = self.dir.path().join("server-data");
        for resource in ["chat", "maps"] {
            let resource = folder.join("resources").join(resource);
            fs::create_dir_all(&resource).unwrap();
            fs::write(resource.join("fxmanifest.lua"), "fx_version 'cerulean'\n").unwrap();
        }
        fs::write(folder.join("server.cfg"), cfg()).unwrap();
        let stored = StoredLaunch {
            settings: LaunchSettings {
                data_path: folder.to_string_lossy().into_owned(),
                cfg_file: "server.cfg".to_owned(),
                onesync: OneSync::On,
                auto_start: false,
                txadmin_events: false,
                extra_arguments: Vec::new(),
                hang_detection: true,
                hang_timeout_seconds: 360,
                stop_hold_seconds: 60,
            },
            version: 1,
            legacy_build_path: None,
        };
        let now = self.clock.now();
        self.db
            .write(move |tx| launch::save(tx, "default", &stored, now))
            .await
            .unwrap();
        folder.join("server.cfg")
    }

    async fn password_again(&self, session: &Session) {
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

    async fn cfg_document(&self, owner: &Session) -> Value {
        let reply = self.call(Method::GET, CFG, Some(owner), None).await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
        reply.json()
    }
}

/// What a review found, as kind and line, but for the endpoints another program of this host
/// holds: whether their port is free is the machine's business, not the editor's.
fn found(review: &Value) -> Vec<(String, u64)> {
    review["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|finding| finding["kind"] != "portInUse")
        .map(|finding| {
            (
                finding["kind"].as_str().unwrap().to_owned(),
                finding["line"].as_u64().unwrap(),
            )
        })
        .collect()
}

fn holds_no_secret(text: &str) {
    assert!(!text.contains("hunter2") && !text.contains("1a2b3c"), "{text}");
}

#[tokio::test]
async fn the_cfg_is_the_owners_to_read_with_a_mark_in_place_of_each_secret() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let manager = server.member(&owner, "grace", "ffff").await;

    let unset = server.call(Method::GET, CFG, Some(&owner), None).await;
    assert_eq!(
        (
            unset.status,
            unset.code(),
            unset.json()["params"]["reason"].clone()
        ),
        (
            StatusCode::CONFLICT,
            json!("cfg_not_editable"),
            json!("notConfigured")
        )
    );
    server.cfg_folder().await;
    for (method, path, body) in [
        (Method::GET, CFG, None),
        (Method::POST, CHECK, Some(json!({ "text": "" }))),
        (Method::POST, DIFF, Some(json!({ "text": "" }))),
        (Method::PUT, CFG, Some(json!({ "text": "", "base": "" }))),
        (Method::GET, VERSIONS, None),
    ] {
        let refused = server.call(method, path, Some(&manager), body).await;
        assert_eq!(refused.status, StatusCode::FORBIDDEN, "{path}");
        assert_eq!(refused.code(), "owner_only", "{path}");
    }

    let reply = server.call(Method::GET, CFG, Some(&owner), None).await;
    holds_no_secret(&reply.text);
    let document = reply.json();
    assert_eq!(document["file"], "server.cfg");
    assert_eq!(document["secrets"], 2);
    assert_eq!(document["versions"], 0);
    assert_eq!(document["bytes"], cfg().len());
    assert_eq!(document["base"].as_str().unwrap().len(), 64);
    let text = document["text"].as_str().unwrap();
    assert!(
        text.contains("sv_licenseKey ") && text.contains("<<secret-1>>") && text.contains("<<secret-2>>")
    );
    assert!(!text.contains('\r'));
    let outline = &document["check"]["outline"];
    assert_eq!(
        outline["resources"],
        json!([
            { "line": 7, "name": "chat", "enabled": true, "alone": true, "missing": false },
            { "line": 8, "name": "maps", "enabled": false, "alone": true, "missing": false },
        ])
    );
    assert_eq!(outline["settings"][1]["name"], "sv_maxclients");
    assert_eq!(outline["settings"][1]["value"], "48");
    assert_eq!(outline["settings"][2]["value"], Value::Null, "the licence key");
    assert_eq!(outline["endpoints"][0]["address"], "0.0.0.0:30120");
    assert_eq!(found(&document["check"]), []);
    assert_eq!(
        (&document["check"]["added"], &document["check"]["removed"]),
        (&json!(0), &json!(0))
    );
}

#[tokio::test]
async fn a_draft_is_checked_and_compared_with_the_file_without_anything_written() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let path = server.cfg_folder().await;
    let document = server.cfg_document(&owner).await;
    let draft = document["text"]
        .as_str()
        .unwrap()
        .replace("sv_maxclients 48", "sv_maxclients 64")
        + "ensure gone\n";

    let check = server
        .call(Method::POST, CHECK, Some(&owner), Some(json!({ "text": draft })))
        .await;
    let diff = server
        .call(Method::POST, DIFF, Some(&owner), Some(json!({ "text": draft })))
        .await;

    assert_eq!(check.status, StatusCode::OK, "{}", check.text);
    let check = check.json();
    assert_eq!(
        check["outline"]["resources"][2],
        json!({ "line": 9, "name": "gone", "enabled": true, "alone": true, "missing": true })
    );
    assert_eq!(
        found(&check),
        [("resourceMissing".to_owned(), 9)],
        "the licence key is read behind its mark"
    );
    assert_eq!((&check["added"], &check["removed"]), (&json!(2), &json!(1)));
    assert_eq!(diff.status, StatusCode::OK, "{}", diff.text);
    holds_no_secret(&diff.text);
    assert_eq!(
        (&diff.json()["added"], &diff.json()["removed"]),
        (&json!(2), &json!(1))
    );
    assert_eq!(fs::read_to_string(path).unwrap(), cfg(), "nothing is written");

    let unknown = server
        .call(
            Method::POST,
            DIFF,
            Some(&owner),
            Some(json!({ "text": "rcon_password <<secret-9>>\n" })),
        )
        .await;
    assert_eq!(
        (
            unknown.status,
            unknown.code(),
            unknown.json()["params"]["reason"].clone()
        ),
        (
            StatusCode::BAD_REQUEST,
            json!("invalid_cfg_text"),
            json!("unknownMark")
        )
    );
}

#[tokio::test]
async fn a_save_takes_the_password_again_keeps_the_file_as_it_was_and_is_audited() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let path = server.cfg_folder().await;
    server.clock.advance(SignedDuration::from_mins(11));
    let document = server.cfg_document(&owner).await;
    let edited = document["text"]
        .as_str()
        .unwrap()
        .replace("sv_hostname \"Lab\"", "sv_hostname \"Caf\u{e9} RP\"")
        .replace("#ensure maps", "ensure maps");
    let save = json!({ "text": edited, "base": document["base"] });

    let without_password = server
        .call(Method::PUT, CFG, Some(&owner), Some(save.clone()))
        .await;
    assert_eq!(
        (without_password.status, without_password.code()),
        (StatusCode::FORBIDDEN, json!("reauth_required"))
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), cfg());
    server.password_again(&owner).await;
    let saved = server
        .call(Method::PUT, CFG, Some(&owner), Some(save.clone()))
        .await;

    assert_eq!(saved.status, StatusCode::OK, "{}", saved.text);
    holds_no_secret(&saved.text);
    let written = fs::read_to_string(&path).unwrap();
    assert_eq!(
        written,
        cfg()
            .replace("sv_hostname \"Lab\"", "sv_hostname \"Caf\u{e9} RP\"")
            .replace("#ensure maps", "ensure maps"),
        "the secrets and the line ends are the file's own"
    );
    let saved = saved.json();
    assert_eq!(saved["versions"], 1);
    assert_ne!(saved["base"], document["base"]);
    assert_eq!(saved["check"]["outline"]["resources"][1]["enabled"], true);
    let leftovers: Vec<_> = fs::read_dir(path.parent().unwrap())
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name != "server.cfg" && name != "resources")
        .collect();
    assert_eq!(leftovers, Vec::<String>::new());

    let audited = server.audited("server_cfg_changed").await;
    assert_eq!(audited.len(), 1);
    assert_eq!(audited[0]["target"], "server.cfg");
    assert_eq!(audited[0]["details"], json!({ "added": 2, "removed": 2 }));
    assert_eq!(audited[0]["actorName"], "ada");

    let stale = server.call(Method::PUT, CFG, Some(&owner), Some(save)).await;
    assert_eq!(
        (stale.status, stale.code()),
        (StatusCode::CONFLICT, json!("cfg_changed"))
    );
    let unknown = json!({ "text": "rcon_password <<secret-7>>\n", "base": saved["base"] });
    let unknown = server.call(Method::PUT, CFG, Some(&owner), Some(unknown)).await;
    assert_eq!(
        (unknown.status, unknown.code()),
        (StatusCode::BAD_REQUEST, json!("invalid_cfg_text"))
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), written);
    assert_eq!(server.audited("server_cfg_changed").await.len(), 1);
}

#[tokio::test]
async fn an_earlier_version_is_put_back_and_the_file_it_replaces_is_kept_too() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let path = server.cfg_folder().await;
    server.clock.advance(SignedDuration::from_mins(11));
    server.password_again(&owner).await;
    let document = server.cfg_document(&owner).await;
    let edited = document["text"].as_str().unwrap().replace("48", "64");
    let saved = server
        .call(
            Method::PUT,
            CFG,
            Some(&owner),
            Some(json!({ "text": edited, "base": document["base"] })),
        )
        .await
        .json();

    let versions = server
        .call(Method::GET, VERSIONS, Some(&owner), None)
        .await
        .json();
    let version = versions[0]["id"].as_i64().unwrap();
    assert_eq!(versions.as_array().unwrap().len(), 1);
    assert_eq!(versions[0]["replacedBy"], "ada");
    assert_eq!(
        (&versions[0]["bytes"], &versions[0]["lines"]),
        (&json!(cfg().len()), &json!(8))
    );
    let diff = server
        .call(Method::GET, &format!("{VERSIONS}/{version}"), Some(&owner), None)
        .await;
    holds_no_secret(&diff.text);
    assert_eq!(
        (&diff.json()["added"], &diff.json()["removed"]),
        (&json!(1), &json!(1))
    );
    let missing = server
        .call(Method::GET, &format!("{VERSIONS}/999"), Some(&owner), None)
        .await;
    assert_eq!(
        (missing.status, missing.code()),
        (StatusCode::NOT_FOUND, json!("cfg_version_not_found"))
    );

    let stale = server
        .call(
            Method::POST,
            &format!("{VERSIONS}/{version}/restore"),
            Some(&owner),
            Some(json!({ "base": document["base"] })),
        )
        .await;
    assert_eq!(
        (stale.status, stale.code()),
        (StatusCode::CONFLICT, json!("cfg_changed"))
    );
    let restored = server
        .call(
            Method::POST,
            &format!("{VERSIONS}/{version}/restore"),
            Some(&owner),
            Some(json!({ "base": saved["base"] })),
        )
        .await;

    assert_eq!(restored.status, StatusCode::OK, "{}", restored.text);
    assert_eq!(fs::read_to_string(&path).unwrap(), cfg());
    assert_eq!(restored.json()["versions"], 2);
    assert_eq!(restored.json()["base"], document["base"]);
    let audited = server.audited("server_cfg_restored").await;
    assert_eq!(
        audited[0]["details"],
        json!({ "version": version, "added": 1, "removed": 1 })
    );
}

#[tokio::test]
async fn a_cfg_that_cannot_be_written_back_as_it_is_read_is_not_edited() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let path = server.cfg_folder().await;
    let reason = |reply: super::Reply| {
        (
            reply.status,
            reply.code(),
            reply.json()["params"]["reason"].clone(),
        )
    };
    let refused = |reason: &str| (StatusCode::CONFLICT, json!("cfg_not_editable"), json!(reason));

    fs::write(&path, b"sv_hostname \"Caf\xe9\"\n").unwrap();
    assert_eq!(
        reason(server.call(Method::GET, CFG, Some(&owner), None).await),
        refused("notUtf8")
    );
    fs::write(&path, "sv_hostname \"<<secret-1>>\"\n").unwrap();
    assert_eq!(
        reason(server.call(Method::GET, CFG, Some(&owner), None).await),
        refused("holdsMarks")
    );
    fs::write(&path, "#".repeat(512 * 1024 + 1)).unwrap();
    assert_eq!(
        reason(server.call(Method::GET, CFG, Some(&owner), None).await),
        refused("tooLarge")
    );
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&path, permissions.clone()).unwrap();
    assert_eq!(
        reason(server.call(Method::GET, CFG, Some(&owner), None).await),
        refused("readOnly")
    );
    #[expect(
        clippy::permissions_set_readonly_false,
        reason = "the test's own file, to delete it"
    )]
    permissions.set_readonly(false);
    fs::set_permissions(&path, permissions).unwrap();
    fs::remove_file(&path).unwrap();
    assert_eq!(
        reason(server.call(Method::GET, CFG, Some(&owner), None).await),
        refused("missing")
    );
    fs::create_dir(&path).unwrap();
    assert_eq!(
        reason(server.call(Method::GET, CFG, Some(&owner), None).await),
        refused("notAFile")
    );
}
