//! The schedule's routes and topic, without an FXServer to restart: who may do what, and what a
//! change answers and audits. `fake-fxserver/tests/runtime.rs` restarts a server through them.

use axum::http::{Method, StatusCode};
use jiff::SignedDuration;
use serde_json::{Value, json};

use super::live::{receive, send};
use super::{OWNER_PASSWORD, Server, Session};

const SCHEDULE: &str = "/api/servers/default/schedule";
const SKIP: &str = "/api/servers/default/schedule/skip";
const DELAY: &str = "/api/servers/default/schedule/delay";
const ONCE: &str = "/api/servers/default/schedule/once";

fn daily(times: &[&str]) -> Value {
    json!({
        "enabled": true,
        "mode": "fixedTimes",
        "times": times,
        "intervalMinutes": 360,
        "timeZone": "Europe/Paris",
        "warnings": [30, 15, 10, 5, 4, 3, 2, 1],
        "skipAfterStart": 120,
    })
}

impl Server {
    async fn save_schedule(&self, session: &Session, settings: Value, version: Value) -> super::Reply {
        self.call(
            Method::PUT,
            SCHEDULE,
            Some(session),
            Some(json!({ "settings": settings, "version": version })),
        )
        .await
    }

    async fn schedule_entries(&self, action: &str) -> Vec<Value> {
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

#[tokio::test]
async fn a_single_restart_takes_server_control_and_the_schedule_permission_is_not_enough() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let planner = server.member(&owner, "grace", "10").await;
    let operator = server.member(&owner, "kenji", "8").await;

    let one_off = server
        .call(Method::PUT, ONCE, Some(&planner), Some(json!({ "inMinutes": 5 })))
        .await;
    let cancelled = server.call(Method::DELETE, ONCE, Some(&planner), None).await;
    let skipped = server.call(Method::POST, SKIP, Some(&planner), None).await;
    let by_the_operator = server
        .call(
            Method::PUT,
            ONCE,
            Some(&operator),
            Some(json!({ "inMinutes": 5 })),
        )
        .await;
    let operator_skips = server.call(Method::POST, SKIP, Some(&operator), None).await;

    for refused in [&one_off, &cancelled] {
        assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.text);
        assert_eq!(refused.json()["params"]["permission"], "serverControl");
    }
    assert_eq!(
        (skipped.status, skipped.code()),
        (StatusCode::CONFLICT, json!("nothing_to_skip")),
        "past the gate: the schedule is off"
    );
    assert_eq!(
        (by_the_operator.status, by_the_operator.code()),
        (StatusCode::CONFLICT, json!("server_not_running"))
    );
    assert_eq!(operator_skips.json()["params"]["permission"], "scheduleManage");
    let denied = server.schedule_entries("permission_denied").await;
    assert_eq!(denied.len(), 3, "both permissions are sensitive: {denied:#?}");
}

#[tokio::test]
async fn changing_the_schedule_takes_the_password_again_and_audits_what_changed() {
    let server = Server::new().await;
    let owner = server.owner().await;

    let saved = server
        .save_schedule(&owner, daily(&["16:00", "04:00"]), json!(null))
        .await;
    let again = server
        .save_schedule(&owner, daily(&["04:00", "16:00"]), json!(1))
        .await;
    let stale = server.save_schedule(&owner, daily(&["05:00"]), json!(null)).await;
    server.clock.advance(SignedDuration::from_mins(11));
    let later = server.save_schedule(&owner, daily(&["05:00"]), json!(1)).await;

    assert_eq!(saved.status, StatusCode::OK, "{}", saved.text);
    let status = saved.json();
    assert_eq!(status["version"], 1);
    assert_eq!(status["settings"]["times"], json!(["04:00", "16:00"]));
    assert_eq!(status["zone"], "Europe/Paris");
    assert_eq!(status["next"]["source"], "schedule");
    assert_eq!(status["upcoming"].as_array().unwrap().len(), 5);
    assert_eq!(again.json()["version"], 1, "the same settings change nothing");
    assert_eq!(
        (stale.status, stale.code()),
        (StatusCode::CONFLICT, json!("conflict"))
    );
    assert_eq!(
        (later.status, later.code()),
        (StatusCode::FORBIDDEN, json!("reauth_required"))
    );
    let changed = server.schedule_entries("schedule_changed").await;
    assert_eq!(changed.len(), 1);
    assert_eq!(
        changed[0]["details"],
        json!({
            "enabled": { "before": false, "after": true },
            "times": { "before": [], "after": ["04:00", "16:00"] },
            "timeZone": { "before": null, "after": "Europe/Paris" },
        })
    );
}

