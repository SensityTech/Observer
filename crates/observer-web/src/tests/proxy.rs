//! Observer behind a reverse proxy that terminates TLS, as Traefik or nginx forward the browser's
//! requests to it: what it believes of their headers, and from whom.

use std::net::SocketAddr;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{HeaderValue, Method, Request, StatusCode, header};
use observer_domain::audit::ActorKind;
use serde_json::{Value, json};

use super::{OWNER_PASSWORD, Reply, Server, Session};

/// The panel's public name, which the proxy passes on as the browser sent it.
const HOST: &str = "observer.test";
/// The browser's page, served over HTTPS by the proxy.
const ORIGIN: &str = "https://observer.test";
/// The browser's address, which only the proxy saw.
const BROWSER: &str = "198.51.100.20";

/// A request as the proxy forwards it from the browser's page, with the address the proxy appends
/// to `X-Forwarded-For` and the scheme it took the request over.
fn forwarded(
    method: Method,
    path: &str,
    session: Option<&Session>,
    body: Option<Value>,
    from: &str,
) -> Request<Body> {
    let mut request = Server::request(method, path, session, body);
    let headers = request.headers_mut();
    headers.insert(header::HOST, HeaderValue::from_static(HOST));
    headers.insert(header::ORIGIN, HeaderValue::from_static(ORIGIN));
    headers.insert("x-forwarded-for", HeaderValue::from_str(from).unwrap());
    headers.insert("x-forwarded-proto", HeaderValue::from_static("https"));
    request
}

/// The same request sent straight to Observer by a host that is not a trusted proxy, its
/// forwarded headers written by hand.
fn direct(mut request: Request<Body>) -> Request<Body> {
    let stranger = SocketAddr::from(([203, 0, 113, 7], 50_000));
    request.extensions_mut().insert(ConnectInfo(stranger));
    request
}

fn login(password: &str, from: &str) -> Request<Body> {
    let body = json!({ "username": "ada", "password": password });
    forwarded(Method::POST, "/api/auth/login", None, Some(body), from)
}

/// The addresses of the owner's live sessions, in order.
async fn session_addresses(server: &Server, owner: &Session) -> Vec<String> {
    let reply = server
        .call(Method::GET, "/api/account/sessions", Some(owner), None)
        .await;
    let mut addresses: Vec<String> = reply
        .json()
        .as_array()
        .unwrap()
        .iter()
        .map(|session| session["ip"].as_str().unwrap().to_owned())
        .collect();
    addresses.sort();
    addresses
}

fn set_cookie(reply: &Reply) -> &str {
    reply.headers[header::SET_COOKIE].to_str().unwrap()
}

#[tokio::test]
async fn an_https_origin_is_accepted_behind_a_trusted_proxy_and_refused_directly() {
    let server = Server::behind_proxy().await;
    server.owner().await;

    let through = server.send(login(OWNER_PASSWORD, BROWSER)).await;
    let straight = server.send(direct(login(OWNER_PASSWORD, BROWSER))).await;
    let mut plain = login(OWNER_PASSWORD, BROWSER);
    plain.headers_mut().remove("x-forwarded-proto");
    let plain = server.send(plain).await;

    assert_eq!(through.status, StatusCode::OK, "{}", through.text);
    assert_eq!(straight.status, StatusCode::FORBIDDEN);
    assert_eq!(straight.code(), "origin_mismatch");
    assert_eq!(
        plain.code(),
        "origin_mismatch",
        "a proxy that does not say HTTPS forwards a plain HTTP page"
    );
}

#[tokio::test]
async fn forwarded_headers_from_a_peer_that_is_not_a_trusted_proxy_are_ignored() {
    for server in [Server::new().await, Server::behind_proxy().await] {
        let owner = server.owner().await;
        // The stranger's own page is plain HTTP; only its headers claim otherwise.
        let mut forged = direct(login(OWNER_PASSWORD, BROWSER));
        forged
            .headers_mut()
            .insert(header::ORIGIN, HeaderValue::from_static("http://observer.test"));

        let reply = server.send(forged).await;

        assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
        assert!(!set_cookie(&reply).contains("Secure"), "{}", set_cookie(&reply));
        assert!(!reply.headers.contains_key(header::STRICT_TRANSPORT_SECURITY));
        let addresses = session_addresses(&server, &owner).await;
        assert!(addresses.contains(&"203.0.113.7".to_owned()), "{addresses:?}");
        assert!(!addresses.contains(&BROWSER.to_owned()), "{addresses:?}");
    }
}

