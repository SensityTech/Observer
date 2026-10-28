use axum::extract::State;
use axum::http::StatusCode;
use axum::{Extension, Json};
use observer_app::Caller;
use observer_domain::accounts::{CreateUserRequest, CreatedUser, TemporaryPassword, UpdateUserRequest, User};
use observer_domain::permissions::{CATALOG, PermissionInfo};

use crate::AppState;
use crate::error::Failure;
use crate::extract::{Id, JsonBody};

pub(crate) async fn catalog() -> Json<&'static [PermissionInfo]> {
    Json(CATALOG)
}

pub(crate) async fn list(State(state): State<AppState>) -> Result<Json<Vec<User>>, Failure> {
    Ok(Json(state.accounts.users().await?))
}

pub(crate) async fn create(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(request): JsonBody<CreateUserRequest>,
) -> Result<(StatusCode, Json<CreatedUser>), Failure> {
    let created = state
        .accounts
        .create_user(&caller, &request.username, request.permissions)
        .await?;
    Ok((StatusCode::CREATED, Json(created)))
}

pub(crate) async fn update(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    Id(id): Id,
    JsonBody(request): JsonBody<UpdateUserRequest>,
) -> Result<Json<User>, Failure> {
    let user = state
        .accounts
        .update_user(&caller, id, request.permissions, request.version)
        .await?;
    Ok(Json(user))
}

pub(crate) async fn delete(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    Id(id): Id,
) -> Result<StatusCode, Failure> {
    state.accounts.delete_user(&caller, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn reset_password(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    Id(id): Id,
) -> Result<Json<TemporaryPassword>, Failure> {
    let temporary_password = state.accounts.reset_password(&caller, id).await?;
    Ok(Json(TemporaryPassword { temporary_password }))
}

pub(crate) async fn revoke_sessions(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    Id(id): Id,
) -> Result<StatusCode, Failure> {
    state.accounts.revoke_sessions(&caller, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
