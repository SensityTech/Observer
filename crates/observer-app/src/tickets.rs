//! One-time tickets that open the websocket. A browser cannot put a header on a websocket request,
//! so the session key cannot come along: the panel trades it for a ticket through an ordinary
//! request, then puts the ticket in the socket's URL. A ticket works once, for 30 seconds, and
//! only next to the cookie of the session it was issued to. It is never stored, only its hash
//! is, and only in memory.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use jiff::{SignedDuration, Timestamp};
use observer_domain::error::ErrorCode;
use observer_store::sessions;

use crate::sessions::live_user;
use crate::{Accounts, Caller, Client, Error, secrets};

const TICKET_LIFETIME: SignedDuration = SignedDuration::from_secs(30);
/// Each tab of the panel trades one for its socket as it connects: a session never needs more at
/// once than it may hold sockets.
const TICKETS_PER_SESSION: usize = 8;

#[derive(Default)]
pub(crate) struct Tickets {
    issued: Mutex<HashMap<[u8; 32], Ticket>>,
}

struct Ticket {
    session_id: i64,
    expires_at: Timestamp,
}

impl Accounts {
    pub fn issue_ticket(&self, caller: &Caller) -> Result<String, Error> {
        let ticket = secrets::random::<32>()?;
        let now = self.clock.now();
        let mut issued = self.tickets.issued.lock().unwrap_or_else(PoisonError::into_inner);
        // Nothing else forgets them: the map holds no more than 30 seconds of requests.
        issued.retain(|_, ticket| now < ticket.expires_at);
        let unspent = issued
            .values()
            .filter(|ticket| ticket.session_id == caller.session_id)
            .count();
        if unspent >= TICKETS_PER_SESSION {
            return Err(ErrorCode::TooManySockets.into());
        }
        issued.insert(
            secrets::sha256(&ticket),
            Ticket {
                session_id: caller.session_id,
                expires_at: now + TICKET_LIFETIME,
            },
        );
        Ok(secrets::hex(&ticket))
    }

    /// Spends `ticket` and returns the caller of the session it was issued to, which `token`, the
    /// session cookie that came with it, must name. The ticket is spent even when this fails.
    pub async fn redeem_ticket(&self, ticket: &str, token: &str, client: Client) -> Result<Caller, Error> {
        let (Some(ticket), Some(token)) = (secrets::from_hex::<32>(ticket), secrets::from_hex::<32>(token))
        else {
            return Err(ErrorCode::Unauthenticated.into());
        };
        let now = self.clock.now();
        let issued = self
            .tickets
            .issued
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&secrets::sha256(&ticket));
        let Some(issued) = issued.filter(|issued| now < issued.expires_at) else {
            return Err(ErrorCode::Unauthenticated.into());
        };

        let token_hash = secrets::sha256(&token);
        let caller = self
            .db
            .read(move |conn| -> Result<_, Error> {
                let session = sessions::find_by_token(conn, &token_hash)?
                    .filter(|session| session.id == issued.session_id);
                let Some(session) = session else {
                    return Ok(None);
                };
                let user = live_user(conn, &session, now)?;
                Ok(user.map(|user| Caller::new(&session, &user, client)))
            })
            .await?
            .ok_or(ErrorCode::Unauthenticated)?;
        if caller.must_change_password {
            return Err(ErrorCode::PasswordChangeRequired.into());
        }
        Ok(caller)
    }
}
