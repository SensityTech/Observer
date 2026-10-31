use axum::extract::State;
use axum::http::StatusCode;
use axum::{Extension, Json};
use observer_app::{Caller, ComparedTo, DiffOptions, Error};
use observer_domain::changes::{
    ChangesSettings, ChangesSettingsView, Comparison, FileDiff, FileText, Purged, ResourceDetail,
    ResourceVersion, StartList,
};
use observer_domain::error::{ApiError, ErrorCode};
use serde::Deserialize;

use crate::AppState;
use crate::error::Failure;
use crate::extract::QueryParams;

/// The most unchanged lines shown around a change, once "Show more lines" asked for them.
const MAX_CONTEXT: usize = 10_000;
const MAX_PATH: usize = 4096;

#[derive(Deserialize)]
pub(crate) struct StartsQuery {
    before: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct PairQuery {
    from: Option<String>,
    to: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct ResourceQuery {
    from: Option<String>,
    to: Option<String>,
    path: String,
}

#[derive(Deserialize)]
pub(crate) struct HistoryQuery {
    name: String,
}

#[derive(Deserialize)]
pub(crate) struct DiffQuery {
    from: Option<String>,
    to: Option<String>,
    path: String,
    page: Option<String>,
    context: Option<String>,
    whitespace: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct FileQuery {
    start: String,
    path: String,
    page: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct PurgeQuery {
    path: String,
}

fn invalid(name: &str) -> Failure {
    Error::from(ApiError::new(ErrorCode::InvalidRequest).with("field", name)).into()
}

fn id(name: &str, value: Option<&str>) -> Result<Option<i64>, Failure> {
    value
        .map(|value| value.parse().map_err(|_| invalid(name)))
        .transpose()
}

fn number(name: &str, value: Option<&str>, default: usize, max: usize) -> Result<usize, Failure> {
    match value {
        None => Ok(default),
        Some(value) => value
            .parse::<usize>()
            .ok()
            .filter(|parsed| *parsed <= max)
            .ok_or_else(|| invalid(name)),
    }
}

/// `to` is a start's id or `disk`, the latest check of the disk.
fn pair(from: Option<&str>, to: Option<&str>) -> Result<(Option<i64>, Option<ComparedTo>), Failure> {
    let to = match to {
        None => None,
        Some("disk") => Some(ComparedTo::Disk),
        Some(id) => Some(ComparedTo::Start(id.parse().map_err(|_| invalid("to"))?)),
    };
    Ok((id("from", from)?, to))
}

/// A path is only a key into the stored listings, compared exactly and never joined to a folder;
/// one that climbs or holds what no listing does is refused before any lookup.
fn path(path: String) -> Result<String, Failure> {
    let climbs = path.split(['/', '\\']).any(|part| part == "..");
    if path.is_empty() || path.len() > MAX_PATH || climbs || path.chars().any(char::is_control) {
        return Err(invalid("path"));
    }
    Ok(path)
}

pub(crate) async fn starts(
    State(state): State<AppState>,
    QueryParams(query): QueryParams<StartsQuery>,
) -> Result<Json<StartList>, Failure> {
    let before = id("before", query.before.as_deref())?;
    Ok(Json(state.changes.starts(before).await?))
}

pub(crate) async fn compare(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    QueryParams(query): QueryParams<PairQuery>,
) -> Result<Json<Comparison>, Failure> {
    let (from, to) = pair(query.from.as_deref(), query.to.as_deref())?;
    Ok(Json(state.changes.compare(&caller, from, to).await?))
}

pub(crate) async fn resource(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    QueryParams(query): QueryParams<ResourceQuery>,
) -> Result<Json<ResourceDetail>, Failure> {
    let (from, to) = pair(query.from.as_deref(), query.to.as_deref())?;
    let path = path(query.path)?;
    Ok(Json(state.changes.resource(&caller, from, to, path).await?))
}

pub(crate) async fn history(
    State(state): State<AppState>,
    QueryParams(query): QueryParams<HistoryQuery>,
) -> Result<Json<Vec<ResourceVersion>>, Failure> {
    if query.name.is_empty() || query.name.len() > 256 {
        return Err(invalid("name"));
    }
    Ok(Json(state.changes.history(query.name).await?))
}

/// Lists and hashes the disk now; the result comes as the latest check.
pub(crate) async fn check(State(state): State<AppState>) -> Result<StatusCode, Failure> {
    state.changes.check().await?;
    Ok(StatusCode::ACCEPTED)
}

pub(crate) async fn diff(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    QueryParams(query): QueryParams<DiffQuery>,
) -> Result<Json<FileDiff>, Failure> {
    let (from, to) = pair(query.from.as_deref(), query.to.as_deref())?;
    let options = DiffOptions {
        context: number("context", query.context.as_deref(), 3, MAX_CONTEXT)?,
        ignore_whitespace: match query.whitespace.as_deref() {
            None | Some("show") => false,
            Some("ignore") => true,
            Some(_) => return Err(invalid("whitespace")),
        },
        page: number("page", query.page.as_deref(), 0, usize::MAX)?,
    };
    let path = path(query.path)?;
    Ok(Json(state.changes.diff(&caller, from, to, path, options).await?))
}

pub(crate) async fn file(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    QueryParams(query): QueryParams<FileQuery>,
) -> Result<Json<FileText>, Failure> {
    let start = id("start", Some(&query.start))?.ok_or_else(|| invalid("start"))?;
    let page = number("page", query.page.as_deref(), 0, usize::MAX)?;
    let path = path(query.path)?;
    Ok(Json(state.changes.file(&caller, start, path, page).await?))
}

pub(crate) async fn settings(State(state): State<AppState>) -> Result<Json<ChangesSettingsView>, Failure> {
    Ok(Json(state.changes.settings_view().await?))
}

pub(crate) async fn change_settings(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    Json(changed): Json<ChangesSettings>,
) -> Result<Json<ChangesSettingsView>, Failure> {
    state.changes.change_settings(&caller, changed).await?;
    Ok(Json(state.changes.settings_view().await?))
}

pub(crate) async fn verify(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
) -> Result<StatusCode, Failure> {
    state.changes.verify(&caller).await?;
    Ok(StatusCode::ACCEPTED)
}

pub(crate) async fn purge(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    QueryParams(query): QueryParams<PurgeQuery>,
) -> Result<Json<Purged>, Failure> {
    let path = path(query.path)?;
    Ok(Json(state.changes.purge(&caller, path).await?))
}