#[tokio::test]
async fn the_client_address_comes_from_the_header_behind_one_proxy_and_behind_a_chain() {
    let server = Server::behind_proxy().await;
    let owner = server.owner().await;

    let one = server.send(login(OWNER_PASSWORD, "198.51.100.20")).await;
    let chain = server
        .send(login(OWNER_PASSWORD, "192.0.2.66, 198.51.100.21, 10.0.0.9"))
        .await;

    assert_eq!(one.status, StatusCode::OK, "{}", one.text);
    assert_eq!(chain.status, StatusCode::OK, "{}", chain.text);
    // The owner's first session came through the proxy without a forwarded address.
    assert_eq!(
        session_addresses(&server, &owner).await,
        ["10.0.0.2", "198.51.100.20", "198.51.100.21"]
    );
}

#[tokio::test]
async fn failed_logins_are_throttled_per_forwarded_client_rather_than_per_proxy() {
    let server = Server::behind_proxy().await;
    server.owner().await;

    let wrong = server.send(login("not the password", "198.51.100.20")).await;
    let hurried = server.send(login(OWNER_PASSWORD, "198.51.100.20")).await;
    let elsewhere = server.send(login(OWNER_PASSWORD, "192.0.2.44")).await;

    assert_eq!(wrong.code(), "invalid_credentials");
    assert_eq!(hurried.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(elsewhere.status, StatusCode::OK, "{}", elsewhere.text);
}

#[tokio::test]
async fn the_cookie_is_secure_and_hsts_is_sent_behind_https_only() {
    let server = Server::behind_proxy().await;
    server.owner().await;

    let https = server.send(login(OWNER_PASSWORD, BROWSER)).await;
    let session = https.session();
    let logout = server
        .send(forwarded(
            Method::POST,
            "/api/auth/logout",
            Some(&session),
            None,
            BROWSER,
        ))
        .await;
    let page = server
        .send(forwarded(Method::GET, "/", None, None, BROWSER))
        .await;
    let mut plain = login(OWNER_PASSWORD, BROWSER);
    let headers = plain.headers_mut();
    headers.insert("x-forwarded-proto", HeaderValue::from_static("http"));
    headers.insert(header::ORIGIN, HeaderValue::from_static("http://observer.test"));
    let plain = server.send(plain).await;

    assert_eq!(https.status, StatusCode::OK, "{}", https.text);
    assert!(set_cookie(&https).contains("; HttpOnly; SameSite=Strict; Secure"));
    assert_eq!(logout.status, StatusCode::NO_CONTENT);
    assert!(set_cookie(&logout).contains("; Secure; Max-Age=0"));
    for reply in [&https, &logout, &page] {
        assert_eq!(
            reply.headers[header::STRICT_TRANSPORT_SECURITY],
            "max-age=31536000"
        );
    }
    assert_eq!(plain.status, StatusCode::OK, "{}", plain.text);
    assert!(!set_cookie(&plain).contains("Secure"), "{}", set_cookie(&plain));
    assert!(!plain.headers.contains_key(header::STRICT_TRANSPORT_SECURITY));
}

#[tokio::test]
async fn the_audit_log_records_the_forwarded_address() {
    let server = Server::behind_proxy().await;
    let token = server
        .accounts
        .create_setup_token(ActorKind::System)
        .await
        .unwrap()
        .unwrap()
        .token;
    let body = json!({ "token": token, "username": "ada", "password": OWNER_PASSWORD });

    let reply = server
        .send(forwarded(
            Method::POST,
            "/api/setup",
            None,
            Some(body),
            "198.51.100.20, 10.0.0.9",
        ))
        .await;

    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.text);
    let owner = reply.session();
    let audit = server
        .call(
            Method::GET,
            "/api/audit?action=setup_completed",
            Some(&owner),
            None,
        )
        .await;
    assert_eq!(audit.json()["entries"][0]["ip"], "198.51.100.20");
}
