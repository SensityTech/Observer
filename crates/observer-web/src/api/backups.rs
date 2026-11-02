//! The game database's backups: the page, the settings and the destinations, a run now, the
//! history and the downloads. Which gate guards what is in the route table.

use axum::body::Body;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use observer_app::Caller;
use observer_domain::backups::{
    BackupDestination, BackupRun, BackupRunsPage, BackupSettingsChange, BackupsView, DestinationChange,
    DestinationDraft, DestinationTestRequest, DestinationTestResult, SourceTestRequest, SourceTestResult,
};
use serde::Deserialize;
use tokio_util::io::ReaderStream;

use crate::AppState;
use crate::error::Failure;
use crate::extract::{Id, JsonBody, QueryParams};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RunsQuery {
    before: Option<i64>,
    limit: Option<u32>,
}

pub(crate) async fn view(State(state): State<AppState>) -> Result<Json<BackupsView>, Failure> {
    Ok(Json(state.backups.view().await?))
}

pub(crate) async fn change_settings(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(change): JsonBody<BackupSettingsChange>,
) -> Result<Json<BackupsView>, Failure> {
    Ok(Json(state.backups.change_settings(&caller, change).await?))
}

/// Connects to the database as the stored settings or a draft say, and reads what it holds.
pub(crate) async fn test_source(
    State(state): State<AppState>,
    JsonBody(request): JsonBody<SourceTestRequest>,
) -> Result<Json<SourceTestResult>, Failure> {
    Ok(Json(state.backups.test_source(request).await?))
}

pub(crate) async fn create_destination(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(draft): JsonBody<DestinationDraft>,
) -> Result<(StatusCode, Json<BackupDestination>), Failure> {
    let destination = state.backups.create_destination(&caller, draft).await?;
    Ok((StatusCode::CREATED, Json(destination)))
}

pub(crate) async fn change_destination(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    Id(id): Id,
    JsonBody(change): JsonBody<DestinationChange>,
) -> Result<Json<BackupDestination>, Failure> {
    Ok(Json(state.backups.change_destination(&caller, id, change).await?))
}

/// Deletes a destination and forgets its copies; the files stay where they are.
pub(crate) async fn delete_destination(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    Id(id): Id,
) -> Result<StatusCode, Failure> {
    state.backups.delete_destination(&caller, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Tries a destination as drafted: what a run needs there, and the key an SFTP host shows.
pub(crate) async fn test_destination(
    State(state): State<AppState>,
    JsonBody(request): JsonBody<DestinationTestRequest>,
) -> Result<Json<DestinationTestResult>, Failure> {
    Ok(Json(state.backups.test_destination(request).await?))
}

pub(crate) async fn run_now(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
) -> Result<(StatusCode, Json<BackupRun>), Failure> {
    let run = state.backups.run_now(&caller).await?;
    Ok((StatusCode::ACCEPTED, Json(run)))
}

pub(crate) async fn cancel(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
) -> Result<StatusCode, Failure> {
    state.backups.cancel(&caller).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn runs(
    State(state): State<AppState>,
    QueryParams(query): QueryParams<RunsQuery>,
) -> Result<Json<BackupRunsPage>, Failure> {
    Ok(Json(state.backups.runs(query.before, query.limit).await?))
}

pub(crate) async fn run(State(state): State<AppState>, Id(id): Id) -> Result<Json<BackupRun>, Failure> {
    Ok(Json(state.backups.run(id).await?))
}

/// Deletes every copy of a backup, wherever it is.
pub(crate) async fn delete_run(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    Id(id): Id,
) -> Result<Json<BackupRun>, Failure> {
    Ok(Json(state.backups.delete_run(&caller, id).await?))
}

/// Streams one copy of a backup, from wherever it is stored.
pub(crate) async fn download(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    Id(id): Id,
) -> Result<Response, Failure> {
    let download = state.backups.download(&caller, id).await?;
    // A name Observer made: the database's name kept to letters, digits, `-` and `_`.
    let mut headers = vec![
        (header::CONTENT_TYPE, "application/gzip".to_owned()),
        (
            header::CONTENT_DISPOSITION,
            format!(
                "attachment; filename=\"{}\"",
                download.file_name.replace(['"', '\\'], "_")
            ),
        ),
    ];
    if let Some(bytes) = download.bytes {
        headers.push((header::CONTENT_LENGTH, bytes.to_string()));
    }
    let body = Body::from_stream(ReaderStream::new(download.reader));
    let mut response = body.into_response();
    for (name, value) in headers {
        if let Ok(value) = value.parse() {
            response.headers_mut().insert(name, value);
        }
    }
    Ok(response)
}
