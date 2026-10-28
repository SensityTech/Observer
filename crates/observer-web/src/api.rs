//! The API's route table. Every route declares its gate here, next to its handler, and nothing
//! else adds routes to the API: what the table says is what the server enforces. Clippy holds
//! everyone to it: `Router::route` and its kin are disallowed outside `add` and the `/api` mount.

mod account;
mod audit;
mod auth;
mod metrics;
mod schedule;
mod servers;
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
use crate::{live, security};

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
    use Gate::{Account, Owner, Permission, Public, Reauthenticated, ReauthenticatedOwner, Session, Ticket};
    use Method as M;
    use Permissions as P;

    // Listing accounts needs the permission; changing one needs the password again as well.
    let read_users = Permission(P::USERS_MANAGE);
    let change_users = Reauthenticated(P::USERS_MANAGE);
    let read_audit = Permission(P::AUDIT_VIEW);
    // Launch settings choose what the host runs: the owner's alone, and their password again to
    // change them.
    let read_launch = Owner;
    let edit_launch = ReauthenticatedOwner;
    let control = Permission(P::SERVER_CONTROL);
    // A command that stops the server or runs what Observer cannot read also takes SERVER_CONTROL,
    // which the handler checks once it has read the line.
    let command = Permission(P::CONSOLE_WRITE);
    let logs = Permission(P::LOGS_DOWNLOAD);
    // The schedule's settings take the password again; skipping, restoring or delaying its next
    // restart are operations, and so is a single restart, which is the server control's.
    let plan = Permission(P::SCHEDULE_MANAGE);
    let edit_schedule = Reauthenticated(P::SCHEDULE_MANAGE);
    let dashboard = Permission(P::DASHBOARD_VIEW);
    vec![
        route(M::GET,    "/health",                     Public,       health),
        route(M::GET,    "/setup",                      Public,       setup::status),
        route(M::POST,   "/setup",                      Public,       setup::complete),
        route(M::POST,   "/auth/login",                 Public,       auth::login),
        route(M::POST,   "/auth/logout",                Account,      auth::logout),
        route(M::GET,    "/auth/me",                    Account,      auth::me),
        route(M::POST,   "/auth/reauth",                Session,      auth::reauth),
        route(M::POST,   "/auth/ws-ticket",             Session,      auth::ws_ticket),
        route(M::GET,    "/ws",                         Ticket,       live::open),
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
        route(M::GET,    "/servers/default/launch",     read_launch,  servers::launch_settings),
        route(M::PUT,    "/servers/default/launch",     edit_launch,  servers::change_launch_settings),
        route(M::POST,   "/servers/default/start",      control,      servers::start),
        route(M::POST,   "/servers/default/stop",       control,      servers::stop),
        route(M::POST,   "/servers/default/restart",    control,      servers::restart),
        route(M::POST,   "/servers/default/console",    command,      servers::command),
        route(M::GET,    "/servers/default/console/logs",        logs, servers::log_files),
        route(M::GET,    "/servers/default/console/logs/{name}", logs, servers::download_log),
        route(M::PUT,    "/servers/default/schedule",       edit_schedule, schedule::change),
        route(M::POST,   "/servers/default/schedule/skip",  plan,          schedule::skip),
        route(M::DELETE, "/servers/default/schedule/skip",  plan,          schedule::restore),
        route(M::POST,   "/servers/default/schedule/delay", plan,          schedule::delay),
        route(M::PUT,    "/servers/default/schedule/once",  control,       schedule::restart_once),
        route(M::DELETE, "/servers/default/schedule/once",  control,       schedule::cancel_once),
        route(M::GET,    "/metrics/series",             dashboard,    metrics::series),
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
        .fold(Router::new(), |router, route| add(router, route, state))
        // Without this, unknown API paths would fall through to the panel's index page.
        .fallback(|| async { Failure::from(ErrorCode::NotFound) })
        .layer(middleware::from_fn_with_state(state.clone(), security::api))
}

#[expect(
    clippy::disallowed_methods,
    reason = "the one place a route joins the API, behind its gate"
)]
fn add(router: Router<AppState>, route: Route, state: &AppState) -> Router<AppState> {
    let guard = Guard {
        state: state.clone(),
        gate: route.gate,
        path: route.path,
    };
    let handler = route
        .handler
        .route_layer(middleware::from_fn_with_state(guard, gate::enforce));
    router.route(route.path, handler)
}

async fn health(State(state): State<AppState>) -> Json<Health> {
    Json(Health {
        status: HealthStatus::Ok,
        version: state.build.version.to_owned(),
    })
}
