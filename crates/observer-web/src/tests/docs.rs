//! The documentation's source over HTTP: who reads it, that what it serves is the contract this
//! build ships, and who is told what the server does of it.

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};

use super::Server;

const DOCS: &str = "/api/docs/integration";

#[tokio::test]
async fn any_signed_in_account_reads_the_contract_this_build_ships() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let nobody = server.member(&owner, "alan", "0").await;

    let read = server.call(Method::GET, DOCS, Some(&nobody), None).await;
    let stranger = server.get(DOCS).await;

    assert_eq!(read.status, StatusCode::OK, "{}", read.text);
    assert_eq!(stranger.status, StatusCode::UNAUTHORIZED);
    let docs = read.json();
    // Whether the server runs is the dashboard's to say.
    assert_eq!(docs["server"], Value::Null);
    let contract = docs["contract"].as_str().unwrap();
    assert!(contract.starts_with("# Integration contract\n"));
    let version = docs["contractVersion"].as_u64().unwrap();
    assert!(contract.contains(&format!("contract version **{version}**")));
    let events: Vec<&str> = docs["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(events.contains(&"observer:shuttingDown"), "{events:?}");
    for event in events {
        assert!(contract.contains(&format!("### `{event}`")), "{event}");
    }
    assert_eq!(
        docs["exports"][0],
        json!({ "name": "getContractVersion", "parameters": [] })
    );
    let example: Vec<&str> = docs["example"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|file| file["name"].as_str())
        .collect();
    assert_eq!(
        example,
        ["observer_example/fxmanifest.lua", "observer_example/server.lua"]
    );
}

#[tokio::test]
async fn the_servers_side_goes_to_whoever_sees_the_dashboard_and_follows_the_launch_settings() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let viewer = server.member(&owner, "grace", "1").await;

    let before = server.call(Method::GET, DOCS, Some(&owner), None).await.json();
    let settings = json!({
        "dataPath": server.dir.path().join("server-data").to_str().unwrap(),
        "cfgFile": "server.cfg",
        "onesync": "on",
        "autoStart": false,
        "txadminEvents": true,
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
    let after = server.call(Method::GET, DOCS, Some(&owner), None).await.json();
    let seen = server.call(Method::GET, DOCS, Some(&viewer), None).await.json();

    assert_eq!(
        before["server"],
        json!({ "bridge": null, "starting": false, "txadminEvents": null, "txadminEventsNext": null })
    );
    // No server runs: only the next start is known.
    assert_eq!(
        after["server"],
        json!({ "bridge": null, "starting": false, "txadminEvents": null, "txadminEventsNext": true })
    );
    assert_eq!(seen["server"], after["server"]);
}
