//! The routes and topics of the default server, without an FXServer to run: what they refuse, why,
//! and what they audit. `fake-fxserver/tests/runtime.rs` drives a server through them.

use std::fs;

use axum::http::{Method, StatusCode, header};
use jiff::SignedDuration;
use observer_domain::launch::{LaunchSettings, OneSync, StoredLaunch};
use observer_domain::server::{LaunchReadiness, ServerState};
use observer_store::{artifacts, launch};
use serde_json::{Value, json};

use super::live::{receive, send};
use super::{Server, Session};

const START: &str = "/api/servers/default/start";
const STOP: &str = "/api/servers/default/stop";
const RESTART: &str = "/api/servers/default/restart";
const CONSOLE: &str = "/api/servers/default/console";
const LOGS: &str = "/api/servers/default/console/logs";

impl Server {
    /// A folder of this test's directory that does not exist.
    fn missing(&self, name: &str) -> String {
        self.dir
            .path()
            .join("nowhere")
            .join(name)
            .to_str()
            .unwrap()
            .to_owned()
    }

    /// Settings the rules accept, for folders that are not there, and a build selected that is
    /// not there either: a start fails at the launch.
    async fn save_unlaunchable(&self, owner: &Session) {
        let now = self.clock.now();
        self.db
            .write(move |tx| {
                let state = artifacts::ArtifactState {
                    selected: Some(37872),
                    ..artifacts::ArtifactState::default()
                };
                artifacts::save_state(tx, "default", &state, now)
            })
            .await
            .unwrap();
        let settings = json!({
            "dataPath": self.missing("server-data"),
            "cfgFile": "server.cfg",
            "onesync": "on",
            "autoStart": false,
            "txadminEvents": false,
            "extraArguments": [],
        });
        let saved = self
            .call(
                Method::PUT,
                "/api/servers/default/launch",
                Some(owner),
                Some(json!({ "settings": settings, "version": null })),
            )
            .await;
        assert_eq!(saved.status, StatusCode::OK, "{}", saved.text);
    }

    async fn command(&self, session: &Session, line: &str) -> super::Reply {
        self.call(
            Method::POST,
            CONSOLE,
            Some(session),
            Some(json!({ "line": line })),
        )
        .await
    }

    pub(super) async fn audited(&self, action: &str) -> Vec<Value> {
        self.audit.publish_new().await.unwrap();
        let path = format!("/api/audit?action={action}");
        let owner = self.login("ada", super::OWNER_PASSWORD).await.session();
        let page = self.call(Method::GET, &path, Some(&owner), None).await;
        page.json()["entries"].as_array().unwrap().clone()
    }
}

#[tokio::test]
async fn starting_stopping_and_restarting_take_server_control_and_say_why_they_cannot() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let viewer = server.member(&owner, "grace", "3").await;

    for path in [START, STOP, RESTART] {
        let refused = server.call(Method::POST, path, Some(&viewer), None).await;
        assert_eq!(refused.status, StatusCode::FORBIDDEN, "{path}");
        assert_eq!(refused.json()["params"]["permission"], "serverControl");
    }
    let denied = server.audited("permission_denied").await;
    assert_eq!(denied.len(), 3);
    assert_eq!(denied[0]["target"], format!("POST {RESTART}"));

    let start = server.call(Method::POST, START, Some(&owner), None).await;
    let stop = server.call(Method::POST, STOP, Some(&owner), None).await;
    let restart = server.call(Method::POST, RESTART, Some(&owner), None).await;
    assert_eq!(
        (start.status, start.code()),
        (StatusCode::CONFLICT, json!("launch_not_configured"))
    );
    assert_eq!(
        (stop.status, stop.code()),
        (StatusCode::CONFLICT, json!("server_not_running"))
    );
    assert_eq!(restart.code(), "launch_not_configured");
    assert_eq!(server.audited("server_started").await, Vec::<Value>::new());
}

