//! The diagnostics over HTTP: who may follow the alerts and the records, and the crash files only
//! the owner gets, with the password again.

use std::fs;

use axum::http::{Method, StatusCode, header};
use jiff::SignedDuration;
use observer_domain::diagnostics::{AnalysisState, CrashFile, CrashFileKind, CrashKind};
use observer_store::diagnostics::{self, NewCrash};
use serde_json::json;

use super::live::{receive, send};
use super::{OWNER_PASSWORD, Server};

const DIAGNOSTICS_VIEW: &str = "1000";
const DASHBOARD_VIEW: &str = "1";

impl Server {
    /// A crash whose files `files` are written, of `size` bytes each, into Observer's crash folder.
    async fn crash_with(&self, files: &[(&str, u64)]) -> i64 {
        let folder = self.dir.path().join("data/crashes");
        fs::create_dir_all(&folder).unwrap();
        for (name, size) in files {
            fs::File::create(folder.join(name))
                .unwrap()
                .set_len(*size)
                .unwrap();
        }
        let files: Vec<CrashFile> = files
            .iter()
            .map(|(name, size)| CrashFile {
                name: (*name).to_owned(),
                bytes: *size,
                kind: CrashFileKind::of(name),
                kept: true,
            })
            .collect();
        let at = self.clock.now();
        self.db
            .write(move |tx| {
                diagnostics::insert_crash(
                    tx,
                    &NewCrash {
                        at,
                        kind: CrashKind::Crash,
                        run_started: Some(at),
                        generation: Some(1),
                        build: Some(35245),
                        exit: Some("exit code 0xFFFFFFFE".to_owned()),
                        exit_code: Some("0xFFFFFFFE".to_owned()),
                        signal: None,
                        hang_cause: None,
                        signature: None,
                        analysis_state: AnalysisState::Unreadable,
                        watchdog_stack: None,
                        files,
                    },
                )
            })
            .await
            .unwrap()
    }

