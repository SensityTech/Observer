//! The live updates websocket over a real HTTP/1.1 connection, held in memory: a ticket, the
//! upgrade through the router and its gate, then what the socket says.

use std::time::Duration;

use axum::http::{Method, StatusCode};
use futures_util::{SinkExt, StreamExt};
use hyper_util::rt::TokioIo;
use hyper_util::service::TowerToHyperService;
use jiff::{SignedDuration, Timestamp};
use observer_domain::audit::{ActorKind, AuditAction, Outcome};
use observer_domain::live::{CLOSE_SESSION_ENDED, PROTOCOL, Topic};
use observer_store::StoreError;
use observer_store::audit::{self, NewAuditEntry};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::{self, Message};

use super::{HOST, ORIGIN, OWNER_PASSWORD, Server, Session};

type Socket = WebSocketStream<DuplexStream>;

/// How long a test waits for the socket to say something.
const PATIENCE: Duration = Duration::from_secs(5);

impl Server {
    async fn ticket(&self, session: &Session) -> String {
        let reply = self
            .call(Method::POST, "/api/auth/ws-ticket", Some(session), None)
            .await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
        reply.json()["ticket"].as_str().unwrap().to_owned()
    }

    /// The upgrade a browser sends with `new WebSocket(url, protocol)`: the page's origin and the
    /// cookies, nothing else of the page's choosing. A refusal answers with its status.
    async fn upgrade(
        &self,
        ticket: &str,
        cookie: &str,
        origin: &str,
        protocol: Option<&str>,
    ) -> Result<Socket, StatusCode> {
        let mut request = format!("ws://{HOST}/api/ws?ticket={ticket}")
            .into_client_request()
            .unwrap();
        let headers = request.headers_mut();
        headers.insert("origin", HeaderValue::from_str(origin).unwrap());
        headers.insert("cookie", HeaderValue::from_str(cookie).unwrap());
        if let Some(protocol) = protocol {
            headers.insert("sec-websocket-protocol", HeaderValue::from_str(protocol).unwrap());
        }

        let (client, server) = tokio::io::duplex(64 * 1024);
        let service = TowerToHyperService::new(self.app.clone());
        tokio::spawn(
            hyper::server::conn::http1::Builder::new()
                .serve_connection(TokioIo::new(server), service)
                .with_upgrades(),
        );
        match tokio_tungstenite::client_async(request, client).await {
            Ok((socket, _)) => Ok(socket),
            Err(tungstenite::Error::Http(response)) => Err(response.status()),
            Err(error) => panic!("the handshake failed: {error}"),
        }
    }

    /// A socket opened the way the panel opens one, past its `hello`.
    pub(super) async fn socket(&self, session: &Session) -> Socket {
        let ticket = self.ticket(session).await;
        let mut socket = self
            .upgrade(&ticket, &session.cookie, ORIGIN, Some(PROTOCOL))
            .await
            .unwrap();
        assert_eq!(receive(&mut socket).await["type"], "hello");
        socket
    }

    /// A user who replaced their temporary password, so that every route is open to them.
    pub(super) async fn member(&self, owner: &Session, name: &str, permissions: &str) -> Session {
        let (session, temporary) = self.user(owner, name, permissions).await;
        let change = json!({ "current": temporary, "next": format!("{name} chose this one") });
        let changed = self
            .call(
                Method::POST,
                "/api/account/password",
                Some(&session),
                Some(change),
            )
            .await;
        assert_eq!(changed.status, StatusCode::NO_CONTENT, "{}", changed.text);
        session
    }

    pub(super) async fn user_id(&self, owner: &Session, name: &str) -> i64 {
        let users = self.call(Method::GET, "/api/users", Some(owner), None).await;
        let users = users.json();
        let user = users
            .as_array()
            .unwrap()
            .iter()
            .find(|user| user["username"] == name)
            .unwrap();
        user["id"].as_i64().unwrap()
    }

