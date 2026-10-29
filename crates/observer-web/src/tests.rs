//! The API over HTTP: requests go through the router, its gates and the services, down to a real
//! database in a temporary directory.

mod live;
mod metrics;
mod schedule;
mod servers;

use std::net::SocketAddr;

use axum::Router;
use axum::body::Body;
use axum::extract::connect_info::MockConnectInfo;
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use jiff::SignedDuration;
use observer_app::{
    Accounts, AuditLog, Clock, Metrics, MetricsSetup, PasswordCost, Running, Schedule, ScheduleSetup,
    ScheduleTask, Servers, Setup, Supervision,
};
use observer_domain::about::Build;
use observer_domain::audit::ActorKind;
use observer_store::Database;
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::ServiceExt;

use crate::api::routes;
use crate::gate::Gate;
use crate::{AppState, Live, SESSION_KEY_HEADER, Services, router};

const HOST: &str = "observer.test:40125";
const ORIGIN: &str = "http://observer.test:40125";
const OWNER_PASSWORD: &str = "correct horse battery staple";

struct Server {
    dir: TempDir,
    app: Router,
    clock: Clock,
    accounts: Accounts,
    audit: AuditLog,
    db: Database,
    live: Live,
    supervision: Supervision,
    metrics: Metrics,
    metrics_clock: Clock,
    _supervised: Running,
    _scheduling: ScheduleTask,
    cookie_name: String,
}

/// What a browser keeps after signing in: the cookie, and the key the panel stores.
#[derive(Debug, Clone)]
struct Session {
    cookie: String,
    key: String,
}

#[derive(Debug)]
struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    text: String,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_str(&self.text).unwrap()
    }

    fn code(&self) -> Value {
        self.json()["code"].clone()
    }

    fn session(&self) -> Session {
        let cookie = self.headers[header::SET_COOKIE].to_str().unwrap();
        Session {
            cookie: cookie.split(';').next().unwrap().to_owned(),
            key: self.json()["sessionKey"].as_str().unwrap().to_owned(),
        }
    }
}

impl Server {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path().join("observer.sqlite3"), dir.path().join("backups"))
            .await
            .unwrap();
        let clock = Clock::manual("2025-01-06T09:00:00Z".parse().unwrap());
        let accounts = Accounts::new(db.clone(), clock.clone(), PasswordCost::TEST)
            .await
            .unwrap();
        let audit = AuditLog::new(db.clone(), clock.clone());
        let servers = Servers::new(db.clone(), clock.clone());
        let (supervision, supervised) = Supervision::start(
            Setup {
                db: db.clone(),
                clock: clock.clone(),
                accounts: accounts.clone(),
                servers: servers.clone(),
                data_dir: dir.path().join("data"),
                console_logs: dir.path().join("data/logs/console"),
                log: None,
            },
            |_| {},
        )
        .await
        .unwrap();
        let (schedule, scheduling) = Schedule::start(
            ScheduleSetup {
                db: db.clone(),
                clock: clock.clone(),
                supervision: supervision.clone(),
            },
            |_| {},
        )
        .await
        .unwrap();
        // A millisecond before a slot ends: once moved past it, the sampler wakes at once.
        let metrics_clock = Clock::manual("2025-01-06T08:59:59.999Z".parse().unwrap());
        let metrics = Metrics::open(MetricsSetup {
            path: dir.path().join("metrics.sqlite3"),
            clock: metrics_clock.clone(),
        })
        .await
        .unwrap();
        let instance_id = db.instance_id().await.unwrap();
        let services = Services {
            accounts: accounts.clone(),
            audit: audit.clone(),
            servers,
            supervision: supervision.clone(),
            metrics: metrics.clone(),
            schedule,
        };
        let state = AppState::new(
            Build {
                version: "0.0.0-test",
                commit: None,
                commit_date: None,
            },
            services,
            &instance_id,
            &["observer.test".to_owned()],
        );
        let live = state.live.clone();
        let app = router(state).layer(MockConnectInfo(SocketAddr::from(([203, 0, 113, 7], 50_000))));
        Self {
            dir,
            app,
            clock,
            accounts,
            audit,
            db,
            live,
            supervision,
            metrics,
            metrics_clock,
            _supervised: supervised,
            _scheduling: scheduling,
            cookie_name: format!("observer_session_{instance_id}"),
        }
    }

    /// A request as the panel sends it: from its own origin, in JSON, with the session if any.
    fn request(method: Method, path: &str, session: Option<&Session>, body: Option<Value>) -> Request<Body> {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, HOST)
            .header(header::ORIGIN, ORIGIN)
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(session) = session {
            request = request
                .header(header::COOKIE, &session.cookie)
                .header(SESSION_KEY_HEADER, &session.key);
        }
        let body = body.map_or_else(Body::empty, |body| Body::from(body.to_string()));
        request.body(body).unwrap()
    }

    async fn send(&self, request: Request<Body>) -> Reply {
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        Reply {
            status,
            headers,
            text: String::from_utf8(bytes.to_vec()).unwrap(),
        }
    }

    async fn call(
        &self,
        method: Method,
        path: &str,
        session: Option<&Session>,
        body: Option<Value>,
    ) -> Reply {
        self.send(Self::request(method, path, session, body)).await
    }

    /// Creates the owner `ada` through the setup link, as the first run does.
    async fn owner(&self) -> Session {
        let token = self
            .accounts
            .create_setup_token(ActorKind::System)
            .await
            .unwrap()
            .unwrap()
            .token;
        let body = json!({ "token": token, "username": "ada", "password": OWNER_PASSWORD });
        let reply = self.call(Method::POST, "/api/setup", None, Some(body)).await;
        assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.text);
        reply.session()
    }

    async fn login(&self, username: &str, password: &str) -> Reply {
        let body = json!({ "username": username, "password": password });
        self.call(Method::POST, "/api/auth/login", None, Some(body)).await
    }

    /// Creates `name` from the owner's session and signs them in with the temporary password.
    async fn user(&self, owner: &Session, name: &str, permissions: &str) -> (Session, String) {
        let body = json!({ "username": name, "permissions": permissions });
        let created = self
            .call(Method::POST, "/api/users", Some(owner), Some(body))
            .await;
        assert_eq!(created.status, StatusCode::CREATED, "{}", created.text);
        let temporary = created.json()["temporaryPassword"].as_str().unwrap().to_owned();
        let reply = self.login(name, &temporary).await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
        (reply.session(), temporary)
    }

    async fn get(&self, path: &str) -> Reply {
        self.call(Method::GET, path, None, None).await
    }
}

