//! File access: the groups of rules a server's files are reached by, and who holds them. The
//! SFTP server asks this service who may log in and what each session may do; the panel's routes
//! ask it to change both.

use std::net::IpAddr;
use std::sync::Arc;

use observer_domain::audit::{ActorKind, AuditAction, Outcome};
use observer_domain::delegation::Account;
use observer_domain::error::{ApiError, ErrorCode};
use observer_domain::files::{
    self, AccessChange, AccessHolder, AccessKey, FileGroup, MyFileAccess, Reach, Rule,
};
use observer_store::files::{self as store, FileAccessRow, FileGroupRow, FileKeyRow, HostKeyRow};
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
/// The most keys one access holds.
const MAX_KEYS: usize = 10;
/// The key types accepted, by their label, with how their key data starts once base64-encoded:
/// the same name again. A plain label over a certificate's blob is refused before any parsing.
const KEY_TYPES: [(&str, &str); 3] = [
    ("ssh-ed25519", "AAAAC3NzaC1lZDI1NTE5"),
    ("ecdsa-sha2-nistp256", "AAAAE2VjZHNhLXNoYTItbmlzdHAyNTY"),
    ("ecdsa-sha2-nistp384", "AAAAE2VjZHNhLXNoYTItbmlzdHAzODQ"),
];
const MAX_KEY_LINE: usize = 8 * 1024;
const MAX_KEY_COMMENT: usize = 255;
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

    pub async fn host_key(&self) -> Result<Option<HostKeyRow>, Error> {
        Ok(self.db.read(store::host_key).await?)
    }

    /// Stores the SFTP server's key at its first start, and audits it. Returns the key in force,
    /// which is another start's when one stored a key first.
    pub async fn store_host_key(&self, key: HostKeyRow) -> Result<HostKeyRow, Error> {
        let now = self.clock.now();
        self.db
            .write(move |tx| {
                if store::insert_host_key(tx, &key)? {
                    Record::operator(ActorKind::System, AuditAction::FileHostKeyCreated)
                        .details(json!({ "fingerprint": key.fingerprint }))
                        .write(tx, now)?;
                    return Ok(key);
                }
                store::host_key(tx)?.ok_or_else(|| StoreError::Sqlite(rusqlite_missing()).into())
            })
            .await
    }

    /// Records a temporary upload file before it is created, so that a crash leaves nothing behind
    /// the next start does not remove.
    pub async fn record_upload(&self, root: String, folder: String, name: String) -> Result<i64, Error> {
        let now = self.clock.now();
        Ok(self
            .db
            .write(move |tx| store::record_upload(tx, SERVER, &root, &folder, &name, now))
            .await?)
    }

    pub async fn forget_upload(&self, id: i64) -> Result<(), Error> {
        Ok(self.db.write(move |tx| store::forget_upload(tx, id)).await?)
    }

    /// The temporary files a crash may have left, for the start's sweep.
    pub async fn uploads(&self) -> Result<Vec<store::UploadRow>, Error> {
        Ok(self.db.read(store::uploads).await?)
    }

    /// Writes one audit entry for what a session did or was refused.
    pub async fn record_session(
        &self,
        grant: &Grant,
        ip: String,
        action: AuditAction,
        outcome: Outcome,
        target: String,
        details: Value,
    ) -> Result<(), Error> {
        let now = self.clock.now();
        let record = Record::file_session(grant.user_id, grant.access_id, &grant.login, &ip, action)
            .outcome(outcome)
            .target(target)
            .details(details);
        Ok(self.db.write(move |tx| record.write(tx, now)).await?)
    }

    /// Whether a session may go on as it logged in: the same access, key and groups, still
    /// usable. Any change ends the session rather than changing its rights mid-transfer.
    pub async fn still_granted(&self, grant: &Grant, ip: IpAddr) -> Result<bool, Error> {
        let now = self.clock.now();
        let grant = grant.clone();
        Ok(self
            .db
            .read(move |conn| {
                let Some(access) = store::access(conn, grant.access_id)? else {
                    return Ok::<_, StoreError>(false);
                };
                let Some(key) = store::keys(conn, access.id)?
                    .into_iter()
                    .find(|key| key.id == grant.key_id)
                else {
                    return Ok(false);
                };
                let groups: Vec<(i64, Vec<Rule>)> = store::access_groups(conn, access.id)?
                    .into_iter()
                    .map(|group| (group.id, group.rules))
                    .collect();
                let user_usable = match access.user_id {
                    Some(id) => users::get(conn, id)?.is_some_and(|user| !user.must_change_password),
                    None => true,
                };
                let root = launch::get(conn, SERVER)?.map(|launch| launch.settings.data_path);
                Ok(access.enabled
                    && access.expires_at.is_none_or(|end| end > now)
                    && key.expires_at.is_none_or(|end| end > now)
                    && files::network_allows(&access.networks, ip)
                    && user_usable
                    && groups == grant.groups
                    && root.as_deref() == Some(grant.root.as_str()))
            })
            .await?)
    }

    /// The signed-in account's login, keys and groups.
    pub async fn my_access(&self, caller: &Caller) -> Result<MyFileAccess, Error> {
        let user_id = caller.user_id;
        let (keys, groups) = self
            .db
            .read(move |conn| match store::user_access(conn, user_id)? {
                Some(access) => Ok::<_, StoreError>((
                    store::keys(conn, access.id)?,
                    store::access_groups(conn, access.id)?,
                )),
                None => Ok((Vec::new(), Vec::new())),
            })
            .await?;
        Ok(MyFileAccess {
            login: caller.username.clone(),
            keys: keys.into_iter().map(access_key).collect(),
            groups: groups.into_iter().map(|row| file_group(group(row))).collect(),
        })
    }

    /// Registers a key to the signed-in account. Only its holder adds one: a key someone else
    /// added could log in as them.
    pub async fn add_own_key(&self, caller: &Caller, line: &str) -> Result<AccessKey, Error> {
        let parsed = parse_key(line)?;
        let now = self.clock.now();
        let user_id = caller.user_id;
        let record = Record::by(caller, AuditAction::FileKeyAdded).target(caller.username.clone());
        let key = self
            .db
            .write(move |tx| {
                let access = store::user_access_or_new(tx, user_id, now)?;
                if store::keys(tx, access.id)?.len() >= MAX_KEYS {
                    return Err(ApiError::new(ErrorCode::TooManyKeys).with("max", MAX_KEYS).into());
                }
                let mut key = FileKeyRow {
                    id: 0,
                    access_id: access.id,
                    algorithm: parsed.algorithm,
                    public_key: parsed.blob,
                    fingerprint: parsed.fingerprint,
                    comment: parsed.comment,
                    created_at: now,
                    expires_at: None,
                    last_used_at: None,
                    last_used_ip: None,
                };
                key.id = store::insert_key(tx, &key)?.ok_or(ErrorCode::KeyInUse)?;
                record
                    .details(json!({ "fingerprint": key.fingerprint, "algorithm": key.algorithm, "comment": key.comment }))
                    .write(tx, now)?;
                Ok::<_, Error>(key)
            })
            .await?;
        self.changed();
        Ok(access_key(key))
    }

    pub async fn remove_own_key(&self, caller: &Caller, key_id: i64) -> Result<(), Error> {
        let now = self.clock.now();
        let user_id = caller.user_id;
        let record = Record::by(caller, AuditAction::FileKeyRemoved).target(caller.username.clone());
        self.db
            .write(move |tx| {
                let access = store::user_access(tx, user_id)?.ok_or(ErrorCode::NotFound)?;
                let key = store::delete_key(tx, access.id, key_id)?.ok_or(ErrorCode::NotFound)?;
                record
                    .details(json!({ "fingerprint": key.fingerprint, "algorithm": key.algorithm, "comment": key.comment }))
                    .write(tx, now)?;
                Ok::<_, Error>(())
            })
            .await?;
        self.changed();
        Ok(())
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

/// A pasted public key, checked.
struct ParsedKey {
    algorithm: String,
    blob: Vec<u8>,
    fingerprint: String,
    comment: String,
}

fn invalid_key(reason: &str) -> Error {
    ApiError::new(ErrorCode::InvalidPublicKey)
        .with("reason", reason)
        .into()
}

/// Reads one line of an OpenSSH `.pub` file: `type base64 [comment]`.
fn parse_key(line: &str) -> Result<ParsedKey, Error> {
    let line = line.trim();
    if line.is_empty() || line.len() > MAX_KEY_LINE || line.contains(['\n', '\r']) {
        return Err(invalid_key("format"));
    }
    let mut parts = line.splitn(3, char::is_whitespace);
    let label = parts.next().unwrap_or_default();
    let data = parts.next().unwrap_or_default().trim();
    if label.contains("-cert-v01@") {
        return Err(invalid_key("certificate"));
    }
    let Some((_, prefix)) = KEY_TYPES.iter().find(|(known, _)| *known == label) else {
        let ssh_like = label.starts_with("ssh-") || label.starts_with("ecdsa-") || label.starts_with("sk-");
        return Err(invalid_key(if ssh_like { "algorithm" } else { "format" }));
    };
    if !data.starts_with(prefix) {
        // The data names another type than the label: a certificate's blob behind a plain label
        // is refused here, before the parser sees it.
        return Err(invalid_key("certificate"));
    }
    let key =
        ssh_key::PublicKey::from_openssh(&format!("{label} {data}")).map_err(|_| invalid_key("format"))?;
    let blob = key.to_bytes().map_err(|_| invalid_key("format"))?;
    let comment: String = parts
        .next()
        .unwrap_or_default()
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_KEY_COMMENT)
        .collect();
    Ok(ParsedKey {
        algorithm: label.to_owned(),
        blob,
        fingerprint: key.fingerprint(ssh_key::HashAlg::Sha256).to_string(),
        comment,
    })
}

