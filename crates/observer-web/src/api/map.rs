//! The live map: its settings and image for the panel, the owner's image from a host path, and the
//! settings' changes. The positions come over the websocket.

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use observer_app::Caller;
use observer_domain::error::ErrorCode;
use observer_domain::map::{MapImage, MapImageRequest, MapSettings, MapView};
use tokio_util::io::ReaderStream;

use crate::AppState;
use crate::error::Failure;
use crate::extract::JsonBody;

pub(crate) async fn view(State(state): State<AppState>) -> Result<Json<MapView>, Failure> {
    Ok(Json(state.map()?.view().await?))
}

/// The image, which the panel asks for under its version: kept by the browser, for this account
/// alone, until a new one comes.
pub(crate) async fn image(State(state): State<AppState>) -> Result<Response, Failure> {
    let (path, kind) = state.map()?.image_file().await?;
    let file = tokio::fs::File::open(&path)
        .await
        .map_err(|_| Failure::from(ErrorCode::NoMapImage))?;
    let mut response = Body::from_stream(ReaderStream::new(file)).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(kind.content_type()),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=31536000, immutable"),
    );
    Ok(response)
}

pub(crate) async fn set_image(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(request): JsonBody<MapImageRequest>,
) -> Result<Json<MapImage>, Failure> {
    Ok(Json(state.map()?.set_image(&caller, &request.path).await?))
}

pub(crate) async fn remove_image(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
) -> Result<StatusCode, Failure> {
    state.map()?.remove_image(&caller).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn change_settings(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(settings): JsonBody<MapSettings>,
) -> Result<Json<MapSettings>, Failure> {
    let map = state.map()?;
    map.change_settings(&caller, settings).await?;
    Ok(Json(map.settings().await?))
}
