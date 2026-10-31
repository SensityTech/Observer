//! File access: the groups of rules a server's files are reached by, and who holds them. The
//! SFTP server asks this service who may log in and what each session may do; the panel's routes
//! ask it to change both.

use std::net::IpAddr;
use std::sync::Arc;

use observer_domain::audit::{AuditAction, Outcome};
use observer_domain::delegation::Account;
use observer_domain::error::{ApiError, ErrorCode};
use observer_domain::files::{self, AccessChange, AccessHolder, Reach, Rule};
use observer_store::files::{self as store, FileAccessRow, FileGroupRow};
use observer_store::login_failures::FILES;
use observer_store::{Connection, Database, StoreError, launch, users};
use serde_json::{Value, json};
use tokio::sync::watch;

use crate::audit::Record;
use crate::throttle;
use crate::{Accounts, Caller, Clock, Error};

/// The only server today.
pub const SERVER: &str = "default";
const MAX_GROUP_NAME: usize = 64;
const MAX_RULES: usize = 64;
/// A row of SFTP login failures with no lock and no failure for this long is forgotten.
const FORGET_FAILURES_AFTER: jiff::SignedDuration = jiff::SignedDuration::from_hours(24);

/// What a login came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Login {
    Granted(Grant),
    /// No such login, or not its key: never a failure.
    Refused,
    /// The key is the login's, but the access cannot log in now: counted toward the lock.
    Failed,
    Locked,
    /// No server folder is saved yet.
    NoServer,
}

/// A session's access, read once at its login.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    pub access_id: i64,
    pub key_id: i64,
    /// The account's id; `None` for a pipeline.
    pub user_id: Option<i64>,
    /// The username as stored, or `+name` for a pipeline.
    pub login: String,
    pub groups: Vec<(i64, Vec<Rule>)>,
    /// The server's data folder, as the launch settings hold it.
    pub root: String,
    pub max_rate: Option<u64>,
    pub daily_budget: Option<u64>,
}

#[derive(Clone)]
pub struct Files {
    db: Database,
    clock: Clock,
    accounts: Accounts,
    /// Bumped on every change to groups and accesses, so that live sessions check theirs again.
    changes: Arc<watch::Sender<()>>,
}

/// A group with what it reaches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub row: FileGroupRow,
    pub reach: Reach,
}

impl Files {
    pub fn new(db: Database, clock: Clock, accounts: Accounts) -> Self {
        Self {
            db,
            clock,
            accounts,
            changes: Arc::new(watch::Sender::new(())),
        }
    }

    /// Changes to groups and accesses, for the sessions they may end.
    pub fn changes(&self) -> watch::Receiver<()> {
        self.changes.subscribe()
    }

    pub async fn groups(&self) -> Result<Vec<Group>, Error> {
        let rows = self.db.read(|conn| store::groups(conn, SERVER)).await?;
        Ok(rows.into_iter().map(group).collect())
    }

    /// Creates a group. Rules decide which parts of the host are reachable, so only the owner
    /// writes them.
    pub async fn create_group(
        &self,
        caller: &Caller,
        name: String,
        description: String,
        rules: Vec<Rule>,
    ) -> Result<Group, Error> {
        owner(caller)?;
        check_group(&name, &rules)?;
        let now = self.clock.now();
        let record = Record::by(caller, AuditAction::FileGroupCreated).target(name.clone());
        let id = self
            .db
            .write(move |tx| {
                let id = store::insert_group(tx, SERVER, &name, &description, &rules, now)?
                    .ok_or(ErrorCode::NameTaken)?;
                record
                    .details(json!({ "rules": rules, "reach": files::reach(&rules) }))
                    .write(tx, now)?;
                Ok::<_, Error>(id)
            })
            .await?;
        self.changed();
        self.group(id).await
    }

