//! Maintenance mode: its switch, the players who still get in, and its settings. A pass is
//! addressed by its id: no licence goes in an address.

use axum::extract::State;
use axum::http::StatusCode;
use axum::{Extension, Json};
use observer_app::Caller;
use observer_domain::maintenance::{
    AddPass, MaintenancePass, MaintenancePasses, MaintenancePreview, MaintenanceSettings, SetMaintenance,
};

use crate::AppState;
use crate::error::Failure;
use crate::extract::{Id, JsonBody};

pub(crate) async fn preview(State(state): State<AppState>) -> Result<Json<MaintenancePreview>, Failure> {
    Ok(Json(state.admission.maintenance_preview().await?))
}

pub(crate) async fn set(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(request): JsonBody<SetMaintenance>,
) -> Result<StatusCode, Failure> {
    state.admission.set_maintenance(&caller, request).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn settings(State(state): State<AppState>) -> Result<Json<MaintenanceSettings>, Failure> {
    Ok(Json(state.admission.maintenance_settings().await?))
}

pub(crate) async fn change_settings(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(settings): JsonBody<MaintenanceSettings>,
) -> Result<Json<MaintenanceSettings>, Failure> {
    state
        .admission
        .change_maintenance_settings(&caller, settings)
        .await?;
    Ok(Json(state.admission.maintenance_settings().await?))
}

pub(crate) async fn passes(State(state): State<AppState>) -> Result<Json<MaintenancePasses>, Failure> {
    Ok(Json(state.admission.passes().await?))
}

pub(crate) async fn add_pass(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(request): JsonBody<AddPass>,
) -> Result<(StatusCode, Json<MaintenancePass>), Failure> {
    Ok((
        StatusCode::CREATED,
        Json(state.admission.add_pass(&caller, request).await?),
    ))
}

pub(crate) async fn remove_pass(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    Id(id): Id,
) -> Result<StatusCode, Failure> {
    state.admission.remove_pass(&caller, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
