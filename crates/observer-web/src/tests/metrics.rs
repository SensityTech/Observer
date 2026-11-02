//! The statistics: the series route and its bounds, and the live topic.

use axum::http::{Method, StatusCode};
use jiff::SignedDuration;
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
async fn thread_shares_and_growth_take_the_dashboard_permission() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let console = server.member(&owner, "grace", "2").await;
    let viewer = server.member(&owner, "kenji", "1").await;

    for path in ["/api/metrics/threads", "/api/metrics/growth"] {
        let refused = server.call(Method::GET, path, Some(&console), None).await;
        let signed_out = server.call(Method::GET, path, None, None).await;
        assert_eq!(refused.status, StatusCode::FORBIDDEN, "{path}");
        assert_eq!(signed_out.status, StatusCode::UNAUTHORIZED, "{path}");
    }
    let threads = server
        .call(Method::GET, "/api/metrics/threads", Some(&viewer), None)
        .await;
    let growth = server
        .call(Method::GET, "/api/metrics/growth", Some(&viewer), None)
        .await;

    assert_eq!(threads.status, StatusCode::OK, "{}", threads.text);
    let end = (NOW / 5 + 1) * 5;
    assert_eq!(
        threads.json(),
        json!({ "from": end - 3600, "to": end, "measured": 0, "process": null, "threads": [] })
    );
    assert_eq!(growth.status, StatusCode::OK, "{}", growth.text);
    assert_eq!(growth.json(), Value::Null, "no run under way");
}

#[tokio::test]
async fn the_series_say_how_many_samples_each_bucket_holds() {
    let server = Server::new().await;
    let owner = server.owner().await;

    let reply = server
        .call(
            Method::GET,
            "/api/metrics/series?range=6h&series=players",
            Some(&owner),
            None,
        )
        .await;

    let body = reply.json();
    for coverage in ["observed", "online"] {
        let buckets = body[coverage].as_array().unwrap();
        assert_eq!(buckets.len(), 720, "{coverage}");
        assert!(buckets.iter().all(|count| count == 0), "{coverage}");
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
    // Time runs ten times faster than it does, so that a slot ends every half second.
    let clock = server.metrics_clock.clone();
    let ticker = tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            clock.advance(SignedDuration::from_millis(500));
        }
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
    ticker.abort();

    assert_eq!(
        empty,
        json!({ "type": "snapshot", "topic": "metrics.live", "data": { "latest": null } })
    );
    assert_eq!(
        (&event["type"], &event["topic"]),
        (&json!("event"), &json!("metrics.live"))
    );
    let sample = &event["data"];
    let ts = sample["ts"].as_i64().unwrap();
    assert!(
        ts >= NOW / 5 * 5 - 5 && ts % 5 == 0,
        "a slot that ended after the start: {ts}"
    );
    assert_eq!(sample["state"], "stopped");
    assert_eq!(sample["values"]["dashboardClients"], 1.0, "this socket");
    assert!(sample["values"]["observerWorkingSet"].as_f64().unwrap() > 0.0);
    assert!(sample["values"].get("players").is_none(), "no server, no players");
    assert_eq!(sample["children"], json!([]));
    assert!(sample["cores"].as_u64().unwrap() >= 1);
    assert_eq!(snapshot["data"]["latest"]["ts"], sample["ts"]);
}

/// Masks: the dashboard, the drops' statistics with the dashboard, and the file changes too.
const DASHBOARD: &str = "1";
const DROPS: &str = "10001";
const DROPS_AND_CHANGES: &str = "30001";

#[tokio::test]
async fn the_timeline_reads_any_window_with_its_percentiles_and_usual_range() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let console = server.member(&owner, "grace", "2").await;
    let viewer = server.member(&owner, "kenji", DASHBOARD).await;
    let path = format!(
        "/api/metrics/timeline?from={}&to={NOW}&series=players,mainTick,mainThreadCpu",
        NOW - 86_400
    );

    let refused = server.call(Method::GET, &path, Some(&console), None).await;
    let answered = server.call(Method::GET, &path, Some(&viewer), None).await;

    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    assert_eq!(answered.status, StatusCode::OK, "{}", answered.text);
    let body = answered.json();
    // A day that does not start on a multiple of two minutes takes one bucket more.
    assert_eq!((&body["step"], &body["buckets"]), (&json!(120), &json!(721)));
    assert_eq!(body["series"][2]["series"], "mainThreadCpu");
    assert_eq!(body["frames"]["p99"].as_array().unwrap().len(), 721);
    assert_eq!(body["frames"]["since"], Value::Null, "no histogram yet");
    assert_eq!(body["usual"]["low"].as_array().unwrap().len(), 721);
    let only_cpu = server
        .call(
            Method::GET,
            &format!(
                "/api/metrics/timeline?from={}&to={NOW}&series=fxsCpu&points=48",
                NOW - 86_400
            ),
            Some(&viewer),
            None,
        )
        .await
        .json();
    assert_eq!(
        (&only_cpu["step"], &only_cpu["frames"], &only_cpu["usual"]),
        (&json!(1800), &Value::Null, &Value::Null)
    );
}