fn access_key(row: FileKeyRow) -> AccessKey {
    AccessKey {
        id: row.id,
        algorithm: row.algorithm,
        fingerprint: row.fingerprint,
        comment: row.comment,
        created_at: row.created_at,
        expires_at: row.expires_at,
        last_used_at: row.last_used_at,
        last_used_ip: row.last_used_ip,
    }
}

/// A group as the panel shows it.
pub fn file_group(group: Group) -> FileGroup {
    FileGroup {
        id: group.row.id,
        name: group.row.name,
        description: group.row.description,
        rules: group.row.rules,
        reach: group.reach,
        version: group.row.version,
    }
}

fn rusqlite_missing() -> observer_store::RusqliteError {
    observer_store::RusqliteError::QueryReturnedNoRows
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

#[cfg(test)]
mod tests {
    use ssh_key::private::{Ed25519Keypair, KeypairData};

    use super::*;

    fn line(seed: u8) -> String {
        let key = ssh_key::PrivateKey::new(
            KeypairData::Ed25519(Ed25519Keypair::from_seed(&[seed; 32])),
            "ada@laptop",
        )
        .unwrap();
        key.public_key().to_openssh().unwrap()
    }

    fn reason(result: Result<ParsedKey, Error>) -> String {
        match result {
            Err(Error::Refused(refusal)) => refusal.params["reason"].as_str().unwrap().to_owned(),
            other => panic!("{:?}", other.map(|key| key.fingerprint)),
        }
    }

    #[test]
    fn a_pasted_key_is_one_plain_line_of_an_accepted_type() {
        let parsed = parse_key(&format!("  {}  \n", line(1))).unwrap();
        assert_eq!(parsed.algorithm, "ssh-ed25519");
        assert_eq!(parsed.comment, "ada@laptop");
        assert!(parsed.fingerprint.starts_with("SHA256:"));

        let data = line(1).split_whitespace().nth(1).unwrap().to_owned();
        assert_eq!(
            reason(parse_key(&format!("ssh-ed25519-cert-v01@openssh.com {data}"))),
            "certificate"
        );
        assert_eq!(
            reason(parse_key(&format!("ecdsa-sha2-nistp256 {data}"))),
            "certificate"
        );
        assert_eq!(reason(parse_key("ssh-rsa AAAAB3NzaC1yc2E= old")), "algorithm");
        assert_eq!(reason(parse_key("-----BEGIN OPENSSH PRIVATE KEY-----")), "format");
        assert_eq!(reason(parse_key(&format!("ssh-ed25519 {data}x"))), "format");
        assert_eq!(reason(parse_key(&format!("{}\n{}", line(1), line(2)))), "format");
    }
}