    /// Writes `count` audit entries straight into the log, as a burst of activity would.
    async fn burst(&self, count: usize) {
        self.db
            .write(move |tx| {
                for _ in 0..count {
                    audit::insert(
                        tx,
                        &NewAuditEntry {
                            at: "2025-01-06T09:00:00Z".parse::<Timestamp>().unwrap(),
                            actor_kind: ActorKind::Cli,
                            actor_id: None,
                            actor_name: None,
                            ip: None,
                            user_agent: None,
                            action: AuditAction::UserUnlocked,
                            target: Some("grace".to_owned()),
                            outcome: Outcome::Success,
                            details: json!({}),
                            expires_at: "2026-01-06T09:00:00Z".parse::<Timestamp>().unwrap(),
                        },
                    )?;
                }
                Ok::<_, StoreError>(())
            })
            .await
            .unwrap();
    }
}

pub(super) async fn send(socket: &mut Socket, message: Value) {
    socket
        .send(Message::Text(message.to_string().into()))
        .await
        .unwrap();
}

/// The next message Observer sends, past its pings.
pub(super) async fn receive(socket: &mut Socket) -> Value {
    loop {
        let frame = tokio::time::timeout(PATIENCE, socket.next())
            .await
            .expect("a message in time")
            .expect("the socket is open")
            .unwrap();
        match frame {
            Message::Text(text) => return serde_json::from_str(text.as_str()).unwrap(),
            Message::Ping(_) | Message::Pong(_) => {}
            other => panic!("expected a message, got {other:?}"),
        }
    }
}

