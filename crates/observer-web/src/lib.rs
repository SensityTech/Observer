//! The HTTP side of Observer: the API, the embedded panel and the headers every response carries.
//! It stays a thin adapter; the rules live in `observer-app`.

mod api;
mod csv;
mod error;
mod extract;
mod gate;
mod panel;
mod security;
#[cfg(test)]
mod tests;

use axum::{Router, middleware};
use observer_app::{Accounts, AuditLog};
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::TraceLayer;

pub use gate::SESSION_KEY_HEADER;

/// The largest request body the API accepts. Nothing it takes comes close; artifacts are
/// downloaded by Observer itself, never uploaded.
const MAX_BODY_BYTES: usize = 1024 * 1024;

#[derive(Clone)]
pub struct AppState {
    pub version: &'static str,
    pub accounts: Accounts,
    pub audit: AuditLog,
    /// Cookies ignore ports, so the name carries the instance id: two Observers on one host never
    /// read each other's sessions.
    cookie: String,
}

impl AppState {
    pub fn new(version: &'static str, accounts: Accounts, audit: AuditLog, instance_id: &str) -> Self {
        Self {
            version,
            accounts,
            audit,
            cookie: format!("observer_session_{instance_id}"),
        }
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .nest("/api", api::router(&state))
        .fallback(panel::serve)
        .layer(RequestBodyLimitLayer::new(MAX_BODY_BYTES))
        .layer(CatchPanicLayer::new())
        .layer(middleware::from_fn(security::headers))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
