//! The allowlist: its two lists, the answers to requests, the licences approved ahead, and its
//! settings. A request is addressed by its code and an approval by its id: no licence goes in an
//! address.

use axum::extract::State;
use axum::http::StatusCode;
use axum::{Extension, Json};
use observer_app::Caller;
use observer_domain::allowlist::{
    AllowlistSettings, Approval, ApproveRequest, ApprovedPage, DeniedRequests, DenyRequests, RequestPage,
};
use serde::Deserialize;

use crate::AppState;
use crate::error::Failure;
use crate::extract::{Id, JsonBody, Name, QueryParams};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ListQuery {
    search: Option<String>,
    after: Option<String>,
}

pub(crate) async fn requests(
    State(state): State<AppState>,
    QueryParams(query): QueryParams<ListQuery>,
) -> Result<Json<RequestPage>, Failure> {
    let page = state
        .admission
        .requests(query.search.as_deref(), query.after.as_deref())
        .await?;
    Ok(Json(page))
}

pub(crate) async fn approved(
    State(state): State<AppState>,
    QueryParams(query): QueryParams<ListQuery>,
) -> Result<Json<ApprovedPage>, Failure> {
    let page = state
        .admission
        .approved(query.search.as_deref(), query.after.as_deref())
        .await?;
    Ok(Json(page))
}

pub(crate) async fn approve_request(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    Name(code): Name,
) -> Result<Json<Approval>, Failure> {
    Ok(Json(state.admission.approve_request(&caller, &code).await?))
}

pub(crate) async fn deny_request(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    Name(code): Name,
) -> Result<StatusCode, Failure> {
    state.admission.deny_request(&caller, &code).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn deny_requests(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(request): JsonBody<DenyRequests>,
) -> Result<Json<DeniedRequests>, Failure> {
    Ok(Json(state.admission.deny_requests(&caller, request).await?))
}

pub(crate) async fn approve(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(request): JsonBody<ApproveRequest>,
) -> Result<(StatusCode, Json<Approval>), Failure> {
    Ok((
        StatusCode::CREATED,
        Json(state.admission.approve(&caller, request).await?),
    ))
}

pub(crate) async fn remove_approval(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    Id(id): Id,
) -> Result<StatusCode, Failure> {
    state.admission.remove_approval(&caller, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn settings(State(state): State<AppState>) -> Result<Json<AllowlistSettings>, Failure> {
    Ok(Json(state.admission.allowlist_settings().await?))
}

pub(crate) async fn change_settings(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(settings): JsonBody<AllowlistSettings>,
) -> Result<Json<AllowlistSettings>, Failure> {
    state
        .admission
        .change_allowlist_settings(&caller, settings)
        .await?;
    Ok(Json(state.admission.allowlist_settings().await?))
}