const PUBLIC: &[(Method, &str)] = &[
    (Method::GET, "/health"),
    (Method::GET, "/setup"),
    (Method::POST, "/setup"),
    (Method::POST, "/auth/login"),
];

#[test]
fn only_the_public_allow_list_is_open_without_a_session() {
    for route in routes() {
        let listed = PUBLIC.contains(&(route.method.clone(), route.path));
        assert_eq!(
            route.gate == Gate::Public,
            listed,
            "{} {} must be in the public allow-list exactly when its gate is public",
            route.method,
            route.path
        );
    }
}

#[test]
fn no_route_is_declared_twice() {
    let routes = routes();
    for (index, route) in routes.iter().enumerate() {
        assert!(
            routes[index + 1..]
                .iter()
                .all(|other| (&other.method, other.path) != (&route.method, route.path)),
            "{} {}",
            route.method,
            route.path
        );
    }
}

#[tokio::test]
async fn every_gated_route_refuses_a_request_without_a_session() {
    let server = Server::new().await;
    server.owner().await;

    for route in routes().into_iter().filter(|route| route.gate != Gate::Public) {
        let path = format!(
            "/api{}",
            route
                .path
                .replace("{id}", "1")
                .replace("{name}", "console-2026-10-28.log")
        );
        let reply = server
            .call(route.method.clone(), &path, None, Some(json!({})))
            .await;

        assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{} {path}", route.method);
        assert_eq!(reply.code(), "unauthenticated", "{} {path}", route.method);
    }
}

/// The router itself, not only the table: any method on any declared path either meets its gate,
/// is a public route, or does not exist.
#[tokio::test]
async fn no_method_on_an_api_path_skips_the_gate() {
    let server = Server::new().await;
    server.owner().await;
    let methods = [
        Method::GET,
        Method::HEAD,
        Method::POST,
        Method::PUT,
        Method::PATCH,
        Method::DELETE,
        Method::OPTIONS,
    ];

    for route in routes() {
        let path = format!(
            "/api{}",
            route
                .path
                .replace("{id}", "1")
                .replace("{name}", "console-2026-10-28.log")
        );
        for method in methods.clone() {
            if PUBLIC.contains(&(method.clone(), route.path))
                || (method == Method::HEAD && PUBLIC.contains(&(Method::GET, route.path)))
            {
                continue;
            }
            let reply = server.call(method.clone(), &path, None, Some(json!({}))).await;
            assert!(
                matches!(
                    reply.status,
                    StatusCode::UNAUTHORIZED | StatusCode::METHOD_NOT_ALLOWED
                ),
                "{method} {path} answered {}",
                reply.status
            );
        }
    }
}

