use axum::extract::State;
use axum::http::StatusCode;
use axum::{Extension, Json};
use jiff::tz::TimeZone;
use observer_app::{Caller, Error};
use observer_domain::drops::DropsRange;
use observer_domain::error::{ApiError, ErrorCode};
use observer_domain::metrics::{MetricsSeries, Range, RunGrowth, Series, ThreadShares};
use observer_domain::statistics::{
    FrameDistribution, LinkDismissal, MAX_TIMELINE_SERIES, MOST_POINTS, PlayerHours, RunMemory, Timeline,
    TimelineEvents,
};
use serde::Deserialize;
use serde_json::Value;

use crate::AppState;
use crate::error::Failure;
use crate::extract::{JsonBody, QueryParams};

#[derive(Deserialize)]
pub(crate) struct SeriesQuery {
    range: Option<String>,
    series: Option<String>,
}

/// `?range=24h&series=players,slots`: one of the fixed ranges, and one to sixteen known series.
pub(crate) async fn series(
    State(state): State<AppState>,
    QueryParams(query): QueryParams<SeriesQuery>,
) -> Result<Json<MetricsSeries>, Failure> {
    let refused = |name: &str, given: Option<String>| {
        Failure::from(Error::from(
            ApiError::new(ErrorCode::InvalidRequest).with(name, given.map_or(Value::Null, Value::from)),
        ))
    };
    let range = query
        .range
        .as_deref()
        .and_then(Range::parse)
        .ok_or_else(|| refused("range", query.range.clone()))?;
    let series = Series::parse_list(query.series.as_deref().unwrap_or_default())
        .map_err(|_| refused("series", query.series.clone()))?;
    Ok(Json(state.metrics.series(range, &series).await?))
}

pub(crate) async fn threads(State(state): State<AppState>) -> Result<Json<ThreadShares>, Failure> {
    Ok(Json(state.metrics.threads().await?))
}

/// `null` while no run is under way.
pub(crate) async fn growth(State(state): State<AppState>) -> Result<Json<Option<RunGrowth>>, Failure> {
    Ok(Json(state.metrics.growth().await?))
}

fn refused(name: &str, given: Option<String>) -> Failure {
    Failure::from(Error::from(
        ApiError::new(ErrorCode::InvalidRequest).with(name, given.map_or(Value::Null, Value::from)),
    ))
}

/// A Unix second, or the request is refused naming the parameter.
fn seconds(name: &str, given: Option<&String>) -> Result<i64, Failure> {
    given
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| refused(name, given.cloned()))
}

#[derive(Deserialize)]
pub(crate) struct TimelineQuery {
    from: Option<String>,
    to: Option<String>,
    series: Option<String>,
    points: Option<String>,
}

/// `?from=&to=&series=players,mainTick&points=720`: a window in Unix seconds, one to
/// [`MAX_TIMELINE_SERIES`] known series, and at most that many buckets.
pub(crate) async fn timeline(
    State(state): State<AppState>,
    QueryParams(query): QueryParams<TimelineQuery>,
) -> Result<Json<Timeline>, Failure> {
    let from = seconds("from", query.from.as_ref())?;
    let to = seconds("to", query.to.as_ref())?;
    let points = match &query.points {
        None => MOST_POINTS,
        Some(points) => points
            .parse()
            .map_err(|_| refused("points", Some(points.clone())))?,
    };
    let series = Series::parse_list_of(query.series.as_deref().unwrap_or_default(), MAX_TIMELINE_SERIES)
        .map_err(|_| refused("series", query.series.clone()))?;
    Ok(Json(state.metrics.timeline(from, to, points, &series).await?))
}

#[derive(Deserialize)]
pub(crate) struct WindowQuery {
    from: Option<String>,
    to: Option<String>,
}

/// `?from=&to=`: what happened over the window, each part behind its own permission.
pub(crate) async fn events(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    QueryParams(query): QueryParams<WindowQuery>,
) -> Result<Json<TimelineEvents>, Failure> {
    let from = seconds("from", query.from.as_ref())?;
    let to = seconds("to", query.to.as_ref())?;
    Ok(Json(state.statistics.events(&caller, from, to).await?))
}

/// Says a link between a crash wave and a change is not related, or takes that back.
pub(crate) async fn link(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(dismissal): JsonBody<LinkDismissal>,
) -> Result<StatusCode, Failure> {
    state.statistics.dismiss(&caller, dismissal).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PeriodQuery {
    #[serde(default = "week")]
    range: DropsRange,
}

fn week() -> DropsRange {
    DropsRange::Week
}

pub(crate) async fn frames(
    State(state): State<AppState>,
    QueryParams(query): QueryParams<PeriodQuery>,
) -> Result<Json<FrameDistribution>, Failure> {
    Ok(Json(state.metrics.frames(query.range).await?))
}

/// Days and hours in the schedule's time zone, as the drops page has them.
pub(crate) async fn hours(State(state): State<AppState>) -> Result<Json<PlayerHours>, Failure> {
    let zone = TimeZone::get(&state.schedule.status().borrow().zone).unwrap_or(TimeZone::UTC);
    Ok(Json(state.metrics.hours(zone).await?))
}

pub(crate) async fn runs(
    State(state): State<AppState>,
    QueryParams(query): QueryParams<PeriodQuery>,
) -> Result<Json<RunMemory>, Failure> {
    Ok(Json(state.metrics.runs(query.range).await?))
}
