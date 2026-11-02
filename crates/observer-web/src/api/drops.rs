//! Players' drops: the statistics, which name nobody, the drop log and its export behind
//! `PLAYERS_VIEW`, and the owner's retention.

use axum::extract::State;
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use jiff::Timestamp;
use jiff::civil::Date;
use observer_app::{Caller, DayRates, LogQuery};
use observer_domain::drops::{
    ChartCategory, CrashDrops, DropCategory, DropChanges, DropKind, DropLog, DropLogRow, DropSeries,
    DropUptime, DropsRange, DropsSettings, DropsSummary, ResourceTable, SignatureTable,
};
use serde::Deserialize;

use crate::AppState;
use crate::csv;
use crate::error::Failure;
use crate::extract::{JsonBody, QueryParams};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RangeQuery {
    #[serde(default = "week")]
    range: DropsRange,
}

fn week() -> DropsRange {
    DropsRange::Week
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SeriesQuery {
    #[serde(default = "week")]
    range: DropsRange,
    #[serde(default = "unexpected")]
    category: ChartCategory,
}

fn unexpected() -> ChartCategory {
    ChartCategory::Unexpected
}

pub(crate) async fn summary(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    QueryParams(query): QueryParams<RangeQuery>,
) -> Result<Json<DropsSummary>, Failure> {
    Ok(Json(
        state.drops()?.summary(query.range, caller.permissions).await?,
    ))
}

pub(crate) async fn series(
    State(state): State<AppState>,
    QueryParams(query): QueryParams<SeriesQuery>,
) -> Result<Json<DropSeries>, Failure> {
    Ok(Json(state.drops()?.series(query.range, query.category).await?))
}

pub(crate) async fn signatures(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    QueryParams(query): QueryParams<RangeQuery>,
) -> Result<Json<SignatureTable>, Failure> {
    Ok(Json(
        state.drops()?.signatures(query.range, caller.permissions).await?,
    ))
}

pub(crate) async fn resources(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    QueryParams(query): QueryParams<RangeQuery>,
) -> Result<Json<ResourceTable>, Failure> {
    Ok(Json(
        state.drops()?.resources(query.range, caller.permissions).await?,
    ))
}

pub(crate) async fn changes(
    State(state): State<AppState>,
    QueryParams(query): QueryParams<RangeQuery>,
) -> Result<Json<DropChanges>, Failure> {
    Ok(Json(state.drops()?.changes(query.range).await?))
}

pub(crate) async fn uptime(
    State(state): State<AppState>,
    QueryParams(query): QueryParams<RangeQuery>,
) -> Result<Json<DropUptime>, Failure> {
    Ok(Json(state.drops()?.uptime(query.range).await?))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct LogParams {
    range: Option<DropsRange>,
    category: Option<DropCategory>,
    kind: Option<DropKind>,
    resource: Option<String>,
    signature: Option<i64>,
    run: Option<i64>,
    player: Option<String>,
    profile: Option<i64>,
    group: Option<i64>,
    before: Option<String>,
}

impl From<LogParams> for LogQuery {
    fn from(params: LogParams) -> Self {
        Self {
            range: params.range,
            category: params.category,
            kind: params.kind,
            resource: params.resource.filter(|resource| !resource.is_empty()),
            signature: params.signature,
            run: params.run,
            player: params.player.filter(|player| !player.trim().is_empty()),
            profile: params.profile,
            group: params.group,
            before: params.before,
        }
    }
}

pub(crate) async fn log(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    QueryParams(params): QueryParams<LogParams>,
) -> Result<Json<DropLog>, Failure> {
    let query = LogQuery::from(params);
    Ok(Json(state.drops()?.log(&query, caller.permissions).await?))
}

/// The filtered log as CSV, audited before it is sent.
pub(crate) async fn export_log(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    QueryParams(params): QueryParams<LogParams>,
) -> Result<Response, Failure> {
    let mut query = LogQuery::from(params);
    query.before = None;
    let rows = state
        .drops()?
        .export_log(&caller, &query, caller.permissions)
        .await?;
    Ok(attachment("observer-player-drops.csv", log_csv(&rows)))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RatesQuery {
    from: Date,
    to: Date,
}

/// Daily drops and play per category, audited before it is sent.
pub(crate) async fn export_rates(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    QueryParams(query): QueryParams<RatesQuery>,
) -> Result<Response, Failure> {
    let days = state.drops()?.export_rates(&caller, query.from, query.to).await?;
    Ok(attachment("observer-drop-rates.csv", rates_csv(&days)))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CrashQuery {
    started: Timestamp,
    generation: u32,
}

pub(crate) async fn crash(
    State(state): State<AppState>,
    QueryParams(query): QueryParams<CrashQuery>,
) -> Result<Json<Option<CrashDrops>>, Failure> {
    Ok(Json(state.drops()?.crash(query.started, query.generation).await?))
}

pub(crate) async fn settings(State(state): State<AppState>) -> Result<Json<DropsSettings>, Failure> {
    Ok(Json(state.drops()?.settings().await?))
}

pub(crate) async fn change_settings(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    JsonBody(settings): JsonBody<DropsSettings>,
) -> Result<Json<DropsSettings>, Failure> {
    state.drops()?.change_settings(&caller, settings).await?;
    Ok(Json(state.drops()?.settings().await?))
}

fn attachment(name: &str, body: String) -> Response {
    let disposition = format!("attachment; filename=\"{name}\"");
    (
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_owned()),
            (header::CONTENT_DISPOSITION, disposition),
        ],
        body,
    )
        .into_response()
}

const LOG_HEADER: &str = "at,player_id,license,name,category,kind,resource,signature,reason,length_ms,loading,run,generation,closed_by_observer\r\n";

/// Names are chosen by players and reasons written by resources: every cell goes through the
/// spreadsheet guard.
fn log_csv(rows: &[DropLogRow]) -> String {
    let mut text = LOG_HEADER.to_owned();
    for row in rows {
        let player = row.player.as_ref();
        text.push_str(&csv::line(&[
            row.at.to_string(),
            player.map(|player| player.id.to_string()).unwrap_or_default(),
            player.map(|player| player.license.clone()).unwrap_or_default(),
            player.and_then(|player| player.name.clone()).unwrap_or_default(),
            row.category.as_str().to_owned(),
            row.kind.map(DropKind::as_str).unwrap_or_default().to_owned(),
            row.resource.clone().unwrap_or_default(),
            row.signature.clone().unwrap_or_default(),
            row.reason.clone().unwrap_or_default(),
            row.length_ms.map(|ms| ms.to_string()).unwrap_or_default(),
            row.loading.map(|loading| loading.to_string()).unwrap_or_default(),
            row.run.to_string(),
            row.generation.to_string(),
            row.synthesized.to_string(),
        ]));
    }
    text
}

#[expect(
    clippy::cast_precision_loss,
    reason = "milliseconds of play and counts far below 2^52"
)]
fn rates_csv(days: &[DayRates]) -> String {
    let mut header = vec!["day".to_owned(), "played_hours".to_owned(), "sessions".to_owned()];
    for category in DropCategory::ALL {
        header.push(format!("{}_drops", category.as_str()));
        header.push(format!("{}_per_100_hours", category.as_str()));
    }
    let mut text = csv::line(&header);
    for day in days {
        let hours = day.played_ms as f64 / 3_600_000.0;
        let mut cells = vec![day.day.clone(), format!("{hours:.3}"), day.sessions.to_string()];
        for category in DropCategory::ALL {
            let drops = day
                .drops
                .iter()
                .find(|(known, _)| *known == category)
                .map_or(0, |(_, drops)| *drops);
            cells.push(drops.to_string());
            // The server's own stops and crashes are no player rate.
            let players = !matches!(category, DropCategory::ServerStop | DropCategory::ServerCrash);
            cells.push(if hours > 0.0 && players {
                format!("{:.3}", drops as f64 * 100.0 / hours)
            } else {
                String::new()
            });
        }
        text.push_str(&csv::line(&cells));
    }
    text
}