#[tokio::test]
async fn a_start_without_a_selected_build_is_refused_and_the_status_says_why() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let settings = json!({
        "dataPath": server.missing("server-data"),
        "cfgFile": "server.cfg",
        "onesync": "on",
        "autoStart": false,
        "txadminEvents": false,
        "extraArguments": [],
    });
    let saved = server
        .call(
            Method::PUT,
            "/api/servers/default/launch",
            Some(&owner),
            Some(json!({ "settings": settings, "version": null })),
        )
        .await;
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.text);

    let start = server.call(Method::POST, START, Some(&owner), None).await;
    let mut socket = server.socket(&owner).await;
    send(&mut socket, json!({ "type": "sub", "topic": "server.status" })).await;
    let status = receive(&mut socket).await;

    assert_eq!(start.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(start.code(), "no_build_selected");
    assert_eq!(status["data"]["launch"], "noBuild");
    assert_eq!(server.audited("server_started").await, Vec::<Value>::new());
}

#[tokio::test]
async fn stored_settings_this_host_refuses_are_named_before_anything_starts() {
    let server = Server::new().await;
    let owner = server.owner().await;
    // Settings stored by another platform's Observer, or before a rule was added.
    let stored = StoredLaunch {
        settings: LaunchSettings {
            data_path: "server-data".to_owned(),
            cfg_file: "server.cfg".to_owned(),
            onesync: OneSync::On,
            auto_start: true,
            txadmin_events: false,
            extra_arguments: Vec::new(),
            hang_detection: true,
            hang_timeout_seconds: 360,
        },
        version: 1,
        legacy_build_path: None,
    };
    let now = server.clock.now();
    server
        .db
        .write(move |tx| launch::save(tx, "default", &stored, now))
        .await
        .unwrap();

    let start = server.call(Method::POST, START, Some(&owner), None).await;

    assert_eq!(start.status, StatusCode::BAD_REQUEST);
    assert_eq!(start.code(), "invalid_launch_settings");
    assert_eq!(
        start.json()["params"]["issues"],
        json!([{ "field": "dataPath", "index": null, "problem": "notAbsolute" }])
    );
}

#[tokio::test]
async fn a_failed_start_names_the_step_that_failed_but_none_of_the_owners_folders() {
    let server = Server::new().await;
    let owner = server.owner().await;
    server.save_unlaunchable(&owner).await;
    // Neither reads the launch settings: they are the owner's.
    let operator = server.member(&owner, "kenji", "9").await;
    let viewer = server.member(&owner, "grace", "1").await;

    let start = server.call(Method::POST, START, Some(&operator), None).await;
    let mut socket = server.socket(&viewer).await;
    send(&mut socket, json!({ "type": "sub", "topic": "server.status" })).await;
    let status = receive(&mut socket).await;
    let started = server.audited("server_started").await;

    assert_eq!(start.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(start.code(), "launch_failed");
    assert_eq!(start.json()["params"], json!({ "reason": "missingData" }));
    assert_eq!(
        status["data"]["cause"],
        json!({ "kind": "spawnFailed", "reason": "missingData" })
    );
    assert_eq!(started.len(), 1);
    assert_eq!(started[0]["outcome"], "failure");
    assert_eq!(started[0]["actorName"], "kenji");
    assert_eq!(started[0]["details"], json!({ "reason": "missingData" }));
    for seen in [start.text, status.to_string(), started[0].to_string()] {
        assert!(!seen.contains("nowhere"), "a folder reached a non-owner: {seen}");
    }
    let (console, _) = server.supervision.console(None);
    assert!(
        console.lines.iter().any(|line| line
            .5
            .starts_with("cannot start FXServer: the server data folder")
            && line.5.contains("nowhere")),
        "the console keeps the full reason: {:?}",
        console.lines
    );
    assert_eq!(server.supervision.status().borrow().state, ServerState::Stopped);
}

#[tokio::test]
async fn a_command_that_stops_the_server_also_takes_server_control() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let console = server.member(&owner, "grace", "4").await;
    let reader = server.member(&owner, "kenji", "2").await;

    let plain = server.command(&console, "status").await;
    let refused: Vec<_> = ["quit", "stop observer", "exec other.cfg; status", "_crash"]
        .into_iter()
        .map(|line| (line, server.command(&console, line)))
        .collect();
    let read_only = server.command(&reader, "status").await;
    let owner_quits = server.command(&owner, "quit").await;

    assert_eq!(
        (plain.status, plain.code()),
        (StatusCode::CONFLICT, json!("server_not_running")),
        "a plain command passes every check, then finds no server"
    );
    for (line, reply) in refused {
        let reply = reply.await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{line}");
        assert_eq!(reply.json()["params"]["permission"], "serverControl", "{line}");
    }
    assert_eq!(read_only.json()["params"]["permission"], "consoleWrite");
    assert_eq!(owner_quits.code(), "server_not_running");
    let denied = server.audited("permission_denied").await;
    assert!(denied.iter().any(|entry| {
        entry["actorName"] == "grace"
            && entry["target"] == format!("POST {CONSOLE}")
            && entry["details"]["permission"] == "serverControl"
    }));
}

