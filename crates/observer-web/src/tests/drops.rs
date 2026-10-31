//! Players' drops over HTTP: statistics for `DROPS_VIEW`, names and reasons only for
//! `PLAYERS_VIEW` even right after such a caller filled the cache, identifiers only for
//! `PLAYER_IDENTIFIERS_VIEW`, exports audited, and the retention the owner's.

use axum::http::{Method, StatusCode};
use jiff::SignedDuration;
use observer_domain::drops::{DropCategory, DropKind};
use observer_domain::players::IdentifierKind;
use observer_store::MetricsDatabase;
use observer_store::metrics::drops::{self as stored_drops, Grouping};
use observer_store::metrics::runs::{self, RunKey, StartCause};
use observer_store::metrics::sessions::{self, Closure, NewSession};
use observer_store::players::{self, Seen};
use serde_json::json;

use super::{Server, Session};

const DROPS_VIEW: &str = "10000";
const PLAYERS_VIEW: &str = "4000";
const LICENSE: &str = "license:0123456789abcdef0123456789abcdef01234567";
const ADA: [u8; 16] = [7; 16];

impl Server {
    /// Ada, seen by Observer, kicked once by `afk` and crashed three times in the last hours.
    async fn dropped_player(&self) {
        let at = self.clock.now();
        self.db
            .write(move |tx| {
                players::record_seen(
                    tx,
                    &Seen {
                        license: LICENSE,
                        name: Some("Ada"),
                        others: &[(IdentifierKind::Discord, "123456789012345678".to_owned())],
                        at,
                        pseudonym: ADA,
                    },
                )
            })
            .await
            .unwrap();
        let metrics = MetricsDatabase::open(self.dir.path().join("metrics.sqlite3"))
            .await
            .unwrap();
        let now_ms = at.as_millisecond();
        metrics
            .write(move |tx| -> Result<(), observer_store::StoreError> {
                let run = RunKey {
                    observer_started: now_ms - 86_400_000,
                    generation: 1,
                };
                runs::start(tx, run, now_ms - 86_400_000, StartCause::Requested)?;
                for (id, category) in [
                    (1, DropCategory::Kick),
                    (2, DropCategory::Crash),
                    (3, DropCategory::Crash),
                    (4, DropCategory::Crash),
                ] {
                    let seen_ms = now_ms - (5 - id) * 3_600_000;
                    sessions::insert(
                        tx,
                        &NewSession {
                            id,
                            run,
                            net_id: u32::try_from(id).unwrap(),
                            joined_ms: Some(seen_ms),
                            seen_ms,
                        },
                    )?;
                    sessions::close(
                        tx,
                        &Closure {
                            id,
                            ended_ms: seen_ms + 1_800_000,
                            length_ms: Some(1_800_000),
                            category,
                            kind: (category == DropCategory::Kick).then_some(DropKind::Resource),
                            native: None,
                            resource: (category == DropCategory::Kick).then(|| "afk".to_owned()),
                            crash_text: None,
                            reason_text: (category == DropCategory::Kick)
                                .then(|| "AFK for 31 minutes".to_owned()),
                            loading: None,
                            synthesized: false,
                        },
                    )?;
                    tx.execute("UPDATE player_session SET player = ?1 WHERE id = ?2", (ADA, id))?;
                }
                for session in stored_drops::unclassified(tx, 10)? {
                    let grouping = if session.category == DropCategory::Kick {
                        Grouping::Reason("AFK for {n} minutes")
                    } else {
                        Grouping::None
                    };
                    stored_drops::classify(tx, &session, grouping, 1)?;
                }
                Ok(())
            })
            .await
            .unwrap();
        metrics.close().await.unwrap();
    }
}