/// The code the socket closes with, past whatever it sent before.
async fn closed(socket: &mut Socket) -> u16 {
    loop {
        let frame = tokio::time::timeout(PATIENCE, socket.next())
            .await
            .expect("a close in time");
        match frame {
            Some(Ok(Message::Close(Some(close)))) => return u16::from(close.code),
            Some(Ok(Message::Text(_) | Message::Ping(_) | Message::Pong(_))) => {}
            other => panic!("expected a close frame, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn a_ticket_opens_one_socket_for_its_own_session_from_the_panels_origin() {
    let server = Server::new().await;
    let owner = server.owner().await;
    server.clock.advance(SignedDuration::from_secs(2));
    let phone = server.login("ada", OWNER_PASSWORD).await.session();

    let ticket = server.ticket(&owner).await;
    let mut socket = server
        .upgrade(&ticket, &owner.cookie, ORIGIN, Some(PROTOCOL))
        .await
        .unwrap();
    let hello = receive(&mut socket).await;
    let again = server
        .upgrade(&ticket, &owner.cookie, ORIGIN, Some(PROTOCOL))
        .await;

    assert_eq!(hello["type"], "hello");
    assert_eq!(hello["contract"], 1);
    assert!(hello["connectionId"].is_u64());
    assert_eq!(again.err(), Some(StatusCode::UNAUTHORIZED), "a ticket works once");

    // The same account's other session holds another cookie, which does not go with the ticket.
    let ticket = server.ticket(&owner).await;
    let elsewhere = server
        .upgrade(&ticket, &phone.cookie, ORIGIN, Some(PROTOCOL))
        .await;
    assert_eq!(elsewhere.err(), Some(StatusCode::UNAUTHORIZED));

    // A page FXServer serves on its own port is another origin, even with the cookie.
    let ticket = server.ticket(&owner).await;
    for origin in [
        "http://observer.test:30120",
        "https://observer.test:40125",
        "null",
    ] {
        let foreign = server
            .upgrade(&ticket, &owner.cookie, origin, Some(PROTOCOL))
            .await;
        assert_eq!(foreign.err(), Some(StatusCode::FORBIDDEN), "{origin}");
    }
    let bare = server.upgrade(&ticket, &owner.cookie, ORIGIN, None).await;
    assert_eq!(
        bare.err(),
        Some(StatusCode::BAD_REQUEST),
        "the subprotocol is required"
    );
    let forged = server
        .upgrade(&"ab".repeat(32), &owner.cookie, ORIGIN, Some(PROTOCOL))
        .await;
    assert_eq!(forged.err(), Some(StatusCode::UNAUTHORIZED));
}

#[tokio::test]
async fn a_ticket_expires_after_thirty_seconds() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let ticket = server.ticket(&owner).await;

    server.clock.advance(SignedDuration::from_secs(31));
    let late = server
        .upgrade(&ticket, &owner.cookie, ORIGIN, Some(PROTOCOL))
        .await;

    assert_eq!(late.err(), Some(StatusCode::UNAUTHORIZED));
}

#[tokio::test]
async fn every_topic_needs_its_permission_on_every_subscription() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let nobody = server.member(&owner, "grace", "0").await;

    let mut refused = server.socket(&nobody).await;
    let mut allowed = server.socket(&owner).await;
    for topic in Topic::ALL {
        let sub = json!({ "type": "sub", "topic": topic });
        send(&mut refused, sub.clone()).await;
        send(&mut allowed, sub).await;

        assert_eq!(
            receive(&mut refused).await,
            json!({ "type": "unsubscribed", "topic": topic, "reason": "forbidden" })
        );
        let snapshot = receive(&mut allowed).await;
        assert_eq!(snapshot["type"], "snapshot", "{topic:?}");
        assert_eq!(snapshot["topic"], json!(topic));
    }
}

#[tokio::test]
async fn the_audit_topic_tells_where_the_log_stands_then_sends_each_new_entry() {
    let server = Server::new().await;
    let owner = server.owner().await;
    server.audit.publish_new().await.unwrap();
    let mut socket = server.socket(&owner).await;

    send(&mut socket, json!({ "type": "sub", "topic": "status" })).await;
    let status = receive(&mut socket).await;
    send(&mut socket, json!({ "type": "sub", "topic": "audit" })).await;
    let snapshot = receive(&mut socket).await;
    server.clock.advance(SignedDuration::from_secs(2));
    let reauth = json!({ "password": OWNER_PASSWORD });
    server
        .call(Method::POST, "/api/auth/reauth", Some(&owner), Some(reauth))
        .await;
    server.audit.publish_new().await.unwrap();
    let event = receive(&mut socket).await;

    assert_eq!(status["data"]["version"], "0.0.0-test");
    assert!(status["data"]["startedAt"].is_string());
    let latest = snapshot["data"]["latestId"].as_i64().unwrap();
    assert_eq!(event["type"], "event");
    assert_eq!(event["topic"], "audit");
    assert_eq!(event["data"]["action"], "reauthenticated");
    assert_eq!(event["data"]["actorName"], "ada");
    assert!(event["data"]["id"].as_i64().unwrap() > latest);

    // Messages are handled in order: once the pong is back, so is the unsubscription, and new
    // entries no longer come.
    send(&mut socket, json!({ "type": "unsub", "topic": "audit" })).await;
    send(&mut socket, json!({ "type": "ping" })).await;
    assert_eq!(receive(&mut socket).await, json!({ "type": "pong" }));
    server.burst(3).await;
    server.audit.publish_new().await.unwrap();
    send(&mut socket, json!({ "type": "ping" })).await;
    assert_eq!(receive(&mut socket).await, json!({ "type": "pong" }));
}

#[tokio::test]
async fn a_socket_closes_as_soon_as_its_session_ends() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let grace = server.member(&owner, "grace", "101").await;
    let kenji = server.member(&owner, "kenji", "101").await;
    let mut graces = server.socket(&grace).await;
    let mut kenjis = server.socket(&kenji).await;
    let mut owners = server.socket(&owner).await;
    send(&mut graces, json!({ "type": "sub", "topic": "audit" })).await;
    assert_eq!(receive(&mut graces).await["type"], "snapshot");

    let grace_id = server.user_id(&owner, "grace").await;
    let revoke = format!("/api/users/{grace_id}/revoke-sessions");
    server.call(Method::POST, &revoke, Some(&owner), None).await;
    assert_eq!(closed(&mut graces).await, CLOSE_SESSION_ENDED);

    let kenji_id = server.user_id(&owner, "kenji").await;
    let users = server.call(Method::GET, "/api/users", Some(&owner), None).await;
    let version = users.json()[2]["version"].clone();
    let edit = json!({ "permissions": "1", "version": version });
    let edited = server
        .call(
            Method::PATCH,
            &format!("/api/users/{kenji_id}"),
            Some(&owner),
            Some(edit),
        )
        .await;
    assert_eq!(edited.status, StatusCode::OK, "{}", edited.text);
    assert_eq!(closed(&mut kenjis).await, CLOSE_SESSION_ENDED);

    server
        .call(Method::POST, "/api/auth/logout", Some(&owner), None)
        .await;
    assert_eq!(closed(&mut owners).await, CLOSE_SESSION_ENDED);
}

