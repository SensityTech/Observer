//! Observer's own updates: the newest release and the install under way, for the owner, who
//! installs one with the password again.

use axum::extract::State;
use axum::{Extension, Json};
use observer_app::Caller;
use observer_domain::updates::{InstallUpdate, UpdatesView};

use crate::AppState;
use crate::error::Failure;
use crate::extract::JsonBody;

pub(crate) async fn view(State(state): State<AppState>) -> Result<Json<UpdatesView>, Failure> {
    Ok(Json(state.updates()?.view()))
}

pub(crate) async fn check(State(state): State<AppState>) -> Result<Json<UpdatesView>, Failure> {
    Ok(Json(state.updates()?.check().await))
}

pub(crate) async fn install(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(request): JsonBody<InstallUpdate>,
) -> Result<Json<UpdatesView>, Failure> {
    Ok(Json(state.updates()?.install(&caller, request).await?))
}

pub(crate) async fn cancel(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
) -> Result<Json<UpdatesView>, Failure> {
    Ok(Json(state.updates()?.cancel(&caller).await?))
}