#[tokio::test]
async fn statistics_open_to_drops_view_and_the_log_to_players_view_only() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let statistics = server.member(&owner, "grace", DROPS_VIEW).await;
    let moderator = server.member(&owner, "hopper", PLAYERS_VIEW).await;

    let summary = server
        .call(
            Method::GET,
            "/api/servers/default/drops/summary?range=7d",
            Some(&statistics),
            None,
        )
        .await;
    let refused = server
        .call(
            Method::GET,
            "/api/servers/default/drops/log",
            Some(&statistics),
            None,
        )
        .await;
    // Whoever reads the log may read the statistics too.
    let implied = server
        .call(
            Method::GET,
            "/api/servers/default/drops/summary",
            Some(&moderator),
            None,
        )
        .await;
    let log = server
        .call(
            Method::GET,
            "/api/servers/default/drops/log",
            Some(&moderator),
            None,
        )
        .await;

    assert_eq!(summary.status, StatusCode::OK, "{}", summary.text);
    assert_eq!(summary.json()["tiles"].as_array().unwrap().len(), 4);
    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    assert_eq!(implied.status, StatusCode::OK);
    assert_eq!(log.status, StatusCode::OK, "{}", log.text);
}

#[tokio::test]
async fn a_statistics_viewer_gets_no_names_and_no_reasons_from_an_answer_cached_for_a_log_reader() {
    let server = Server::new().await;
    server.dropped_player().await;
    let owner = server.owner().await;
    let statistics = server.member(&owner, "grace", DROPS_VIEW).await;
    let moderator = server.member(&owner, "hopper", PLAYERS_VIEW).await;
    let read = |session: &Session, path: &'static str| {
        let session = session.clone();
        let server = &server;
        async move { server.call(Method::GET, path, Some(&session), None).await.json() }
    };

    let named = read(&moderator, "/api/servers/default/drops/summary?range=7d").await;
    let anonymous = read(&statistics, "/api/servers/default/drops/summary?range=7d").await;
    let reasons = read(&moderator, "/api/servers/default/drops/resources?range=7d").await;
    let no_reasons = read(&statistics, "/api/servers/default/drops/resources?range=7d").await;

    assert_eq!(named["repeatCrashers"][0]["player"]["name"], "Ada");
    assert_eq!(anonymous["repeatCrashers"][0]["crashes"], 3);
    assert!(anonymous["repeatCrashers"][0]["player"].is_null());
    assert_eq!(
        reasons["resources"][0]["reasons"][0]["template"],
        "AFK for {n} minutes"
    );
    assert_eq!(no_reasons["resources"][0]["resource"], "afk");
    assert!(no_reasons["resources"][0]["reasons"].is_null());
}

#[tokio::test]
async fn the_log_finds_a_player_by_licence_and_refuses_other_identifiers_and_addresses_without_the_bit() {
    let server = Server::new().await;
    server.dropped_player().await;
    let owner = server.owner().await;
    let moderator = server.member(&owner, "hopper", PLAYERS_VIEW).await;
    let log = |filter: &str| format!("/api/servers/default/drops/log?{filter}");

    let found = server
        .call(
            Method::GET,
            &log(&format!("player={LICENSE}")),
            Some(&moderator),
            None,
        )
        .await;
    let identifier = server
        .call(
            Method::GET,
            &log("player=discord:123456789012345678"),
            Some(&moderator),
            None,
        )
        .await;
    let address = server
        .call(Method::GET, &log("player=ip:203.0.113.9"), Some(&owner), None)
        .await;
    let by_owner = server
        .call(
            Method::GET,
            &log("player=discord:123456789012345678"),
            Some(&owner),
            None,
        )
        .await;

    let rows = found.json()["rows"].as_array().unwrap().clone();
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[0]["player"]["license"], LICENSE);
    assert_eq!(identifier.status, StatusCode::FORBIDDEN);
    assert_eq!(address.code(), "address_search");
    assert_eq!(by_owner.json()["rows"].as_array().unwrap().len(), 4);
}

#[tokio::test]
async fn an_export_is_audited_with_the_kinds_of_its_filters_and_never_their_values() {
    let server = Server::new().await;
    server.dropped_player().await;
    let owner = server.owner().await;

    let csv = server
        .call(
            Method::GET,
            &format!("/api/servers/default/drops/log.csv?category=kick&player={LICENSE}"),
            Some(&owner),
            None,
        )
        .await;
    let rates = server
        .call(
            Method::GET,
            "/api/servers/default/drops/rates.csv?from=2025-01-01&to=2025-01-07",
            Some(&owner),
            None,
        )
        .await;

    assert_eq!(csv.status, StatusCode::OK, "{}", csv.text);
    assert_eq!(csv.text.lines().count(), 2);
    assert!(csv.text.contains("AFK for 31 minutes"));
    assert_eq!(rates.status, StatusCode::OK, "{}", rates.text);
    let audit = server
        .call(
            Method::GET,
            "/api/audit?action=player_drops_exported",
            Some(&owner),
            None,
        )
        .await
        .json();
    let details = &audit["entries"][0]["details"];
    assert_eq!(details["filters"], json!(["category", "player"]));
    assert_eq!(details["rows"], 1);
    assert!(!audit.to_string().contains(LICENSE));
}

