//! The server setup over HTTP: the checks of a folder and its cfg, and the save that follows them.

use std::fs;
use std::net::{Ipv4Addr, TcpListener, UdpSocket};

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use jiff::SignedDuration;
use serde_json::{Value, json};

use super::{HOST, SESSION_KEY_HEADER, Server};
use crate::api::routes;
use crate::gate::Gate;

const FOLDER: &str = "/api/servers/default/setup/folder";
const CFG: &str = "/api/servers/default/setup/cfg";
const SETUP: &str = "/api/servers/default/setup";
/// A test fixture, not a real key.
const KEY: &str = "sv_licenseKey cfxk_ObserverLabFixture_k3y";

impl Server {
    /// A server folder of this test's directory holding `chat` and `cfg` as its `server.cfg`.
    fn server_folder(&self, name: &str, cfg: &str) -> String {
        let data = self.dir.path().join(name);
        let chat = data.join("resources/chat");
        fs::create_dir_all(&chat).unwrap();
        fs::write(chat.join("fxmanifest.lua"), "fx_version 'cerulean'\n").unwrap();
        fs::write(data.join("server.cfg"), cfg).unwrap();
        data.to_str().unwrap().to_owned()
    }
}

/// A loopback TCP listener on a port whose UDP twin is free: the checks bind both, and the system
/// hands out a TCP port without looking at UDP.
fn listener() -> TcpListener {
    std::iter::repeat_with(|| TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap())
        .find(|listener| UdpSocket::bind(listener.local_addr().unwrap()).is_ok())
        .unwrap()
}

/// A loopback address whose port was free for TCP and UDP a moment ago.
fn free_address() -> String {
    listener().local_addr().unwrap().to_string()
}

fn endpoints(address: &str) -> String {
    format!("endpoint_add_tcp \"{address}\"\nendpoint_add_udp \"{address}\"\n{KEY}\n")
}

fn settings(data_path: &str) -> Value {
    json!({
        "dataPath": data_path,
        "cfgFile": "server.cfg",
        "onesync": "on",
        "autoStart": false,
        "txadminEvents": false,
        "extraArguments": [],
    })
}

fn change(data_path: &str, version: Option<i64>) -> Value {
    json!({ "settings": settings(data_path), "version": version })
}

#[tokio::test]
async fn a_folder_is_reported_on_whether_or_not_fxserver_can_run_from_it() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let live = server.server_folder("live", &endpoints(&free_address()));
    let check = |data_path: &str| json!({ "dataPath": data_path });

    let accepted = server
        .call(Method::POST, FOLDER, Some(&owner), Some(check(&live)))
        .await;
    let relative = server
        .call(Method::POST, FOLDER, Some(&owner), Some(check("server-data")))
        .await;
    let resources = server
        .call(
            Method::POST,
            FOLDER,
            Some(&owner),
            Some(check(&format!("{live}{}resources", std::path::MAIN_SEPARATOR))),
        )
        .await;
    let unknown = server
        .call(
            Method::POST,
            FOLDER,
            Some(&owner),
            Some(json!({ "dataPath": live, "cfgFile": "server.cfg" })),
        )
        .await;

    assert_eq!(accepted.status, StatusCode::OK, "{}", accepted.text);
    assert_eq!(
        accepted.json(),
        json!({
            "problem": null,
            "pathProblem": null,
            "suggestion": null,
            "resources": { "resources": 1, "categories": 0, "withoutManifest": [], "truncated": false },
            "cfgFiles": ["server.cfg"],
            "detectedCfg": "server.cfg",
        })
    );
    assert_eq!(relative.status, StatusCode::OK, "the report is the answer");
    assert_eq!(relative.json()["problem"], "invalidPath");
    assert_eq!(relative.json()["pathProblem"], "notAbsolute");
    assert_eq!(resources.json()["problem"], "isResources");
    assert_eq!(resources.json()["suggestion"], live);
    assert_eq!(unknown.code(), "invalid_request");
}

#[tokio::test]
async fn a_cfg_is_reported_with_its_files_facts_and_findings() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let address = free_address();
    let live = server.server_folder(
        "live",
        &format!(
            "{}sv_hostname \"Observer lab\"\nsv_maxclients 64\nensure monitor\nset rcon_password \"Hunter2-rcon\"\n",
            endpoints(&address)
        ),
    );
    let check = json!({ "dataPath": live, "cfgFile": "server.cfg", "onesync": "off" });

    let reply = server.call(Method::POST, CFG, Some(&owner), Some(check)).await;

    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
    let report = reply.json();
    assert_eq!(
        (&report["folder"], &report["problem"], &report["pathProblem"]),
        (&Value::Null, &Value::Null, &Value::Null)
    );
    assert_eq!(
        report["files"],
        json!([{ "path": "server.cfg", "state": "read" }])
    );
    assert_eq!(
        report["facts"],
        json!({
            "game": null,
            "endpoints": [
                { "protocol": "tcp", "address": address },
                { "protocol": "udp", "address": address },
            ],
            "maxClients": 64,
            "onesync": null,
            "hostname": "Observer lab",
            "licence": "set",
            "gameBuild": null,
        })
    );
    assert_eq!(
        report["findings"],
        json!([
            {
                "kind": "startsMonitor",
                "severity": "warning",
                "file": "server.cfg",
                "line": 6,
                "params": { "name": "monitor" },
            },
            {
                "kind": "maxClientsNeedOneSync",
                "severity": "warning",
                "file": "server.cfg",
                "line": 5,
                "params": { "count": 64 },
            },
        ])
    );
    assert!(!reply.text.contains("Hunter2-rcon"));
    assert!(!reply.text.contains("cfxk_"));
}