#[tokio::test]
async fn observer_cmd_is_refused_to_everyone_and_the_attempt_audited() {
    let server = Server::new().await;
    let owner = server.owner().await;

    let forged = server.command(&owner, "observer_cmd 7b7d").await;
    let disguised = server.command(&owner, r#"set x "observer\_cmd 7b7d""#).await;
    let empty = server.command(&owner, "   ").await;
    let control = server.command(&owner, "status\u{7}").await;

    for (reply, reason) in [
        (forged, "reserved"),
        (disguised, "reserved"),
        (empty, "empty"),
        (control, "controlCharacter"),
    ] {
        assert_eq!(reply.status, StatusCode::BAD_REQUEST);
        assert_eq!(reply.code(), "command_refused");
        assert_eq!(reply.json()["params"]["reason"], reason);
    }
    let attempts = server.audited("console_command_sent").await;
    assert_eq!(attempts.len(), 1, "a repeat within the window is only counted");
    assert_eq!(attempts[0]["outcome"], "denied");
    assert_eq!(
        attempts[0]["details"],
        json!({ "command": "observer_cmd 7b7d", "refusal": "reserved" })
    );
}

#[tokio::test]
async fn a_user_sends_ten_commands_in_ten_seconds_at_most() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let grace = server.member(&owner, "grace", "4").await;

    for _ in 0..10 {
        assert_eq!(
            server.command(&grace, "status").await.code(),
            "server_not_running"
        );
    }
    let eleventh = server.command(&grace, "status").await;
    let someone_else = server.command(&owner, "status").await;
    server.clock.advance(SignedDuration::from_secs(10));
    let later = server.command(&grace, "status").await;

    assert_eq!(eleventh.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(eleventh.code(), "too_many_commands");
    assert_eq!(eleventh.json()["params"]["retryAfter"], 10);
    assert_eq!(eleventh.headers[header::RETRY_AFTER], "10");
    assert_eq!(someone_else.code(), "server_not_running");
    assert_eq!(later.code(), "server_not_running");
}

#[tokio::test]
async fn log_files_are_listed_newest_first_and_downloaded_by_their_listed_name_only() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let viewer = server.member(&owner, "grace", "3").await;
    let logs = server.dir.path().join("data/logs/console");
    fs::create_dir_all(logs.join("console-2026-10-29.log")).unwrap();
    for (name, text) in [
        ("console-2026-10-27.log", "older\n"),
        (
            "console-2026-10-28.log",
            "2026-10-28T09:00:00.000+01:00 #1 OUT ready\n",
        ),
        ("console-2026-10-28.1.log", "after the rotation\n"),
        ("notes.txt", "not a console log\n"),
    ] {
        fs::write(logs.join(name), text).unwrap();
    }
    fs::write(server.dir.path().join("data/secret.txt"), "outside").unwrap();

    let listed = server.call(Method::GET, LOGS, Some(&owner), None).await;
    let names: Vec<_> = listed
        .json()
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file["name"].clone())
        .collect();
    let file = server
        .call(
            Method::GET,
            &format!("{LOGS}/console-2026-10-28.log"),
            Some(&owner),
            None,
        )
        .await;
    let mut refused = Vec::new();
    for name in [
        "notes.txt",
        "console-2026-10-29.log",
        "..%2Fsecret.txt",
        "..%5Csecret.txt",
        "console-2026-10-30.log",
    ] {
        let path = format!("{LOGS}/{name}");
        refused.push(server.call(Method::GET, &path, Some(&owner), None).await);
    }
    let forbidden = server.call(Method::GET, LOGS, Some(&viewer), None).await;

    assert_eq!(listed.status, StatusCode::OK);
    assert_eq!(
        names,
        [
            "console-2026-10-28.1.log",
            "console-2026-10-28.log",
            "console-2026-10-27.log"
        ]
    );
    assert_eq!(listed.json()[1]["bytes"], 43);
    assert!(listed.json()[1]["modifiedAt"].is_string());
    assert_eq!(file.status, StatusCode::OK);
    assert_eq!(file.text, "2026-10-28T09:00:00.000+01:00 #1 OUT ready\n");
    assert_eq!(
        file.headers[header::CONTENT_DISPOSITION],
        "attachment; filename=\"console-2026-10-28.log\""
    );
    for reply in refused {
        assert_eq!(reply.status, StatusCode::NOT_FOUND, "{}", reply.text);
    }
    assert_eq!(forbidden.json()["params"]["permission"], "logsDownload");
    let downloads = server.audited("console_log_downloaded").await;
    assert_eq!(downloads.len(), 1);
    assert_eq!(downloads[0]["target"], "console-2026-10-28.log");
}

