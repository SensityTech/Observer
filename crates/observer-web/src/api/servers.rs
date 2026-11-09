use axum::body::Body;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use observer_app::{Caller, Servers};
use observer_domain::launch::{LaunchChange, StoredLaunch};
use observer_domain::server::{CommandAck, ConsoleCommand, LogFile};
use observer_domain::server_data::{
    CfgCheck, CfgReport, CfgText, CfgTextQuery, Discovery, FileListing, FilesQuery, FolderCheck,
    FolderListing, FolderQuery, FolderReport,
};
use tokio_util::io::ReaderStream;

use crate::AppState;
use crate::error::Failure;
use crate::extract::{JsonBody, Name};

/// The console route as the table declares it, which a refused `SERVER_CONTROL` is audited as.
const CONSOLE_ROUTE: &str = "/api/servers/default/console";

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

/// A refused folder is what the report says, not a failed request. A path never goes in a URL, so
/// the checks are posted; they change nothing.
pub(crate) async fn check_folder(
    State(state): State<AppState>,
    JsonBody(check): JsonBody<FolderCheck>,
) -> Result<Json<FolderReport>, Failure> {
    Ok(Json(state.supervision.check_folder(check).await?))
}

pub(crate) async fn check_cfg(
    State(state): State<AppState>,
    JsonBody(check): JsonBody<CfgCheck>,
) -> Result<Json<CfgReport>, Failure> {
    Ok(Json(state.supervision.check_cfg(check).await?))
}

pub(crate) async fn list_folders(
    State(state): State<AppState>,
    JsonBody(query): JsonBody<FolderQuery>,
) -> Result<Json<FolderListing>, Failure> {
    Ok(Json(state.supervision.list_folders(query).await?))
}

pub(crate) async fn list_files(
    State(state): State<AppState>,
    JsonBody(query): JsonBody<FilesQuery>,
) -> Result<Json<FileListing>, Failure> {
    Ok(Json(state.supervision.list_files(query).await?))
}

pub(crate) async fn cfg_text(
    State(state): State<AppState>,
    JsonBody(query): JsonBody<CfgTextQuery>,
) -> Result<Json<CfgText>, Failure> {
    Ok(Json(state.supervision.cfg_text(query).await?))
}

/// The search takes a few seconds: the panel asks for it once and when the owner asks again.
pub(crate) async fn discover(State(state): State<AppState>) -> Result<Json<Discovery>, Failure> {
    Ok(Json(state.supervision.discover().await?))
}

/// The setup's save checks the folder and its cfg again before it stores the settings.
pub(crate) async fn save_setup(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(change): JsonBody<LaunchChange>,
) -> Result<Json<StoredLaunch>, Failure> {
    Ok(Json(state.supervision.save_setup(&caller, change).await?))
}

/// Start, stop and restart answer once the supervisor took the request on; the `server.status`
/// topic follows what FXServer does next.
pub(crate) async fn start(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
) -> Result<StatusCode, Failure> {
    state.supervision.start_server(&caller).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn stop(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
) -> Result<StatusCode, Failure> {
    state.supervision.stop_server(&caller).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn restart(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
) -> Result<StatusCode, Failure> {
    state.supervision.restart_server(&caller).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn command(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(command): JsonBody<ConsoleCommand>,
) -> Result<Json<CommandAck>, Failure> {
    let ack = state
        .supervision
        .command(&caller, &command.line, "POST", CONSOLE_ROUTE)
        .await?;
    Ok(Json(ack))
}

pub(crate) async fn log_files(State(state): State<AppState>) -> Result<Json<Vec<LogFile>>, Failure> {
    Ok(Json(state.supervision.log_files().await?))
}

/// Streams one file of the list, read as the client takes it.
pub(crate) async fn download_log(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    Name(name): Name,
) -> Result<Response, Failure> {
    let (file, opened) = state.supervision.open_log(&caller, &name).await?;
    // A name Observer gave the file, `console-<date>[.<n>].log`: nothing in it to escape.
    let headers = [
        (header::CONTENT_TYPE, "text/plain; charset=utf-8".to_owned()),
        (
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{}\"", file.name),
        ),
    ];
    Ok((headers, Body::from_stream(ReaderStream::new(opened))).into_response())
}
