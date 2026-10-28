use axum::extract::State;
use axum::http::StatusCode;
use axum::{Extension, Json};
use observer_app::Caller;
use observer_domain::accounts::{AccountSession, PasswordChangeRequest};

use crate::AppState;
use crate::error::Failure;
use crate::extract::{Id, JsonBody};

pub(crate) async fn change_password(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(request): JsonBody<PasswordChangeRequest>,
) -> Result<StatusCode, Failure> {
    state
        .accounts
        .change_password(&caller, request.current, request.next)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn sessions(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
) -> Result<Json<Vec<AccountSession>>, Failure> {
    Ok(Json(state.accounts.sessions(&caller).await?))
}

pub(crate) async fn revoke_session(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    Id(id): Id,
) -> Result<StatusCode, Failure> {
    state.accounts.revoke_session(&caller, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