#[tokio::test]
async fn the_server_status_topic_says_what_fxserver_does_and_whether_it_can_start() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let mut socket = server.socket(&owner).await;

    send(&mut socket, json!({ "type": "sub", "topic": "server.status" })).await;
    let snapshot = receive(&mut socket).await;
    server.save_unlaunchable(&owner).await;
    let configured = receive(&mut socket).await;
    server.call(Method::POST, START, Some(&owner), None).await;
    let mut failed = receive(&mut socket).await;
    // The state and its cause may come as two changes.
    while failed["data"]["cause"].is_null() {
        failed = receive(&mut socket).await;
    }

    assert_eq!(snapshot["type"], "snapshot");
    assert_eq!(snapshot["topic"], "server.status");
    let data = &snapshot["data"];
    assert_eq!(data["state"], "stopped");
    assert_eq!(data["launch"], json!(LaunchReadiness::NotConfigured));
    assert_eq!(data["generation"], 0);
    for unknown in ["cause", "startedAt", "build", "players", "restartAt"] {
        assert!(data[unknown].is_null(), "{unknown}");
    }
    assert_eq!(data["problems"], json!([]));
    assert_eq!(configured["type"], "event");
    assert_eq!(configured["data"]["launch"], "ready");
    assert_eq!(failed["data"]["state"], "stopped");
    assert_eq!(failed["data"]["cause"]["kind"], "spawnFailed");
}

#[tokio::test]
async fn the_console_topic_resumes_after_the_last_line_the_panel_holds() {
    let server = Server::new().await;
    let owner = server.owner().await;
    server.save_unlaunchable(&owner).await;
    // A failed start leaves a note of Observer's in the console.
    server.call(Method::POST, START, Some(&owner), None).await;
    let mut socket = server.socket(&owner).await;

    send(&mut socket, json!({ "type": "sub", "topic": "console" })).await;
    let first = receive(&mut socket).await;
    let line = first["data"]["lines"][0].clone();
    let seq = line[0].as_u64().unwrap();
    send(
        &mut socket,
        json!({ "type": "sub", "topic": "console", "since": seq }),
    )
    .await;
    let resumed = receive(&mut socket).await;
    server.call(Method::POST, START, Some(&owner), None).await;
    let next = receive(&mut socket).await;
    let from_elsewhere = json!({ "type": "sub", "topic": "console", "since": seq + 1_000_000_000 });
    send(&mut socket, from_elsewhere).await;
    let restarted = receive(&mut socket).await;

    assert_eq!(first["data"]["gap"], false);
    assert_eq!(first["data"]["last"], seq);
    assert_eq!(first["data"]["lines"].as_array().unwrap().len(), 1);
    assert_eq!(line[3], "observer");
    assert_eq!(line[4], "supervisor");
    assert!(
        line[5].as_str().unwrap().starts_with("cannot start FXServer"),
        "{line}"
    );
    assert_eq!(
        resumed["data"],
        json!({ "lines": [], "gap": false, "restarted": false, "last": seq })
    );
    assert_eq!(next["type"], "event");
    assert_eq!(next["topic"], "console");
    assert_eq!(next["data"]["lines"][0][0], seq + 1);
    assert_eq!(
        (&restarted["data"]["gap"], &restarted["data"]["restarted"]),
        (&json!(false), &json!(true)),
        "a number this Observer never gave is an earlier one's"
    );
    assert_eq!(restarted["data"]["lines"].as_array().unwrap().len(), 2);
}
