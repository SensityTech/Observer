use std::net::IpAddr;

use jiff::{SignedDuration, Timestamp};
use observer_domain::accounts::{AccountSession, Me};
use observer_domain::audit::{AuditAction, Outcome};
use observer_domain::delegation::Account;
use observer_domain::error::{ApiError, ErrorCode};
use observer_domain::permissions::{Danger, Permissions};
use observer_store::sessions::{self, NewSession, SessionRow};
use observer_store::users::{self, UserRow};
use observer_store::{Connection, login_failures};
use serde_json::json;

use crate::accounts::{self, Accounts, clear_failures, existing};
use crate::audit::Record;
use crate::{Error, passwords, secrets, throttle};

/// A session ends after this long without a request.
pub(crate) const IDLE: SignedDuration = SignedDuration::from_hours(12);
/// And after this long whatever happens.
const LIFETIME: SignedDuration = SignedDuration::from_hours(7 * 24);
/// How long entering the password again unlocks sensitive actions.
const REAUTHENTICATION: SignedDuration = SignedDuration::from_mins(10);
/// `last_seen_at` is written at most this often, rather than on every request.
const TOUCH_EVERY: SignedDuration = SignedDuration::from_mins(1);

const MAX_USER_AGENT_BYTES: usize = 256;

/// Where a request came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Client {
    pub ip: IpAddr,
    /// At most 256 bytes, cut on a character boundary.
    pub user_agent: Option<String>,
}

impl Client {
    pub fn new(ip: IpAddr, user_agent: Option<&str>) -> Self {
        let user_agent = user_agent.map(|agent| {
            let mut end = agent.len().min(MAX_USER_AGENT_BYTES);
            while !agent.is_char_boundary(end) {
                end -= 1;
            }
            agent[..end].to_owned()
        });
        Self { ip, user_agent }
    }
}

/// The signed-in user behind a request, as found when it arrived.
#[derive(Debug, Clone)]
pub struct Caller {
    pub session_id: i64,
    pub user_id: i64,
    pub username: String,
    pub is_owner: bool,
    /// What the user holds, implications included: every permission for the owner.
    pub permissions: Permissions,
    pub must_change_password: bool,
    pub reauthenticated_at: Option<Timestamp>,
    pub client: Client,
}

impl Caller {
    fn new(session: &SessionRow, user: &UserRow, client: Client) -> Self {
        Self {
            session_id: session.id,
            user_id: user.id,
            username: user.username.clone(),
            is_owner: user.is_owner,
            permissions: accounts::account(user).effective(),
            must_change_password: user.must_change_password,
            reauthenticated_at: session.reauthenticated_at,
            client,
        }
    }

    pub fn me(&self) -> Me {
        Me {
            id: self.user_id,
            username: self.username.clone(),
            is_owner: self.is_owner,
            permissions: self.permissions,
            must_change_password: self.must_change_password,
        }
    }

    pub(crate) fn account(&self) -> Account {
        Account {
            id: self.user_id,
            is_owner: self.is_owner,
            permissions: self.permissions,
        }
    }
}

/// A session just opened by a login or the first-run setup.
pub struct SignedIn {
    /// The cookie's value.
    pub token: String,
    /// Handed to the panel once, which sends it back as a header on every request.
    pub key: String,
    /// What the cookie's `Max-Age` should be: the session's absolute lifetime.
    pub lifetime: SignedDuration,
    pub me: Me,
    pub(crate) session_id: i64,
}

impl std::fmt::Debug for SignedIn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignedIn")
            .field("me", &self.me)
            .finish_non_exhaustive()
    }
}

/// Opens a session for `user`. Only the hashes of its two secrets are stored.
pub(crate) fn open(
    conn: &Connection,
    user: &UserRow,
    client: &Client,
    now: Timestamp,
) -> Result<SignedIn, Error> {
    let token = secrets::random::<32>()?;
    let key = secrets::random::<32>()?;
    let session_id = sessions::insert(
        conn,
        &NewSession {
            user_id: user.id,
            token_hash: &secrets::sha256(&token),
            key_hash: &secrets::sha256(&key),
            auth_epoch: user.auth_epoch,
            created_at: now,
            expires_at: now + LIFETIME,
            ip: Some(&client.ip.to_string()),
            user_agent: client.user_agent.as_deref(),
        },
    )?;
    Ok(SignedIn {
        token: secrets::hex(&token),
        key: secrets::hex(&key),
        lifetime: LIFETIME,
        me: accounts::me(user),
        session_id,
    })
}