#[tokio::test]
async fn the_setup_saves_only_what_lets_fxserver_start_and_audits_it() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let live = server.server_folder("live", &endpoints(&free_address()));
    let silent = server.server_folder("silent", &format!("{KEY}\n"));
    let empty = server.dir.path().join("empty");
    fs::create_dir_all(empty.join("resources")).unwrap();

    let folder_refused = server
        .call(
            Method::PUT,
            SETUP,
            Some(&owner),
            Some(change(empty.to_str().unwrap(), None)),
        )
        .await;
    let finding_refused = server
        .call(Method::PUT, SETUP, Some(&owner), Some(change(&silent, None)))
        .await;
    let saved = server
        .call(Method::PUT, SETUP, Some(&owner), Some(change(&live, None)))
        .await;
    let other = server.server_folder("other", &endpoints(&free_address()));
    let stale = server
        .call(Method::PUT, SETUP, Some(&owner), Some(change(&other, None)))
        .await;

    assert_eq!(folder_refused.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        folder_refused.json(),
        json!({ "code": "setup_blocked", "params": { "folder": "emptyResources" } })
    );
    assert_eq!(
        finding_refused.json(),
        json!({ "code": "setup_blocked", "params": { "findings": ["noEndpoint"] } })
    );
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.text);
    assert_eq!(
        saved.json(),
        json!({ "settings": settings(&live), "version": 1, "legacyBuildPath": null })
    );
    assert_eq!(stale.code(), "conflict");
    let read = server
        .call(Method::GET, "/api/servers/default/launch", Some(&owner), None)
        .await;
    assert_eq!(read.json(), saved.json());
    let changes = server.audited("launch_settings_changed").await;
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0]["details"]["setup"], true);
}

#[tokio::test]
async fn a_port_held_on_this_host_warns_without_blocking_the_save() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let held = listener();
    let address = held.local_addr().unwrap().to_string();
    let live = server.server_folder("live", &endpoints(&address));
    let check = json!({ "dataPath": live, "cfgFile": "server.cfg", "onesync": "on" });

    let report = server.call(Method::POST, CFG, Some(&owner), Some(check)).await;
    let saved = server
        .call(Method::PUT, SETUP, Some(&owner), Some(change(&live, None)))
        .await;

    let in_use: Vec<_> = report.json()["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|finding| finding["kind"] == "portInUse")
        .cloned()
        .collect();
    assert_eq!(
        in_use,
        [json!({
            "kind": "portInUse",
            "severity": "warning",
            "file": "server.cfg",
            "line": 1,
            "params": { "protocol": "tcp", "address": address },
        })]
    );
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.text);
}

#[tokio::test]
async fn only_the_owner_checks_and_saves_and_the_save_takes_the_password_again() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let grace = server.member(&owner, "grace", "fff").await;
    let live = server.server_folder("live", &endpoints(&free_address()));
    let folder = json!({ "dataPath": live });
    let cfg = json!({ "dataPath": live, "cfgFile": "server.cfg", "onesync": "on" });

    let mut refused = Vec::new();
    for (method, path, body) in [
        (Method::POST, FOLDER, folder.clone()),
        (Method::POST, CFG, cfg),
        (Method::PUT, SETUP, change(&live, None)),
    ] {
        let reply = server.call(method.clone(), path, Some(&grace), Some(body)).await;
        refused.push((format!("{method} {path}"), reply.status, reply.code()));
    }
    server.clock.advance(SignedDuration::from_mins(11));
    let checked_later = server
        .call(Method::POST, FOLDER, Some(&owner), Some(folder.clone()))
        .await;
    let saved_later = server
        .call(Method::PUT, SETUP, Some(&owner), Some(change(&live, None)))
        .await;
    let elsewhere = Request::post(FOLDER)
        .header(header::HOST, HOST)
        .header(header::ORIGIN, "http://observer.test:41976")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, &owner.cookie)
        .header(SESSION_KEY_HEADER, &owner.key)
        .body(Body::from(folder.to_string()))
        .unwrap();
    let from_elsewhere = server.send(elsewhere).await;

    for (route, status, code) in &refused {
        assert_eq!(
            (*status, code.as_str()),
            (StatusCode::FORBIDDEN, Some("owner_only")),
            "{route}"
        );
    }
    let denied: Vec<_> = server
        .audited("permission_denied")
        .await
        .iter()
        .map(|entry| entry["target"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        denied,
        [
            format!("PUT {SETUP}"),
            format!("POST {CFG}"),
            format!("POST {FOLDER}")
        ]
    );
    assert_eq!(checked_later.status, StatusCode::OK, "the checks change nothing");
    assert_eq!(saved_later.code(), "reauth_required");
    assert_eq!(from_elsewhere.code(), "origin_mismatch");
}

/// Every route of the setup is gated: the route table's own tests go through them too.
#[test]
fn the_route_table_declares_the_setup_routes_owner_only() {
    let gates: Vec<_> = routes()
        .into_iter()
        .filter(|route| route.path.starts_with("/servers/default/setup"))
        .map(|route| (route.method, route.path, route.gate))
        .collect();

    assert_eq!(
        gates,
        [
            (Method::POST, "/servers/default/setup/folder", Gate::Owner),
            (Method::POST, "/servers/default/setup/cfg", Gate::Owner),
            (Method::PUT, "/servers/default/setup", Gate::ReauthenticatedOwner),
        ]
    );
}
