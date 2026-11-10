//! The server tools: the Tools page, a scan, the fix the owner confirms, an undo and the settings.
//! Reading and scanning are the owner's; anything that writes into the server folder or changes
//! what runs at every start takes the password again.

use axum::extract::State;
use axum::{Extension, Json};
use observer_app::Caller;
use observer_domain::tools::{ToolFixRequest, ToolRun, ToolSettingsChange, ToolUndoRequest, ToolsView};

use crate::AppState;
use crate::error::Failure;
use crate::extract::JsonBody;

pub(crate) async fn view(State(state): State<AppState>) -> Result<Json<ToolsView>, Failure> {
    Ok(Json(state.tools.view().await?))
}

/// Keeps the plan whose digest the request names for the next start of FXServer, which writes it
/// before FXServer starts if a fresh scan plans exactly that.
pub(crate) async fn stage(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(request): JsonBody<ToolFixRequest>,
) -> Result<Json<ToolsView>, Failure> {
    Ok(Json(state.tools.stage(&caller, &request.plan).await?))
}

pub(crate) async fn unstage(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
) -> Result<Json<ToolsView>, Failure> {
    Ok(Json(state.tools.unstage(&caller).await?))
}

/// Scans the server folder and records the report: nothing is written.
pub(crate) async fn scan(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
) -> Result<Json<ToolRun>, Failure> {
    Ok(Json(state.tools.scan(&caller).await?))
}

/// Applies the plan whose digest the request names, once a fresh scan plans exactly that, with
/// FXServer stopped.
pub(crate) async fn fix(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(request): JsonBody<ToolFixRequest>,
) -> Result<Json<ToolRun>, Failure> {
    Ok(Json(state.tools.fix(&caller, &request.plan).await?))
}

pub(crate) async fn undo(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(request): JsonBody<ToolUndoRequest>,
) -> Result<Json<ToolRun>, Failure> {
    Ok(Json(state.tools.undo(&caller, request).await?))
}

pub(crate) async fn change_settings(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(change): JsonBody<ToolSettingsChange>,
) -> Result<Json<ToolsView>, Failure> {
    Ok(Json(state.tools.change_settings(&caller, change).await?))
}