#[tokio::test]
async fn a_subscription_finds_out_that_an_idle_session_expired() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let mut socket = server.socket(&owner).await;

    server.clock.advance(SignedDuration::from_hours(12));
    send(&mut socket, json!({ "type": "sub", "topic": "status" })).await;

    assert_eq!(closed(&mut socket).await, CLOSE_SESSION_ENDED);
}

#[tokio::test]
async fn a_socket_that_falls_behind_starts_over_instead_of_holding_anyone_up() {
    let server = Server::new().await;
    let owner = server.owner().await;
    server.audit.publish_new().await.unwrap();
    let mut slow = server.socket(&owner).await;
    send(&mut slow, json!({ "type": "sub", "topic": "audit" })).await;
    assert_eq!(receive(&mut slow).await["type"], "snapshot");

    server.burst(300).await;
    server.audit.publish_new().await.unwrap();

    assert_eq!(
        receive(&mut slow).await,
        json!({ "type": "resync", "topic": "audit" })
    );
    send(&mut slow, json!({ "type": "sub", "topic": "audit" })).await;
    let fresh = receive(&mut slow).await;
    assert_eq!(fresh["type"], "snapshot");
    assert_eq!(
        fresh["data"]["latestId"],
        json!(server.audit.latest_id().await.unwrap())
    );
}

#[tokio::test]
async fn a_message_observer_cannot_read_gets_an_error_and_a_ping_its_pong() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let mut socket = server.socket(&owner).await;

    socket.send(Message::Text("not json".into())).await.unwrap();
    let garbled = receive(&mut socket).await;
    send(&mut socket, json!({ "type": "sub", "topic": "metrics.live" })).await;
    let unknown = receive(&mut socket).await;
    socket.send(Message::Binary(vec![1, 2, 3].into())).await.unwrap();
    let binary = receive(&mut socket).await;
    send(&mut socket, json!({ "type": "ping" })).await;

    for refused in [garbled, unknown, binary] {
        assert_eq!(refused, json!({ "type": "error", "code": "invalid_request" }));
    }
    assert_eq!(receive(&mut socket).await, json!({ "type": "pong" }));
}

#[tokio::test]
async fn stopping_observer_closes_every_socket_as_going_away() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let mut first = server.socket(&owner).await;
    let mut second = server.socket(&owner).await;

    server.live.shutdown().await;

    assert_eq!(closed(&mut first).await, 1001);
    assert_eq!(closed(&mut second).await, 1001);
}