#[tokio::test]
async fn health_reports_the_running_version() {
    let server = Server::new().await;

    let reply = server.get("/api/health").await;

    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.json(), json!({ "status": "ok", "version": "0.0.0-test" }));
}

#[tokio::test]
async fn about_reports_this_build() {
    let server = Server::new().await;
    let owner = server.owner().await;

    let reply = server.call(Method::GET, "/api/about", Some(&owner), None).await;

    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
    let about = reply.json();
    assert_eq!(about["version"], "0.0.0-test");
    assert_eq!(about["commit"], Value::Null);
    assert_eq!(about["os"], std::env::consts::OS);
    assert_eq!(about["profile"], "debug");
}

/// The list names every crate and version Observer runs: only a signed-in user reads it, through
/// the API, whether or not this build carries one. The panel's file lookup turns `\` into `/`,
/// and on disk resolves `.` and `..`, so every spelling is tried.
#[tokio::test]
async fn the_licence_list_is_never_a_public_panel_file() {
    let server = Server::new().await;
    let owner = server.owner().await;

    for path in [
        "/about/licences.json",
        r"/about\licences.json",
        r"/about\licences.json.gz",
        "/./about/licences.json",
        "/ABOUT/licences.json",
        "/licences.json",
        "/licences.json.br",
        "/../private/licences.json",
        r"/..\private\licences.json",
    ] {
        for session in [None, Some(&owner)] {
            let reply = server.call(Method::GET, path, session, None).await;
            assert_eq!(
                reply.status,
                StatusCode::NOT_FOUND,
                "{path} served {} bytes",
                reply.text.len()
            );
        }
    }
    let listed = server
        .call(Method::GET, "/api/about/licences", Some(&owner), None)
        .await;
    if listed.status == StatusCode::OK {
        assert!(listed.json()["packages"].is_array());
    } else {
        assert_eq!(listed.code(), "not_found");
    }
    assert_eq!(
        server.get("/api/about/licences").await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn unknown_api_paths_are_not_found_rather_than_the_panel() {
    let server = Server::new().await;

    let reply = server.get("/api/nothing-here").await;

    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.code(), "not_found");
}

#[tokio::test]
async fn a_missing_file_with_an_extension_is_not_found() {
    let server = Server::new().await;

    assert_eq!(
        server.get("/assets/missing-0000.js").await.status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn every_response_carries_the_security_headers() {
    let server = Server::new().await;

    let reply = server.get("/api/nothing-here").await;

    let csp = reply.headers[header::CONTENT_SECURITY_POLICY].to_str().unwrap();
    assert!(csp.contains("script-src 'self'"));
    assert!(csp.contains("frame-ancestors 'none'"));
    assert_eq!(reply.headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    assert_eq!(reply.headers[header::REFERRER_POLICY], "no-referrer");
    assert_eq!(reply.headers[header::CACHE_CONTROL], "no-store");
    assert!(!reply.headers.contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN));
}

#[tokio::test]
async fn setup_signs_the_owner_in_with_a_strict_http_only_cookie() {
    let server = Server::new().await;
    assert_eq!(server.get("/api/setup").await.json(), json!({ "required": true }));
    let token = server
        .accounts
        .create_setup_token(ActorKind::System)
        .await
        .unwrap()
        .unwrap()
        .token;

    let body = json!({ "token": token, "username": "ada", "password": OWNER_PASSWORD });
    let reply = server
        .call(Method::POST, "/api/setup", None, Some(body.clone()))
        .await;

    assert_eq!(reply.status, StatusCode::CREATED);
    let cookie = reply.headers[header::SET_COOKIE].to_str().unwrap();
    assert!(cookie.starts_with(&format!("{}=", server.cookie_name)));
    assert!(cookie.contains("; HttpOnly"));
    assert!(cookie.contains("; SameSite=Strict"));
    assert!(cookie.contains("; Path=/"));
    assert!(
        !cookie.contains("Max-Age"),
        "unasked, the cookie ends with the browser"
    );
    let session = reply.json();
    assert_eq!(session["sessionKey"].as_str().unwrap().len(), 64);
    assert_eq!(session["me"]["username"], "ada");
    assert_eq!(session["me"]["isOwner"], true);
    assert_eq!(session["me"]["permissions"], "fff");

    let again = server.call(Method::POST, "/api/setup", None, Some(body)).await;
    assert_eq!(again.status, StatusCode::CONFLICT);
    assert_eq!(again.code(), "already_set_up");
    assert_eq!(
        server.get("/api/setup").await.json(),
        json!({ "required": false })
    );
}

#[tokio::test]
async fn a_request_needs_both_the_session_cookie_and_the_session_key() {
    let server = Server::new().await;
    let session = server.owner().await;

    let cookie_only = Request::get("/api/auth/me")
        .header(header::COOKIE, &session.cookie)
        .body(Body::empty())
        .unwrap();
    let key_only = Request::get("/api/auth/me")
        .header(SESSION_KEY_HEADER, &session.key)
        .body(Body::empty())
        .unwrap();
    let cookie_only = server.send(cookie_only).await;
    let key_only = server.send(key_only).await;
    let both = server
        .call(Method::GET, "/api/auth/me", Some(&session), None)
        .await;

    assert_eq!(cookie_only.status, StatusCode::UNAUTHORIZED);
    assert_eq!(cookie_only.code(), "unauthenticated");
    assert_eq!(key_only.status, StatusCode::UNAUTHORIZED);
    assert_eq!(both.status, StatusCode::OK);
    assert_eq!(both.json()["username"], "ada");
}

#[tokio::test]
async fn a_change_from_another_origin_or_not_in_json_is_refused() {
    let server = Server::new().await;
    let session = server.owner().await;
    let logout = |origin: Option<&str>, fetch_site: Option<&str>, content_type: &str| {
        let mut request = Request::post("/api/auth/logout")
            .header(header::HOST, HOST)
            .header(header::CONTENT_TYPE, content_type)
            .header(header::COOKIE, &session.cookie)
            .header(SESSION_KEY_HEADER, &session.key);
        if let Some(origin) = origin {
            request = request.header(header::ORIGIN, origin);
        }
        if let Some(site) = fetch_site {
            request = request.header("sec-fetch-site", site);
        }
        request.body(Body::empty()).unwrap()
    };

    // FXServer's own port on the same host is another origin.
    for refused in [
        logout(Some("http://observer.test:30120"), None, "application/json"),
        logout(Some("https://observer.test:40125"), None, "application/json"),
        logout(Some("null"), Some("same-origin"), "application/json"),
        logout(None, None, "application/json"),
        logout(None, Some("same-site"), "application/json"),
    ] {
        let reply = server.send(refused).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN);
        assert_eq!(reply.code(), "origin_mismatch");
    }
    let form = server
        .send(logout(Some(ORIGIN), None, "application/x-www-form-urlencoded"))
        .await;
    assert_eq!(form.status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert_eq!(form.code(), "json_required");

    let same_site_login = Request::post("/api/auth/login")
        .header(header::HOST, HOST)
        .header(header::ORIGIN, "http://observer.test:30120")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({ "username": "ada", "password": OWNER_PASSWORD }).to_string(),
        ))
        .unwrap();
    assert_eq!(server.send(same_site_login).await.code(), "origin_mismatch");

    let fetched = server
        .send(logout(
            None,
            Some("same-origin"),
            "application/json; charset=utf-8",
        ))
        .await;
    assert_eq!(fetched.status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn logout_ends_the_session_and_expires_the_cookie() {
    let server = Server::new().await;
    let session = server.owner().await;

    let reply = server
        .call(Method::POST, "/api/auth/logout", Some(&session), None)
        .await;

    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    let cookie = reply.headers[header::SET_COOKIE].to_str().unwrap();
    assert!(cookie.starts_with(&format!("{}=;", server.cookie_name)));
    assert!(cookie.contains("Max-Age=0"));
    let me = server
        .call(Method::GET, "/api/auth/me", Some(&session), None)
        .await;
    assert_eq!(me.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_user_with_a_temporary_password_can_only_change_it() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let (grace, temporary) = server.user(&owner, "grace", "100").await;

    let me = server.call(Method::GET, "/api/auth/me", Some(&grace), None).await;
    assert_eq!(me.json()["mustChangePassword"], true);
    for (method, path) in [
        (Method::GET, "/api/account/sessions"),
        (Method::GET, "/api/audit"),
        (Method::POST, "/api/auth/reauth"),
    ] {
        let reply = server
            .call(method, path, Some(&grace), Some(json!({ "password": temporary })))
            .await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{path}");
        assert_eq!(reply.code(), "password_change_required", "{path}");
    }

    let change = json!({ "current": temporary, "next": "grace chose this one" });
    let changed = server
        .call(Method::POST, "/api/account/password", Some(&grace), Some(change))
        .await;
    assert_eq!(changed.status, StatusCode::NO_CONTENT, "{}", changed.text);

    let sessions = server
        .call(Method::GET, "/api/account/sessions", Some(&grace), None)
        .await;
    assert_eq!(sessions.status, StatusCode::OK);
    assert_eq!(sessions.json()[0]["current"], true);
    let audit = server.call(Method::GET, "/api/audit", Some(&grace), None).await;
    assert_eq!(audit.status, StatusCode::OK);
}

#[tokio::test]
async fn user_changes_need_the_password_entered_within_ten_minutes() {
    let server = Server::new().await;
    let owner = server.owner().await;
    server.clock.advance(SignedDuration::from_mins(11));
    let body = json!({ "username": "grace", "permissions": "1" });

    let refused = server
        .call(Method::POST, "/api/users", Some(&owner), Some(body.clone()))
        .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    assert_eq!(refused.code(), "reauth_required");

    let wrong = server
        .call(
            Method::POST,
            "/api/auth/reauth",
            Some(&owner),
            Some(json!({ "password": "not the password" })),
        )
        .await;
    assert_eq!(wrong.code(), "wrong_password");
    server.clock.advance(SignedDuration::from_secs(2));
    let reauth = server
        .call(
            Method::POST,
            "/api/auth/reauth",
            Some(&owner),
            Some(json!({ "password": OWNER_PASSWORD })),
        )
        .await;
    assert_eq!(reauth.status, StatusCode::NO_CONTENT);

    let created = server
        .call(Method::POST, "/api/users", Some(&owner), Some(body))
        .await;
    assert_eq!(created.status, StatusCode::CREATED);
    assert_eq!(created.json()["user"]["permissions"], "1");
}

#[tokio::test]
async fn a_missing_permission_is_forbidden_and_names_it() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let (grace, temporary) = server.user(&owner, "grace", "2").await;
    let change = json!({ "current": temporary, "next": "grace chose this one" });
    server
        .call(Method::POST, "/api/account/password", Some(&grace), Some(change))
        .await;

    let reply = server.call(Method::GET, "/api/users", Some(&grace), None).await;
    let delete = server
        .call(Method::DELETE, "/api/users/1", Some(&grace), None)
        .await;

    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(
        reply.json(),
        json!({ "code": "forbidden", "params": { "permission": "usersManage" } })
    );
    assert_eq!(delete.code(), "forbidden");
}

#[tokio::test]
async fn refusals_carry_a_code_its_params_and_when_to_retry() {
    let server = Server::new().await;
    server.owner().await;

    let wrong = server.login("ada", "not the password").await;
    let hurried = server.login("ada", OWNER_PASSWORD).await;
    server.clock.advance(SignedDuration::from_secs(2));
    let malformed = server
        .send(
            Request::post("/api/auth/login")
                .header(header::HOST, HOST)
                .header(header::ORIGIN, ORIGIN)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{\"username\": \"ada\""))
                .unwrap(),
        )
        .await;

    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        wrong.json(),
        json!({ "code": "invalid_credentials", "params": {} })
    );
    assert_eq!(hurried.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(hurried.json()["params"]["retryAfter"], 1);
    assert_eq!(hurried.headers[header::RETRY_AFTER], "1");
    assert_eq!(malformed.status, StatusCode::BAD_REQUEST);
    assert_eq!(malformed.code(), "invalid_request");
}

#[tokio::test]
async fn accounts_are_managed_through_the_api_with_versions() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let created = server
        .call(
            Method::POST,
            "/api/users",
            Some(&owner),
            Some(json!({ "username": "grace", "permissions": "4" })),
        )
        .await;
    let user = created.json()["user"].clone();
    assert_eq!(user["permissions"], "6");
    let path = format!("/api/users/{}", user["id"]);

    let edit = |permissions: &str| json!({ "permissions": permissions, "version": user["version"] });
    let updated = server
        .call(Method::PATCH, &path, Some(&owner), Some(edit("100")))
        .await;
    let stale = server
        .call(Method::PATCH, &path, Some(&owner), Some(edit("800")))
        .await;
    let unknown = server
        .call(Method::PATCH, &path, Some(&owner), Some(edit("1000")))
        .await;
    let reset = server
        .call(
            Method::POST,
            &format!("{path}/reset-password"),
            Some(&owner),
            None,
        )
        .await;

    assert_eq!(updated.status, StatusCode::OK);
    assert_eq!(updated.json()["permissions"], "100");
    assert_eq!(stale.status, StatusCode::CONFLICT);
    assert_eq!(stale.code(), "conflict");
    assert_eq!(unknown.json()["params"]["permissions"], "1000");
    assert_eq!(reset.json()["temporaryPassword"].as_str().unwrap().len(), 20);

    let catalog = server
        .call(Method::GET, "/api/permissions", Some(&owner), None)
        .await;
    assert_eq!(
        catalog.json()[2],
        json!({
            "bit": 2,
            "key": "consoleWrite",
            "area": "console",
            "danger": "critical",
            "implies": "2",
        })
    );
    let listed = server.call(Method::GET, "/api/users", Some(&owner), None).await;
    assert_eq!(listed.json().as_array().unwrap().len(), 2);
    let deleted = server.call(Method::DELETE, &path, Some(&owner), None).await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    let owner_id = listed.json()[0]["id"].clone();
    let own = server
        .call(
            Method::DELETE,
            &format!("/api/users/{owner_id}"),
            Some(&owner),
            None,
        )
        .await;
    assert_eq!(own.code(), "self_edit");
    let missing = server
        .call(Method::DELETE, "/api/users/x", Some(&owner), None)
        .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_audit_log_pages_by_filter_and_exports_as_csv() {
    let server = Server::new().await;
    let owner = server.owner().await;

    let page = server
        .call(
            Method::GET,
            "/api/audit?action=setup_completed&limit=10",
            Some(&owner),
            None,
        )
        .await;
    let invalid = server
        .call(Method::GET, "/api/audit?outcome=maybe", Some(&owner), None)
        .await;
    let export = server
        .call(Method::GET, "/api/audit/export.csv?actor=ada", Some(&owner), None)
        .await;

    let entries = page.json()["entries"].clone();
    assert_eq!(entries.as_array().unwrap().len(), 1);
    assert_eq!(entries[0]["actorName"], "ada");
    assert_eq!(page.json()["next"], Value::Null);
    assert_eq!(invalid.code(), "invalid_request");
    assert_eq!(export.status, StatusCode::OK);
    assert_eq!(export.headers[header::CONTENT_TYPE], "text/csv; charset=utf-8");
    let lines: Vec<&str> = export.text.split("\r\n").collect();
    assert!(lines[0].starts_with("id,at,actor_kind,"));
    assert!(lines[1].contains(",audit_exported,"));
    assert!(lines[2].contains(",setup_completed,"));
    assert_eq!(lines.len(), 4, "two entries, the header and the final line break");
}

#[tokio::test]
async fn a_stale_edit_never_reaches_an_account_created_after_a_deletion() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let create = |name: &str, permissions: &str| json!({ "username": name, "permissions": permissions });
    let alice = server
        .call(
            Method::POST,
            "/api/users",
            Some(&owner),
            Some(create("alice", "100")),
        )
        .await
        .json()["user"]
        .clone();
    let alice_path = format!("/api/users/{}", alice["id"]);
    server.call(Method::DELETE, &alice_path, Some(&owner), None).await;
    let bob = server
        .call(Method::POST, "/api/users", Some(&owner), Some(create("bob", "0")))
        .await
        .json()["user"]
        .clone();

    let edit = json!({ "permissions": "180", "version": alice["version"] });
    let stale = server
        .call(Method::PATCH, &alice_path, Some(&owner), Some(edit))
        .await;

    assert_ne!(bob["id"], alice["id"]);
    assert_eq!(stale.status, StatusCode::NOT_FOUND);
    let listed = server.call(Method::GET, "/api/users", Some(&owner), None).await;
    assert_eq!(listed.json()[1]["username"], "bob");
    assert_eq!(listed.json()[1]["permissions"], "0");
}

#[tokio::test]
async fn a_denied_request_is_audited_by_its_route_rather_than_its_path() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let (grace, temporary) = server.user(&owner, "grace", "1").await;
    let change = json!({ "current": temporary, "next": "grace chose this one" });
    server
        .call(Method::POST, "/api/account/password", Some(&grace), Some(change))
        .await;

    let long = format!("/api/users/{}", "A".repeat(4000));
    let denied = server.call(Method::DELETE, &long, Some(&grace), None).await;

    assert_eq!(denied.code(), "forbidden");
    let audit = server
        .call(
            Method::GET,
            "/api/audit?action=permission_denied",
            Some(&owner),
            None,
        )
        .await;
    assert_eq!(audit.json()["entries"][0]["target"], "DELETE /api/users/{id}");
}

