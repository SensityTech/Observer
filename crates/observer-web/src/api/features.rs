//! The switches of Observer's optional features. Reading them takes the settings permission;
//! turning one on or off is the owner's, with the password again: it opens or closes a port, or
//! starts or stops reading the host's folders.

use axum::extract::State;
use axum::{Extension, Json};
use observer_app::Caller;
use observer_domain::error::ErrorCode;
use observer_domain::features::{Feature, FeatureList, FeatureSwitch};

use crate::AppState;
use crate::error::Failure;
use crate::extract::{JsonBody, Name};

pub(crate) async fn list(State(state): State<AppState>) -> Json<FeatureList> {
    Json(state.features.view())
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
