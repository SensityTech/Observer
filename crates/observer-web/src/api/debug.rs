//! What the resources of a server that counts send and receive. Reading it takes the diagnostics
//! permission; saying a host is known is the owner's.

use axum::extract::State;
use axum::http::StatusCode;
use axum::{Extension, Json};
use observer_app::Caller;
use observer_domain::debug::{DebugBags, DebugEvents, DebugHttp, DebugQuery, DebugStatus, KnownHost};

use crate::AppState;
use crate::error::Failure;
use crate::extract::{JsonBody, QueryParams};

pub(crate) async fn status(State(state): State<AppState>) -> Json<DebugStatus> {
    Json(state.debug.status())
}

pub(crate) async fn events(
    State(state): State<AppState>,
    QueryParams(query): QueryParams<DebugQuery>,
) -> Json<DebugEvents> {
    Json(state.debug.events(query.window))
}

pub(crate) async fn bags(
    State(state): State<AppState>,
    QueryParams(query): QueryParams<DebugQuery>,
) -> Json<DebugBags> {
    Json(state.debug.bags(query.window))
}

pub(crate) async fn http(
    State(state): State<AppState>,
    QueryParams(query): QueryParams<DebugQuery>,
) -> Json<DebugHttp> {
    Json(state.debug.http(query.window))
}

pub(crate) async fn know_host(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(known): JsonBody<KnownHost>,
) -> Result<StatusCode, Failure> {
    state.debug.know_host(&caller, known.resource, known.host).await?;
    Ok(StatusCode::NO_CONTENT)
}
