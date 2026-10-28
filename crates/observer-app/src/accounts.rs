use std::sync::Arc;

use jiff::{SignedDuration, Timestamp};
use observer_domain::accounts::{Me, User};
use observer_domain::delegation::Account;
use observer_domain::error::{ApiError, ErrorCode};
use observer_domain::permissions::Permissions;
use observer_store::users::{self, UserRow};
use observer_store::{Connection, Database, login_failures, sessions, setup_tokens};

use crate::audit::Record;
use crate::passwords::{PasswordCost, Passwords};
use crate::throttle::{self, Attempts};
use crate::{Clock, Error};

/// How long a temporary password works, from its creation.
pub(crate) const TEMPORARY_PASSWORD_LIFETIME: SignedDuration = SignedDuration::from_hours(72);
/// A row of login failures with no lock and no failure for this long is forgotten.
const FORGET_FAILURES_AFTER: SignedDuration = SignedDuration::from_hours(24);

/// Accounts, sessions, first-run setup and login throttling: everything that decides who is
/// signed in and what they may do to other accounts.
#[derive(Clone)]
pub struct Accounts {
    pub(crate) db: Database,
    pub(crate) clock: Clock,
    pub(crate) passwords: Arc<Passwords>,
    pub(crate) attempts: Arc<Attempts>,
}

impl Accounts {
    /// Prepares the password hasher, whose dummy hash takes one full Argon2 run.
    pub async fn new(db: Database, clock: Clock, cost: PasswordCost) -> Result<Self, Error> {
        Ok(Self {
            db,
            passwords: Arc::new(Passwords::new(cost).await?),
            attempts: Arc::new(Attempts::new(clock.clone())),
            clock,
        })
    }

    /// Upkeep, run every minute: closed windows of failed logins go to the audit log, and expired
    /// sessions, setup tokens and old failures are forgotten.
    pub async fn sweep(&self) -> Result<(), Error> {
        let now = self.clock.now();
        self.db
            .write(move |tx| {
                for mut row in login_failures::unaudited_since_before(tx, now - throttle::WINDOW)? {
                    if let Some(failures) = throttle::take_unaudited(&mut row) {
                        Record::login_failures(&failures).write(tx, now)?;
                        login_failures::save(tx, &row)?;
                    }
                }
                login_failures::delete_quiet(tx, now - FORGET_FAILURES_AFTER, now)?;
                sessions::delete_expired(tx, now - crate::sessions::IDLE, now)?;
                setup_tokens::delete_expired(tx, now)?;
                Ok(())
            })
            .await
    }
}

/// Writes the pair's pending failures to the audit log, then forgets them: the user just proved
/// they know the password.
pub(crate) fn clear_failures(
    conn: &Connection,
    username: &str,
    network: &str,
    now: Timestamp,
) -> Result<(), Error> {
    if let Some(mut row) = login_failures::get(conn, username, network)? {
        if let Some(failures) = throttle::take_unaudited(&mut row) {
            Record::login_failures(&failures).write(conn, now)?;
        }
        login_failures::delete(conn, username, network)?;
    }
    Ok(())
}

pub(crate) fn account(user: &UserRow) -> Account {
    Account {
        id: user.id,
        is_owner: user.is_owner,
        permissions: user.permissions,
    }
}

pub(crate) fn me(user: &UserRow) -> Me {
    Me {
        id: user.id,
        username: user.username.clone(),
        is_owner: user.is_owner,
        permissions: account(user).effective(),
        must_change_password: user.must_change_password,
    }
}

pub(crate) fn user_view(user: &UserRow) -> User {
    User {
        id: user.id,
        username: user.username.clone(),
        is_owner: user.is_owner,
        permissions: account(user).effective(),
        must_change_password: user.must_change_password,
        version: user.version,
        created_at: user.created_at,
        last_login_at: user.last_login_at,
    }
}

/// Reads back an account the same transaction just wrote, or that the caller just checked.
pub(crate) fn existing(conn: &Connection, id: i64) -> Result<UserRow, Error> {
    users::get(conn, id)?.ok_or_else(|| ErrorCode::NotFound.into())
}

/// 3 to 32 characters among letters, digits, `_`, `.` and `-`.
pub(crate) fn check_username(username: &str) -> Result<(), Error> {
    let valid = (3..=32).contains(&username.len())
        && username
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'));
    if valid {
        Ok(())
    } else {
        Err(ErrorCode::InvalidUsername.into())
    }
}

/// Permissions from a client may only name bits the catalog knows.
pub(crate) fn check_known(permissions: Permissions) -> Result<(), Error> {
    let unknown = permissions.unknown();
    if unknown.is_empty() {
        Ok(())
    } else {
        Err(ApiError::new(ErrorCode::UnknownPermissions)
            .with("permissions", unknown.to_hex())
            .into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usernames_are_short_and_plain() {
        for valid in ["ada", "Grace.Hopper", "ops-team_2", &"a".repeat(32)] {
            check_username(valid).unwrap();
        }
        for invalid in ["ab", "with space", "élodie", "ada@host", &"a".repeat(33), ""] {
            assert_eq!(
                check_username(invalid).unwrap_err().code(),
                Some(ErrorCode::InvalidUsername),
                "{invalid:?}"
            );
        }
    }
}
