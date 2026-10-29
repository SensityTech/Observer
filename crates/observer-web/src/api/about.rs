use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::Response;
use observer_domain::about::About;
use observer_domain::error::ErrorCode;

use crate::AppState;
use crate::error::Failure;
use crate::panel;

pub(crate) async fn about(State(state): State<AppState>) -> Json<About> {
    Json(About::of(state.build))
}

/// The list `cargo xtask web` and `cargo xtask dist` embed with the panel, behind the sign-in: it
/// names every crate and version Observer runs.
pub(crate) async fn licences(headers: HeaderMap) -> Result<Response, Failure> {
    panel::licences(&headers).ok_or_else(|| ErrorCode::NotFound.into())
}
