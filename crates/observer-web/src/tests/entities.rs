//! The server's entities over HTTP: who reads the list, who deletes from it, what a filter that
//! names nothing gets, the cadence as a setting, and the topic that keeps the server listing.

use axum::http::{Method, StatusCode};
use serde_json::json;

use super::live::{receive, send};
use super::{OWNER_PASSWORD, Server};

const DASHBOARD_VIEW: &str = "1";
const ENTITIES_VIEW: &str = "8000000";
const ENTITIES_DELETE: &str = "10000000";
const SETTINGS_VIEW: &str = "200";
const SETTINGS_MANAGE: &str = "600";
const ENTITIES: &str = "/api/servers/default/entities";

fn deletion() -> serde_json::Value {
    json!({ "run": "1-4210", "targets": [{ "netId": 12, "serial": 3, "model": 7 }] })
}

#[tokio::test]
async fn the_entities_are_read_under_one_permission_and_deleted_under_another() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let nobody = server.member(&owner, "alan", DASHBOARD_VIEW).await;
    let reader = server.member(&owner, "grace", ENTITIES_VIEW).await;
    let cleaner = server.member(&owner, "hopper", ENTITIES_DELETE).await;
    let delete = format!("{ENTITIES}/delete");

    let refused = server.call(Method::GET, ENTITIES, Some(&nobody), None).await;
    let page = server.call(Method::GET, ENTITIES, Some(&reader), None).await;
    let map = server
        .call(
            Method::GET,
            &format!("{ENTITIES}/map?kinds=vehicle&cell=500"),
            Some(&reader),
            None,
        )
        .await;
    let by_reader = server
        .call(Method::POST, &delete, Some(&reader), Some(deletion()))
        .await;
    let by_cleaner = server
        .call(Method::POST, &delete, Some(&cleaner), Some(deletion()))
        .await;
    let read_by_cleaner = server.call(Method::GET, ENTITIES, Some(&cleaner), None).await;

    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.text);
    assert_eq!(page.status, StatusCode::OK, "{}", page.text);
    let page = page.json();
    assert_eq!(page["items"], json!([]));
    assert_eq!(
        (&page["live"]["run"], &page["live"]["at"]),
        (&json!(null), &json!(null))
    );
    assert_eq!(page["ownersShown"], false);
    assert_eq!(
        page["limits"],
        json!({
            "pageSize": 100, "maxDelete": 100, "typedConfirmationFrom": 20, "maxMapPoints": 5000,
            "staleSweeps": 3,
        })
    );
    assert_eq!(map.status, StatusCode::OK, "{}", map.text);
    assert_eq!(
        (&map.json()["points"], &map.json()["cell"]),
        (&json!([]), &json!(null))
    );
    assert_eq!(by_reader.status, StatusCode::FORBIDDEN, "{}", by_reader.text);
    // Nobody follows the list here, so the server lists nothing to delete from.
    assert_eq!(by_cleaner.status, StatusCode::CONFLICT, "{}", by_cleaner.text);
    assert_eq!(by_cleaner.code(), "entities_not_listed");
    assert_eq!(
        read_by_cleaner.status,
        StatusCode::OK,
        "deleting takes seeing with it"
    );
    assert_eq!(server.audited("entities_deleted").await.len(), 0);
}

#[tokio::test]
async fn a_filter_that_names_nothing_is_refused_rather_than_ignored() {
    let server = Server::new().await;
    let owner = server.owner().await;

    for query in [
        "kinds=hovercraft",
        "class=tank",
        "owner=someone",
        "sort=colour",
        "page=-1",
        "bucket=first",
        "colour=red",
    ] {
        let reply = server
            .call(Method::GET, &format!("{ENTITIES}?{query}"), Some(&owner), None)
            .await;

        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{query}: {}", reply.text);
        assert_eq!(reply.code(), "invalid_request", "{query}");
    }
    let odd_cell = server
        .call(
            Method::GET,
            &format!("{ENTITIES}/map?cell=123"),
            Some(&owner),
            None,
        )
        .await;
    let half_a_view = server
        .call(
            Method::GET,
            &format!("{ENTITIES}/map?west=0&east=100"),
            Some(&owner),
            None,
        )
        .await;
    let a_view = server
        .call(
            Method::GET,
            &format!("{ENTITIES}/map?west=-200.5&east=100&south=-1300&north=-900&cell=25"),
            Some(&owner),
            None,
        )
        .await;
    let named = server
        .call(
            Method::GET,
            &format!(
                "{ENTITIES}?kinds=vehicle,ped&class=car&resource=&owner=none&bucket=0&search=%2312&sort=netId&descending=true&page=2"
            ),
            Some(&owner),
            None,
        )
        .await;
    let too_many = server
        .call(
            Method::POST,
            &format!("{ENTITIES}/delete"),
            Some(&owner),
            Some(json!({
                "run": "1-4210",
                "targets": vec![json!({ "netId": 12, "serial": 3, "model": 7 }); 101],
            })),
        )
        .await;

    assert_eq!(odd_cell.code(), "invalid_request");
    assert_eq!(half_a_view.code(), "invalid_request");
    assert_eq!(a_view.status, StatusCode::OK, "{}", a_view.text);
    assert_eq!(
        a_view.json()["kinds"],
        json!({ "vehicles": 0, "peds": 0, "objects": 0 })
    );
    assert_eq!(named.status, StatusCode::OK, "{}", named.text);
    assert_eq!(named.json()["ownersShown"], true);
    assert_eq!(too_many.status, StatusCode::BAD_REQUEST, "{}", too_many.text);
    assert_eq!(
        too_many.json(),
        json!({ "code": "too_many_entities", "params": { "max": 100 } })
    );
}

