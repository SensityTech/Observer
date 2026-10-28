use axum::extract::State;
use axum::{Extension, Json};
use observer_app::{Caller, Servers};
use observer_domain::launch::{LaunchChange, StoredLaunch};

use crate::AppState;
use crate::error::Failure;
use crate::extract::JsonBody;

/// Null until the owner first saves them.
pub(crate) async fn launch_settings(
    State(state): State<AppState>,
) -> Result<Json<Option<StoredLaunch>>, Failure> {
    Ok(Json(state.servers.launch_settings(Servers::DEFAULT).await?))
}

pub(crate) async fn change_launch_settings(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(change): JsonBody<LaunchChange>,
) -> Result<Json<StoredLaunch>, Failure> {
    let saved = state
        .servers
        .change_launch_settings(&caller, Servers::DEFAULT, change.settings, change.version)
        .await?;
    Ok(Json(saved))
}