impl Accounts {
    pub async fn login(&self, username: &str, password: String, client: &Client) -> Result<SignedIn, Error> {
        // Bounds the throttle's key; the audit log only ever names existing accounts.
        let username: String = username.chars().take(64).collect();
        let network = throttle::network(client.ip);
        let attempt = self.attempts.reserve(&network)?;
        self.refuse_if_locked(&username, &network, true).await?;
        self.slow_down_if_targeted(&username).await?;

        let user = {
            let username = username.clone();
            self.db.read(move |conn| users::find(conn, &username)).await?
        };
        let verdict = self
            .passwords
            .verify(
                password.clone(),
                user.as_ref().map(|user| user.password_hash.clone()),
            )
            .await?;
        let now = self.clock.now();
        let user = match user {
            Some(user) if verdict.matches => user,
            _ => {
                self.record_failure(&username, &network).await?;
                return Err(ErrorCode::InvalidCredentials.into());
            }
        };
        if user.must_change_password && user.password_expires_at.is_none_or(|at| at <= now) {
            self.record_failure(&username, &network).await?;
            return Err(ErrorCode::TemporaryPasswordExpired.into());
        }
        let rehash = if verdict.outdated {
            Some(self.passwords.hash(password).await?)
        } else {
            None
        };

        let client = client.clone();
        let signed_in = self
            .db
            .write(move |tx| -> Result<_, Error> {
                // The password was checked outside this transaction. A password change or a
                // sign-out everywhere since then must not let a session in on the old check.
                let mut user = users::get(tx, user.id)?
                    .filter(|current| {
                        current.password_hash == user.password_hash && current.auth_epoch == user.auth_epoch
                    })
                    .ok_or(ErrorCode::InvalidCredentials)?;
                clear_failures(tx, &username, &network, now)?;
                if let Some(hash) = rehash {
                    user.password_hash = hash;
                }
                user.last_login_at = Some(now);
                users::update(tx, &user)?;
                let signed_in = open(tx, &user, &client, now)?;
                Record::signed_in(&user, &client, AuditAction::LoginSucceeded)
                    .target(&user.username)
                    .details(json!({ "sessionId": signed_in.session_id }))
                    .write(tx, now)?;
                Ok(signed_in)
            })
            .await?;
        attempt.succeeded();
        Ok(signed_in)
    }

    /// Finds the session behind a request. It takes both secrets: the cookie's token, which other
    /// ports of the host can see, and the session key, which only the panel's origin holds.
    pub async fn authenticate(&self, token: &str, key: &str, client: Client) -> Result<Caller, Error> {
        let (Some(token), Some(key)) = (secrets::from_hex::<32>(token), secrets::from_hex::<32>(key)) else {
            return Err(ErrorCode::Unauthenticated.into());
        };
        let (token_hash, key_hash) = (secrets::sha256(&token), secrets::sha256(&key));
        let now = self.clock.now();

        let caller = self
            .db
            .write(move |tx| -> Result<Option<Caller>, Error> {
                let Some(mut session) = sessions::find_by_token(tx, &token_hash)? else {
                    return Ok(None);
                };
                // A stolen cookie alone gets nowhere, and does not end the real session either.
                if !secrets::same(&session.key_hash, &key_hash) {
                    return Ok(None);
                }
                let user = users::get(tx, session.user_id)?.filter(|user| {
                    user.auth_epoch == session.auth_epoch
                        && now < session.last_seen_at + IDLE
                        && now < session.expires_at
                });
                let Some(user) = user else {
                    sessions::delete(tx, session.id)?;
                    return Ok(None);
                };
                if now >= session.last_seen_at + TOUCH_EVERY {
                    session.last_seen_at = now;
                    sessions::update(tx, &session)?;
                }
                Ok(Some(Caller::new(&session, &user, client)))
            })
            .await?;
        caller.ok_or_else(|| ErrorCode::Unauthenticated.into())
    }

