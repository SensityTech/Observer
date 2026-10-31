//! File access: the groups of rules a server's files are reached by, and who holds them. The
//! SFTP server asks this service who may log in and what each session may do; the panel's routes
//! ask it to change both.

use std::sync::Arc;

use observer_domain::audit::{AuditAction, Outcome};
use observer_domain::delegation::Account;
use observer_domain::error::{ApiError, ErrorCode};
use observer_domain::files::{self, AccessChange, AccessHolder, Reach, Rule};
use observer_store::files::{self as store, FileAccessRow, FileGroupRow};
use observer_store::{Database, users};
use serde_json::{Value, json};
use tokio::sync::watch;

use crate::audit::Record;
use crate::{Accounts, Caller, Clock, Error};

/// The only server today.
pub const SERVER: &str = "default";
const MAX_GROUP_NAME: usize = 64;
const MAX_RULES: usize = 64;

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
