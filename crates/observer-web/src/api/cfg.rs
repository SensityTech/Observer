//! The launch's cfg, edited from the panel: the owner's to read, with its secrets as marks, and
//! theirs to write once the password is given again.

use axum::extract::{Path, State};
use axum::{Extension, Json};
use observer_app::Caller;
use observer_domain::cfg::{CfgDocument, CfgDraft, CfgRestore, CfgReview, CfgSave, CfgVersion};
use observer_domain::changes::TextDiff;

use crate::AppState;
use crate::error::Failure;
use crate::extract::JsonBody;

pub(crate) async fn document(State(state): State<AppState>) -> Result<Json<CfgDocument>, Failure> {
    Ok(Json(state.supervision.cfg_document().await?))
}

/// What the text sets and starts and what the scan finds, as it is typed: nothing is written.
pub(crate) async fn check(
    State(state): State<AppState>,
    JsonBody(draft): JsonBody<CfgDraft>,
) -> Result<Json<CfgReview>, Failure> {
    Ok(Json(state.supervision.check_cfg_draft(draft).await?))
}

/// The lines a save of the text would change: nothing is written.
pub(crate) async fn diff(
    State(state): State<AppState>,
    JsonBody(draft): JsonBody<CfgDraft>,
) -> Result<Json<TextDiff>, Failure> {
    Ok(Json(state.supervision.diff_cfg_draft(draft).await?))
}

pub(crate) async fn save(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(save): JsonBody<CfgSave>,
) -> Result<Json<CfgDocument>, Failure> {
    Ok(Json(state.supervision.save_cfg(&caller, save).await?))
}

pub(crate) async fn versions(State(state): State<AppState>) -> Result<Json<Vec<CfgVersion>>, Failure> {
    Ok(Json(state.supervision.cfg_versions().await?))
}

/// The lines putting the version back would change.
pub(crate) async fn version_diff(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<TextDiff>, Failure> {
    Ok(Json(state.supervision.cfg_version_diff(id).await?))
}

pub(crate) async fn restore(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<i64>,
    JsonBody(restore): JsonBody<CfgRestore>,
) -> Result<Json<CfgDocument>, Failure> {
    Ok(Json(state.supervision.restore_cfg(&caller, id, restore).await?))
}