    /// Refuses a caller who lacks `permission`. Refusing a sensitive or critical change is
    /// audited: those are the attempts worth knowing about. `path` is the route as the table
    /// declares it, never the one a client typed.
    pub async fn authorize(
        &self,
        caller: &Caller,
        permission: Permissions,
        method: &str,
        path: &str,
    ) -> Result<(), Error> {
        if caller.permissions.contains(permission) {
            return Ok(());
        }
        let info = permission.iter().next();
        let key = info.map_or("", |info| info.key);
        let mutation = !matches!(method, "GET" | "HEAD");
        if mutation && info.is_some_and(|info| info.danger >= Danger::Sensitive) {
            let route = format!("{method} {path}");
            let record = Record::by(caller, AuditAction::PermissionDenied)
                .target(&route)
                .outcome(Outcome::Denied)
                .details(json!({ "permission": key }));
            self.record_denial(caller, route, record).await?;
        }
        Err(ApiError::new(ErrorCode::Forbidden).with("permission", key).into())
    }

    /// Refuses a caller who has not entered their password in the last ten minutes.
    pub fn require_reauthentication(&self, caller: &Caller) -> Result<(), Error> {
        let now = self.clock.now();
        if caller
            .reauthenticated_at
            .is_some_and(|at| now < at + REAUTHENTICATION)
        {
            Ok(())
        } else {
            Err(ErrorCode::ReauthRequired.into())
        }
    }

    pub async fn reauthenticate(&self, caller: &Caller, password: String) -> Result<(), Error> {
        self.confirm_password(caller, password, AuditAction::Reauthenticated)
            .await?;
        let record = Record::by(caller, AuditAction::Reauthenticated).target(&caller.username);
        let (session_id, username) = (caller.session_id, caller.username.clone());
        let network = throttle::network(caller.client.ip);
        let now = self.clock.now();
        self.db
            .write(move |tx| {
                clear_failures(tx, &username, &network, now)?;
                let mut session = sessions::get(tx, session_id)?.ok_or(ErrorCode::Unauthenticated)?;
                session.reauthenticated_at = Some(now);
                sessions::update(tx, &session)?;
                record.write(tx, now)?;
                Ok(())
            })
            .await
    }

    pub async fn logout(&self, caller: &Caller) -> Result<(), Error> {
        let record = Record::by(caller, AuditAction::Logout).target(&caller.username);
        let session_id = caller.session_id;
        let now = self.clock.now();
        self.db
            .write(move |tx| {
                sessions::delete(tx, session_id)?;
                record.write(tx, now)?;
                Ok(())
            })
            .await
    }

    /// Changes the caller's own password. Every other session of theirs ends; this one stays, and
    /// counts as just re-authenticated.
    pub async fn change_password(&self, caller: &Caller, current: String, next: String) -> Result<(), Error> {
        passwords::check_new(&next)?;
        if next == current {
            return Err(ErrorCode::PasswordUnchanged.into());
        }
        self.confirm_password(caller, current, AuditAction::PasswordChanged)
            .await?;
        let hash = self.passwords.hash(next).await?;

        let record = Record::by(caller, AuditAction::PasswordChanged).target(&caller.username);
        let (user_id, session_id) = (caller.user_id, caller.session_id);
        let now = self.clock.now();
        self.db
            .write(move |tx| {
                let mut user = existing(tx, user_id)?;
                user.password_hash = hash;
                user.must_change_password = false;
                user.password_expires_at = None;
                user.auth_epoch += 1;
                user.version += 1;
                users::update(tx, &user)?;

                let mut session = sessions::get(tx, session_id)?.ok_or(ErrorCode::Unauthenticated)?;
                session.auth_epoch = user.auth_epoch;
                session.reauthenticated_at = Some(now);
                sessions::update(tx, &session)?;
                sessions::delete_for_user(tx, user_id, Some(session_id))?;
                record.write(tx, now)?;
                Ok(())
            })
            .await
    }

    /// The caller's live sessions, the most recently used first.
    pub async fn sessions(&self, caller: &Caller) -> Result<Vec<AccountSession>, Error> {
        let (user_id, current) = (caller.user_id, caller.session_id);
        let now = self.clock.now();
        let rows = self
            .db
            .read(move |conn| sessions::list_for_user(conn, user_id))
            .await?;
        Ok(rows
            .into_iter()
            .map(|session| AccountSession {
                id: session.id,
                created_at: session.created_at,
                last_seen_at: session.last_seen_at,
                expires_at: session.expires_at.min(session.last_seen_at + IDLE),
                ip: session.ip,
                user_agent: session.user_agent,
                current: session.id == current,
            })
            .filter(|session| session.expires_at > now)
            .collect())
    }

