use jiff::{SignedDuration, Timestamp};
use observer_domain::audit::{ActorKind, AuditAction, AuditEntry, AuditFilter, AuditPage, Outcome};
use observer_store::audit::{self, NewAuditEntry};
use observer_store::users::UserRow;
use observer_store::{Connection, Database, StoreError, settings};
use serde_json::{Value, json};

use crate::throttle::Unaudited;
use crate::{Caller, Client, Clock, Error};

const DEFAULT_RETENTION_DAYS: i64 = 365;
const MIN_RETENTION_DAYS: i64 = 90;
const DEFAULT_PAGE: u32 = 50;
const MAX_PAGE: u32 = 200;
const EXPORT_BATCH: u32 = 500;

/// One audit entry in the making. It is written in the transaction of the change it records, so
/// the change and its trace commit or fail together.
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

    /// The failures of one closed window, as a single entry.
    pub(crate) fn login_failures(failures: &Unaudited) -> Self {
        Self {
            ip: Some(failures.network.clone()),
            target: Some(failures.username.clone()),
            outcome: Outcome::Failure,
            details: json!({
                "username": failures.username,
                "network": failures.network,
                "failures": failures.failures,
                "firstAt": failures.first,
                "lastAt": failures.last,
            }),
            ..Self::new(AuditAction::LoginFailed, ActorKind::Anonymous)
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

/// The owner sets it; it never goes under the floor, whatever is stored.
fn retention(conn: &Connection) -> Result<SignedDuration, StoreError> {
    let days = settings::get(conn, "audit_retention_days")?
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(DEFAULT_RETENTION_DAYS)
        .max(MIN_RETENTION_DAYS);
    Ok(SignedDuration::from_hours(days * 24))
}

/// Reading, exporting and pruning the audit log.
#[derive(Clone)]
pub struct AuditLog {
    db: Database,
    clock: Clock,
}

impl AuditLog {
    pub fn new(db: Database, clock: Clock) -> Self {
        Self { db, clock }
    }

    /// One page, newest first. The limit defaults to 50 and is capped at 200.
    pub async fn page(&self, filter: AuditFilter) -> Result<AuditPage, Error> {
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
        let record = Record::by(caller, AuditAction::AuditExported)
            .details(serde_json::to_value(filter).unwrap_or_default());
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
        let now = self.clock.now();
        Ok(self
            .db
            .write(move |tx| {
                audit::advance_cutoff(tx, now)?;
                audit::purge(tx)
            })
            .await?)
    }
}