#[tokio::test]
async fn settings_and_bodies_the_rules_refuse_are_named() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let mut settings = daily(&["04:00", "04:10", "25:00"]);
    settings["warnings"] = json!([30, 0]);

    let invalid = server.save_schedule(&owner, settings, json!(null)).await;
    let unknown_field = server
        .save_schedule(&owner, json!({ "enabled": true }), json!(null))
        .await;
    let zero = server
        .call(Method::POST, DELAY, Some(&owner), Some(json!({ "minutes": 0 })))
        .await;
    let both = server
        .call(
            Method::PUT,
            ONCE,
            Some(&owner),
            Some(json!({ "inMinutes": 5, "at": "04:00" })),
        )
        .await;

    assert_eq!(invalid.status, StatusCode::BAD_REQUEST);
    assert_eq!(invalid.code(), "invalid_schedule");
    assert_eq!(
        invalid.json()["params"]["issues"],
        json!([
            { "field": "time", "index": 2, "problem": "invalidTime" },
            { "field": "time", "index": 1, "problem": "tooClose" },
            { "field": "warning", "index": 1, "problem": "outOfRange" },
        ])
    );
    for refused in [&unknown_field, &zero, &both] {
        assert_eq!(
            (refused.status, refused.code()),
            (StatusCode::BAD_REQUEST, json!("invalid_request")),
            "{}",
            refused.text
        );
    }
}

#[tokio::test]
async fn the_schedule_topic_is_open_to_the_dashboard_the_schedule_and_the_controls_only() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let readers = [
        server.member(&owner, "grace", "1").await,
        server.member(&owner, "kenji", "10").await,
        server.member(&owner, "cid", "8").await,
    ];
    let console = server.member(&owner, "lin", "2").await;

    for reader in &readers {
        let mut socket = server.socket(reader).await;
        send(&mut socket, json!({ "type": "sub", "topic": "server.schedule" })).await;
        let snapshot = receive(&mut socket).await;
        assert_eq!(snapshot["type"], "snapshot");
        assert_eq!(snapshot["data"]["settings"]["enabled"], false);
        assert_eq!(snapshot["data"]["version"], Value::Null);
    }
    let mut refused = server.socket(&console).await;
    send(&mut refused, json!({ "type": "sub", "topic": "server.schedule" })).await;
    assert_eq!(
        receive(&mut refused).await,
        json!({ "type": "unsubscribed", "topic": "server.schedule", "reason": "forbidden" })
    );
}

#[tokio::test]
async fn a_saved_schedule_reaches_its_followers_as_it_changes() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let mut socket = server.socket(&owner).await;
    send(&mut socket, json!({ "type": "sub", "topic": "server.schedule" })).await;
    assert_eq!(receive(&mut socket).await["type"], "snapshot");

    let saved = server.save_schedule(&owner, daily(&["04:00"]), json!(null)).await;
    let event = receive(&mut socket).await;

    assert_eq!(saved.status, StatusCode::OK);
    assert_eq!(event["type"], "event");
    assert_eq!(event["topic"], "server.schedule");
    assert_eq!(event["data"], saved.json());
}