#[cfg(test)]
mod tests {
    use observer_domain::drops::DropPlayer;

    use super::*;

    #[test]
    fn a_players_name_that_a_spreadsheet_would_run_comes_out_as_text() {
        let row = DropLogRow {
            id: 1,
            at: "2026-10-31T10:00:00Z".parse().unwrap(),
            player: Some(DropPlayer {
                id: 7,
                license: "license:0123456789abcdef0123456789abcdef01234567".to_owned(),
                name: Some("=HYPERLINK(\"http://x\")".to_owned()),
            }),
            category: DropCategory::Kick,
            kind: Some(DropKind::Resource),
            resource: Some("afk".to_owned()),
            signature_id: None,
            signature: None,
            label: None,
            reason: Some("AFK, too long".to_owned()),
            length_ms: Some(60_000),
            loading: None,
            run: 3,
            generation: 2,
            synthesized: false,
            incident: false,
        };

        let csv = log_csv(&[row]);
        let line = csv.lines().nth(1).unwrap();

        assert!(line.contains(",\"'=HYPERLINK(\"\"http://x\"\")\","));
        assert!(line.contains(",\"AFK, too long\","));
    }

    #[test]
    fn a_day_without_play_has_no_rate() {
        let csv = rates_csv(&[DayRates {
            day: "2026-10-31".to_owned(),
            played_ms: 0,
            sessions: 0,
            drops: Vec::new(),
        }]);

        assert!(csv.lines().nth(1).unwrap().starts_with("2026-10-31,0.000,0,0,,"));
    }
}
