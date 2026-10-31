use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use jiff::{SignedDuration, Timestamp};
use observer_domain::audit::{
    ActorKind, AuditAction, AuditEntry, AuditFilter, AuditPage, Outcome, StopSource,
};
use observer_domain::error::ErrorCode;
use observer_store::audit::{self, NewAuditEntry};
use observer_store::users::{self, UserRow};
use observer_store::{Connection, Database, StoreError, settings};
use serde_json::{Value, json};
use tokio::sync::broadcast;

use crate::accounts::MAX_USERNAME_BYTES;
use crate::throttle::{self, Unaudited};
use crate::{Caller, Client, Clock, Error};

const DEFAULT_RETENTION_DAYS: i64 = 365;
const MIN_RETENTION_DAYS: i64 = 90;
const DEFAULT_PAGE: u32 = 50;
const MAX_PAGE: u32 = 200;
const EXPORT_BATCH: u32 = 500;
/// How many new entries a follower may fall behind by before it misses some.
const FOLLOWERS_BACKLOG: usize = 256;
/// The purge cutoff stays this far behind Observer's clock. SQLite refuses a cutoff past its own
/// present, which it reads from a coarser clock on Windows, a few milliseconds behind.
const CUTOFF_MARGIN: SignedDuration = SignedDuration::from_secs(1);

/// One audit entry in the making. It is written in the transaction of the change it records, so
/// the change and its trace commit or fail together.
#[derive(Clone)]
pub(crate) struct Record {
    actor_kind: ActorKind,
    actor_id: Option<i64>,
    actor_name: Option<String>,
    ip: Option<String>,
    user_agent: Option<String>,
    action: AuditAction,
    target: Option<String>,
    outcome: Outcome,
    details: Value,
}

impl Record {
    fn new(action: AuditAction, actor_kind: ActorKind) -> Self {
        Self {
            actor_kind,
            actor_id: None,
            actor_name: None,
            ip: None,
            user_agent: None,
            action,
            target: None,
            outcome: Outcome::Success,
            details: json!({}),
        }
    }

    pub(crate) fn by(caller: &Caller, action: AuditAction) -> Self {
        Self::user(caller.user_id, &caller.username, &caller.client, action)
    }

    /// For the moment a user signs in, before there is a caller.
    pub(crate) fn signed_in(user: &UserRow, client: &Client, action: AuditAction) -> Self {
        Self::user(user.id, &user.username, client, action)
    }

    pub(crate) fn operator(kind: ActorKind, action: AuditAction) -> Self {
        Self::new(action, kind)
    }

    /// The failures of one closed window, as a single entry. A username that names no account
    /// is left out: it is as likely to be a password typed in the wrong field, and the audit log
    /// keeps no secret.
    pub(crate) fn login_failures(conn: &Connection, failures: &Unaudited) -> Result<Self, StoreError> {
        let mut details = json!({
            "network": failures.network,
            "failures": failures.failures,
            "firstAt": failures.first,
            "lastAt": failures.last,
        });
        let account = users::find(conn, &failures.username)?.map(|user| user.username);
        if let Some(username) = &account {
            details["username"] = json!(username);
        }
        Ok(Self {
            ip: Some(failures.network.clone()),
            target: account,
            outcome: Outcome::Failure,
            details,
            ..Self::new(AuditAction::LoginFailed, ActorKind::Anonymous)
        })
    }

    /// An entry by an SFTP session: its account, or its pipeline as a `service` actor named
    /// `+name` with the access's id.
    pub(crate) fn file_session(
        user_id: Option<i64>,
        access_id: i64,
        login: &str,
        ip: &str,
        action: AuditAction,
    ) -> Self {
        let (kind, id) = match user_id {
            Some(id) => (ActorKind::User, id),
            None => (ActorKind::Service, access_id),
        };
        Self {
            actor_id: Some(id),
            actor_name: Some(login.to_owned()),
            ip: Some(ip.to_owned()),
            ..Self::new(action, kind)
        }
    }

    /// The SFTP failures of one closed window, as a single entry naming the login tried.
    pub(crate) fn file_login_failures(failures: &Unaudited) -> Self {
        Self {
            ip: Some(failures.network.clone()),
            target: Some(failures.username.clone()),
            outcome: Outcome::Failure,
            details: json!({
                "network": failures.network,
                "failures": failures.failures,
                "firstAt": failures.first,
                "lastAt": failures.last,
            }),
            ..Self::new(AuditAction::FileLoginFailed, ActorKind::Anonymous)
        }
    }