    /// The entries of `action` the audit log holds, newest first.
    async fn entries(&self, action: &str) -> Vec<serde_json::Value> {
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

    async fn reauthenticate(&self, session: &super::Session) {
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

fn file_route(id: i64, name: &str) -> String {
    format!("/api/servers/default/crashes/{id}/files/{name}")
}

fn files_route(id: i64) -> String {
    format!("/api/servers/default/crashes/{id}/files")
}

#[tokio::test]
async fn the_alerts_open_to_the_dashboard_and_the_records_to_their_own_permission() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let dashboard = server.member(&owner, "grace", DASHBOARD_VIEW).await;
    let diagnostician = server.member(&owner, "hopper", DIAGNOSTICS_VIEW).await;

    let mut grace = server.socket(&dashboard).await;
    send(&mut grace, json!({ "type": "sub", "topic": "server.alerts" })).await;
    let alerts = receive(&mut grace).await;
    send(
        &mut grace,
        json!({ "type": "sub", "topic": "server.diagnostics" }),
    )
    .await;
    let refused = receive(&mut grace).await;
    let mut hopper = server.socket(&diagnostician).await;
    send(&mut hopper, json!({ "type": "sub", "topic": "server.alerts" })).await;
    let hopper_alerts = receive(&mut hopper).await;
    send(
        &mut hopper,
        json!({ "type": "sub", "topic": "server.diagnostics" }),
    )
    .await;
    let snapshot = receive(&mut hopper).await;

    assert_eq!(alerts["type"], "snapshot");
    assert_eq!(alerts["data"]["alerts"], json!([]));
    assert_eq!(
        refused,
        json!({ "type": "unsubscribed", "topic": "server.diagnostics", "reason": "forbidden" })
    );
    assert_eq!(hopper_alerts["topic"], "server.alerts");
    assert_eq!(snapshot["type"], "snapshot");
    assert_eq!(snapshot["topic"], "server.diagnostics");
    assert_eq!(snapshot["data"]["crashFolder"]["maxFiles"], 10);
    assert_eq!(snapshot["data"]["processes"], json!([]));
}

#[tokio::test]
async fn a_crash_file_goes_to_the_owner_alone_with_the_password_entered_again() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let everything = server.member(&owner, "grace", "7fff").await;
    let id = server
        .crash_with(&[
            ("6c1f0b2e.dmp", 1_300),
            ("6c1f0b2e-full.dmp", 64 * 1024 * 1024 + 1),
        ])
        .await;

    let read_by_other = server
        .call(
            Method::GET,
            &file_route(id, "6c1f0b2e.dmp"),
            Some(&everything),
            None,
        )
        .await;
    let deleted_by_other = server
        .call(Method::DELETE, &files_route(id), Some(&everything), None)
        .await;
    server.clock.advance(SignedDuration::from_mins(11));
    let stale = server
        .call(Method::GET, &file_route(id, "6c1f0b2e.dmp"), Some(&owner), None)
        .await;
    server.reauthenticate(&owner).await;
    let file = server
        .call(Method::GET, &file_route(id, "6c1f0b2e.dmp"), Some(&owner), None)
        .await;
    let full = server
        .call(
            Method::GET,
            &file_route(id, "6c1f0b2e-full.dmp"),
            Some(&owner),
            None,
        )
        .await;
    let unlisted = server
        .call(
            Method::GET,
            &file_route(id, "..%2Fobserver.sqlite3"),
            Some(&owner),
            None,
        )
        .await;
    let unknown = server
        .call(
            Method::GET,
            &file_route(id + 1, "6c1f0b2e.dmp"),
            Some(&owner),
            None,
        )
        .await;

    assert_eq!(read_by_other.code(), "owner_only");
    assert_eq!(deleted_by_other.code(), "owner_only");
    assert_eq!(stale.code(), "reauth_required");
    assert_eq!(file.status, StatusCode::OK);
    assert_eq!(file.text.len(), 1_300);
    assert_eq!(file.headers[header::CONTENT_TYPE], "application/octet-stream");
    assert_eq!(
        file.headers[header::CONTENT_DISPOSITION],
        "attachment; filename=\"6c1f0b2e.dmp\""
    );
    assert_eq!(full.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(full.json()["params"]["maxBytes"], 64 * 1024 * 1024);
    assert_eq!(unlisted.code(), "crash_file_not_found");
    assert_eq!(unknown.code(), "crash_not_found");
    let downloads = server.entries("crash_file_downloaded").await;
    assert_eq!(downloads.len(), 1);
    assert_eq!(
        downloads[0]["details"],
        json!({ "crash": id, "file": "6c1f0b2e.dmp", "bytes": 1_300 })
    );
    let denied = server.entries("permission_denied").await;
    let targets: Vec<&str> = denied
        .iter()
        .filter(|entry| entry["details"]["ownerOnly"] == true)
        .filter_map(|entry| entry["target"].as_str())
        .collect();
    assert_eq!(targets.len(), 2, "{denied:?}");
    assert!(targets.contains(&"GET /api/servers/default/crashes/{id}/files/{name}"));
    assert!(targets.contains(&"DELETE /api/servers/default/crashes/{id}/files"));
}

#[tokio::test]
async fn deleting_a_crash_s_files_keeps_its_record_and_is_audited() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let id = server
        .crash_with(&[("a1b2.dmp", 100), ("a1b2-full.dmp", 400)])
        .await;

    let deleted = server
        .call(Method::DELETE, &files_route(id), Some(&owner), None)
        .await;
    let gone = server
        .call(Method::GET, &file_route(id, "a1b2.dmp"), Some(&owner), None)
        .await;

    assert_eq!(deleted.status, StatusCode::OK, "{}", deleted.text);
    let files = deleted.json()["files"].clone();
    assert_eq!(files[0]["kept"], false);
    assert_eq!(files[1]["kept"], false);
    assert!(!server.dir.path().join("data/crashes/a1b2.dmp").exists());
    assert_eq!(gone.code(), "crash_file_not_found");
    let audited = server.entries("crash_files_deleted").await;
    assert_eq!(
        audited[0]["details"],
        json!({ "crash": id, "names": ["a1b2.dmp", "a1b2-full.dmp"], "bytes": 500 })
    );
}
