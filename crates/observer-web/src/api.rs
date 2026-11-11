//! The API's route table. Every route declares its gate here, next to its handler, and nothing
//! else adds routes to the API: what the table says is what the server enforces. Clippy holds
//! everyone to it: `Router::route` and its kin are disallowed outside `add` and the `/api` mount.

mod about;
mod account;
mod allowlist;
mod artifacts;
mod audit;
mod auth;
mod backups;
mod bans;
mod cfg;
mod changes;
mod debug;
mod diagnostics;
mod docs;
mod drops;
mod entities;
mod features;
mod files;
mod maintenance;
mod map;
mod metrics;
mod players;
mod resources;
mod schedule;
mod servers;
mod settings;
mod setup;
mod tools;
mod updates;
mod users;

use axum::extract::State;
use axum::handler::Handler;
use axum::http::Method;
use axum::routing::{MethodFilter, MethodRouter, on};
use axum::{Json, Router, middleware};
use observer_domain::api::{Health, HealthStatus};
use observer_domain::error::ErrorCode;
use observer_domain::features::Feature;
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
#[expect(clippy::too_many_lines, reason = "every route of the API, read best as one table")]
pub(crate) fn routes() -> Vec<Route> {
    use Gate::{Account, Owner, Permission, PermissionReadsAudited, Public, Reauthenticated, ReauthenticatedOwner, Session, Ticket};
    use Method as M;
    use Permissions as P;

    // Listing accounts needs the permission; changing one needs the password again as well.
    let read_users = Permission(P::USERS_MANAGE);
    let change_users = Reauthenticated(P::USERS_MANAGE);
    let read_audit = Permission(P::AUDIT_VIEW);
    // Groups decide which parts of the host are reachable, so they are the owner's to write, with
    // the launch settings' gate; a manager of file access gives and takes them away.
    let read_files = Permission(P::FILES_MANAGE);
    let change_files = Reauthenticated(P::FILES_MANAGE);
    // Launch settings choose what the host runs: the owner's alone, and their password again to
    // change them. The setup's checks, its folder listing and its search for servers read the
    // host's folders, which is the owner's alone too: posted, so a path never goes in an address
    // and a refusal is audited.
    let read_launch = Owner;
    let edit_launch = ReauthenticatedOwner;
    let control = Permission(P::SERVER_CONTROL);
    // A command that stops the server or runs what Observer cannot read also takes SERVER_CONTROL,
    // which the handler checks once it has read the line.
    let command = Permission(P::CONSOLE_WRITE);
    let logs = Permission(P::LOGS_DOWNLOAD);
    // Starting, stopping and restarting a resource, and having FXServer read its folders again:
    // one permission of their own, less than the console's, which runs any command.
    let resources = Permission(P::RESOURCES_CONTROL);
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
    // Bans are seen, made and lifted under three permissions. The refusal message is a text
    // Observer shows players, so changing it takes the settings' permission and the password again.
    let bans = Permission(P::BANS_VIEW);
    let ban = Permission(P::PLAYERS_BAN);
    let lift = Permission(P::BANS_LIFT);
    let read_ban_settings = Permission(P::SETTINGS_VIEW);
    let change_ban_settings = Reauthenticated(P::SETTINGS_MANAGE);
    // The allowlist's lists hold names and licences, the players' own; answering a request,
    // approving a licence or taking an approval back is its own permission. Its switch and its
    // message go by the rule of the bans' message.
    let allowlist = Permission(P::PLAYERS_VIEW);
    let admit = Permission(P::ALLOWLIST_MANAGE);
    // Maintenance closes the server to players as a stop does: turning it on or off is the server
    // control's, and so is reading the message players will get before doing it. That message and
    // the list of those who still get in go by the rule of the bans' message.
    let maintain = Permission(P::SERVER_CONTROL);
    // Where players stand is its own permission. The image comes from a host path, which is the
    // owner's to read, with the password again; the calibration and colours are settings.
    let map = Permission(P::PLAYERS_MAP);
    let map_image = ReauthenticatedOwner;
    let change_map = Reauthenticated(P::SETTINGS_MANAGE);
    // The server's entities are seen under one permission and deleted under another. Who owns
    // one is told only to who also sees the live map, which the service checks. How often the
    // server lists them is a setting.
    let entities = Permission(P::ENTITIES_VIEW);
    let delete_entities = Permission(P::ENTITIES_DELETE);
    let read_entities_settings = Permission(P::SETTINGS_VIEW);
    let change_entities = Reauthenticated(P::SETTINGS_MANAGE);
    // Statistics name nobody; the log, its export and reason groups are `PLAYERS_VIEW`'s.
    let drops = Permission(P::DROPS_VIEW);
    // What changed between starts: the history for its permission, the contents behind the critical
    // one, a refused read of them audited; the settings, the purge and a full verification are the
    // owner's, with the password again to change anything.
    let changes = Permission(P::CHANGES_VIEW);
    let contents = PermissionReadsAudited(P::CHANGES_CONTENT);
    // The server tools read the whole server folder, which is the owner's; a fix, an undo or the
    // settings write into it, now or at every start, so they take the password again.
    let read_tools = Owner;
    let change_tools = ReauthenticatedOwner;
    // `observer.toml` names the host's folders and addresses; the panel only reads it.
    let read_bootstrap = Owner;
    // A feature turned on may open a port or read the host's folders: the owner's alone, as the
    // launch settings are.
    let read_features = Permission(P::SETTINGS_VIEW);
    let read_debug = Permission(P::DIAGNOSTICS_VIEW);
    let judge_hosts = Owner;
    let switch_features = ReauthenticatedOwner;
    // A backup is the whole game database. Seeing the backups and starting one are permissions;
    // taking one out takes the critical one and the password again. The settings and destinations
    // decide what is dumped and where the database goes, so they are the owner's, with the password
    // again to change them; their tests reach hosts anyone could name, the owner's alone too.
    let read_backups = Permission(P::BACKUPS_VIEW);
    let run_backups = Permission(P::BACKUPS_RUN);
    let download_backups = Reauthenticated(P::BACKUPS_DOWNLOAD);
    let test_backups = Owner;
    let change_backups = ReauthenticatedOwner;
    // A release replaces the executable the host runs: the owner's alone, with the password again
    // to install one. Reading and checking reach GitHub only, with nothing of the owner's.
    let read_updates = Owner;
    let install_update = ReauthenticatedOwner;
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
        route(M::GET,    "/account/files",              Session,      files::my_access),
        // A key added logs in as its holder: the password again first.
        route(M::POST,   "/account/files/keys",         Reauthenticated(P::NONE), files::add_key),
        route(M::DELETE, "/account/files/keys/{id}",    Session,      files::remove_key),
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
        route(M::GET,    "/servers/default/files/groups",      read_files,   files::groups),
        route(M::POST,   "/servers/default/files/groups",      edit_launch,  files::create_group),
        route(M::PUT,    "/servers/default/files/groups/{id}", edit_launch,  files::update_group),
        route(M::DELETE, "/servers/default/files/groups/{id}", edit_launch,  files::delete_group),
        route(M::PUT,    "/files/accesses/users/{id}",         change_files, files::set_user_groups),
        route(M::GET,    "/files/accesses",                    read_files,   files::accesses),
        route(M::POST,   "/files/services",                    edit_launch,  files::create_service),
        route(M::DELETE, "/files/services/{id}",               change_files, files::delete_service),
        route(M::GET,    "/servers/default/launch",     read_launch,  servers::launch_settings),
        route(M::PUT,    "/servers/default/launch",     edit_launch,  servers::change_launch_settings),
        route(M::GET,    "/servers/default/launch/cfg-convars", read_launch, servers::cfg_convars),
        route(M::GET,    "/convars",                    read_launch,  servers::convar_catalog),
        route(M::POST,   "/servers/default/setup/folder", read_launch, servers::check_folder),
        route(M::POST,   "/servers/default/setup/cfg",    read_launch, servers::check_cfg),
        route(M::POST,   "/servers/default/setup/folders",  read_launch, servers::list_folders),
        route(M::POST,   "/servers/default/setup/files",    read_launch, servers::list_files),
        route(M::POST,   "/servers/default/setup/cfg/text", read_launch, servers::cfg_text),
        route(M::POST,   "/servers/default/setup/discover", read_launch, servers::discover),
        route(M::PUT,    "/servers/default/setup",        edit_launch, servers::save_setup),
        route(M::GET,    "/servers/default/cfg",          read_launch, cfg::document),
        route(M::PUT,    "/servers/default/cfg",          edit_launch, cfg::save),
        route(M::POST,   "/servers/default/cfg/check",    read_launch, cfg::check),
        route(M::POST,   "/servers/default/cfg/diff",     read_launch, cfg::diff),
        route(M::GET,    "/servers/default/cfg/versions", read_launch, cfg::versions),
        route(M::GET,    "/servers/default/cfg/versions/{id}",         read_launch, cfg::version_diff),
        route(M::POST,   "/servers/default/cfg/versions/{id}/restore", edit_launch, cfg::restore),
        route(M::POST,   "/servers/default/start",      control,      servers::start),
        route(M::POST,   "/servers/default/start/extend", control,    servers::extend_start),
        route(M::GET,    "/servers/default/start/warnings", read_tools, servers::start_warnings),
        route(M::POST,   "/servers/default/stop",       control,      servers::stop),
        route(M::POST,   "/servers/default/restart",    control,      servers::restart),
        route(M::POST,   "/servers/default/console",    command,      servers::command),
        route(M::GET,    "/servers/default/console/logs",        logs, servers::log_files),
        route(M::GET,    "/servers/default/console/logs/{name}", logs, servers::download_log),
        route(M::POST,   "/servers/default/resources/start",   resources, resources::start),
        route(M::POST,   "/servers/default/resources/stop",    resources, resources::stop),
        route(M::POST,   "/servers/default/resources/restart", resources, resources::restart),
        route(M::POST,   "/servers/default/resources/rescan",  resources, resources::rescan),
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
        route(M::GET,    "/metrics/timeline",           dashboard,    metrics::timeline),
        route(M::GET,    "/metrics/events",             dashboard,    metrics::events),
        route(M::PUT,    "/metrics/links",              drops,        metrics::link),
        route(M::GET,    "/metrics/frames",             dashboard,    metrics::frames),
        route(M::GET,    "/metrics/hours",              dashboard,    metrics::hours),
        route(M::GET,    "/metrics/runs",               dashboard,    metrics::runs),
        route(M::GET,    "/about",                      Session,      about::about),
        route(M::GET,    "/about/licences",             Session,      about::licences),
        route(M::GET,    "/docs/integration",           Session,      docs::integration),
        route(M::GET,    "/servers/default/crashes/{id}/files/{name}", dumps, diagnostics::download_file),
        route(M::DELETE, "/servers/default/crashes/{id}/files",        dumps, diagnostics::delete_files),
        route(M::GET,    "/servers/default/players",           players, players::page),
        route(M::GET,    "/servers/default/players/online",    players, players::online),
        route(M::GET,    "/servers/default/players/community", players, players::community),
        route(M::GET,    "/servers/default/players/settings",  Owner,   players::settings),
        route(M::PUT,    "/servers/default/players/settings",  erase,   players::change_settings),
        route(M::GET,    "/servers/default/players/import",    Owner,   players::import_status),
        route(M::POST,   "/servers/default/players/import",    erase,   players::import),
        route(M::GET,    "/servers/default/players/map",           map,        map::view),
        route(M::PUT,    "/servers/default/players/map/settings",  change_map, map::change_settings),
        route(M::GET,    "/servers/default/players/map/image",     map,        map::image),
        route(M::PUT,    "/servers/default/players/map/image",     map_image,  map::set_image),
        route(M::DELETE, "/servers/default/players/map/image",     map_image,  map::remove_image),
        route(M::GET,    "/servers/default/entities",          entities,               entities::page),
        route(M::GET,    "/servers/default/entities/map",      entities,               entities::map),
        route(M::POST,   "/servers/default/entities/delete",   delete_entities,        entities::delete),
        route(M::GET,    "/servers/default/entities/settings", read_entities_settings, entities::settings),
        route(M::PUT,    "/servers/default/entities/settings", change_entities,        entities::change_settings),
        route(M::GET,    "/servers/default/players/{id}",      players, players::profile),
        route(M::GET,    "/servers/default/players/{id}/bans", bans,    bans::of_player),
        route(M::GET,    "/servers/default/bans",              bans,    bans::page),
        route(M::POST,   "/servers/default/bans",              ban,     bans::create),
        route(M::GET,    "/servers/default/bans/settings",     read_ban_settings,   bans::settings),
        route(M::PUT,    "/servers/default/bans/settings",     change_ban_settings, bans::change_settings),
        route(M::POST,   "/servers/default/bans/{code}/lift",  lift,    bans::lift),
        route(M::GET,    "/servers/default/allowlist/requests",                allowlist, allowlist::requests),
        route(M::POST,   "/servers/default/allowlist/requests/deny",           admit,     allowlist::deny_requests),
        route(M::POST,   "/servers/default/allowlist/requests/{code}/approve", admit,     allowlist::approve_request),
        route(M::DELETE, "/servers/default/allowlist/requests/{code}",         admit,     allowlist::deny_request),
        route(M::GET,    "/servers/default/allowlist/approved",                allowlist, allowlist::approved),
        route(M::POST,   "/servers/default/allowlist/approved",                admit,     allowlist::approve),
        route(M::DELETE, "/servers/default/allowlist/approved/{id}",           admit,     allowlist::remove_approval),
        route(M::GET,    "/servers/default/allowlist/settings",     read_ban_settings,   allowlist::settings),
        route(M::PUT,    "/servers/default/allowlist/settings",     change_ban_settings, allowlist::change_settings),
        route(M::GET,    "/servers/default/maintenance",                maintain,            maintenance::preview),
        route(M::PUT,    "/servers/default/maintenance",                maintain,            maintenance::set),
        route(M::GET,    "/servers/default/maintenance/settings",       read_ban_settings,   maintenance::settings),
        route(M::PUT,    "/servers/default/maintenance/settings",       change_ban_settings, maintenance::change_settings),
        route(M::GET,    "/servers/default/maintenance/passes",         read_ban_settings,   maintenance::passes),
        route(M::POST,   "/servers/default/maintenance/passes",         change_ban_settings, maintenance::add_pass),
        route(M::DELETE, "/servers/default/maintenance/passes/{id}",    change_ban_settings, maintenance::remove_pass),
        route(M::GET,    "/servers/default/drops/summary",     drops,   drops::summary),
        route(M::GET,    "/servers/default/drops/series",      drops,   drops::series),
        route(M::GET,    "/servers/default/drops/signatures",  drops,   drops::signatures),
        route(M::GET,    "/servers/default/drops/resources",   drops,   drops::resources),
        route(M::GET,    "/servers/default/drops/changes",     drops,   drops::changes),
        route(M::GET,    "/servers/default/drops/uptime",      drops,   drops::uptime),
        route(M::GET,    "/servers/default/drops/rates.csv",   drops,   drops::export_rates),
        route(M::GET,    "/servers/default/drops/crash",       drops,   drops::crash),
        route(M::GET,    "/servers/default/drops/sessions",    drops,   drops::sessions),
        route(M::GET,    "/servers/default/drops/effects",     drops,   drops::effects),
        route(M::GET,    "/servers/default/drops/log",         players, drops::log),
        route(M::GET,    "/servers/default/drops/log.csv",     players, drops::export_log),
        route(M::GET,    "/servers/default/drops/settings",    Owner,   drops::settings),
        route(M::PUT,    "/servers/default/drops/settings",    erase,   drops::change_settings),
        route(M::DELETE, "/servers/default/players/{id}",      erase,   players::forget),
        route(M::GET,    "/servers/default/changes",          changes,  changes::compare),
        route(M::GET,    "/servers/default/changes/starts",   changes,  changes::starts),
        route(M::GET,    "/servers/default/changes/resource", changes,  changes::resource),
        route(M::GET,    "/servers/default/changes/history",  changes,  changes::history),
        route(M::POST,   "/servers/default/changes/check",    changes,  changes::check),
        route(M::GET,    "/servers/default/changes/diff",     contents, changes::diff),
        route(M::GET,    "/servers/default/changes/file",     contents, changes::file),
        route(M::GET,    "/servers/default/changes/settings", Owner,    changes::settings),
        route(M::PUT,    "/servers/default/changes/settings", ReauthenticatedOwner, changes::change_settings),
        route(M::POST,   "/servers/default/changes/verify",   ReauthenticatedOwner, changes::verify),
        route(M::DELETE, "/servers/default/changes/contents", ReauthenticatedOwner, changes::purge),
        route(M::GET,    "/tools",                                   read_tools,   tools::view),
        route(M::POST,   "/tools/duplicate-archetypes/scan",         read_tools,   tools::scan),
        route(M::POST,   "/tools/duplicate-archetypes/fix",          change_tools, tools::fix),
        route(M::POST,   "/tools/duplicate-archetypes/undo",         change_tools, tools::undo),
        route(M::POST,   "/tools/duplicate-archetypes/stage",        change_tools, tools::stage),
        route(M::DELETE, "/tools/duplicate-archetypes/stage",        change_tools, tools::unstage),
        route(M::PUT,    "/tools/duplicate-archetypes/settings",     change_tools, tools::change_settings),
        route(M::GET,    "/settings/observer",                       read_bootstrap, settings::observer),
        route(M::GET,    "/features",           read_features,   features::list),
        route(M::PUT,    "/features/{feature}", switch_features, features::switch),
        route(M::GET,    "/settings/sftp",      read_features,   features::sftp_listen),
        route(M::PUT,    "/settings/sftp",      switch_features, features::set_sftp_listen),
        route(M::GET,    "/debug/status",       read_debug,      debug::status),
        route(M::GET,    "/debug/events",       read_debug,      debug::events),
        route(M::GET,    "/debug/bags",         read_debug,      debug::bags),
        route(M::GET,    "/debug/http",         read_debug,      debug::http),
        route(M::POST,   "/debug/http/known",   judge_hosts,     debug::know_host),
        route(M::GET,    "/servers/default/backups",                      read_backups,     backups::view),
        route(M::PUT,    "/servers/default/backups/settings",             change_backups,   backups::change_settings),
        route(M::POST,   "/servers/default/backups/source/test",          test_backups,     backups::test_source),
        route(M::POST,   "/servers/default/backups/destinations",         change_backups,   backups::create_destination),
        route(M::POST,   "/servers/default/backups/destinations/test",    test_backups,     backups::test_destination),
        route(M::PUT,    "/servers/default/backups/destinations/{id}",    change_backups,   backups::change_destination),
        route(M::DELETE, "/servers/default/backups/destinations/{id}",    change_backups,   backups::delete_destination),
        route(M::POST,   "/servers/default/backups/run",                  run_backups,      backups::run_now),
        route(M::DELETE, "/servers/default/backups/run",                  run_backups,      backups::cancel),
        route(M::GET,    "/servers/default/backups/runs",                 read_backups,     backups::runs),
        route(M::GET,    "/servers/default/backups/runs/{id}",            read_backups,     backups::run),
        route(M::DELETE, "/servers/default/backups/runs/{id}",            change_backups,   backups::delete_run),
        route(M::GET,    "/servers/default/backups/copies/{id}/download", download_backups, backups::download),
        route(M::GET,    "/updates",         read_updates,   updates::view),
        route(M::POST,   "/updates/check",   read_updates,   updates::check),
        route(M::POST,   "/updates/install", install_update, updates::install),
        route(M::DELETE, "/updates/install", install_update, updates::cancel),
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
        feature: Feature::of_route(route.path),
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
