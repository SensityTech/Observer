use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use observer_domain::api::{Health, HealthStatus};

use crate::AppState;

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/health", get(health))
        // Without this, unknown API paths would fall through to the panel's index page.
        .fallback(|| async { StatusCode::NOT_FOUND })
}

async fn health(State(state): State<AppState>) -> Json<Health> {
    Json(Health {
        status: HealthStatus::Ok,
        version: state.version.to_owned(),
    })
}