#[tokio::test]
async fn the_cadence_is_a_setting_with_the_settings_permissions() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let reader = server.member(&owner, "grace", ENTITIES_VIEW).await;
    let viewer = server.member(&owner, "alan", SETTINGS_VIEW).await;
    let manager = server.member(&owner, "sarah", SETTINGS_MANAGE).await;
    let path = format!("{ENTITIES}/settings");
    let every = |seconds: u8| Some(json!({ "everySeconds": seconds }));

    let hidden = server.call(Method::GET, &path, Some(&reader), None).await;
    let read = server.call(Method::GET, &path, Some(&viewer), None).await;
    let by_viewer = server.call(Method::PUT, &path, Some(&viewer), every(30)).await;
    let wrong = server.call(Method::PUT, &path, Some(&manager), every(7)).await;
    let changed = server.call(Method::PUT, &path, Some(&manager), every(30)).await;

    assert_eq!(hidden.status, StatusCode::FORBIDDEN);
    assert_eq!(read.json(), json!({ "everySeconds": 15 }));
    assert_eq!(by_viewer.status, StatusCode::FORBIDDEN);
    assert_eq!(wrong.code(), "invalid_entities_settings");
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.text);
    assert_eq!(changed.json(), json!({ "everySeconds": 30 }));
    let audited = server.audited("entities_settings_changed").await;
    assert_eq!(audited.len(), 1);
    assert_eq!(audited[0]["details"]["from"]["everySeconds"], 15);
}

#[tokio::test]
async fn the_list_is_followed_with_its_permission_and_goes_with_its_feature() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let nobody = server.member(&owner, "alan", DASHBOARD_VIEW).await;
    let subscribe = json!({ "type": "sub", "topic": "server.entities" });
    let mut refused = server.socket(&nobody).await;
    let mut socket = server.socket(&owner).await;

    send(&mut refused, subscribe.clone()).await;
    let forbidden = receive(&mut refused).await;
    send(&mut socket, subscribe.clone()).await;
    let snapshot = receive(&mut socket).await;
    let reauth = server
        .call(
            Method::POST,
            "/api/auth/reauth",
            Some(&owner),
            Some(json!({ "password": OWNER_PASSWORD })),
        )
        .await;
    let switched = server
        .call(
            Method::PUT,
            "/api/features/entities",
            Some(&owner),
            Some(json!({ "enabled": false })),
        )
        .await;
    let withdrawn = receive(&mut socket).await;
    let page = server.call(Method::GET, ENTITIES, Some(&owner), None).await;
    let players = server
        .call(
            Method::GET,
            "/api/servers/default/players/online",
            Some(&owner),
            None,
        )
        .await;

    assert_eq!(
        forbidden,
        json!({ "type": "unsubscribed", "topic": "server.entities", "reason": "forbidden" })
    );
    assert_eq!(
        (&snapshot["type"], &snapshot["topic"]),
        (&json!("snapshot"), &json!("server.entities"))
    );
    assert_eq!(snapshot["data"]["run"], json!(null));
    assert_eq!(snapshot["data"]["listed"], 0);
    assert_eq!(reauth.status, StatusCode::NO_CONTENT, "{}", reauth.text);
    assert_eq!(switched.status, StatusCode::OK, "{}", switched.text);
    let entities = &switched.json()["features"][9];
    assert_eq!(entities["feature"], "entities");
    assert_eq!(
        (&entities["enabled"], &entities["running"]),
        (&json!(false), &json!(false))
    );
    assert_eq!(entities["reach"]["routes"], json!(["/servers/default/entities"]));
    assert_eq!(entities["reach"]["topics"], json!(["server.entities"]));
    assert_eq!(entities["reach"]["serverFolder"], "untouched");
    assert_eq!(
        withdrawn,
        json!({ "type": "unsubscribed", "topic": "server.entities", "reason": "disabled" })
    );
    assert_eq!(
        page.json(),
        json!({ "code": "feature_disabled", "params": { "feature": "entities" } })
    );
    assert_eq!(
        players.status,
        StatusCode::OK,
        "the players go on without the entities"
    );
}
