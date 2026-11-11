use axum::extract::State;
use axum::{Extension, Json};
use observer_app::Caller;
use observer_domain::resources::{ResourceAck, ResourceOrder};

use crate::AppState;
use crate::error::Failure;
use crate::extract::JsonBody;

/// The orders answer once FXServer's console has the line; the `server.resources` topic follows
/// what FXServer does of it.
pub(crate) async fn start(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(order): JsonBody<ResourceOrder>,
) -> Result<Json<ResourceAck>, Failure> {
    Ok(Json(
        state.supervision.start_resource(&caller, &order.name).await?,
    ))
}

pub(crate) async fn stop(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(order): JsonBody<ResourceOrder>,
) -> Result<Json<ResourceAck>, Failure> {
    Ok(Json(state.supervision.stop_resource(&caller, &order.name).await?))
}

pub(crate) async fn restart(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(order): JsonBody<ResourceOrder>,
) -> Result<Json<ResourceAck>, Failure> {
    let ack = state
        .supervision
        .restart_resource(&caller, &order.name, order.dependants)
        .await?;
    Ok(Json(ack))
}

pub(crate) async fn rescan(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
) -> Result<Json<ResourceAck>, Failure> {
    Ok(Json(state.supervision.rescan_resources(&caller).await?))
}
