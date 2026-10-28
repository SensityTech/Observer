//! The HTTP side of Observer: the API, the embedded panel and the headers every response carries.
//! It stays a thin adapter; the rules live in `observer-app`.

mod api;
mod csv;
mod error;
mod extract;
mod gate;
mod live;
mod panel;
mod security;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use axum::extract::Request;
use axum::{Router, middleware};
use observer_app::{Accounts, AuditLog, Servers, Supervision};
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::TraceLayer;
use tracing::Span;

pub use gate::SESSION_KEY_HEADER;
pub use live::Live;

/// The largest request body the API accepts. Nothing it takes comes close; artifacts are
/// downloaded by Observer itself, never uploaded.
const MAX_BODY_BYTES: usize = 1024 * 1024;

#[derive(Clone)]
pub struct AppState {
    pub version: &'static str,
    pub accounts: Accounts,
    pub audit: AuditLog,
    pub servers: Servers,
    pub supervision: Supervision,
    pub live: Live,
    /// Cookies ignore ports, so the name carries the instance id: two Observers on one host never
    /// read each other's sessions.
    cookie: String,
    /// The names the panel is reached by, besides addresses and `localhost`.
    hosts: Arc<[String]>,
}

impl AppState {
    pub fn new(
        version: &'static str,
        accounts: Accounts,
        audit: AuditLog,
        servers: Servers,
        supervision: Supervision,
        instance_id: &str,
        hosts: &[String],
    ) -> Self {
        Self {
            version,
            accounts,
            audit,
            servers,
            supervision,
            live: Live::new(),
            cookie: format!("observer_session_{instance_id}"),
            hosts: hosts.into(),
        }
    }
}

pub fn router(state: AppState) -> Router {
    mount_api(Router::new(), &state)
        .fallback(panel::serve)
        .layer(RequestBodyLimitLayer::new(MAX_BODY_BYTES))
        .layer(CatchPanicLayer::new())
        .layer(middleware::from_fn(security::headers))
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