    pub async fn update_group(
        &self,
        caller: &Caller,
        id: i64,
        version: i64,
        name: String,
        description: String,
        rules: Vec<Rule>,
    ) -> Result<Group, Error> {
        owner(caller)?;
        check_group(&name, &rules)?;
        let now = self.clock.now();
        let record = Record::by(caller, AuditAction::FileGroupChanged).target(name.clone());
        self.db
            .write(move |tx| {
                let before = store::group(tx, id)?.ok_or(ErrorCode::NotFound)?;
                if name != before.name && store::groups(tx, SERVER)?.iter().any(|other| other.id != id && other.name.eq_ignore_ascii_case(&name)) {
                    return Err(ErrorCode::NameTaken.into());
                }
                if !store::update_group(tx, id, version, &name, &description, &rules, now)? {
                    return Err(ErrorCode::Conflict.into());
                }
                record
                    .details(json!({
                        "before": { "name": before.name, "rules": before.rules, "reach": files::reach(&before.rules) },
                        "after": { "name": name, "rules": rules, "reach": files::reach(&rules) },
                    }))
                    .write(tx, now)?;
                Ok::<_, Error>(())
            })
            .await?;
        self.changed();
        self.group(id).await
    }

    pub async fn delete_group(&self, caller: &Caller, id: i64) -> Result<(), Error> {
        owner(caller)?;
        let now = self.clock.now();
        let caller = caller.clone();
        self.db
            .write(move |tx| {
                let group = store::group(tx, id)?.ok_or(ErrorCode::NotFound)?;
                store::delete_group(tx, id)?;
                Record::by(&caller, AuditAction::FileGroupDeleted)
                    .target(group.name)
                    .details(json!({ "rules": group.rules, "reach": files::reach(&group.rules) }))
                    .write(tx, now)?;
                Ok::<_, Error>(())
            })
            .await?;
        self.changed();
        Ok(())
    }

    /// The groups an account holds, its access created empty the first time.
    pub async fn user_groups(&self, user_id: i64) -> Result<Vec<Group>, Error> {
        let rows = self
            .db
            .read(move |conn| match store::user_access(conn, user_id)? {
                Some(access) => store::access_groups(conn, access.id),
                None => Ok(Vec::new()),
            })
            .await?;
        Ok(rows.into_iter().map(group).collect())
    }

    /// Replaces the groups an account holds. A holder of `FILES_MANAGE` gives Read and Assets
    /// groups; a Code group, or a removal that lifts another group's denial, takes the owner.
    pub async fn set_user_groups(
        &self,
        caller: &Caller,
        user_id: i64,
        group_ids: Vec<i64>,
    ) -> Result<(), Error> {
        let now = self.clock.now();
        let editor = Account {
            id: caller.user_id,
            is_owner: caller.is_owner,
            permissions: caller.permissions,
        };
        let record = Record::by(caller, AuditAction::FileAccessChanged);
        let result = self
            .db
            .write(move |tx| {
                let user = users::get(tx, user_id)?.ok_or(ErrorCode::NotFound)?;
                let holder = AccessHolder {
                    user_id: user.id,
                    is_owner: user.is_owner,
                    permissions: user.permissions,
                };
                let access = store::user_access_or_new(tx, user_id, now)?;
                let before = store::access_groups(tx, access.id)?;
                let mut after = Vec::with_capacity(group_ids.len());
                for id in &group_ids {
                    let group = store::group(tx, *id)?.ok_or(ErrorCode::NotFound)?;
                    if !after.iter().any(|kept: &FileGroupRow| kept.id == group.id) {
                        after.push(group);
                    }
                }
                let added: Vec<&FileGroupRow> = after
                    .iter()
                    .filter(|group| !before.iter().any(|old| old.id == group.id))
                    .collect();
                let removed: Vec<&FileGroupRow> = before
                    .iter()
                    .filter(|group| !after.iter().any(|new| new.id == group.id))
                    .collect();
                let kept: Vec<&[Rule]> = after.iter().map(|group| group.rules.as_slice()).collect();
                let lifts = removed
                    .iter()
                    .any(|group| files::lifts_denial(&group.rules, &kept));
                let change = if added.is_empty() && !lifts {
                    AccessChange::TakeAway
                } else {
                    AccessChange::Give
                };
                files::may_manage(&editor, Some(&holder), change)?;
                if !editor.is_owner
                    && (lifts
                        || added
                            .iter()
                            .any(|group| files::reach(&group.rules) == Reach::Code))
                {
                    return Err(ErrorCode::OwnerOnly.into());
                }
                let ids: Vec<i64> = after.iter().map(|group| group.id).collect();
                store::set_access_groups(tx, access.id, &ids)?;
                record
                    .target(user.username)
                    .details(json!({
                        "groupsAdded": added.iter().map(|group| &group.name).collect::<Vec<_>>(),
                        "groupsRemoved": removed.iter().map(|group| &group.name).collect::<Vec<_>>(),
                    }))
                    .write(tx, now)?;
                Ok::<_, Error>(())
            })
            .await;
        match result {
            Ok(()) => {
                self.changed();
                Ok(())
            }
            Err(error) => self.audit_refusal(caller, user_id, error).await,
        }
    }

