//! The players: who is connected, the list and its search, a profile, the community's figures,
//! and the owner's retentions and erasures. Players are addressed by their row id, never by an
//! identifier, so that request paths in logs carry none.

use axum::extract::State;
use axum::http::StatusCode;
use axum::{Extension, Json};
use observer_app::Caller;
use observer_domain::players::{
    Community, OnlinePlayers, PlayerPage, PlayerProfile, PlayerSort, PlayersImport, PlayersImportRequest,
    PlayersSettings,
};
use serde::Deserialize;

use crate::AppState;
use crate::error::Failure;
use crate::extract::{Id, JsonBody, QueryParams};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PageQuery {
    search: Option<String>,
    #[serde(default)]
    sort: PlayerSort,
    after: Option<String>,
}

pub(crate) async fn page(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    QueryParams(query): QueryParams<PageQuery>,
) -> Result<Json<PlayerPage>, Failure> {
    let page = state
        .players()?
        .page(
            query.search.as_deref(),
            query.sort,
            query.after.as_deref(),
            caller.permissions,
        )
        .await?;
    Ok(Json(page))
}

pub(crate) async fn online(State(state): State<AppState>) -> Result<Json<OnlinePlayers>, Failure> {
    Ok(Json(state.players()?.online().await?))
}

pub(crate) async fn community(State(state): State<AppState>) -> Result<Json<Community>, Failure> {
    Ok(Json(state.players()?.community().await?))
}

pub(crate) async fn profile(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    Id(id): Id,
) -> Result<Json<PlayerProfile>, Failure> {
    Ok(Json(state.players()?.profile(id, caller.permissions).await?))
}

pub(crate) async fn forget(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    Id(id): Id,
) -> Result<StatusCode, Failure> {
    state.players()?.forget(&caller, id).await?;
    // The drops' cached statistics may hold what the player wrote.
    if let Some(drops) = &state.drops {
        drops.clear();
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Starts the import of txAdmin's players database from a host path, once the file was read and
/// checked whole.
pub(crate) async fn import(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(request): JsonBody<PlayersImportRequest>,
) -> Result<(StatusCode, Json<PlayersImport>), Failure> {
    let import = state.players()?.import(&caller, &request.path).await?;
    Ok((StatusCode::ACCEPTED, Json(import)))
}

pub(crate) async fn import_status(
    State(state): State<AppState>,
) -> Result<Json<Option<PlayersImport>>, Failure> {
    Ok(Json(state.players()?.import_status()))
}

pub(crate) async fn settings(State(state): State<AppState>) -> Result<Json<PlayersSettings>, Failure> {
    Ok(Json(state.players()?.settings().await?))
}

pub(crate) async fn change_settings(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(settings): JsonBody<PlayersSettings>,
) -> Result<Json<PlayersSettings>, Failure> {
    state.players()?.change_settings(&caller, settings).await?;
    Ok(Json(state.players()?.settings().await?))
}
