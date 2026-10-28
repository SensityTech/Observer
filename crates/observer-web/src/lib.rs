//! The HTTP side of Observer: the API, the embedded panel and the headers every response carries.

mod api;
mod panel;
mod security;

use axum::{Router, middleware};
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::TraceLayer;

/// The largest request body the API accepts. Nothing it takes comes close; artifacts are
/// downloaded by Observer itself, never uploaded.
const MAX_BODY_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone)]
pub struct AppState {
    pub version: &'static str,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .nest("/api", api::router())
        .fallback(panel::serve)
        .layer(RequestBodyLimitLayer::new(MAX_BODY_BYTES))
        .layer(CatchPanicLayer::new())
        .layer(middleware::from_fn(security::headers))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use super::*;

    async fn get(path: &str) -> axum::response::Response {
        let app = router(AppState {
            version: "0.0.0-test",
        });
        app.oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn health_reports_the_running_version() {
        let response = get("/api/health").await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "status": "ok", "version": "0.0.0-test" })
        );
    }

    #[tokio::test]
    async fn unknown_api_paths_are_not_found_rather_than_the_panel() {
        assert_eq!(get("/api/nothing-here").await.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn a_missing_file_with_an_extension_is_not_found() {
        assert_eq!(
            get("/assets/missing-0000.js").await.status(),
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn every_response_carries_the_security_headers() {
        let response = get("/api/nothing-here").await;
        let headers = response.headers();

        let csp = headers[header::CONTENT_SECURITY_POLICY].to_str().unwrap();
        assert!(csp.contains("script-src 'self'"));
        assert!(csp.contains("frame-ancestors 'none'"));
        assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        assert_eq!(headers[header::REFERRER_POLICY], "no-referrer");
    }
}