#[tokio::test]
async fn the_audit_log_names_changed_launch_settings_without_showing_what_only_the_owner_reads() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let auditor = server.member(&owner, "grace", "100").await;
    server.audit.publish_new().await.unwrap();
    let mut socket = server.socket(&auditor).await;
    send(&mut socket, json!({ "type": "sub", "topic": "audit" })).await;
    assert_eq!(receive(&mut socket).await["type"], "snapshot");

    let build = if cfg!(windows) {
        r"C:\Hidden\FXServer\build"
    } else {
        "/srv/hidden/fxserver/build"
    };
    let data = if cfg!(windows) {
        r"C:\Hidden\FXServer\server-data"
    } else {
        "/srv/hidden/fxserver/server-data"
    };
    let settings = json!({
        "buildPath": build,
        "dataPath": data,
        "cfgFile": "configs/hidden.cfg",
        "onesync": "legacy",
        "autoStart": true,
        "txadminEvents": false,
        "extraArguments": [
            { "name": "sv_hostname", "value": "Hidden lab" },
            { "name": "discord_webhook", "value": "https://discord.com/api/webhooks/1/hidden-token" },
        ],
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
    server.audit.publish_new().await.unwrap();

    let read = server
        .call(Method::GET, "/api/servers/default/launch", Some(&auditor), None)
        .await;
    assert_eq!(read.code(), "owner_only");
    let event = receive(&mut socket).await;
    let listed = server
        .call(
            Method::GET,
            "/api/audit?action=launch_settings_changed",
            Some(&auditor),
            None,
        )
        .await;
    let exported = server
        .call(
            Method::GET,
            "/api/audit/export.csv?action=launch_settings_changed",
            Some(&auditor),
            None,
        )
        .await;

    assert_eq!(event["data"]["action"], "launch_settings_changed");
    let details = &listed.json()["entries"][0]["details"];
    assert_eq!(event["data"]["details"], *details);
    assert_eq!(
        *details,
        json!({
            "buildPath": true,
            "dataPath": true,
            "cfgFile": true,
            "onesync": { "before": null, "after": "legacy" },
            "autoStart": { "before": null, "after": true },
            "txadminEvents": { "before": null, "after": false },
            "extraArguments": { "set": ["sv_hostname", "discord_webhook"], "removed": [] },
        })
    );
    for shown in [event.to_string(), listed.text, exported.text] {
        assert!(!shown.contains("Hidden"), "{shown}");
        assert!(!shown.contains("hidden"), "{shown}");
    }
}

#[tokio::test]
async fn a_frame_larger_than_any_message_drops_the_socket_before_its_payload_comes() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let ticket = server.ticket(&owner).await;
    let (mut client, socket) = tokio::io::duplex(64 * 1024);
    let service = TowerToHyperService::new(server.app.clone());
    tokio::spawn(
        hyper::server::conn::http1::Builder::new()
            .serve_connection(TokioIo::new(socket), service)
            .with_upgrades(),
    );
    let handshake = format!(
        "GET /api/ws?ticket={ticket} HTTP/1.1\r\nHost: {HOST}\r\nOrigin: {ORIGIN}\r\nCookie: {}\r\n\
         Connection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Protocol: {PROTOCOL}\r\n\r\n",
        owner.cookie
    );
    client.write_all(handshake.as_bytes()).await.unwrap();
    let mut response = vec![0; 1024];
    let read = client.read(&mut response).await.unwrap();
    assert!(response[..read].starts_with(b"HTTP/1.1 101"));

    // A masked text frame announcing 8 MiB, of which only the header and a few bytes arrive.
    let mut frame = vec![0x81, 0xff];
    frame.extend_from_slice(&(8u64 << 20).to_be_bytes());
    frame.extend_from_slice(&[1, 2, 3, 4]);
    frame.extend_from_slice(&[b'a'; 512]);
    client.write_all(&frame).await.unwrap();

    let mut rest = Vec::new();
    let dropped = tokio::time::timeout(PATIENCE, client.read_to_end(&mut rest)).await;
    assert!(
        matches!(dropped, Ok(Ok(_))),
        "Observer kept waiting for the rest of the frame"
    );
}

#[tokio::test]
async fn a_session_holds_a_few_sockets_and_unspent_tickets_at_most() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let mut sockets = Vec::new();
    for _ in 0..8 {
        sockets.push(server.socket(&owner).await);
    }

    let ticket = server.ticket(&owner).await;
    let one_more = server
        .upgrade(&ticket, &owner.cookie, ORIGIN, Some(PROTOCOL))
        .await;
    assert_eq!(one_more.err(), Some(StatusCode::TOO_MANY_REQUESTS));

    // A closed socket frees its place once Observer sees it go.
    let mut closed = sockets.pop().unwrap();
    closed.close(None).await.unwrap();
    let mut reopened = None;
    for _ in 0..50 {
        let ticket = server.ticket(&owner).await;
        if let Ok(socket) = server
            .upgrade(&ticket, &owner.cookie, ORIGIN, Some(PROTOCOL))
            .await
        {
            reopened = Some(socket);
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(reopened.is_some(), "the closed socket's place was freed");

    let mut tickets = Vec::new();
    for _ in 0..8 {
        tickets.push(server.ticket(&owner).await);
    }
    let refused = server
        .call(Method::POST, "/api/auth/ws-ticket", Some(&owner), None)
        .await;
    assert_eq!(refused.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(refused.code(), "too_many_sockets");
    // Another session of the same account has its own.
    server.clock.advance(SignedDuration::from_secs(2));
    let phone = server.login("ada", OWNER_PASSWORD).await.session();
    server.socket(&phone).await;
}