    fn user(id: i64, name: &str, client: &Client, action: AuditAction) -> Self {
        Self {
            actor_id: Some(id),
            actor_name: Some(name.to_owned()),
            ip: Some(client.ip.to_string()),
            user_agent: client.user_agent.clone(),
            ..Self::new(action, ActorKind::User)
        }
    }

    pub(crate) fn target(mut self, target: impl Into<String>) -> Self {
        self.target = Some(target.into());
        self
    }

    pub(crate) fn outcome(mut self, outcome: Outcome) -> Self {
        self.outcome = outcome;
        self
    }

    pub(crate) fn details(mut self, details: Value) -> Self {
        self.details = details;
        self
    }

    /// Writes the entry, kept for the retention in force now: a later change to the retention
    /// does not apply to it.
    pub(crate) fn write(self, conn: &Connection, now: Timestamp) -> Result<(), StoreError> {
        let expires_at = now + retention(conn)?;
        audit::insert(
            conn,
            &NewAuditEntry {
                at: now,
                actor_kind: self.actor_kind,
                actor_id: self.actor_id,
                actor_name: self.actor_name,
                ip: self.ip,
                user_agent: self.user_agent,
                action: self.action,
                target: self.target,
                outcome: self.outcome,
                details: self.details,
                expires_at,
            },
        )?;
        Ok(())
    }
}

/// Refusals come in bursts when someone probes. The first refusal of a caller on one action is
/// written at once; the ones that follow within the window are counted, and the count is written
/// when the window closes. The log then grows with the number of things tried, not of attempts.
#[derive(Default)]
pub(crate) struct Denials {
    bursts: Mutex<HashMap<(i64, String), Burst>>,
}

struct Burst {
    first: Record,
    since: Timestamp,
    repeated: u32,
    last: Timestamp,
}

impl Burst {
    /// The entry for the refusals that followed the first, if any did.
    fn repeats(&self) -> Option<Record> {
        (self.repeated > 0).then(|| {
            let mut record = self.first.clone();
            if let Some(details) = record.details.as_object_mut() {
                details.insert("repeated".to_owned(), json!(self.repeated));
                details.insert("lastAt".to_owned(), json!(self.last));
            }
            record
        })
    }
}

impl Denials {
    /// The entries to write when `user_id` is refused `action`: none while a window for the pair
    /// is open, else the refusal itself, after the count of the window it replaces.
    pub(crate) fn refuse(&self, user_id: i64, action: String, record: Record, now: Timestamp) -> Vec<Record> {
        let mut bursts = self.bursts.lock().unwrap_or_else(PoisonError::into_inner);
        let key = (user_id, action);
        let mut due = Vec::new();
        if let Some(burst) = bursts.get_mut(&key) {
            if now < burst.since + throttle::WINDOW {
                burst.repeated += 1;
                burst.last = now;
                return due;
            }
            due.extend(burst.repeats());
        }
        bursts.insert(
            key,
            Burst {
                first: record.clone(),
                since: now,
                repeated: 0,
                last: now,
            },
        );
        due.push(record);
        due
    }

    /// The counts of the windows closed by `now`, which are then forgotten.
    pub(crate) fn closed(&self, now: Timestamp) -> Vec<Record> {
        let mut bursts = self.bursts.lock().unwrap_or_else(PoisonError::into_inner);
        let mut due = Vec::new();
        bursts.retain(|_, burst| {
            let open = now < burst.since + throttle::WINDOW;
            if !open {
                due.extend(burst.repeats());
            }
            open
        });
        due
    }
}

/// A filter's actor is a username: one longer than any username matches nothing, and an export
/// would only carry it into the audit log.
fn check_filter(filter: &AuditFilter) -> Result<(), Error> {
    if filter
        .actor
        .as_ref()
        .is_some_and(|actor| actor.len() > MAX_USERNAME_BYTES)
    {
        Err(ErrorCode::InvalidRequest.into())
    } else {
        Ok(())
    }
}

/// The owner sets it; it never goes under the floor, whatever is stored.
fn retention(conn: &Connection) -> Result<SignedDuration, StoreError> {
    let days = settings::get(conn, "audit_retention_days")?
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(DEFAULT_RETENTION_DAYS)
        .max(MIN_RETENTION_DAYS);
    Ok(SignedDuration::from_hours(days * 24))
}