#[tokio::test]
async fn an_audit_filter_naming_no_possible_user_is_refused_before_it_is_recorded() {
    let server = Server::new().await;
    let owner = server.owner().await;

    let path = format!("/api/audit/export.csv?actor={}", "Z".repeat(33));
    let export = server.call(Method::GET, &path, Some(&owner), None).await;

    assert_eq!(export.status, StatusCode::BAD_REQUEST);
    assert_eq!(export.code(), "invalid_request");
    let audit = server
        .call(
            Method::GET,
            "/api/audit?action=audit_exported",
            Some(&owner),
            None,
        )
        .await;
    assert_eq!(audit.json()["entries"], json!([]));
}

#[tokio::test]
async fn the_session_cookie_outlives_the_browser_only_when_asked() {
    let server = Server::new().await;
    server.owner().await;
    let login = |remember: Option<bool>| {
        let mut body = json!({ "username": "ada", "password": OWNER_PASSWORD });
        if let Some(remember) = remember {
            body["remember"] = json!(remember);
        }
        Server::request(Method::POST, "/api/auth/login", None, Some(body))
    };

    let mut cookies = Vec::new();
    for remember in [None, Some(false), Some(true)] {
        server.clock.advance(SignedDuration::from_secs(2));
        let reply = server.send(login(remember)).await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
        cookies.push(reply.headers[header::SET_COOKIE].to_str().unwrap().to_owned());
    }

    for browser_only in &cookies[..2] {
        assert!(!browser_only.contains("Max-Age"), "{browser_only}");
        assert!(!browser_only.contains("Expires"), "{browser_only}");
        assert!(browser_only.contains("; HttpOnly; SameSite=Strict"));
    }
    assert!(cookies[2].ends_with(&format!("; Max-Age={}", 7 * 24 * 3600)));
}

