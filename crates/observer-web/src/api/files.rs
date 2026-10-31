use axum::extract::State;
use axum::http::StatusCode;
use axum::{Extension, Json};
use observer_app::{Caller, file_group};
use observer_domain::error::ErrorCode;
use observer_domain::files::{
    AccessKey, AddKeyRequest, CreateServiceRequest, CreatedServiceAccess, FileAccesses, FileGroup,
    FileGroupRequest, MyFileAccess, UserGroupsRequest,
};

use crate::AppState;
use crate::error::Failure;
use crate::extract::{Id, JsonBody};

pub(crate) async fn groups(State(state): State<AppState>) -> Result<Json<Vec<FileGroup>>, Failure> {
    let groups = state.files.groups().await?;
    Ok(Json(groups.into_iter().map(file_group).collect()))
}

pub(crate) async fn create_group(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(request): JsonBody<FileGroupRequest>,
) -> Result<(StatusCode, Json<FileGroup>), Failure> {
    let group = state
        .files
        .create_group(&caller, request.name, request.description, request.rules)
        .await?;
    Ok((StatusCode::CREATED, Json(file_group(group))))
}

pub(crate) async fn update_group(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    Id(id): Id,
    JsonBody(request): JsonBody<FileGroupRequest>,
) -> Result<Json<FileGroup>, Failure> {
    let version = request.version.ok_or(ErrorCode::InvalidRequest)?;
    let group = state
        .files
        .update_group(
            &caller,
            id,
            version,
            request.name,
            request.description,
            request.rules,
        )
        .await?;
    Ok(Json(file_group(group)))
}

pub(crate) async fn delete_group(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    Id(id): Id,
) -> Result<StatusCode, Failure> {
    state.files.delete_group(&caller, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn set_user_groups(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    Id(user_id): Id,
    JsonBody(request): JsonBody<UserGroupsRequest>,
) -> Result<StatusCode, Failure> {
    state
        .files
        .set_user_groups(&caller, user_id, request.groups, request.previous)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn my_access(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
) -> Result<Json<MyFileAccess>, Failure> {
    Ok(Json(state.files.my_access(&caller).await?))
}

pub(crate) async fn add_key(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(request): JsonBody<AddKeyRequest>,
) -> Result<(StatusCode, Json<AccessKey>), Failure> {
    let key = state.files.add_own_key(&caller, &request.public_key).await?;
    Ok((StatusCode::CREATED, Json(key)))
}

pub(crate) async fn remove_key(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    Id(id): Id,
) -> Result<StatusCode, Failure> {
    state.files.remove_own_key(&caller, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn accesses(State(state): State<AppState>) -> Result<Json<FileAccesses>, Failure> {
    Ok(Json(state.files.accesses().await?))
}

pub(crate) async fn create_service(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(request): JsonBody<CreateServiceRequest>,
) -> Result<(StatusCode, Json<CreatedServiceAccess>), Failure> {
    let created = state.files.create_service(&caller, request).await?;
    Ok((StatusCode::CREATED, Json(created)))
}

pub(crate) async fn delete_service(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    Id(id): Id,
) -> Result<StatusCode, Failure> {
    state.files.delete_service(&caller, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
