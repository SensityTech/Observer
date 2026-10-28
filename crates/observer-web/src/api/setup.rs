use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use observer_domain::accounts::{SetupRequest, SetupStatus};

use crate::AppState;
use crate::api::auth;
use crate::error::Failure;
use crate::extract::{JsonBody, RemoteClient};

pub(crate) async fn status(State(state): State<AppState>) -> Result<Json<SetupStatus>, Failure> {
    let required = state.accounts.setup_required().await?;
    Ok(Json(SetupStatus { required }))
}

/// Creates the owner and signs them in, exactly as a login would.
pub(crate) async fn complete(
    State(state): State<AppState>,
    RemoteClient(client): RemoteClient,
    JsonBody(request): JsonBody<SetupRequest>,
) -> Result<Response, Failure> {
    let signed_in = state
        .accounts
        .complete_setup(&request.token, &request.username, request.password, &client)
        .await?;
    Ok(auth::signed_in(&state, StatusCode::CREATED, signed_in))
}
