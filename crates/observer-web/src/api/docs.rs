//! What the panel's Documentation page is built from. The contract holds no secret, so any
//! signed-in account reads it, as the About page; the service leaves the server's state out for
//! an account that does not see it.

use axum::extract::State;
use axum::{Extension, Json};
use observer_app::Caller;
use observer_domain::docs::IntegrationDocs;

use crate::AppState;
use crate::error::Failure;

pub(crate) async fn integration(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
) -> Result<Json<IntegrationDocs>, Failure> {
    Ok(Json(
        observer_app::integration_docs(&caller, &state.supervision, &state.servers).await?,
    ))
}