    /// The account's access, if it has one.
    pub async fn user_access(&self, user_id: i64) -> Result<Option<FileAccessRow>, Error> {
        Ok(self
            .db
            .read(move |conn| store::user_access(conn, user_id))
            .await?)
    }

    /// Whether `fingerprint` is one of the keys of the access `login` names. An offered key that is
    /// not is never a failure: clients offer every key they hold.
    pub async fn key_registered(&self, login: String, fingerprint: String) -> Result<bool, Error> {
        Ok(self
            .db
            .read(move |conn| {
                Ok::<_, StoreError>(match access_for_login(conn, &login)? {
                    Some((access, _)) => store::key(conn, access.id, &fingerprint)?.is_some(),
                    None => false,
                })
            })
            .await?)
    }

    /// Checks a login whose signature the SSH layer verified. A key registered to the login on an
    /// access that cannot log in now counts as a failure toward the pair's lock; anything else
    /// that refuses is not a failure.
    pub async fn login(&self, login: String, fingerprint: String, ip: IpAddr) -> Result<Login, Error> {
        let now = self.clock.now();
        let network = throttle::network(ip);
        self.db
            .write(move |tx| {
                if let Some(row) = FILES.get(tx, &login, &network)?
                    && throttle::locked_for(&row, now).is_some()
                {
                    return Ok(Login::Locked);
                }
                let Some((access, user)) = access_for_login(tx, &login)? else {
                    return Ok(Login::Refused);
                };
                let Some(key) = store::key(tx, access.id, &fingerprint)? else {
                    return Ok(Login::Refused);
                };
                let groups = store::access_groups(tx, access.id)?;
                let usable = access.enabled
                    && access.expires_at.is_none_or(|end| end > now)
                    && key.expires_at.is_none_or(|end| end > now)
                    && !groups.is_empty()
                    && files::network_allows(&access.networks, ip)
                    && user.as_ref().is_none_or(|user| !user.must_change_password);
                if !usable {
                    let row = FILES.get(tx, &login, &network)?;
                    let (row, closed) = throttle::record_failure(row, &login, &network, now);
                    if let Some(failures) = closed {
                        Record::file_login_failures(&failures).write(tx, now)?;
                    }
                    FILES.save(tx, &row)?;
                    return Ok(Login::Failed);
                }
                let Some(launch) = launch::get(tx, SERVER)? else {
                    return Ok(Login::NoServer);
                };
                store::record_login(tx, access.id, key.id, &ip.to_string(), now)?;
                Ok::<_, Error>(Login::Granted(Grant {
                    access_id: access.id,
                    key_id: key.id,
                    user_id: user.as_ref().map(|user| user.id),
                    login: user.map_or_else(|| login.clone(), |user| user.username),
                    groups: groups.into_iter().map(|group| (group.id, group.rules)).collect(),
                    root: launch.settings.data_path,
                    max_rate: access.max_rate,
                    daily_budget: access.daily_budget,
                }))
            })
            .await
    }

