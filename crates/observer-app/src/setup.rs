use jiff::SignedDuration;
use observer_domain::audit::{ActorKind, AuditAction};
use observer_domain::error::ErrorCode;
use observer_domain::permissions::Permissions;
use observer_store::setup_tokens;
use observer_store::users::{self, NewUser};

use crate::accounts::{Accounts, check_username, existing};
use crate::audit::Record;
use crate::sessions::{self, Client, SignedIn};
use crate::{Error, passwords, secrets, throttle};

const SETUP_TOKEN_LIFETIME: SignedDuration = SignedDuration::from_mins(30);
const SETUP_TOKEN_BYTES: usize = 20;

impl Accounts {
    /// Whether the owner still has to be created.
    pub async fn setup_required(&self) -> Result<bool, Error> {
        Ok(self.db.read(users::none).await?)
    }

    /// A fresh setup token, for the link that creates the owner, or `None` once an owner exists.
    /// Only its hash is stored, and any valid stored token works, so the command line can make one
    /// for a running Observer.
    pub async fn create_setup_token(&self, actor: ActorKind) -> Result<Option<String>, Error> {
        let token = secrets::random::<SETUP_TOKEN_BYTES>()?;
        let hash = secrets::sha256(&token);
        let now = self.clock.now();
        let created = self
            .db
            .write(move |tx| -> Result<_, Error> {
                if !users::none(tx)? {
                    return Ok(false);
                }
                setup_tokens::delete_expired(tx, now)?;
                setup_tokens::insert(tx, &hash, now, now + SETUP_TOKEN_LIFETIME)?;
                Record::operator(actor, AuditAction::SetupLinkCreated).write(tx, now)?;
                Ok(true)
            })
            .await?;
        Ok(created.then(|| secrets::base32(&token)))
    }

    /// Creates the owner with a setup token and signs them in. The token is spent in the same
    /// transaction, so a second attempt fails whatever the timing.
    pub async fn complete_setup(
        &self,
        token: &str,
        username: &str,
        password: String,
        client: &Client,
    ) -> Result<SignedIn, Error> {
        check_username(username)?;
        passwords::check_new(&password)?;
        let hash = secrets::from_base32(token)
            .filter(|bytes| bytes.len() == SETUP_TOKEN_BYTES)
            .map(|bytes| secrets::sha256(&bytes))
            .ok_or(ErrorCode::InvalidSetupToken)?;
        let network = throttle::network(client.ip);
        let attempt = self.attempts.reserve(client.ip, false)?;

        // A wrong token is turned away before it costs a hash.
        let now = self.clock.now();
        self.db
            .read(move |conn| -> Result<(), Error> {
                if !users::none(conn)? {
                    Err(ErrorCode::AlreadySetUp.into())
                } else if !setup_tokens::is_valid(conn, &hash, now)? {
                    Err(ErrorCode::InvalidSetupToken.into())
                } else {
                    Ok(())
                }
            })
            .await?;
        let password_hash = self.passwords.hash(password).await?;

        let (username, client) = (username.to_owned(), client.clone());
        let now = self.clock.now();
        let signed_in = self
            .db
            .write(move |tx| -> Result<_, Error> {
                if !users::none(tx)? {
                    return Err(ErrorCode::AlreadySetUp.into());
                }
                if !setup_tokens::consume(tx, &hash, now)? {
                    return Err(ErrorCode::InvalidSetupToken.into());
                }
                setup_tokens::delete_all(tx)?;
                let id = users::insert(
                    tx,
                    &NewUser {
                        username: &username,
                        password_hash: &password_hash,
                        is_owner: true,
                        permissions: Permissions::NONE,
                        must_change_password: false,
                        password_expires_at: None,
                        created_at: now,
                    },
                )?
                .ok_or(ErrorCode::NameTaken)?;
                let mut owner = existing(tx, id)?;
                owner.last_login_at = Some(now);
                users::update(tx, &owner)?;
                let signed_in = sessions::open(tx, &owner, &client, now)?;
                Record::signed_in(&owner, &client, AuditAction::SetupCompleted)
                    .target(&owner.username)
                    .write(tx, now)?;
                Ok(signed_in)
            })
            .await?;
        attempt.succeeded();
        self.attempts.signed_in(&signed_in.me.username, &network);
        Ok(signed_in)
    }
}