#[tokio::test]
async fn the_timeline_refuses_a_window_it_cannot_read() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let twenty_five: Vec<&str> = Series::ALL[..25].iter().map(|series| series.name()).collect();
    let twenty_five = twenty_five.join(",");
    let day = NOW - 86_400;

    for (query, params) in [
        (format!("to={NOW}&series=players"), json!({ "from": null })),
        (
            format!("from=yesterday&to={NOW}&series=players"),
            json!({ "from": "yesterday" }),
        ),
        (
            format!("from={NOW}&to={day}&series=players"),
            json!({ "from": NOW, "to": day }),
        ),
        (
            format!("from={}&to={NOW}&series=players", NOW - 30),
            json!({ "from": NOW - 30, "to": NOW }),
        ),
        (
            format!("from={day}&to={NOW}&series=players&points=many"),
            json!({ "points": "many" }),
        ),
        (
            format!("from={day}&to={NOW}&series={twenty_five}"),
            json!({ "series": twenty_five }),
        ),
    ] {
        let reply = server
            .call(
                Method::GET,
                &format!("/api/metrics/timeline?{query}"),
                Some(&owner),
                None,
            )
            .await;

        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{query}");
        assert_eq!(
            reply.json(),
            json!({ "code": "invalid_request", "params": params }),
            "{query}"
        );
    }
}

#[tokio::test]
async fn events_leave_out_what_the_caller_may_not_see_and_say_so() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let console = server.member(&owner, "grace", "2").await;
    let viewer = server.member(&owner, "kenji", DASHBOARD).await;
    let everything = server.member(&owner, "noor", DROPS_AND_CHANGES).await;
    let path = format!("/api/metrics/events?from={}&to={NOW}", NOW - 86_400);

    let refused = server.call(Method::GET, &path, Some(&console), None).await;
    let runs_only = server.call(Method::GET, &path, Some(&viewer), None).await;
    let all = server.call(Method::GET, &path, Some(&everything), None).await;
    let reversed = server
        .call(
            Method::GET,
            &format!("/api/metrics/events?from={NOW}&to={}", NOW - 60),
            Some(&viewer),
            None,
        )
        .await;

    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    assert_eq!(runs_only.status, StatusCode::OK, "{}", runs_only.text);
    assert_eq!(
        runs_only.json(),
        json!({ "events": [], "links": [], "omitted": ["changes", "drops"] })
    );
    assert_eq!(all.status, StatusCode::OK, "{}", all.text);
    assert_eq!(all.json()["omitted"], json!([]));
    assert_eq!(reversed.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn saying_a_link_is_not_related_takes_both_views_and_is_audited() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let viewer = server.member(&owner, "kenji", DASHBOARD).await;
    let drops = server.member(&owner, "grace", DROPS).await;
    let both = server.member(&owner, "noor", DROPS_AND_CHANGES).await;
    let body = json!({ "link": "start:4>wave:9:120", "dismissed": true });

    let without_drops = server
        .call(
            Method::PUT,
            "/api/metrics/links",
            Some(&viewer),
            Some(body.clone()),
        )
        .await;
    let without_changes = server
        .call(
            Method::PUT,
            "/api/metrics/links",
            Some(&drops),
            Some(body.clone()),
        )
        .await;
    let dismissed = server
        .call(Method::PUT, "/api/metrics/links", Some(&both), Some(body))
        .await;
    let malformed = server
        .call(
            Method::PUT,
            "/api/metrics/links",
            Some(&both),
            Some(json!({ "link": "wave", "dismissed": true })),
        )
        .await;

    assert_eq!(without_drops.status, StatusCode::FORBIDDEN);
    assert_eq!(without_changes.status, StatusCode::FORBIDDEN);
    assert_eq!(without_changes.json()["params"]["permission"], "changesView");
    assert_eq!(dismissed.status, StatusCode::NO_CONTENT, "{}", dismissed.text);
    assert_eq!(malformed.status, StatusCode::BAD_REQUEST);
    let audit = server
        .call(
            Method::GET,
            "/api/audit?action=statistics_link_dismissed",
            Some(&owner),
            None,
        )
        .await
        .json();
    assert_eq!(audit["entries"][0]["target"], "start:4>wave:9:120");
    assert_eq!(audit["entries"][0]["actorName"], "noor");
}

