//! The API's route table. Every route declares its gate here, next to its handler, and nothing
//! else adds routes to the API: what the table says is what the server enforces. Clippy holds
//! everyone to it: `Router::route` and its kin are disallowed outside `add` and the `/api` mount.

mod about;
mod account;
mod artifacts;
mod audit;
mod auth;
mod diagnostics;
mod metrics;
mod players;
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
    // change them. The setup's checks read the host's folders, which is the owner's alone too.
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
    // Builds are chosen among those Cfx.re signed or the owner imported; installing, choosing,
    // deleting and the settings take the password again, cancelling only goes back. Bringing a
    // folder of the host in is the owner's alone. `now` also takes SERVER_CONTROL, which the
    // handler checks: it restarts the server.
    let builds = Permission(P::ARTIFACTS_VIEW);
    let manage_builds = Permission(P::ARTIFACTS_MANAGE);
    let change_builds = Reauthenticated(P::ARTIFACTS_MANAGE);
    // A crash's files hold FXServer's memory: the owner's alone, with the password again.
    let dumps = ReauthenticatedOwner;
    // Names, licences and play time; identifiers and linked accounts need PLAYER_IDENTIFIERS_VIEW
    // too, which the service checks on what is asked. Retentions and erasures are the owner's.
    let players = Permission(P::PLAYERS_VIEW);
    let erase = ReauthenticatedOwner;
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
        route(M::POST,   "/servers/default/setup/folder", read_launch, servers::check_folder),
        route(M::POST,   "/servers/default/setup/cfg",    read_launch, servers::check_cfg),
        route(M::PUT,    "/servers/default/setup",        edit_launch, servers::save_setup),
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
        route(M::GET,    "/servers/default/artifacts/catalog",        builds,        artifacts::catalog),
        route(M::POST,   "/servers/default/artifacts/install",        change_builds, artifacts::install),
        route(M::POST,   "/servers/default/artifacts/import",         edit_launch,   artifacts::import),
        route(M::DELETE, "/servers/default/artifacts/job",            manage_builds, artifacts::cancel_job),
        route(M::PUT,    "/servers/default/artifacts/selection",      change_builds, artifacts::select),
        route(M::DELETE, "/servers/default/artifacts/pending",        manage_builds, artifacts::cancel_pending),
        route(M::DELETE, "/servers/default/artifacts/builds/{build}", change_builds, artifacts::delete),
        route(M::GET,    "/servers/default/artifacts/cleanup",        manage_builds, artifacts::cleanup_preview),
        route(M::POST,   "/servers/default/artifacts/cleanup",        change_builds, artifacts::cleanup),
        route(M::PUT,    "/servers/default/artifacts/settings",       change_builds, artifacts::change_settings),
        route(M::GET,    "/metrics/threads",            dashboard,    metrics::threads),
        route(M::GET,    "/metrics/growth",             dashboard,    metrics::growth),
        route(M::GET,    "/about",                      Session,      about::about),
        route(M::GET,    "/about/licences",             Session,      about::licences),
        route(M::GET,    "/servers/default/crashes/{id}/files/{name}", dumps, diagnostics::download_file),
        route(M::DELETE, "/servers/default/crashes/{id}/files",        dumps, diagnostics::delete_files),
        route(M::GET,    "/servers/default/players",           players, players::page),
        route(M::GET,    "/servers/default/players/online",    players, players::online),
        route(M::GET,    "/servers/default/players/community", players, players::community),
        route(M::GET,    "/servers/default/players/settings",  Owner,   players::settings),
        route(M::PUT,    "/servers/default/players/settings",  erase,   players::change_settings),
        route(M::GET,    "/servers/default/players/{id}",      players, players::profile),
        route(M::DELETE, "/servers/default/players/{id}",      erase,   players::forget),
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
