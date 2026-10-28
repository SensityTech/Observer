use observer_domain::accounts::{CreatedUser, User};
use observer_domain::audit::{ActorKind, AuditAction};
use observer_domain::delegation;
use observer_domain::error::ErrorCode;
use observer_domain::permissions::Permissions;
use observer_store::users::{self, NewUser, UserRow};
use observer_store::{Connection, login_failures, sessions};
use serde_json::json;

use crate::accounts::{
    Accounts, TEMPORARY_PASSWORD_LIFETIME, account, check_known, check_username, existing, user_view,
};
use crate::audit::Record;
use crate::sessions::Caller;
use crate::{Error, secrets, throttle};

impl Accounts {
    /// Every account, the owner first.
    pub async fn users(&self) -> Result<Vec<User>, Error> {
        let rows = self.db.read(users::list).await?;
        Ok(rows.iter().map(user_view).collect())
    }

    /// Creates an account with a temporary password, which it must change at its first login.
    pub async fn create_user(
        &self,
        caller: &Caller,
        username: &str,
        permissions: Permissions,
    ) -> Result<CreatedUser, Error> {
        check_username(username)?;
        check_known(permissions)?;
        let granted = delegation::grant(&caller.account(), permissions)?;
        let temporary_password = secrets::temporary_password()?;
        let hash = self.passwords.hash(temporary_password.clone()).await?;

        let record = Record::by(caller, AuditAction::UserCreated).target(username);
        let username = username.to_owned();
        let now = self.clock.now();
        let user = self
            .db
            .write(move |tx| -> Result<_, Error> {
                let id = users::insert(
                    tx,
                    &NewUser {
                        username: &username,
                        password_hash: &hash,
                        is_owner: false,
                        permissions: granted,
                        must_change_password: true,
                        password_expires_at: Some(now + TEMPORARY_PASSWORD_LIFETIME),
                        created_at: now,
                    },
                )?
                .ok_or(ErrorCode::NameTaken)?;
                record
                    .details(json!({ "userId": id, "permissions": granted.to_hex() }))
                    .write(tx, now)?;
                existing(tx, id)
            })
            .await?;
        Ok(CreatedUser {
            user: user_view(&user),
            temporary_password,
        })
    }

    /// Sets an account's permissions. `version` must be the one the editor read. Every session of
    /// the account ends, so the new permissions apply from its next login.
    pub async fn update_user(
        &self,
        caller: &Caller,
        id: i64,
        permissions: Permissions,
        version: i64,
    ) -> Result<User, Error> {
        check_known(permissions)?;
        let editor = caller.account();
        let record = Record::by(caller, AuditAction::UserPermissionsChanged);
        let now = self.clock.now();
        let user = self
            .db
            .write(move |tx| -> Result<_, Error> {
                let mut user = existing(tx, id)?;
                let next = delegation::change(&editor, &account(&user), permissions)?;
                if user.version != version {
                    return Err(ErrorCode::Conflict.into());
                }
                let previous = user.permissions;
                if next == previous {
                    return Ok(user);
                }
                user.permissions = next;
                user.version += 1;
                user.auth_epoch += 1;
                users::update(tx, &user)?;
                sessions::delete_for_user(tx, id, None)?;
                record
                    .target(&user.username)
                    .details(json!({
                        "userId": id,
                        "added": (next & !previous).to_hex(),
                        "removed": (previous & !next).to_hex(),
                        "permissions": next.to_hex(),
                    }))
                    .write(tx, now)?;
                Ok(user)
            })
            .await?;
        Ok(user_view(&user))
    }

    pub async fn delete_user(&self, caller: &Caller, id: i64) -> Result<(), Error> {
        let editor = caller.account();
        let record = Record::by(caller, AuditAction::UserDeleted);
        let now = self.clock.now();
        self.db
            .write(move |tx| {
                let user = existing(tx, id)?;
                delegation::manage(&editor, &account(&user))?;
                users::delete(tx, id)?;
                record
                    .target(&user.username)
                    .details(json!({ "userId": id, "permissions": user.permissions.to_hex() }))
                    .write(tx, now)?;
                Ok(())
            })
            .await
    }

