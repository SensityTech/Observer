//! The HTTP side of Observer: the API, the embedded panel and the headers every response carries.
//! It stays a thin adapter; the rules live in `observer-app`.

mod api;
mod csv;
mod error;
mod extract;
mod gate;
mod live;
mod panel;
mod proxy;
mod security;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use axum::extract::Request;
use axum::{Router, middleware};
use observer_app::{
    Accounts, Artifacts, AuditLog, Changes, Diagnostics, Drops, Files, Metrics, Players, Schedule, Servers,
    Statistics, Supervision, Tools,
};
use observer_domain::about::Build;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::TraceLayer;
use tracing::Span;

pub use gate::SESSION_KEY_HEADER;
pub use live::Live;
pub use proxy::{InvalidProxy, TrustedProxy};

/// The largest request body the API accepts. Nothing it takes comes close; artifacts are
/// downloaded by Observer itself, never uploaded.
const MAX_BODY_BYTES: usize = 1024 * 1024;

#[derive(Clone)]
pub struct AppState {
    pub build: Build,
    pub accounts: Accounts,
    pub audit: AuditLog,
    pub servers: Servers,
    pub supervision: Supervision,
    pub metrics: Metrics,
    pub schedule: Schedule,
    pub artifacts: Artifacts,
    pub diagnostics: Diagnostics,
    pub files: Files,
    pub players: Players,
    pub drops: Drops,
    pub changes: Changes,
    pub tools: Tools,
    pub statistics: Statistics,
    pub live: Live,
    /// Cookies ignore ports, so the name carries the instance id: two Observers on one host never
    /// read each other's sessions.
    cookie: String,
    /// The names the panel is reached by, besides addresses and `localhost`.
    hosts: Arc<[String]>,
    proxies: Arc<[TrustedProxy]>,
}

/// The services the API stands on.
pub struct Services {
    pub accounts: Accounts,
    pub audit: AuditLog,
    pub servers: Servers,
    pub supervision: Supervision,
    pub metrics: Metrics,
    pub schedule: Schedule,
    pub artifacts: Artifacts,
    pub diagnostics: Diagnostics,
    pub files: Files,
    pub players: Players,
    pub drops: Drops,
    pub changes: Changes,
    pub tools: Tools,
    pub statistics: Statistics,
}

impl AppState {
    pub fn new(
        build: Build,
        services: Services,
        instance_id: &str,
        hosts: &[String],
        proxies: &[TrustedProxy],
    ) -> Self {
        let Services {
            accounts,
            audit,
            servers,
            supervision,
            metrics,
            schedule,
            artifacts,
            diagnostics,
            files,
            players,
            drops,
            changes,
            tools,
            statistics,
        } = services;
        Self {
            build,
            accounts,
            audit,
            servers,
            supervision,
            metrics,
            schedule,
            artifacts,
            diagnostics,
            files,
            players,
            drops,
            changes,
            tools,
            statistics,
            live: Live::new(),
            cookie: format!("observer_session_{instance_id}"),
            hosts: hosts.into(),
            proxies: proxies.into(),
        }
    }
}

pub fn router(state: AppState) -> Router {
    mount_api(Router::new(), &state)
        .fallback(panel::serve)
        .layer(RequestBodyLimitLayer::new(MAX_BODY_BYTES))
        .layer(CatchPanicLayer::new())
        .layer(middleware::from_fn(security::headers))
        .layer(middleware::from_fn_with_state(state.clone(), proxy::hear))
        .layer(TraceLayer::new_for_http().make_span_with(span))
        .with_state(state)
}

/// A request's span names its path without the query, which can hold a websocket ticket.
fn span(request: &Request) -> Span {
    tracing::debug_span!(
        "request",
        method = %request.method(),
        path = request.uri().path(),
        version = ?request.version(),
    )
}

#[expect(
    clippy::disallowed_methods,
    reason = "the API, built from its route table alone"
)]
fn mount_api(router: Router<AppState>, state: &AppState) -> Router<AppState> {
    router.nest("/api", api::router(state))
}
