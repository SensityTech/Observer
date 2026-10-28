use axum::body::Body;
use axum::extract::State;
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use futures_util::stream;
use observer_app::Caller;
use observer_domain::audit::{AuditFilter, AuditPage};
use tracing::error;

use crate::AppState;
use crate::csv;
use crate::error::Failure;
use crate::extract::QueryParams;

pub(crate) async fn page(
    State(state): State<AppState>,
    QueryParams(filter): QueryParams<AuditFilter>,
) -> Result<Json<AuditPage>, Failure> {
    Ok(Json(state.audit.page(filter).await?))
}

/// Where an export stands between two batches.
enum Export {
    Header,
    Below(Option<i64>),
    Done,
}

/// Streams every matching entry as CSV, newest first, in batches read as the client takes them.
/// The export itself is audited before the first byte leaves.
pub(crate) async fn export(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    QueryParams(filter): QueryParams<AuditFilter>,
) -> Result<Response, Failure> {
    state.audit.record_export(&caller, &filter).await?;

    let audit = state.audit.clone();
    let lines = stream::unfold(Export::Header, move |export| {
        let (audit, filter) = (audit.clone(), filter.clone());
        async move {
            match export {
                Export::Header => Some((Ok(csv::HEADER.to_owned()), Export::Below(None))),
                Export::Below(before) => match audit.export_batch(filter, before).await {
                    Ok(entries) if entries.is_empty() => None,
                    Ok(entries) => {
                        let next = entries.last().map(|entry| entry.id);
                        Some((Ok(csv::rows(&entries)), Export::Below(next)))
                    }
                    Err(failure) => {
                        error!(error = %failure, "the audit export stopped");
                        Some((Err(failure), Export::Done))
                    }
                },
                Export::Done => None,
            }
        }
    });

    let headers = [
        (header::CONTENT_TYPE, "text/csv; charset=utf-8"),
        (
            header::CONTENT_DISPOSITION,
            "attachment; filename=\"observer-audit.csv\"",
        ),
    ];
    Ok((headers, Body::from_stream(lines)).into_response())
}
