//! The audit log as the panel reads it: a closed catalog of actions, who did them and how it went.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

/// Every action the audit log records. Stored by name, so a variant is never renamed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum AuditAction {
    SetupCompleted,
    SetupLinkCreated,
    LoginSucceeded,
    /// One entry per username, network and 15-minute window, with the count in `details`.
    LoginFailed,
    Logout,
    SessionRevoked,
    PasswordChanged,
    UserCreated,
    UserDeleted,
    UserPermissionsChanged,
    UserPasswordReset,
    UserUnlocked,
    /// Only for actions whose permission is sensitive or critical.
    PermissionDenied,
    AuditExported,
    /// A signed-in user entered their password again, which unlocks sensitive actions for a while.
    /// A wrong one is a failure, as a wrong current password is for `PasswordChanged`.
    Reauthenticated,
}

impl AuditAction {
    pub const ALL: [Self; 15] = [
        Self::SetupCompleted,
        Self::SetupLinkCreated,
        Self::LoginSucceeded,
        Self::LoginFailed,
        Self::Logout,
        Self::SessionRevoked,
        Self::PasswordChanged,
        Self::UserCreated,
        Self::UserDeleted,
        Self::UserPermissionsChanged,
        Self::UserPasswordReset,
        Self::UserUnlocked,
        Self::PermissionDenied,
        Self::AuditExported,
        Self::Reauthenticated,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SetupCompleted => "setup_completed",
            Self::SetupLinkCreated => "setup_link_created",
            Self::LoginSucceeded => "login_succeeded",
            Self::LoginFailed => "login_failed",
            Self::Logout => "logout",
            Self::SessionRevoked => "session_revoked",
            Self::PasswordChanged => "password_changed",
            Self::UserCreated => "user_created",
            Self::UserDeleted => "user_deleted",
            Self::UserPermissionsChanged => "user_permissions_changed",
            Self::UserPasswordReset => "user_password_reset",
            Self::UserUnlocked => "user_unlocked",
            Self::PermissionDenied => "permission_denied",
            Self::AuditExported => "audit_exported",
            Self::Reauthenticated => "reauthenticated",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|action| action.as_str() == name)
    }
}

/// Who acted: a signed-in user, someone at the host's own window, the command line, Observer
/// itself, or an anonymous client such as a failed login.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ActorKind {
    User,
    Local,
    Cli,
    System,
    Anonymous,
}

impl ActorKind {
    pub const ALL: [Self; 5] = [Self::User, Self::Local, Self::Cli, Self::System, Self::Anonymous];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Local => "local",
            Self::Cli => "cli",
            Self::System => "system",
            Self::Anonymous => "anonymous",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Outcome {
    Success,
    Failure,
    /// Refused for lack of permission.
    Denied,
}

impl Outcome {
    pub const ALL: [Self; 3] = [Self::Success, Self::Failure, Self::Denied];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
            Self::Denied => "denied",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|outcome| outcome.as_str() == name)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AuditEntry {
    pub id: i64,
    pub at: Timestamp,
    pub actor_kind: ActorKind,
    /// The user's id when the actor was a user. It outlives the account, unlike a foreign key.
    pub actor_id: Option<i64>,
    pub actor_name: Option<String>,
    pub ip: Option<String>,
    pub user_agent: Option<String>,
    pub action: AuditAction,
    pub target: Option<String>,
    pub outcome: Outcome,
    #[ts(type = "Record<string, unknown>")]
    pub details: Value,
}

/// `GET /api/audit`: newest first.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct AuditPage {
    pub entries: Vec<AuditEntry>,
    /// The `before` of the next page, or null on the last one.
    pub next: Option<i64>,
}

/// The query of `GET /api/audit` and `GET /api/audit/export.csv`. `from` is inclusive, `to`
/// exclusive; `before` and `limit` only apply to the page.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export, optional_fields)]
pub struct AuditFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<AuditAction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<Outcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<Timestamp>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<Timestamp>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_names_match_the_wire_names() {
        for action in AuditAction::ALL {
            assert_eq!(serde_json::to_value(action).unwrap(), action.as_str());
            assert_eq!(AuditAction::from_name(action.as_str()), Some(action));
        }
        for kind in ActorKind::ALL {
            assert_eq!(serde_json::to_value(kind).unwrap(), kind.as_str());
            assert_eq!(ActorKind::from_name(kind.as_str()), Some(kind));
        }
        for outcome in Outcome::ALL {
            assert_eq!(serde_json::to_value(outcome).unwrap(), outcome.as_str());
            assert_eq!(Outcome::from_name(outcome.as_str()), Some(outcome));
        }
        assert_eq!(AuditAction::from_name("user_renamed"), None);
    }
}
