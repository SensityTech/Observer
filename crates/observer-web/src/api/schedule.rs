//! The restart schedule. Every route answers the whole status, which the `server.schedule` topic
//! also carries.

use axum::extract::State;
use axum::{Extension, Json};
use observer_app::Caller;
use observer_domain::schedule::{DelayRequest, OneOffRequest, ScheduleChange, ScheduleStatus};

use crate::AppState;
use crate::error::Failure;
use crate::extract::JsonBody;

pub(crate) async fn change(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(change): JsonBody<ScheduleChange>,
) -> Result<Json<ScheduleStatus>, Failure> {
    Ok(Json(state.schedule.change_settings(&caller, change).await?))
}

pub(crate) async fn skip(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
) -> Result<Json<ScheduleStatus>, Failure> {
    Ok(Json(state.schedule.skip(&caller).await?))
}

pub(crate) async fn restore(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
) -> Result<Json<ScheduleStatus>, Failure> {
    Ok(Json(state.schedule.restore(&caller).await?))
}

pub(crate) async fn delay(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(delay): JsonBody<DelayRequest>,
) -> Result<Json<ScheduleStatus>, Failure> {
    Ok(Json(state.schedule.delay(&caller, delay.minutes).await?))
}

pub(crate) async fn restart_once(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(request): JsonBody<OneOffRequest>,
) -> Result<Json<ScheduleStatus>, Failure> {
    Ok(Json(state.schedule.restart_once(&caller, request).await?))
}

pub(crate) async fn cancel_once(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
) -> Result<Json<ScheduleStatus>, Failure> {
    Ok(Json(state.schedule.cancel_once(&caller).await?))
}
