//! What the staff does to the players connected now: a kick, a kick of everyone, an announcement,
//! a message to one player, and the kick message. A player is addressed by their server id.

use axum::extract::State;
use axum::http::StatusCode;
use axum::{Extension, Json};
use observer_app::Caller;
use observer_domain::moderation::{
    AnnouncementRequest, DirectMessageRequest, KickAllRequest, KickRequest, KickSettings, KickedAll,
};

use crate::AppState;
use crate::error::Failure;
use crate::extract::JsonBody;

pub(crate) async fn kick(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(request): JsonBody<KickRequest>,
) -> Result<StatusCode, Failure> {
    state.moderation.kick(&caller, request).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn kick_all(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(request): JsonBody<KickAllRequest>,
) -> Result<Json<KickedAll>, Failure> {
    Ok(Json(state.moderation.kick_all(&caller, request).await?))
}

pub(crate) async fn announce(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(request): JsonBody<AnnouncementRequest>,
) -> Result<StatusCode, Failure> {
    state.moderation.announce(&caller, request).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn message(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(request): JsonBody<DirectMessageRequest>,
) -> Result<StatusCode, Failure> {
    state.moderation.message(&caller, request).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn settings(State(state): State<AppState>) -> Result<Json<KickSettings>, Failure> {
    Ok(Json(state.moderation.settings().await?))
}

pub(crate) async fn change_settings(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(settings): JsonBody<KickSettings>,
) -> Result<Json<KickSettings>, Failure> {
    state.moderation.change_settings(&caller, settings).await?;
    Ok(Json(state.moderation.settings().await?))
}
