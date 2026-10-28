//! What the panel sends and receives for setup, sign-in, the user's own account and user
//! management. Requests that carry a password do not derive `Debug`, so it never reaches a log.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::permissions::Permissions;

/// `GET /api/setup`: whether the owner still has to be created.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct SetupStatus {
    pub required: bool,
}

/// `POST /api/setup`. The token comes from the setup link's fragment.
#[derive(Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct SetupRequest {
    pub token: String,
    pub username: String,
    pub password: String,
    /// See `LoginRequest::remember`.
    #[serde(default)]
    pub remember: bool,
}

/// `POST /api/auth/login`.
#[derive(Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
    /// Keep the session cookie after the browser closes. Without it the cookie lasts as long as the
    /// browser does, so a sign-in on a shared computer ends with it. The session's own lifetimes
    /// are the same either way.
    #[serde(default)]
    pub remember: bool,
}

/// The answer to a login or a completed setup, next to the session cookie.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LoginResponse {
    /// The second half of the session, shown once. The panel keeps it in `localStorage` and sends
    /// it as `X-Observer-Session-Key` on every `/api` request.
    pub session_key: String,
    pub me: Me,
}

/// `GET /api/auth/me`: the signed-in user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Me {
    pub id: i64,
    pub username: String,
    pub is_owner: bool,
    /// What the user holds, implications included: every permission for the owner.
    pub permissions: Permissions,
    /// Until the password is changed, the API refuses everything but `me`, the password change
    /// and logout.
    pub must_change_password: bool,
}

/// `POST /api/auth/reauth`.
#[derive(Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct ReauthRequest {
    pub password: String,
}

/// `POST /api/account/password`.
#[derive(Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct PasswordChangeRequest {
    pub current: String,
    pub next: String,
}

/// One of the signed-in user's sessions, from `GET /api/account/sessions`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AccountSession {
    pub id: i64,
    pub created_at: Timestamp,
    pub last_seen_at: Timestamp,
    /// The earlier of the idle and the absolute expiry.
    pub expires_at: Timestamp,
    pub ip: Option<String>,
    pub user_agent: Option<String>,
    /// The session making this request.
    pub current: bool,
}

/// An account, as `GET /api/users` lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub is_owner: bool,
    /// What the user holds, implications included: every permission for the owner.
    pub permissions: Permissions,
    pub must_change_password: bool,
    /// Sent back with an edit, which fails with `conflict` if the account changed in between.
    pub version: i64,
    pub created_at: Timestamp,
    pub last_login_at: Option<Timestamp>,
}

/// `POST /api/users`.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct CreateUserRequest {
    pub username: String,
    pub permissions: Permissions,
}

/// `PATCH /api/users/{id}`.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct UpdateUserRequest {
    pub permissions: Permissions,
    pub version: i64,
}

/// A new account and its temporary password, which is shown once and never again.
#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CreatedUser {
    pub user: User,
    pub temporary_password: String,
}

/// `POST /api/users/{id}/reset-password`: the new temporary password, shown once.
#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TemporaryPassword {
    pub temporary_password: String,
}
