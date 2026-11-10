//! The switches of Observer's optional features, and the address SFTP listens on. Reading them
//! takes the settings permission; turning a feature on or off and saying where SFTP listens are
//! the owner's, with the password again: they open or close a port, or start or stop reading the
//! host's folders.

use std::net::SocketAddr;

use axum::extract::State;
use axum::{Extension, Json};
use observer_app::Caller;
use observer_domain::error::{ApiError, ErrorCode};
use observer_domain::features::{Feature, FeatureList, FeatureSwitch, SftpListen, SftpListenChange};

use crate::AppState;
use crate::error::Failure;
use crate::extract::{JsonBody, Name};

pub(crate) async fn list(State(state): State<AppState>) -> Json<FeatureList> {
    Json(state.features.view().await)
}

pub(crate) async fn switch(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    Name(name): Name,
    JsonBody(switch): JsonBody<FeatureSwitch>,
) -> Result<Json<FeatureList>, Failure> {
    let feature = Feature::from_name(&name).ok_or(ErrorCode::NotFound)?;
    Ok(Json(state.features.set(&caller, feature, switch.enabled).await?))
}

pub(crate) async fn sftp_listen(State(state): State<AppState>) -> Json<SftpListen> {
    Json(state.features.sftp())
}

pub(crate) async fn set_sftp_listen(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(change): JsonBody<SftpListenChange>,
) -> Result<Json<SftpListen>, Failure> {
    let listen = match change.listen {
        Some(listen) => Some(listen.trim().parse::<SocketAddr>().map_err(|_| {
            observer_app::Error::from(ApiError::new(ErrorCode::InvalidSftpListen).with("reason", "address"))
        })?),
        None => None,
    };
    Ok(Json(state.features.set_sftp_listen(&caller, listen).await?))
}
