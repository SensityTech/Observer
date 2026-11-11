//! Bans: the list and its search, a ban, its edits and its lifting, the bans of a profile, the
//! refusal message and the templates. A ban is addressed by its code, which names no player.

use axum::extract::State;
use axum::http::StatusCode;
use axum::{Extension, Json};
use observer_app::Caller;
use observer_domain::bans::{
    Ban, BanEditRequest, BanFilter, BanPage, BanRequest, BanTemplate, BansSettings, LiftRequest,
};
use serde::Deserialize;

use crate::AppState;
use crate::error::Failure;
use crate::extract::{Id, JsonBody, Name, QueryParams};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PageQuery {
    #[serde(default)]
    filter: BanFilter,
    search: Option<String>,
    after: Option<String>,
}

pub(crate) async fn page(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    QueryParams(query): QueryParams<PageQuery>,
) -> Result<Json<BanPage>, Failure> {
    let page = state
        .bans
        .page(
            query.filter,
            query.search.as_deref(),
            query.after.as_deref(),
            caller.permissions,
        )
        .await?;
    Ok(Json(page))
}

pub(crate) async fn create(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(request): JsonBody<BanRequest>,
) -> Result<(StatusCode, Json<Ban>), Failure> {
    Ok((StatusCode::CREATED, Json(state.bans.ban(&caller, request).await?)))
}

pub(crate) async fn edit(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    Name(code): Name,
    JsonBody(request): JsonBody<BanEditRequest>,
) -> Result<Json<Ban>, Failure> {
    Ok(Json(state.bans.edit(&caller, &code, request).await?))
}

pub(crate) async fn lift(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    Name(code): Name,
    JsonBody(request): JsonBody<LiftRequest>,
) -> Result<StatusCode, Failure> {
    state.bans.lift(&caller, &code, request).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn templates(State(state): State<AppState>) -> Result<Json<Vec<BanTemplate>>, Failure> {
    Ok(Json(state.bans.templates().await?))
}

pub(crate) async fn of_player(State(state): State<AppState>, Id(id): Id) -> Result<Json<Vec<Ban>>, Failure> {
    Ok(Json(state.bans.of_player(id).await?))
}

pub(crate) async fn settings(State(state): State<AppState>) -> Result<Json<BansSettings>, Failure> {
    Ok(Json(state.bans.settings().await?))
}

pub(crate) async fn change_settings(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(settings): JsonBody<BansSettings>,
) -> Result<Json<BansSettings>, Failure> {
    state.bans.change_settings(&caller, settings).await?;
    Ok(Json(state.bans.settings().await?))
}
