use std::net::IpAddr;

use axum::Extension;
use axum::extract::{Request, State};
use axum::http::uri::Authority;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use observer_domain::error::ErrorCode;

use crate::AppState;
use crate::error::Failure;
use crate::proxy::Remote;

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

/// Every API request must name a host Observer is reached by, and every change must come from the
/// panel's own origin, as JSON. A page on another origin, the game server's port on the same host
/// included, can send neither without a preflight that Observer never answers. Reads change
/// nothing, and no API answer is ever cached.
pub(crate) async fn api(
    State(state): State<AppState>,
    Extension(remote): Extension<Remote>,
    request: Request,
    next: Next,
) -> Response {
    if !known_host(request.headers(), &state.hosts) {
        return Failure::from(ErrorCode::UnknownHost).into_response();
    }
    let mutation = !matches!(*request.method(), Method::GET | Method::HEAD | Method::OPTIONS);
    if mutation
        && let Err(failure) = same_origin(request.headers(), remote).and_then(|()| json(request.headers()))
    {
        return failure.into_response();
    }

    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// The origin check compares the `Origin` with the `Host` the browser sent, so it alone cannot tell
/// this server from a stranger's page whose name was rebound to this host's address: that page is
/// its own origin. Only the name tells them apart. An address cannot be rebound, nor can
/// `localhost`; any other name must be one the owner listed. A request without a `Host` is not
/// from a browser.
fn known_host(headers: &HeaderMap, names: &[String]) -> bool {
    let Some(host) = headers.get(header::HOST) else {
        return true;
    };
    let Some(authority) = host.to_str().ok().and_then(|host| host.parse::<Authority>().ok()) else {
        return false;
    };
    let name = authority.host();
    name.trim_start_matches('[')
        .trim_end_matches(']')
        .parse::<IpAddr>()
        .is_ok()
        || name.eq_ignore_ascii_case("localhost")
        || names.iter().any(|known| known.eq_ignore_ascii_case(name))
}

/// The `Origin` must be this server's exact scheme, host and port. Without one, the browser's own
/// `Sec-Fetch-Site: same-origin` will do.
fn same_origin(headers: &HeaderMap, remote: Remote) -> Result<(), Failure> {
    let same = match headers.get(header::ORIGIN) {
        Some(origin) => is_own_origin(origin, headers, remote),
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

/// A browser always sends the `Origin` of the page opening a websocket, and it must be this
/// server's exact one: no other page, the game server's included, opens a socket with the
/// user's cookie.
pub(crate) fn own_origin(headers: &HeaderMap, remote: Remote) -> Result<(), Failure> {
    if headers
        .get(header::ORIGIN)
        .is_some_and(|origin| is_own_origin(origin, headers, remote))
    {
        Ok(())
    } else {
        Err(ErrorCode::OriginMismatch.into())
    }
}

/// The scheme is the one the browser used, which only a trusted proxy can make HTTPS; the host is
/// the one it asked for, which the proxy passes on unchanged.
fn is_own_origin(origin: &HeaderValue, headers: &HeaderMap, remote: Remote) -> bool {
    headers
        .get(header::HOST)
        .and_then(|host| host.to_str().ok())
        .is_some_and(|host| origin.as_bytes() == format!("{}://{host}", remote.scheme()).as_bytes())
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