#[tokio::test]
async fn the_drop_retention_is_the_owners_with_the_password_again_and_within_its_range() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let path = "/api/servers/default/drops/settings";
    server.clock.advance(SignedDuration::from_mins(11));

    let unconfirmed = server
        .call(
            Method::PUT,
            path,
            Some(&owner),
            Some(json!({ "sessionDays": 30 })),
        )
        .await;
    server.reauthenticate(&owner).await;
    let too_short = server
        .call(Method::PUT, path, Some(&owner), Some(json!({ "sessionDays": 7 })))
        .await;
    let changed = server
        .call(
            Method::PUT,
            path,
            Some(&owner),
            Some(json!({ "sessionDays": 30 })),
        )
        .await;

    assert_eq!(unconfirmed.code(), "reauth_required");
    assert_eq!(too_short.code(), "invalid_drops_settings");
    assert_eq!(too_short.json()["params"]["min"], 14);
    assert_eq!(changed.json()["sessionDays"], 30);
}

#[tokio::test]
async fn an_example_text_only_one_player_crashed_with_stays_with_log_readers() {
    let server = Server::new().await;
    let now_ms = server.clock.now().as_millisecond();
    let metrics = MetricsDatabase::open(server.dir.path().join("metrics.sqlite3"))
        .await
        .unwrap();
    metrics
        .write(move |tx| -> Result<(), observer_store::StoreError> {
            let run = RunKey {
                observer_started: now_ms - 86_400_000,
                generation: 1,
            };
            runs::start(tx, run, now_ms - 86_400_000, StartCause::Requested)?;
            sessions::insert(
                tx,
                &NewSession {
                    id: 1,
                    run,
                    net_id: 1,
                    joined_ms: Some(now_ms - 3_600_000),
                    seen_ms: now_ms - 3_600_000,
                },
            )?;
            sessions::close(
                tx,
                &Closure {
                    id: 1,
                    ended_ms: now_ms - 1_800_000,
                    length_ms: Some(1_800_000),
                    category: DropCategory::Crash,
                    kind: None,
                    native: Some(2),
                    resource: None,
                    crash_text: Some("ERR_GFX_D3D_INIT".to_owned()),
                    reason_text: None,
                    loading: None,
                    synthesized: false,
                },
            )?;
            tx.execute("UPDATE player_session SET player = ?1", [ADA])?;
            let session = stored_drops::unclassified(tx, 1)?.remove(0);
            stored_drops::classify(
                tx,
                &session,
                Grouping::Signature(stored_drops::NewSignature {
                    signature: "error:ERR_GFX_D3D_INIT",
                    form: "error",
                    module: None,
                    game_build: None,
                    label: None,
                    resource: None,
                    normaliser: 1,
                }),
                1,
            )
        })
        .await
        .unwrap();
    metrics.close().await.unwrap();
    let owner = server.owner().await;
    let statistics = server.member(&owner, "grace", DROPS_VIEW).await;
    let path = "/api/servers/default/drops/signatures?range=7d";

    let for_statistics = server
        .call(Method::GET, path, Some(&statistics), None)
        .await
        .json();
    let for_owner = server.call(Method::GET, path, Some(&owner), None).await.json();

    assert_eq!(
        for_statistics["signatures"][0]["signature"],
        "error:ERR_GFX_D3D_INIT"
    );
    assert_eq!(for_statistics["signatures"][0]["examples"], json!([]));
    assert_eq!(
        for_owner["signatures"][0]["examples"],
        json!(["ERR_GFX_D3D_INIT"])
    );
}
