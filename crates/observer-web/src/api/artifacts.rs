//! FXServer's builds. Every change answers the whole status, which the `server.artifacts` topic
//! also carries; the cleanup answers what it did, the catalog its view.

use axum::extract::State;
use axum::{Extension, Json};
use observer_app::Caller;
use observer_domain::artifacts::{
    ArtifactSettingsChange, ArtifactsStatus, CatalogQuery, CatalogView, CleanupPreview, CleanupRequest,
    CleanupResult, ImportBuild, InstallBuild, SelectBuild,
};
use observer_domain::error::ErrorCode;

use crate::AppState;
use crate::error::Failure;
use crate::extract::{Id, JsonBody, QueryParams};

/// The selection route as the table declares it, which a refused `SERVER_CONTROL` is audited as.
const SELECTION_ROUTE: &str = "/api/servers/default/artifacts/selection";

pub(crate) async fn catalog(
    State(state): State<AppState>,
    QueryParams(query): QueryParams<CatalogQuery>,
) -> Result<Json<CatalogView>, Failure> {
    Ok(Json(
        state.artifacts.catalog(query.refresh.unwrap_or(false)).await?,
    ))
}

pub(crate) async fn install(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(request): JsonBody<InstallBuild>,
) -> Result<Json<ArtifactsStatus>, Failure> {
    let status = state
        .artifacts
        .install(&caller, request.build, request.allow_end_of_life)
        .await?;
    Ok(Json(status))
}

pub(crate) async fn import(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(request): JsonBody<ImportBuild>,
) -> Result<Json<ArtifactsStatus>, Failure> {
    let status = state
        .artifacts
        .import(&caller, &request.folder, request.build)
        .await?;
    Ok(Json(status))
}

pub(crate) async fn cancel_job(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
) -> Result<Json<ArtifactsStatus>, Failure> {
    Ok(Json(state.artifacts.cancel_job(&caller).await?))
}

pub(crate) async fn select(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(request): JsonBody<SelectBuild>,
) -> Result<Json<ArtifactsStatus>, Failure> {
    let status = state
        .artifacts
        .select(&caller, request, "PUT", SELECTION_ROUTE)
        .await?;
    Ok(Json(status))
}

pub(crate) async fn cancel_pending(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
) -> Result<Json<ArtifactsStatus>, Failure> {
    Ok(Json(state.artifacts.cancel_pending(&caller).await?))
}

/// A number no build can have names nothing installed.
pub(crate) async fn delete(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    Id(build): Id,
) -> Result<Json<ArtifactsStatus>, Failure> {
    let build = u32::try_from(build).map_err(|_| ErrorCode::BuildNotInstalled)?;
    Ok(Json(state.artifacts.delete(&caller, build).await?))
}

pub(crate) async fn cleanup_preview(State(state): State<AppState>) -> Result<Json<CleanupPreview>, Failure> {
    Ok(Json(state.artifacts.cleanup_preview().await?))
}

pub(crate) async fn cleanup(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(request): JsonBody<CleanupRequest>,
) -> Result<Json<CleanupResult>, Failure> {
    Ok(Json(state.artifacts.cleanup(&caller, request.builds).await?))
}

pub(crate) async fn change_settings(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(change): JsonBody<ArtifactSettingsChange>,
) -> Result<Json<ArtifactsStatus>, Failure> {
    Ok(Json(state.artifacts.change_settings(&caller, change).await?))
}
