//! The API's route table. Every route declares its gate here, next to its handler, and nothing
//! else adds routes to the API: what the table says is what the server enforces.

mod account;
mod audit;
mod auth;
mod setup;
mod users;

use axum::extract::State;
use axum::handler::Handler;
use axum::http::Method;
use axum::routing::{MethodFilter, MethodRouter, on};
use axum::{Json, Router, middleware};
use observer_domain::api::{Health, HealthStatus};
use observer_domain::error::ErrorCode;
use observer_domain::permissions::Permissions;

use crate::AppState;
use crate::error::Failure;
use crate::gate::{self, Gate, Guard};
use crate::security;

pub(crate) struct Route {
    /// The handler already routes it; the tests that walk the table read it here.
    #[cfg_attr(not(test), expect(dead_code))]
    pub method: Method,
    /// Relative to `/api`.
    pub path: &'static str,
    pub gate: Gate,
    handler: MethodRouter<AppState>,
}

#[rustfmt::skip]
pub(crate) fn routes() -> Vec<Route> {
    use Gate::{Account, Permission, Public, Reauthenticated, Session};
    use Method as M;
    use Permissions as P;

    // Listing accounts needs the permission; changing one needs the password again as well.
    let read_users = Permission(P::USERS_MANAGE);
    let change_users = Reauthenticated(P::USERS_MANAGE);
    let read_audit = Permission(P::AUDIT_VIEW);
    vec![
        route(M::GET,    "/health",                     Public,       health),
        route(M::GET,    "/setup",                      Public,       setup::status),
        route(M::POST,   "/setup",                      Public,       setup::complete),
        route(M::POST,   "/auth/login",                 Public,       auth::login),
        route(M::POST,   "/auth/logout",                Account,      auth::logout),
        route(M::GET,    "/auth/me",                    Account,      auth::me),
        route(M::POST,   "/auth/reauth",                Session,      auth::reauth),
        route(M::POST,   "/account/password",           Account,      account::change_password),
        route(M::GET,    "/account/sessions",           Session,      account::sessions),
        route(M::DELETE, "/account/sessions/{id}",      Session,      account::revoke_session),
        route(M::GET,    "/permissions",                read_users,   users::catalog),
        route(M::GET,    "/users",                      read_users,   users::list),
        route(M::POST,   "/users",                      change_users, users::create),
        route(M::PATCH,  "/users/{id}",                 change_users, users::update),
        route(M::DELETE, "/users/{id}",                 change_users, users::delete),
        route(M::POST,   "/users/{id}/reset-password",  change_users, users::reset_password),
        route(M::POST,   "/users/{id}/revoke-sessions", change_users, users::revoke_sessions),
        route(M::GET,    "/audit",                      read_audit,   audit::page),
        route(M::GET,    "/audit/export.csv",           read_audit,   audit::export),
    ]
}

fn route<H, T>(method: Method, path: &'static str, gate: Gate, handler: H) -> Route
where
    H: Handler<T, AppState>,
    T: 'static,
{
    let filter = MethodFilter::try_from(method.clone()).expect("the table only uses standard methods");
    Route {
        method,
        path,
        gate,
        handler: on(filter, handler),
    }
}

pub(crate) fn router(state: &AppState) -> Router<AppState> {
    routes()
        .into_iter()
        .fold(Router::new(), |router, route| {
            let guard = Guard {
                state: state.clone(),
                gate: route.gate,
            };
            let handler = route
                .handler
                .route_layer(middleware::from_fn_with_state(guard, gate::enforce));
            router.route(route.path, handler)
        })
        // Without this, unknown API paths would fall through to the panel's index page.
        .fallback(|| async { Failure::from(ErrorCode::NotFound) })
        .layer(middleware::from_fn(security::api))
}

async fn health(State(state): State<AppState>) -> Json<Health> {
    Json(Health {
        status: HealthStatus::Ok,
        version: state.version.to_owned(),
    })
}
