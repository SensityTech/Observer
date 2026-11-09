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

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::Request;
use axum::{Router, middleware};
use observer_app::{
    Accounts, Artifacts, AuditLog, Backups, Changes, Diagnostics, Drops, Error, Features, Files, Metrics,
    Players, Schedule, Servers, Statistics, Supervision, Tools, Updates,
};
use observer_domain::about::Build;
use observer_domain::features::Feature;
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
    pub schedule: Schedule,
    pub artifacts: Artifacts,
    pub files: Files,
    pub tools: Tools,
    pub statistics: Statistics,
    pub features: Features,
    /// The services of the features Observer started without are absent. Their routes and topics
    /// are refused before anything reaches for them, through the accessors below.
    diagnostics: Option<Diagnostics>,
    metrics: Option<Metrics>,
    players: Option<Players>,
    drops: Option<Drops>,
    changes: Option<Changes>,
    updates: Option<Updates>,
    pub backups: Backups,
    pub live: Live,
    /// Cookies ignore ports, so the name carries the instance id: two Observers on one host never
    /// read each other's sessions.
    cookie: String,
    /// The names the panel is reached by, besides addresses and `localhost`.
    hosts: Arc<[String]>,
    proxies: Arc<[TrustedProxy]>,
    bootstrap: Arc<Bootstrap>,
}

/// What the owner reads of `observer.toml` on the Settings page besides the hosts and proxies,
/// which the panel's requests are checked against.
pub struct Bootstrap {
    pub config_file: PathBuf,
    pub listen: SocketAddr,
}

/// The services the API stands on.
pub struct Services {
    pub accounts: Accounts,
    pub audit: AuditLog,
    pub servers: Servers,
    pub supervision: Supervision,
    pub schedule: Schedule,
    pub artifacts: Artifacts,
    pub files: Files,
    pub tools: Tools,
    pub statistics: Statistics,
    pub features: Features,
    /// `None` for each feature that does not run at Observer's start.
    pub diagnostics: Option<Diagnostics>,
    pub metrics: Option<Metrics>,
    pub players: Option<Players>,
    pub drops: Option<Drops>,
    pub changes: Option<Changes>,
    pub updates: Option<Updates>,
    pub backups: Backups,
}

impl AppState {
    pub fn new(
        build: Build,
        services: Services,
        instance_id: &str,
        hosts: &[String],
        proxies: &[TrustedProxy],
        bootstrap: Bootstrap,
    ) -> Self {
        let Services {
            accounts,
            audit,
            servers,
            supervision,
            schedule,
            artifacts,
            files,
            tools,
            features,
            diagnostics,
            metrics,
            players,
            drops,
            changes,
            updates,
            backups,
            statistics,
        } = services;
        Self {
            build,
            accounts,
            audit,
            servers,
            supervision,
            schedule,
            artifacts,
            files,
            tools,
            features,
            diagnostics,
            metrics,
            players,
            drops,
            changes,
            updates,
            backups,
            statistics,
            live: Live::new(),
            cookie: format!("observer_session_{instance_id}"),
            hosts: hosts.into(),
            proxies: proxies.into(),
            bootstrap: Arc::new(bootstrap),
        }
    }
}

impl AppState {
    pub(crate) fn diagnostics(&self) -> Result<&Diagnostics, Error> {
        self.diagnostics
            .as_ref()
            .ok_or_else(|| self.features.refusal(Feature::Diagnostics))
    }

    pub(crate) fn metrics(&self) -> Result<&Metrics, Error> {
        self.metrics
            .as_ref()
            .ok_or_else(|| self.features.refusal(Feature::Metrics))
    }

    pub(crate) fn players(&self) -> Result<&Players, Error> {
        self.players
            .as_ref()
            .ok_or_else(|| self.features.refusal(Feature::Players))
    }

    pub(crate) fn drops(&self) -> Result<&Drops, Error> {
        self.drops
            .as_ref()
            .ok_or_else(|| self.features.refusal(Feature::Drops))
    }

    pub(crate) fn changes(&self) -> Result<&Changes, Error> {
        self.changes
            .as_ref()
            .ok_or_else(|| self.features.refusal(Feature::Changes))
    }

    pub(crate) fn updates(&self) -> Result<&Updates, Error> {
        self.updates
            .as_ref()
            .ok_or_else(|| self.features.refusal(Feature::Updates))
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