    /// Gives an account a new temporary password and ends its sessions. Returns the password,
    /// which is shown once.
    pub async fn reset_password(&self, caller: &Caller, id: i64) -> Result<String, Error> {
        let editor = caller.account();
        let record = Record::by(caller, AuditAction::UserPasswordReset);
        self.reset(record, move |tx| {
            let user = existing(tx, id)?;
            delegation::manage(&editor, &account(&user))?;
            Ok(user)
        })
        .await
    }

    /// Ends every session of an account.
    pub async fn revoke_sessions(&self, caller: &Caller, id: i64) -> Result<(), Error> {
        let editor = caller.account();
        let record = Record::by(caller, AuditAction::SessionRevoked);
        let now = self.clock.now();
        self.db
            .write(move |tx| {
                let mut user = existing(tx, id)?;
                delegation::manage(&editor, &account(&user))?;
                user.auth_epoch += 1;
                users::update(tx, &user)?;
                let ended = sessions::delete_for_user(tx, id, None)?;
                record
                    .target(&user.username)
                    .details(json!({ "userId": id, "sessions": ended }))
                    .write(tx, now)?;
                Ok(())
            })
            .await
    }

    /// `observer user reset-password`: the way back in for anyone, the owner included, from the
    /// host itself.
    pub async fn reset_password_from_cli(&self, username: &str) -> Result<String, Error> {
        let record = Record::operator(ActorKind::Cli, AuditAction::UserPasswordReset);
        let username = username.to_owned();
        self.reset(record, move |tx| {
            users::find(tx, &username)?.ok_or_else(|| ErrorCode::NotFound.into())
        })
        .await
    }

    /// `observer user unlock`: lifts the login locks on a username from every network. The
    /// failures they held still reach the audit log. Returns how many networks were locked out.
    pub async fn unlock(&self, username: &str) -> Result<usize, Error> {
        let username = username.to_owned();
        let now = self.clock.now();
        self.db
            .write(move |tx| -> Result<_, Error> {
                let user = users::find(tx, &username)?.ok_or(ErrorCode::NotFound)?;
                let mut locked = 0;
                for mut row in login_failures::for_username(tx, &user.username)? {
                    locked += usize::from(throttle::locked_for(&row, now).is_some());
                    if let Some(failures) = throttle::take_unaudited(&mut row) {
                        Record::login_failures(&failures).write(tx, now)?;
                    }
                }
                login_failures::delete_for_username(tx, &user.username)?;
                Record::operator(ActorKind::Cli, AuditAction::UserUnlocked)
                    .target(&user.username)
                    .details(json!({ "userId": user.id, "lockedNetworks": locked }))
                    .write(tx, now)?;
                Ok(locked)
            })
            .await
    }

    /// Hashes a new temporary password, then gives it to the account `target` finds and allows.
    async fn reset<F>(&self, record: Record, target: F) -> Result<String, Error>
    where
        F: FnOnce(&Connection) -> Result<UserRow, Error> + Send + 'static,
    {
        let temporary_password = secrets::temporary_password()?;
        let hash = self.passwords.hash(temporary_password.clone()).await?;
        let now = self.clock.now();
        self.db
            .write(move |tx| -> Result<_, Error> {
                let mut user = target(tx)?;
                user.password_hash = hash;
                user.must_change_password = true;
                user.password_expires_at = Some(now + TEMPORARY_PASSWORD_LIFETIME);
                user.auth_epoch += 1;
                user.version += 1;
                users::update(tx, &user)?;
                sessions::delete_for_user(tx, user.id, None)?;
                record
                    .target(&user.username)
                    .details(json!({ "userId": user.id }))
                    .write(tx, now)?;
                Ok(())
            })
            .await?;
        Ok(temporary_password)
    }
}
