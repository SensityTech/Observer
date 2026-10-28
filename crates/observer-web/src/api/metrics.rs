use axum::Json;
use axum::extract::State;
use observer_app::Error;
use observer_domain::error::{ApiError, ErrorCode};
use observer_domain::metrics::{MetricsSeries, Range, Series};
use serde::Deserialize;
use serde_json::Value;

use crate::AppState;
use crate::error::Failure;
use crate::extract::QueryParams;

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
