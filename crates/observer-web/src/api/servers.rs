use axum::extract::State;
use axum::{Extension, Json};
use observer_app::{Caller, Servers};
use observer_domain::launch::LaunchSettings;

use crate::AppState;
use crate::error::Failure;
use crate::extract::JsonBody;

/// Null until the owner first saves them.
pub(crate) async fn launch_settings(
    State(state): State<AppState>,
) -> Result<Json<Option<LaunchSettings>>, Failure> {
    Ok(Json(state.servers.launch_settings(Servers::DEFAULT).await?))
}

pub(crate) async fn change_launch_settings(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(settings): JsonBody<LaunchSettings>,
) -> Result<Json<LaunchSettings>, Failure> {
    let saved = state
        .servers
        .change_launch_settings(&caller, Servers::DEFAULT, settings)
        .await?;
    Ok(Json(saved))
}