/// Reading, following, exporting and pruning the audit log.
#[derive(Clone)]
pub struct AuditLog {
    db: Database,
    clock: Clock,
    followers: broadcast::Sender<Arc<AuditEntry>>,
    /// The newest entry handed to the followers, once `publish_new` has looked.
    published: Arc<Mutex<Option<i64>>>,
}

impl AuditLog {
    pub fn new(db: Database, clock: Clock) -> Self {
        Self {
            db,
            clock,
            followers: broadcast::Sender::new(FOLLOWERS_BACKLOG),
            published: Arc::default(),
        }
    }

    /// The entries `publish_new` finds from now on. A follower that falls more than 256 entries
    /// behind is told it lagged, and reads the log again.
    pub fn follow(&self) -> broadcast::Receiver<Arc<AuditEntry>> {
        self.followers.subscribe()
    }

    /// The newest entry's id, or `None` while the log is empty.
    pub async fn latest_id(&self) -> Result<Option<i64>, Error> {
        Ok(self.db.read(audit::latest_id).await?)
    }

    /// Hands the entries written since the last call to the followers, and says how many there
    /// were. Every writer of the log is found this way, the command line included, which runs in
    /// another process. The first call only notes where the log ends.
    pub async fn publish_new(&self) -> Result<usize, Error> {
        let published = *self.published.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(mut after) = published else {
            let latest = self.latest_id().await?;
            *self.published.lock().unwrap_or_else(PoisonError::into_inner) = Some(latest.unwrap_or(0));
            return Ok(0);
        };
        let mut published = 0;
        loop {
            let entries = self
                .db
                .read(move |conn| audit::after(conn, after, EXPORT_BATCH))
                .await?;
            let Some(last) = entries.last() else {
                return Ok(published);
            };
            after = last.id;
            *self.published.lock().unwrap_or_else(PoisonError::into_inner) = Some(after);
            published += entries.len();
            let more = entries.len() == EXPORT_BATCH as usize;
            for entry in entries {
                // Nobody following is no failure: the entries are in the log for whoever reads it.
                let _ = self.followers.send(Arc::new(entry));
            }
            if !more {
                return Ok(published);
            }
        }
    }

    /// One page, newest first. The limit defaults to 50 and is capped at 200.
    pub async fn page(&self, filter: AuditFilter) -> Result<AuditPage, Error> {
        check_filter(&filter)?;
        let limit = filter.limit.unwrap_or(DEFAULT_PAGE).clamp(1, MAX_PAGE);
        let mut entries = self
            .db
            .read(move |conn| audit::page(conn, &filter, filter.before, limit + 1))
            .await?;
        let next = if entries.len() > limit as usize {
            entries.truncate(limit as usize);
            entries.last().map(|entry| entry.id)
        } else {
            None
        };
        Ok(AuditPage { entries, next })
    }

    /// Records an export before it starts, with the filters it uses.
    pub async fn record_export(&self, caller: &Caller, filter: &AuditFilter) -> Result<(), Error> {
        check_filter(filter)?;
        let record = Record::by(caller, AuditAction::AuditExported)
            .details(serde_json::to_value(filter).unwrap_or_default());
        let now = self.clock.now();
        self.db
            .write(move |tx| record.write(tx, now).map_err(Error::from))
            .await
    }

    /// Records that Observer is stopping, before FXServer is: someone at the host asked, from the
    /// window or the console, or a signal did.
    pub async fn record_stop(&self, from: StopSource) -> Result<(), Error> {
        let actor = match from {
            StopSource::Window | StopSource::Console => ActorKind::Local,
            StopSource::Signal => ActorKind::System,
        };
        let record = Record::operator(actor, AuditAction::ObserverStopped).details(json!({ "from": from }));
        let now = self.clock.now();
        self.db
            .write(move |tx| record.write(tx, now).map_err(Error::from))
            .await
    }

    /// The entries an export holds after `before`, in batches; an empty batch ends it. The
    /// filter's own `before` and `limit` do not apply.
    pub async fn export_batch(
        &self,
        filter: AuditFilter,
        before: Option<i64>,
    ) -> Result<Vec<AuditEntry>, Error> {
        Ok(self
            .db
            .read(move |conn| audit::page(conn, &filter, before, EXPORT_BATCH))
            .await?)
    }

    /// Deletes the entries whose retention has ended. Observer runs it daily.
    pub async fn purge(&self) -> Result<usize, Error> {
        let cutoff = self.clock.now() - CUTOFF_MARGIN;
        Ok(self
            .db
            .write(move |tx| {
                audit::advance_cutoff(tx, cutoff)?;
                audit::purge(tx)
            })
            .await?)
    }
}
