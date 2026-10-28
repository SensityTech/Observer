use axum::extract::Request;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use observer_domain::error::ErrorCode;

use crate::error::Failure;

/// Everything the panel loads comes from Observer itself. `data:` images carry the grain texture.
const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; script-src 'self'; style-src 'self'; \
    img-src 'self' data:; font-src 'self'; connect-src 'self'; object-src 'none'; base-uri 'none'; \
    form-action 'self'; frame-ancestors 'none'";

const PERMISSIONS_POLICY: &str = "camera=(), geolocation=(), microphone=(), payment=(), usb=()";

pub(crate) async fn headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();

    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CONTENT_SECURITY_POLICY),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    headers.insert(
        HeaderName::from_static("cross-origin-opener-policy"),
        HeaderValue::from_static("same-origin"),
    );
    headers.insert(
        HeaderName::from_static("cross-origin-resource-policy"),
        HeaderValue::from_static("same-origin"),
    );
    headers.insert(
        HeaderName::from_static("permissions-policy"),
        HeaderValue::from_static(PERMISSIONS_POLICY),
    );
    response
}

/// Every API change must come from the panel's own origin, as JSON. A page on another origin, the
/// game server's port on the same host included, can send neither without a preflight that Observer
/// never answers. Reads change nothing, and no API answer is ever cached.
pub(crate) async fn api(request: Request, next: Next) -> Response {
    let mutation = !matches!(*request.method(), Method::GET | Method::HEAD | Method::OPTIONS);
    if mutation && let Err(failure) = same_origin(request.headers()).and_then(|()| json(request.headers())) {
        return failure.into_response();
    }

    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// The `Origin` must be this server's exact scheme, host and port. Without one, the browser's own
/// `Sec-Fetch-Site: same-origin` will do.
fn same_origin(headers: &HeaderMap) -> Result<(), Failure> {
    let same = match headers.get(header::ORIGIN) {
        Some(origin) => headers
            .get(header::HOST)
            .and_then(|host| host.to_str().ok())
            .is_some_and(|host| origin.as_bytes() == format!("http://{host}").as_bytes()),
        None => headers
            .get("sec-fetch-site")
            .is_some_and(|site| site == "same-origin"),
    };
    if same {
        Ok(())
    } else {
        Err(ErrorCode::OriginMismatch.into())
    }
}

fn json(headers: &HeaderMap) -> Result<(), Failure> {
    let json = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|essence| essence.trim().eq_ignore_ascii_case("application/json"));
    if json {
        Ok(())
    } else {
        Err(ErrorCode::JsonRequired.into())
    }
}
