use axum::extract::State;
use axum::http::StatusCode;
use axum::{Extension, Json};
use observer_app::{Caller, Group};
use observer_domain::error::ErrorCode;
use observer_domain::files::{FileGroup, FileGroupRequest, UserGroupsRequest};

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
        .set_user_groups(&caller, user_id, request.groups)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

fn file_group(group: Group) -> FileGroup {
    FileGroup {
        id: group.row.id,
        name: group.row.name,
        description: group.row.description,
        rules: group.row.rules,
        reach: group.reach,
        version: group.row.version,
    }
}