#[tokio::test]
async fn a_request_for_a_name_observer_was_not_given_is_refused() {
    let server = Server::new().await;
    server.owner().await;
    let login = |host: &str| {
        Request::post("/api/auth/login")
            .header(header::HOST, host)
            .header(header::ORIGIN, format!("http://{host}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({ "username": "ada", "password": "not the password" }).to_string(),
            ))
            .unwrap()
    };

    // A stranger's page whose name now resolves to this host would pass as its own origin.
    let rebound = server.send(login("rebind.attacker.example:40125")).await;
    let read = server
        .send(
            Request::get("/api/setup")
                .header(header::HOST, "rebind.attacker.example:40125")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

    assert_eq!(rebound.status, StatusCode::MISDIRECTED_REQUEST);
    assert_eq!(rebound.code(), "unknown_host");
    assert_eq!(read.code(), "unknown_host");
    for known in [
        "OBSERVER.test:40125",
        "localhost:40125",
        "127.0.0.1:40125",
        "192.168.1.20:40125",
        "[::1]:40125",
    ] {
        server.clock.advance(SignedDuration::from_secs(2));
        let reply = server.send(login(known)).await;
        assert_eq!(reply.code(), "invalid_credentials", "{known}");
    }
}

/// A folder of the test's host, as its platform writes paths.
fn folder(name: &str) -> String {
    if cfg!(windows) {
        format!(r"C:\FXServer\{name}")
    } else {
        format!("/srv/fxserver/{name}")
    }
}

fn launch_settings() -> Value {
    json!({
        "buildPath": folder("build"),
        "dataPath": folder("server-data"),
        "cfgFile": "server.cfg",
        "onesync": "on",
        "autoStart": true,
        "txadminEvents": false,
        "extraArguments": [{ "name": "sv_enforceGameBuild", "value": "3258" }],
    })
}

const LAUNCH: &str = "/api/servers/default/launch";

/// A change made from `version`, the one last read.
fn change(settings: Value, version: Option<i64>) -> Value {
    let mut change = json!({ "version": version });
    change["settings"] = settings;
    change
}

#[tokio::test]
async fn the_owner_reads_and_saves_the_launch_settings_with_their_password_again() {
    let server = Server::new().await;
    let owner = server.owner().await;

    let unset = server.call(Method::GET, LAUNCH, Some(&owner), None).await;
    let saved = server
        .call(
            Method::PUT,
            LAUNCH,
            Some(&owner),
            Some(change(launch_settings(), None)),
        )
        .await;
    let read = server.call(Method::GET, LAUNCH, Some(&owner), None).await;
    let mut edited = launch_settings();
    edited["onesync"] = json!("legacy");
    let outdated = server
        .call(
            Method::PUT,
            LAUNCH,
            Some(&owner),
            Some(change(edited.clone(), None)),
        )
        .await;
    server.clock.advance(SignedDuration::from_mins(11));
    let stale = server
        .call(Method::PUT, LAUNCH, Some(&owner), Some(change(edited, Some(1))))
        .await;

    assert_eq!(unset.status, StatusCode::OK);
    assert_eq!(unset.json(), Value::Null);
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.text);
    assert_eq!(
        saved.json(),
        json!({ "settings": launch_settings(), "version": 1 })
    );
    assert_eq!(read.json(), saved.json());
    assert_eq!(outdated.status, StatusCode::CONFLICT);
    assert_eq!(outdated.code(), "conflict");
    assert_eq!(stale.code(), "reauth_required");
}

