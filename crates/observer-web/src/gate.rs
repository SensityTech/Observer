use axum::extract::{Request, State};
use axum::http::{HeaderMap, Uri, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use observer_app::{Caller, Client};
use observer_domain::error::ErrorCode;
use observer_domain::permissions::Permissions;

use crate::AppState;
use crate::error::Failure;
use crate::extract::RemoteClient;
use crate::security;

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
    /// The owner, whatever anyone else holds: launch configuration amounts to running code on the
    /// host, which no permission grants.
    Owner,
    /// The owner, with the password entered within the last ten minutes.
    ReauthenticatedOwner,
    /// A websocket ticket in the query, with the cookie of the session it was issued to, from the
    /// panel's own origin. A browser sends no header of its choosing with a websocket request, so
    /// the ticket stands in for the session key.
    Ticket,
}

#[derive(Clone)]
pub(crate) struct Guard {
    pub state: AppState,
    pub gate: Gate,
    /// The route as the table declares it, relative to `/api`.
    pub path: &'static str,
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
    // The route rather than the path asked for: a refusal is audited, and the audit log must not
    // take whatever a client puts in a URL.
    let path = format!("/api{}", guard.path);
    let admitted = if guard.gate == Gate::Ticket {
        admit_ticket(&guard, client, request.uri(), request.headers()).await
    } else {
        admit(&guard, client, request.headers(), &method, &path).await
    };
    match admitted {
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
    match guard.gate {
        Gate::Permission(permission) | Gate::Reauthenticated(permission) => {
            accounts.authorize(&caller, permission, method, path).await?;
        }
        Gate::Owner | Gate::ReauthenticatedOwner => accounts.authorize_owner(&caller, method, path).await?,
        Gate::Public | Gate::Account | Gate::Session | Gate::Ticket => {}
    }
    if let Gate::Reauthenticated(_) | Gate::ReauthenticatedOwner = guard.gate {
        accounts.require_reauthentication(&caller)?;
    }
    Ok(caller)
}

async fn admit_ticket(
    guard: &Guard,
    client: Client,
    uri: &Uri,
    headers: &HeaderMap,
) -> Result<Caller, Failure> {
    security::own_origin(headers)?;
    let ticket = uri
        .query()
        .into_iter()
        .flat_map(|query| query.split('&'))
        .find_map(|pair| pair.strip_prefix("ticket="));
    let (Some(ticket), Some(token)) = (ticket, cookie(headers, &guard.state.cookie)) else {
        return Err(ErrorCode::Unauthenticated.into());
    };
    Ok(guard.state.accounts.redeem_ticket(ticket, token, client).await?)
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
/// serve yet. Without a `Max-Age`, the browser drops it when it closes.
pub(crate) fn session_cookie(name: &str, token: &str, max_age_seconds: Option<i64>) -> String {
    let cookie = format!("{name}={token}; Path=/; HttpOnly; SameSite=Strict");
    match max_age_seconds {
        Some(seconds) => format!("{cookie}; Max-Age={seconds}"),
        None => cookie,
    }
}

pub(crate) fn expired_cookie(name: &str) -> String {
    session_cookie(name, "", Some(0))
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