    /// Ends one of the caller's own sessions; anyone else's is not found.
    pub async fn revoke_session(&self, caller: &Caller, id: i64) -> Result<(), Error> {
        let record = Record::by(caller, AuditAction::SessionRevoked)
            .target(&caller.username)
            .details(json!({ "sessionId": id }));
        let user_id = caller.user_id;
        let now = self.clock.now();
        self.db
            .write(move |tx| {
                match sessions::get(tx, id)? {
                    Some(session) if session.user_id == user_id => sessions::delete(tx, id)?,
                    _ => return Err(ErrorCode::NotFound.into()),
                };
                record.write(tx, now)?;
                Ok(())
            })
            .await
    }

    /// Checks the caller's password, throttled like a login. A wrong one is audited as a failed
    /// `attempted`, in the caller's name: they are signed in, not anonymous.
    async fn confirm_password(
        &self,
        caller: &Caller,
        password: String,
        attempted: AuditAction,
    ) -> Result<(), Error> {
        let network = throttle::network(caller.client.ip);
        let attempt = self.attempts.reserve(&network)?;
        // Their failures are in the audit log already, each in their name.
        self.refuse_if_locked(&caller.username, &network, false).await?;

        let user_id = caller.user_id;
        let hash = self
            .db
            .read(move |conn| Ok::<_, Error>(users::get(conn, user_id)?.map(|user| user.password_hash)))
            .await?;
        if hash.is_none() {
            return Err(ErrorCode::Unauthenticated.into());
        }
        if !self.passwords.verify(password, hash).await?.matches {
            let record = Record::by(caller, attempted)
                .target(&caller.username)
                .outcome(Outcome::Failure);
            let username = caller.username.clone();
            let now = self.clock.now();
            self.db
                .write(move |tx| -> Result<_, Error> {
                    let row = login_failures::get(tx, &username, &network)?;
                    let row = throttle::record_confirmation_failure(row, &username, &network, now);
                    login_failures::save(tx, &row)?;
                    record.write(tx, now)?;
                    Ok(())
                })
                .await?;
            return Err(ErrorCode::WrongPassword.into());
        }
        attempt.succeeded();
        Ok(())
    }

    /// Refuses an attempt on a locked pair. The refusal counts among the pair's anonymous failures
    /// for the audit log when `audited` is set.
    async fn refuse_if_locked(&self, username: &str, network: &str, audited: bool) -> Result<(), Error> {
        let (username, network) = (username.to_owned(), network.to_owned());
        let now = self.clock.now();
        let wait = self
            .db
            .write(move |tx| -> Result<_, Error> {
                let Some(row) = login_failures::get(tx, &username, &network)? else {
                    return Ok(None);
                };
                let Some(wait) = throttle::locked_for(&row, now) else {
                    return Ok(None);
                };
                if !audited {
                    return Ok(Some(wait));
                }
                let (row, closed) = throttle::record_refused(row, now);
                if let Some(failures) = closed {
                    Record::login_failures(tx, &failures)?.write(tx, now)?;
                }
                login_failures::save(tx, &row)?;
                Ok(Some(wait))
            })
            .await?;
        match wait {
            Some(wait) => Err(throttle::too_many_attempts(wait)),
            None => Ok(()),
        }
    }

    async fn slow_down_if_targeted(&self, username: &str) -> Result<(), Error> {
        let username = username.to_owned();
        let now = self.clock.now();
        let rows = self
            .db
            .read(move |conn| login_failures::for_username(conn, &username))
            .await?;
        if let Some(pause) = throttle::account_slowdown(&rows, now) {
            tokio::time::sleep(pause).await;
        }
        Ok(())
    }

    async fn record_failure(&self, username: &str, network: &str) -> Result<(), Error> {
        let (username, network) = (username.to_owned(), network.to_owned());
        let now = self.clock.now();
        self.db
            .write(move |tx| {
                let row = login_failures::get(tx, &username, &network)?;
                let (row, closed) = throttle::record_failure(row, &username, &network, now);
                if let Some(failures) = closed {
                    Record::login_failures(tx, &failures)?.write(tx, now)?;
                }
                login_failures::save(tx, &row)?;
                Ok(())
            })
            .await
    }
}