#[tokio::test]
async fn a_feature_turned_off_is_left_out_of_the_events_and_refuses_what_needs_it() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let path = format!("/api/metrics/events?from={}&to={NOW}", NOW - 86_400);
    let off = Some(json!({ "enabled": false }));

    let changes_off = server
        .call(Method::PUT, "/api/features/changes", Some(&owner), off.clone())
        .await;
    let without_changes = server.call(Method::GET, &path, Some(&owner), None).await;
    let effects_without_changes = server
        .call(
            Method::GET,
            "/api/servers/default/drops/effects",
            Some(&owner),
            None,
        )
        .await;
    let drops_off = server
        .call(Method::PUT, "/api/features/drops", Some(&owner), off)
        .await;
    let without_either = server.call(Method::GET, &path, Some(&owner), None).await;
    let effects = server
        .call(
            Method::GET,
            "/api/servers/default/drops/effects",
            Some(&owner),
            None,
        )
        .await;
    let dismissed = server
        .call(
            Method::PUT,
            "/api/metrics/links",
            Some(&owner),
            Some(json!({ "link": "start:4>wave:9:120", "dismissed": true })),
        )
        .await;

    assert_eq!(changes_off.status, StatusCode::OK, "{}", changes_off.text);
    assert_eq!(without_changes.status, StatusCode::OK, "{}", without_changes.text);
    assert_eq!(without_changes.json()["omitted"], json!(["changes"]));
    assert_eq!(
        effects_without_changes.status,
        StatusCode::OK,
        "{}",
        effects_without_changes.text
    );
    assert_eq!(drops_off.status, StatusCode::OK, "{}", drops_off.text);
    assert_eq!(without_either.json()["omitted"], json!(["changes", "drops"]));
    let refused = json!({ "code": "feature_disabled", "params": { "feature": "drops" } });
    assert_eq!(effects.status, StatusCode::NOT_FOUND);
    assert_eq!(effects.json(), refused);
    assert_eq!(dismissed.status, StatusCode::NOT_FOUND);
    assert_eq!(dismissed.json(), refused);
}

#[tokio::test]
async fn the_insights_take_the_dashboard_or_the_drops_permission() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let viewer = server.member(&owner, "kenji", DASHBOARD).await;
    let drops = server.member(&owner, "grace", DROPS).await;
    let both = server.member(&owner, "noor", DROPS_AND_CHANGES).await;

    for path in [
        "/api/metrics/frames?range=7d",
        "/api/metrics/hours",
        "/api/metrics/runs?range=30d",
    ] {
        let answered = server.call(Method::GET, path, Some(&viewer), None).await;
        assert_eq!(answered.status, StatusCode::OK, "{path}: {}", answered.text);
    }
    for path in [
        "/api/servers/default/drops/sessions?range=7d",
        "/api/servers/default/drops/effects",
    ] {
        let refused = server.call(Method::GET, path, Some(&viewer), None).await;
        let answered = server.call(Method::GET, path, Some(&drops), None).await;
        assert_eq!(refused.status, StatusCode::FORBIDDEN, "{path}");
        assert_eq!(answered.status, StatusCode::OK, "{path}: {}", answered.text);
    }
    let hours = server
        .call(Method::GET, "/api/metrics/hours", Some(&viewer), None)
        .await
        .json();
    assert_eq!(hours["players"].as_array().unwrap().len(), 168);
    assert_eq!(hours["quietest"], Value::Null, "nobody played yet");
    let frames = server
        .call(Method::GET, "/api/metrics/frames?range=24h", Some(&viewer), None)
        .await
        .json();
    assert_eq!(
        (&frames["range"], &frames["now"]["ticks"]),
        (&json!("24h"), &json!(0))
    );
    let bad = server
        .call(Method::GET, "/api/metrics/frames?range=2d", Some(&viewer), None)
        .await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);
    let builds_only = server
        .call(
            Method::GET,
            "/api/servers/default/drops/effects",
            Some(&drops),
            None,
        )
        .await
        .json();
    let with_changes = server
        .call(
            Method::GET,
            "/api/servers/default/drops/effects",
            Some(&both),
            None,
        )
        .await
        .json();
    assert_eq!(
        (&builds_only["changes"], &with_changes["changes"]),
        (&json!(false), &json!(true))
    );
}
