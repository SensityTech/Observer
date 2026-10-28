use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use observer_app::{Caller, SignedIn};
use observer_domain::accounts::{LoginRequest, LoginResponse, Me, ReauthRequest};

use crate::AppState;
use crate::error::Failure;
use crate::extract::{JsonBody, RemoteClient};
use crate::gate;

pub(crate) async fn login(
    State(state): State<AppState>,
    RemoteClient(client): RemoteClient,
    JsonBody(request): JsonBody<LoginRequest>,
) -> Result<Response, Failure> {
    let signed_in = state
        .accounts
        .login(&request.username, request.password, &client)
        .await?;
    Ok(self::signed_in(&state, StatusCode::OK, signed_in))
}

/// The cookie takes the session's token; the body hands the panel its key, once.
pub(crate) fn signed_in(state: &AppState, status: StatusCode, signed_in: SignedIn) -> Response {
    let cookie = gate::session_cookie(&state.cookie, &signed_in.token, signed_in.lifetime.as_secs());
    let body = LoginResponse {
        session_key: signed_in.key,
        me: signed_in.me,
    };
    (status, [(header::SET_COOKIE, cookie)], Json(body)).into_response()
}

pub(crate) async fn logout(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
) -> Result<Response, Failure> {
    state.accounts.logout(&caller).await?;
    let cookie = gate::expired_cookie(&state.cookie);
    Ok((StatusCode::NO_CONTENT, [(header::SET_COOKIE, cookie)]).into_response())
}

pub(crate) async fn me(Extension(caller): Extension<Caller>) -> Json<Me> {
    Json(caller.me())
}

pub(crate) async fn reauth(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(request): JsonBody<ReauthRequest>,
) -> Result<StatusCode, Failure> {
    state.accounts.reauthenticate(&caller, request.password).await?;
    Ok(StatusCode::NO_CONTENT)
}