    /// Upkeep, run every minute: closed windows of failed SFTP logins go to the audit log, and
    /// quiet rows are forgotten.
    pub async fn sweep(&self) -> Result<(), Error> {
        let now = self.clock.now();
        self.db
            .write(move |tx| {
                for mut row in FILES.unaudited_since_before(tx, now - throttle::WINDOW)? {
                    if let Some(failures) = throttle::take_unaudited(&mut row) {
                        Record::file_login_failures(&failures).write(tx, now)?;
                        FILES.save(tx, &row)?;
                    }
                }
                FILES.delete_quiet(tx, now - FORGET_FAILURES_AFTER, now)?;
                Ok::<_, Error>(())
            })
            .await
    }

    async fn group(&self, id: i64) -> Result<Group, Error> {
        let row = self.db.read(move |conn| store::group(conn, id)).await?;
        row.map(group).ok_or_else(|| ErrorCode::NotFound.into())
    }

    fn changed(&self) {
        self.changes.send_replace(());
    }

    /// Passes a refusal through, after auditing it when the delegation or the owner's rules
    /// refused it.
    async fn audit_refusal<T>(&self, caller: &Caller, user_id: i64, error: Error) -> Result<T, Error> {
        let Error::Refused(refusal) = &error else {
            return Err(error);
        };
        if !matches!(
            refusal.code,
            ErrorCode::SelfEdit | ErrorCode::OwnerProtected | ErrorCode::Hierarchy | ErrorCode::OwnerOnly
        ) {
            return Err(error);
        }
        let name = self
            .db
            .read(move |conn| users::get(conn, user_id))
            .await?
            .map(|user| user.username);
        let mut details = refusal.params.clone();
        details.insert("attempted".to_owned(), json!(AuditAction::FileAccessChanged));
        details.insert("refusal".to_owned(), json!(refusal.code));
        details.insert("userId".to_owned(), json!(user_id));
        let mut record = Record::by(caller, AuditAction::PermissionDenied)
            .outcome(Outcome::Denied)
            .details(Value::Object(details));
        if let Some(name) = name {
            record = record.target(name);
        }
        self.accounts
            .record_denial(caller, format!("file_access_changed {user_id}"), record)
            .await?;
        Err(error)
    }
}

/// The access a login names, with its account: the username for a person, `+name` for a pipeline.
fn access_for_login(
    conn: &Connection,
    login: &str,
) -> Result<Option<(FileAccessRow, Option<users::UserRow>)>, StoreError> {
    if let Some(name) = login.strip_prefix('+') {
        return Ok(store::service_access(conn, name)?.map(|access| (access, None)));
    }
    let Some(user) = users::find(conn, login)? else {
        return Ok(None);
    };
    Ok(store::user_access(conn, user.id)?.map(|access| (access, Some(user))))
}

fn group(row: FileGroupRow) -> Group {
    let reach = files::reach(&row.rules);
    Group { row, reach }
}

fn owner(caller: &Caller) -> Result<(), Error> {
    if caller.is_owner {
        Ok(())
    } else {
        Err(ErrorCode::OwnerOnly.into())
    }
}

fn check_group(name: &str, rules: &[Rule]) -> Result<(), Error> {
    if name.trim().is_empty()
        || name.len() > MAX_GROUP_NAME
        || name.chars().any(char::is_control)
        || rules.len() > MAX_RULES
    {
        return Err(ErrorCode::InvalidRequest.into());
    }
    for (index, rule) in rules.iter().enumerate() {
        if let Err(field) = rule.check() {
            return Err(ApiError::new(ErrorCode::InvalidRule)
                .with("index", index)
                .with("field", field.key())
                .into());
        }
    }
    Ok(())
}
