use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use observer_app::Caller;
use observer_domain::diagnostics::Crash;
use observer_domain::error::ErrorCode;
use tokio_util::io::ReaderStream;

use crate::AppState;
use crate::error::Failure;
use crate::extract::Id;

/// Streams one of a crash's files, by the name its record lists: never a path.
pub(crate) async fn download_file(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path((id, name)): Path<(String, String)>,
) -> Result<Response, Failure> {
    let id: i64 = id.parse().map_err(|_| ErrorCode::CrashNotFound)?;
    let (file, opened) = state.diagnostics.open_file(&caller, id, &name).await?;
    // A name FXServer or Observer gave the file: a GUID or `hang-<n>-<time>`, nothing to escape.
    let headers = [
        (header::CONTENT_TYPE, "application/octet-stream".to_owned()),
        (header::CONTENT_LENGTH, file.bytes.to_string()),
        (
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{}\"", file.name.replace(['"', '\\'], "_")),
        ),
    ];
    Ok((headers, Body::from_stream(ReaderStream::new(opened))).into_response())
}

/// Deletes a crash's files from the host; the record and its analysis stay.
pub(crate) async fn delete_files(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    Id(id): Id,
) -> Result<Json<Crash>, Failure> {
    Ok(Json(state.diagnostics.delete_files(&caller, id).await?))
}
