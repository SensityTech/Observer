//! The statistics: the series route and its bounds, and the live topic.

use axum::http::{Method, StatusCode};
use observer_app::{DatabaseFiles, Sources};
use observer_domain::metrics::Series;
use serde_json::{Value, json};

use super::Server;
use super::live::{receive, send};

/// 2025-01-06T08:59:59.999Z, the metrics clock of the test server.
const NOW: i64 = 1_736_153_999;

#[tokio::test]
async fn the_series_take_the_dashboard_permission() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let console = server.member(&owner, "grace", "2").await;
    let viewer = server.member(&owner, "kenji", "1").await;
    let path = "/api/metrics/series?range=1h&series=players,mainTick";

    let refused = server.call(Method::GET, path, Some(&console), None).await;
    let answered = server.call(Method::GET, path, Some(&viewer), None).await;

    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    assert_eq!(answered.status, StatusCode::OK, "{}", answered.text);
    let body = answered.json();
    let end = (NOW / 5 + 1) * 5;
    assert_eq!(
        (
            &body["range"],
            &body["step"],
            &body["from"],
            &body["buckets"],
            &body["last"]
        ),
        (
            &json!("1h"),
            &json!(5),
            &json!(end - 3600),
            &json!(720),
            &Value::Null
        )
    );
    assert_eq!(body["series"][0]["series"], "players");
    assert_eq!(body["series"][1]["series"], "mainTick");
    let values = body["series"][0]["values"].as_array().unwrap();
    assert_eq!(values.len(), 720);
    assert!(values.iter().all(Value::is_null), "nothing was measured yet");
    assert!(
        body["series"][0].get("min").is_none(),
        "one sample per bucket in an hour"
    );
    assert_eq!(body["markers"], json!([]));
}

#[tokio::test]
async fn the_series_refuse_anything_but_a_fixed_range_and_one_to_sixteen_known_series() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let seventeen: Vec<&str> = Series::ALL[..17].iter().map(|series| series.name()).collect();
    let seventeen = seventeen.join(",");

    for (query, name, given) in [
        ("series=players", "range", Value::Null),
        ("range=2h&series=players", "range", json!("2h")),
        ("range=1H&series=players", "range", json!("1H")),
        ("range=1h", "series", Value::Null),
        ("range=1h&series=", "series", json!("")),
        (
            "range=1h&series=players,ping_avg_ms",
            "series",
            json!("players,ping_avg_ms"),
        ),
        (
            "range=1h&series=players,players",
            "series",
            json!("players,players"),
        ),
        (
            &format!("range=7d&series={seventeen}"),
            "series",
            json!(seventeen),
        ),
    ] {
        let reply = server
            .call(
                Method::GET,
                &format!("/api/metrics/series?{query}"),
                Some(&owner),
                None,
            )
            .await;

        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{query}");
        assert_eq!(
            reply.json(),
            json!({ "code": "invalid_request", "params": { name: given } }),
            "{query}"
        );
    }
}

#[tokio::test]
async fn the_live_topic_gives_the_newest_sample_then_each_one_taken() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let mut socket = server.socket(&owner).await;
    send(&mut socket, json!({ "type": "sub", "topic": "metrics.live" })).await;
    let empty = receive(&mut socket).await;

    let sampling = server.metrics.start(Sources {
        supervision: server.supervision.clone(),
        sockets: server.live.sockets(),
        data_dir: server.dir.path().to_path_buf(),
        databases: DatabaseFiles {
            state: server.dir.path().join("observer.sqlite3"),
            metrics: server.dir.path().join("metrics.sqlite3"),
        },
    });
    let event = receive(&mut socket).await;
    send(&mut socket, json!({ "type": "sub", "topic": "metrics.live" })).await;
    let snapshot = loop {
        let message = receive(&mut socket).await;
        if message["type"] == "snapshot" {
            break message;
        }
    };
    sampling.close().await;

    assert_eq!(
        empty,
        json!({ "type": "snapshot", "topic": "metrics.live", "data": { "latest": null } })
    );
    assert_eq!(
        (&event["type"], &event["topic"]),
        (&json!("event"), &json!("metrics.live"))
    );
    let sample = &event["data"];
    // The slot that ended before the wake, a tenth of a second after 08:59:59.999.
    assert_eq!(sample["ts"], NOW / 5 * 5 - 5);
    assert_eq!(sample["state"], "stopped");
    assert_eq!(sample["values"]["dashboardClients"], 1.0, "this socket");
    assert!(sample["values"]["observerWorkingSet"].as_f64().unwrap() > 0.0);
    assert!(sample["values"].get("players").is_none(), "no server, no players");
    assert_eq!(sample["children"], json!([]));
    assert!(sample["cores"].as_u64().unwrap() >= 1);
    assert_eq!(snapshot["data"]["latest"]["ts"], sample["ts"]);
}
