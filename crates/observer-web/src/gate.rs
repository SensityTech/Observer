use axum::extract::{OriginalUri, Request, State};
use axum::http::{HeaderMap, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use observer_app::{Caller, Client};
use observer_domain::error::ErrorCode;
use observer_domain::permissions::Permissions;

use crate::AppState;
use crate::error::Failure;
use crate::extract::RemoteClient;

/// The second half of a session, which only the panel's origin can read from `localStorage`.
pub const SESSION_KEY_HEADER: &str = "x-observer-session-key";

/// What a route requires before its handler runs. Every route declares one in the route table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Gate {
    /// Open to anyone. Only the routes of the public allow-list may use it.
    Public,
    /// Any session, even one whose user must change their password before anything else.
    Account,
    /// Any session whose user has a password of their own.
    Session,
    Permission(Permissions),
    /// The permission, and the password entered within the last ten minutes.
    Reauthenticated(Permissions),
}

#[derive(Clone)]
pub(crate) struct Guard {
    pub state: AppState,
    pub gate: Gate,
}

/// Runs a route's gate, then hands the handler the caller it found.
pub(crate) async fn enforce(
    State(guard): State<Guard>,
    RemoteClient(client): RemoteClient,
    mut request: Request,
    next: Next,
) -> Response {
    if guard.gate == Gate::Public {
        return next.run(request).await;
    }
    let method = request.method().to_string();
    let path = request
        .extensions()
        .get::<OriginalUri>()
        .map_or_else(|| request.uri().path(), |uri| uri.path())
        .to_owned();
    match admit(&guard, client, request.headers(), &method, &path).await {
        Ok(caller) => {
            request.extensions_mut().insert(caller);
            next.run(request).await
        }
        Err(failure) => failure.into_response(),
    }
}

async fn admit(
    guard: &Guard,
    client: Client,
    headers: &HeaderMap,
    method: &str,
    path: &str,
) -> Result<Caller, Failure> {
    let accounts = &guard.state.accounts;
    let token = cookie(headers, &guard.state.cookie);
    let key = headers
        .get(SESSION_KEY_HEADER)
        .and_then(|value| value.to_str().ok());
    let (Some(token), Some(key)) = (token, key) else {
        return Err(ErrorCode::Unauthenticated.into());
    };
    let caller = accounts.authenticate(token, key, client).await?;

    if caller.must_change_password && guard.gate != Gate::Account {
        return Err(ErrorCode::PasswordChangeRequired.into());
    }
    if let Gate::Permission(permission) | Gate::Reauthenticated(permission) = guard.gate {
        accounts.authorize(&caller, permission, method, path).await?;
    }
    if let Gate::Reauthenticated(_) = guard.gate {
        accounts.require_reauthentication(&caller)?;
    }
    Ok(caller)
}

/// The value of the cookie called `name`, among every `Cookie` header.
fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .find_map(|pair| {
            let (key, value) = pair.trim().split_once('=')?;
            (key == name).then_some(value)
        })
}

/// The session cookie. It is `HttpOnly` so scripts never see it, and `SameSite=Strict` so other
/// sites never send it. `Secure` and the `__Host-` prefix come with TLS, which Observer does not
/// serve yet.
pub(crate) fn session_cookie(name: &str, token: &str, max_age_seconds: i64) -> String {
    format!("{name}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={max_age_seconds}")
}

pub(crate) fn expired_cookie(name: &str) -> String {
    session_cookie(name, "", 0)
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    #[test]
    fn the_session_cookie_is_found_among_others_by_its_exact_name() {
        let mut headers = HeaderMap::new();
        headers.append(
            header::COOKIE,
            HeaderValue::from_static("theme=dark; observer_session_ab=1"),
        );
        headers.append(
            header::COOKIE,
            HeaderValue::from_static("observer_session_abcd=2f; lang=en"),
        );

        assert_eq!(cookie(&headers, "observer_session_abcd"), Some("2f"));
        assert_eq!(cookie(&headers, "observer_session_ab"), Some("1"));
        assert_eq!(cookie(&headers, "observer_session_abc"), None);
    }
}