#[tokio::test]
async fn launch_settings_with_issues_are_refused_with_each_issue() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let mut settings = launch_settings();
    settings["cfgFile"] = json!("../server.cfg");
    settings["extraArguments"] = json!([{ "name": "exec", "value": "other.cfg; quit" }]);
    let mut unknown = launch_settings();
    unknown["onesync"] = json!("sometimes");

    let refused = server
        .call(Method::PUT, LAUNCH, Some(&owner), Some(change(settings, None)))
        .await;
    let malformed = server
        .call(Method::PUT, LAUNCH, Some(&owner), Some(change(unknown, None)))
        .await;
    let bare = server
        .call(Method::PUT, LAUNCH, Some(&owner), Some(launch_settings()))
        .await;

    assert_eq!(refused.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        refused.json(),
        json!({
            "code": "invalid_launch_settings",
            "params": { "issues": [
                { "field": "cfgFile", "index": null, "problem": "invalidCfgFile" },
                { "field": "argumentName", "index": 0, "problem": "reserved" },
                { "field": "argumentValue", "index": 0, "problem": "forbiddenCharacter" },
            ] },
        })
    );
    assert_eq!(malformed.code(), "invalid_request");
    assert_eq!(bare.code(), "invalid_request", "the version is required");
    let stored = server.call(Method::GET, LAUNCH, Some(&owner), None).await;
    assert_eq!(stored.json(), Value::Null);
}

#[tokio::test]
async fn a_user_holding_every_permission_still_cannot_touch_the_launch_settings() {
    let server = Server::new().await;
    let owner = server.owner().await;
    let (grace, temporary) = server.user(&owner, "grace", "fff").await;
    let password = json!({ "current": temporary, "next": "grace chose this one" });
    server
        .call(
            Method::POST,
            "/api/account/password",
            Some(&grace),
            Some(password),
        )
        .await;

    let read = server.call(Method::GET, LAUNCH, Some(&grace), None).await;
    let write = server
        .call(
            Method::PUT,
            LAUNCH,
            Some(&grace),
            Some(change(launch_settings(), None)),
        )
        .await;

    assert_eq!(read.status, StatusCode::FORBIDDEN);
    assert_eq!(read.code(), "owner_only");
    assert_eq!(write.code(), "owner_only");
    let denied = server
        .call(
            Method::GET,
            "/api/audit?action=permission_denied",
            Some(&owner),
            None,
        )
        .await;
    let entries = denied.json()["entries"].clone();
    assert_eq!(entries.as_array().unwrap().len(), 1);
    assert_eq!(entries[0]["target"], "PUT /api/servers/default/launch");
    assert_eq!(entries[0]["actorName"], "grace");
}
