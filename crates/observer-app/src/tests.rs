//! The services end to end, over a real database in a temporary directory.

use jiff::{SignedDuration, Timestamp};
use observer_domain::audit::{ActorKind, AuditAction, AuditEntry, AuditFilter, Outcome};
use observer_domain::error::ErrorCode;
use observer_domain::launch::{ExtraArgument, LaunchSettings, OneSync, StoredLaunch};
use observer_domain::permissions::Permissions as P;
use observer_store::{Database, StoreError, users};
use serde_json::json;
use tempfile::TempDir;
use tokio::task::JoinHandle;

use crate::{Accounts, AuditLog, Caller, Client, Clock, PasswordCost, Servers, SignedIn};

const OWNER_PASSWORD: &str = "correct horse battery staple";

/// A fresh Observer database, the services over it, and a clock that only moves when told. It
/// starts in the past, since the audit cutoff may never pass the real present.
struct Lab {
    _dir: TempDir,
    db: Database,
    clock: Clock,
    accounts: Accounts,
    audit: AuditLog,
    servers: Servers,
}

/// A signed-in browser: the session's two secrets and where it connects from.
#[derive(Debug)]
struct Browser {
    signed_in: SignedIn,
    client: Client,
}

impl Lab {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path().join("observer.sqlite3"), dir.path().join("backups"))
            .await
            .unwrap();
        let clock = Clock::manual("2025-01-06T09:00:00Z".parse().unwrap());
        let accounts = Accounts::new(db.clone(), clock.clone(), PasswordCost::TEST)
            .await
            .unwrap();
        let audit = AuditLog::new(db.clone(), clock.clone());
        let servers = Servers::new(db.clone(), clock.clone());
        Self {
            _dir: dir,
            db,
            clock,
            accounts,
            audit,
            servers,
        }
    }

    /// Creates the owner `ada` with a setup token, as the first run does.
    async fn owner(&self) -> Browser {
        let token = self
            .accounts
            .create_setup_token(ActorKind::System)
            .await
            .unwrap()
            .unwrap()
            .token;
        let signed_in = self
            .accounts
            .complete_setup(&token, "ada", OWNER_PASSWORD.to_owned(), &home())
            .await
            .unwrap();
        Browser {
            signed_in,
            client: home(),
        }
    }

    /// The caller behind the browser's next request.
    async fn caller(&self, browser: &Browser) -> Caller {
        self.authenticate(browser).await.unwrap()
    }

    async fn authenticate(&self, browser: &Browser) -> Result<Caller, ErrorCode> {
        self.accounts
            .authenticate(
                &browser.signed_in.token,
                &browser.signed_in.key,
                browser.client.clone(),
            )
            .await
            .map_err(|error| error.code().unwrap())
    }

    /// Creates a user from the owner's browser and has them replace their temporary password,
    /// signed in from the office.
    async fn user(&self, owner: &Browser, name: &str, permissions: P) -> Browser {
        let owner = self.caller(owner).await;
        let created = self
            .accounts
            .create_user(&owner, name, permissions)
            .await
            .unwrap();
        let browser = self
            .sign_in(name, &created.temporary_password, &office())
            .await
            .unwrap();
        let caller = self.caller(&browser).await;
        self.accounts
            .change_password(&caller, created.temporary_password, password_of(name))
            .await
            .unwrap();
        self.pause();
        browser
    }

    async fn sign_in(&self, name: &str, password: &str, client: &Client) -> Result<Browser, ErrorCode> {
        let signed_in = self
            .accounts
            .login(name, password.to_owned(), client)
            .await
            .map_err(|error| error.code().unwrap())?;
        Ok(Browser {
            signed_in,
            client: client.clone(),
        })
    }

    /// Starts a login and returns once its password check waits for a hashing slot, which the
    /// caller holds with `Passwords::hold`.
    async fn login_waiting(
        &self,
        name: &str,
        password: &str,
        client: Client,
    ) -> JoinHandle<Result<SignedIn, crate::Error>> {
        let accounts = self.accounts.clone();
        let (name, password) = (name.to_owned(), password.to_owned());
        let login = tokio::spawn(async move { accounts.login(&name, password, &client).await });
        while self.accounts.passwords.waiting() == 0 {
            tokio::task::yield_now().await;
        }
        login
    }

    async fn entries(&self, action: AuditAction) -> Vec<AuditEntry> {
        let filter = AuditFilter {
            action: Some(action),
            ..AuditFilter::default()
        };
        self.audit.page(filter).await.unwrap().entries
    }

    /// Lets the one-second pause a network takes after a failed or finished attempt pass.
    fn pause(&self) {
        self.clock.advance(SignedDuration::from_secs(2));
    }

    async fn set_retention_days(&self, days: u32) {
        self.db
            .write(move |tx| {
                tx.execute(
                    "INSERT INTO settings (key, value) VALUES ('audit_retention_days', ?1)",
                    [days.to_string()],
                )
                .map_err(StoreError::from)
            })
            .await
            .unwrap();
    }
}

fn home() -> Client {
    Client::new("203.0.113.7".parse().unwrap(), Some("Firefox"))
}

fn office() -> Client {
    Client::new("198.51.100.20".parse().unwrap(), Some("Chrome"))
}

fn password_of(name: &str) -> String {
    format!("{name}'s own passphrase")
}

fn code<T>(result: Result<T, crate::Error>) -> ErrorCode {
    result.err().unwrap().code().unwrap()
}

#[tokio::test]
async fn the_setup_token_creates_the_owner_once() {
    let lab = Lab::new().await;
    assert!(lab.accounts.setup_required().await.unwrap());
    let link = lab
        .accounts
        .create_setup_token(ActorKind::System)
        .await
        .unwrap()
        .unwrap();
    let token = link.token;
    assert_eq!(token.len(), 32);
    assert_eq!(link.expires_at, lab.clock.now() + SignedDuration::from_mins(30));

    let signed_in = lab
        .accounts
        .complete_setup(&token.to_lowercase(), "ada", OWNER_PASSWORD.to_owned(), &home())
        .await
        .unwrap();
    assert!(signed_in.me.is_owner);
    assert_eq!(signed_in.me.permissions, P::all());

    let again = lab
        .accounts
        .complete_setup(&token, "grace", OWNER_PASSWORD.to_owned(), &office())
        .await;
    assert_eq!(code(again), ErrorCode::AlreadySetUp);
    assert!(!lab.accounts.setup_required().await.unwrap());
    assert_eq!(
        lab.accounts.create_setup_token(ActorKind::Cli).await.unwrap(),
        None
    );

    let completed = lab.entries(AuditAction::SetupCompleted).await;
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].actor_name.as_deref(), Some("ada"));
    assert_eq!(completed[0].ip.as_deref(), Some("203.0.113.7"));
    let link = &lab.entries(AuditAction::SetupLinkCreated).await[0];
    assert_eq!(link.actor_kind, ActorKind::System);
    assert_eq!(link.details, json!({}));
}

#[tokio::test]
async fn a_setup_token_expires_after_thirty_minutes_and_a_forged_one_never_works() {
    let lab = Lab::new().await;
    let token = lab
        .accounts
        .create_setup_token(ActorKind::Cli)
        .await
        .unwrap()
        .unwrap()
        .token;

    let forged = lab
        .accounts
        .complete_setup(&"A".repeat(32), "ada", OWNER_PASSWORD.to_owned(), &home())
        .await;
    assert_eq!(code(forged), ErrorCode::InvalidSetupToken);

    lab.clock.advance(SignedDuration::from_mins(30));
    let late = lab
        .accounts
        .complete_setup(&token, "ada", OWNER_PASSWORD.to_owned(), &office())
        .await;
    assert_eq!(code(late), ErrorCode::InvalidSetupToken);
    assert!(lab.accounts.setup_required().await.unwrap());
}

#[tokio::test]
async fn setup_refuses_a_bad_username_or_a_short_password_without_spending_the_token() {
    let lab = Lab::new().await;
    let token = lab
        .accounts
        .create_setup_token(ActorKind::System)
        .await
        .unwrap()
        .unwrap()
        .token;

    let short = lab
        .accounts
        .complete_setup(&token, "ada", "too short".to_owned(), &home())
        .await;
    let invalid = lab
        .accounts
        .complete_setup(&token, "a d", OWNER_PASSWORD.to_owned(), &home())
        .await;

    assert_eq!(code(short), ErrorCode::PasswordTooShort);
    assert_eq!(code(invalid), ErrorCode::InvalidUsername);
    lab.accounts
        .complete_setup(&token, "ada", OWNER_PASSWORD.to_owned(), &home())
        .await
        .unwrap();
}

#[tokio::test]
async fn a_session_needs_both_the_cookie_token_and_the_session_key() {
    let lab = Lab::new().await;
    let laptop = lab.owner().await;
    let phone = lab.sign_in("ada", OWNER_PASSWORD, &office()).await.unwrap();

    let mixed = lab
        .accounts
        .authenticate(&laptop.signed_in.token, &phone.signed_in.key, home())
        .await;
    let garbled = lab
        .accounts
        .authenticate(&laptop.signed_in.token, "not hex", home())
        .await;

    assert_eq!(code(mixed), ErrorCode::Unauthenticated);
    assert_eq!(code(garbled), ErrorCode::Unauthenticated);
    // A wrong key does not end the real session.
    lab.caller(&laptop).await;
}

#[tokio::test]
async fn a_session_ends_after_twelve_idle_hours_or_seven_days_in_all() {
    let lab = Lab::new().await;
    let idle = lab.owner().await;
    let busy = lab.sign_in("ada", OWNER_PASSWORD, &office()).await.unwrap();

    lab.clock.advance(SignedDuration::from_hours(11));
    lab.caller(&busy).await;
    lab.clock.advance(SignedDuration::from_hours(1));
    assert_eq!(
        lab.authenticate(&idle).await.err(),
        Some(ErrorCode::Unauthenticated)
    );

    // Used every eleven hours, the busy session lives through its seventh day, and no further.
    for _ in 0..15 {
        lab.caller(&busy).await;
        lab.clock.advance(SignedDuration::from_hours(11));
    }
    assert_eq!(
        lab.authenticate(&busy).await.err(),
        Some(ErrorCode::Unauthenticated)
    );
}

#[tokio::test]
async fn changing_a_password_ends_every_other_session_and_keeps_this_one() {
    let lab = Lab::new().await;
    let laptop = lab.owner().await;
    let phone = lab.sign_in("ada", OWNER_PASSWORD, &office()).await.unwrap();
    let caller = lab.caller(&laptop).await;

    lab.accounts
        .change_password(&caller, OWNER_PASSWORD.to_owned(), password_of("ada"))
        .await
        .unwrap();

    let after = lab.caller(&laptop).await;
    lab.accounts.require_reauthentication(&after).unwrap();
    assert_eq!(
        lab.authenticate(&phone).await.err(),
        Some(ErrorCode::Unauthenticated)
    );
    lab.pause();
    assert_eq!(
        lab.sign_in("ada", OWNER_PASSWORD, &home()).await.err(),
        Some(ErrorCode::InvalidCredentials)
    );
    lab.pause();
    lab.sign_in("ada", &password_of("ada"), &home()).await.unwrap();
}

#[tokio::test]
async fn a_login_rehashes_a_password_stored_with_older_parameters() {
    let lab = Lab::new().await;
    lab.owner().await;
    let stronger = PasswordCost {
        memory_kib: 16,
        passes: 2,
    };
    let accounts = Accounts::new(lab.db.clone(), lab.clock.clone(), stronger)
        .await
        .unwrap();

    accounts
        .login("ada", OWNER_PASSWORD.to_owned(), &office())
        .await
        .unwrap();

    let stored = lab
        .db
        .read(|conn| users::find(conn, "ada"))
        .await
        .unwrap()
        .unwrap();
    assert!(stored.password_hash.starts_with("$argon2id$v=19$m=16,t=2,p=1$"));
    lab.sign_in("ada", OWNER_PASSWORD, &home()).await.unwrap();
}

#[tokio::test]
async fn a_login_checked_against_a_password_changed_meanwhile_opens_no_session() {
    let lab = Lab::new().await;
    lab.owner().await;
    let next = lab.accounts.passwords.hash(password_of("ada")).await.unwrap();

    let held = lab.accounts.passwords.hold().await;
    let login = lab.login_waiting("ada", OWNER_PASSWORD, office()).await;
    // The new password lands as a password change writes it, while the old one is being checked.
    lab.db
        .write(move |tx| {
            let mut ada = users::find(tx, "ada")?.unwrap();
            ada.password_hash = next;
            ada.auth_epoch += 1;
            users::update(tx, &ada)
        })
        .await
        .unwrap();
    drop(held);

    assert_eq!(code(login.await.unwrap()), ErrorCode::InvalidCredentials);
    assert_eq!(lab.entries(AuditAction::LoginSucceeded).await, []);
}

#[tokio::test]
async fn a_login_checked_while_its_user_was_signed_out_everywhere_opens_no_session() {
    let lab = Lab::new().await;
    let owner = lab.owner().await;
    let grace = lab.user(&owner, "grace", P::CONSOLE_VIEW).await;
    let grace_id = lab.caller(&grace).await.user_id;
    let admin = lab.caller(&owner).await;

    let held = lab.accounts.passwords.hold().await;
    let login = lab.login_waiting("grace", &password_of("grace"), home()).await;
    lab.accounts.revoke_sessions(&admin, grace_id).await.unwrap();
    drop(held);

    assert_eq!(code(login.await.unwrap()), ErrorCode::InvalidCredentials);
    // Only the sign-in that came before the revocation.
    assert_eq!(lab.entries(AuditAction::LoginSucceeded).await.len(), 1);
}

#[tokio::test]
async fn a_password_change_needs_the_current_password_and_a_different_one() {
    let lab = Lab::new().await;
    let owner = lab.owner().await;
    let caller = lab.caller(&owner).await;

    let wrong = lab
        .accounts
        .change_password(&caller, "not the password".to_owned(), password_of("ada"))
        .await;
    let same = lab
        .accounts
        .change_password(&caller, OWNER_PASSWORD.to_owned(), OWNER_PASSWORD.to_owned())
        .await;

    assert_eq!(code(wrong), ErrorCode::WrongPassword);
    assert_eq!(code(same), ErrorCode::PasswordUnchanged);
}

#[tokio::test]
async fn revoking_sessions_or_changing_permissions_signs_the_user_out_everywhere() {
    let lab = Lab::new().await;
    let owner = lab.owner().await;
    let grace = lab.user(&owner, "grace", P::CONSOLE_VIEW).await;
    let grace_id = lab.caller(&grace).await.user_id;
    let admin = lab.caller(&owner).await;

    lab.accounts.revoke_sessions(&admin, grace_id).await.unwrap();
    assert_eq!(
        lab.authenticate(&grace).await.err(),
        Some(ErrorCode::Unauthenticated)
    );

    let again = lab
        .sign_in("grace", &password_of("grace"), &home())
        .await
        .unwrap();
    let version = lab.accounts.users().await.unwrap()[1].version;
    let updated = lab
        .accounts
        .update_user(&admin, grace_id, P::CONSOLE_WRITE, version)
        .await
        .unwrap();
    assert_eq!(updated.permissions, P::CONSOLE_WRITE | P::CONSOLE_VIEW);
    assert_eq!(
        lab.authenticate(&again).await.err(),
        Some(ErrorCode::Unauthenticated)
    );

    let changed = &lab.entries(AuditAction::UserPermissionsChanged).await[0];
    assert_eq!(changed.target.as_deref(), Some("grace"));
    assert_eq!(changed.details["added"], json!("4"));
    assert_eq!(changed.details["removed"], json!("0"));
    let revoked = &lab.entries(AuditAction::SessionRevoked).await[0];
    assert_eq!(revoked.details["sessions"], json!(1));
}

#[tokio::test]
async fn an_edit_from_a_stale_version_is_a_conflict() {
    let lab = Lab::new().await;
    let owner = lab.owner().await;
    let grace = lab.user(&owner, "grace", P::CONSOLE_VIEW).await;
    let grace_id = lab.caller(&grace).await.user_id;
    let admin = lab.caller(&owner).await;
    let read = lab.accounts.users().await.unwrap()[1].version;

    lab.accounts
        .update_user(&admin, grace_id, P::AUDIT_VIEW, read)
        .await
        .unwrap();
    let stale = lab
        .accounts
        .update_user(&admin, grace_id, P::LOGS_DOWNLOAD, read)
        .await;

    assert_eq!(code(stale), ErrorCode::Conflict);
    assert_eq!(lab.accounts.users().await.unwrap()[1].permissions, P::AUDIT_VIEW);
}

#[tokio::test]
async fn a_manager_can_neither_escalate_nor_touch_a_stronger_account() {
    let lab = Lab::new().await;
    let owner = lab.owner().await;
    let manager = lab.user(&owner, "grace", P::USERS_MANAGE | P::CONSOLE_VIEW).await;
    let operator = lab
        .user(&owner, "linus", P::CONSOLE_VIEW | P::SERVER_CONTROL)
        .await;
    let operator_id = lab.caller(&operator).await.user_id;
    let manager = lab.caller(&manager).await;
    let owner = lab.caller(&owner).await;

    let escalation = lab
        .accounts
        .create_user(&manager, "eve", P::SETTINGS_MANAGE)
        .await
        .err()
        .unwrap();
    assert_eq!(escalation.code(), Some(ErrorCode::Escalation));
    let crate::Error::Refused(refusal) = escalation else {
        unreachable!()
    };
    assert_eq!(refusal.params["permissions"], json!("600"));

    assert_eq!(
        code(lab.accounts.delete_user(&manager, operator_id).await),
        ErrorCode::Hierarchy
    );
    assert_eq!(
        code(lab.accounts.reset_password(&manager, owner.user_id).await),
        ErrorCode::OwnerProtected
    );
    assert_eq!(
        code(lab.accounts.revoke_sessions(&manager, manager.user_id).await),
        ErrorCode::SelfEdit
    );
    assert_eq!(
        code(
            lab.accounts
                .create_user(&manager, "eve", P::from_hex("1000").unwrap())
                .await
        ),
        ErrorCode::UnknownPermissions
    );
    assert_eq!(
        code(lab.accounts.create_user(&owner, "LINUS", P::NONE).await),
        ErrorCode::NameTaken
    );

    let created = lab
        .accounts
        .create_user(&manager, "eve", P::CONSOLE_VIEW)
        .await
        .unwrap();
    lab.accounts.delete_user(&manager, created.user.id).await.unwrap();
    let deleted = &lab.entries(AuditAction::UserDeleted).await[0];
    assert_eq!(deleted.actor_name.as_deref(), Some("grace"));
    assert_eq!(deleted.target.as_deref(), Some("eve"));
}

#[tokio::test]
async fn a_new_user_must_replace_the_temporary_password_within_seventy_two_hours() {
    let lab = Lab::new().await;
    let owner = lab.caller(&lab.owner().await).await;
    let created = lab
        .accounts
        .create_user(&owner, "grace", P::AUDIT_VIEW)
        .await
        .unwrap();
    assert!(created.user.must_change_password);
    assert_eq!(created.temporary_password.len(), 20);

    let browser = lab
        .sign_in("grace", &created.temporary_password, &office())
        .await
        .unwrap();
    assert!(browser.signed_in.me.must_change_password);
    assert!(lab.caller(&browser).await.must_change_password);

    let reset = lab
        .accounts
        .reset_password(&owner, created.user.id)
        .await
        .unwrap();
    assert_eq!(
        lab.authenticate(&browser).await.err(),
        Some(ErrorCode::Unauthenticated)
    );
    lab.clock.advance(SignedDuration::from_hours(72));
    assert_eq!(
        lab.sign_in("grace", &reset, &office()).await.err(),
        Some(ErrorCode::TemporaryPasswordExpired)
    );
}

#[tokio::test]
async fn a_pair_is_locked_after_five_failures_while_other_networks_and_names_go_on() {
    let lab = Lab::new().await;
    lab.owner().await;
    for _ in 0..5 {
        lab.pause();
        assert_eq!(
            lab.sign_in("ada", "guess guess guess", &office()).await.err(),
            Some(ErrorCode::InvalidCredentials)
        );
    }

    lab.pause();
    let locked = lab
        .accounts
        .login("ada", OWNER_PASSWORD.to_owned(), &office())
        .await;
    let crate::Error::Refused(refusal) = locked.err().unwrap() else {
        unreachable!()
    };
    assert_eq!(refusal.code, ErrorCode::TooManyAttempts);
    assert_eq!(refusal.params["retryAfter"], json!(58));

    // The owner at home is unaffected, and so is another name from the locked network.
    lab.sign_in("ada", OWNER_PASSWORD, &home()).await.unwrap();
    lab.pause();
    assert_eq!(
        lab.sign_in("grace", "guess guess guess", &office()).await.err(),
        Some(ErrorCode::InvalidCredentials)
    );

    lab.clock.advance(SignedDuration::from_mins(1));
    lab.sign_in("ada", OWNER_PASSWORD, &office()).await.unwrap();
}

#[tokio::test]
async fn an_unknown_username_fails_like_a_wrong_password_and_counts_the_same() {
    let lab = Lab::new().await;
    lab.owner().await;

    for _ in 0..5 {
        lab.pause();
        assert_eq!(
            lab.sign_in("nobody", "guess guess guess", &office()).await.err(),
            Some(ErrorCode::InvalidCredentials)
        );
    }
    lab.pause();

    assert_eq!(
        lab.sign_in("nobody", "guess guess guess", &office()).await.err(),
        Some(ErrorCode::TooManyAttempts)
    );
}

#[tokio::test]
async fn a_network_gets_one_attempt_in_flight_and_a_pause_after_a_failure() {
    let lab = Lab::new().await;
    lab.owner().await;

    assert_eq!(
        lab.sign_in("ada", "guess guess guess", &office()).await.err(),
        Some(ErrorCode::InvalidCredentials)
    );
    assert_eq!(
        lab.sign_in("ada", OWNER_PASSWORD, &office()).await.err(),
        Some(ErrorCode::TooManyAttempts)
    );
    lab.sign_in("ada", OWNER_PASSWORD, &home()).await.unwrap();
    lab.sign_in("ada", OWNER_PASSWORD, &home()).await.unwrap();
}

#[tokio::test]
async fn failed_logins_reach_the_audit_log_as_one_entry_per_window() {
    let lab = Lab::new().await;
    lab.owner().await;
    for _ in 0..3 {
        lab.pause();
        lab.sign_in("ada", "guess guess guess", &office())
            .await
            .unwrap_err();
    }
    lab.accounts.sweep().await.unwrap();
    assert_eq!(lab.entries(AuditAction::LoginFailed).await, []);

    lab.clock.advance(SignedDuration::from_mins(15));
    lab.accounts.sweep().await.unwrap();

    let failed = lab.entries(AuditAction::LoginFailed).await;
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].actor_kind, ActorKind::Anonymous);
    assert_eq!(failed[0].outcome, Outcome::Failure);
    assert_eq!(failed[0].ip.as_deref(), Some("198.51.100.20"));
    assert_eq!(failed[0].details["failures"], json!(3));
    assert_eq!(failed[0].details["username"], json!("ada"));
    lab.accounts.sweep().await.unwrap();
    assert_eq!(lab.entries(AuditAction::LoginFailed).await.len(), 1);
}

#[tokio::test]
async fn unlocking_from_the_command_line_lifts_the_lock_and_keeps_the_trace() {
    let lab = Lab::new().await;
    lab.owner().await;
    for _ in 0..5 {
        lab.pause();
        lab.sign_in("ada", "guess guess guess", &office())
            .await
            .unwrap_err();
    }
    lab.pause();

    assert_eq!(lab.accounts.unlock("ADA").await.unwrap(), 1);

    lab.sign_in("ada", OWNER_PASSWORD, &office()).await.unwrap();
    assert_eq!(
        lab.entries(AuditAction::LoginFailed).await[0].details["failures"],
        json!(5)
    );
    let unlocked = &lab.entries(AuditAction::UserUnlocked).await[0];
    assert_eq!(unlocked.actor_kind, ActorKind::Cli);
    assert_eq!(unlocked.target.as_deref(), Some("ada"));
    assert_eq!(code(lab.accounts.unlock("nobody").await), ErrorCode::NotFound);
}

#[tokio::test]
async fn the_command_line_resets_even_the_owner_who_must_then_choose_a_password() {
    let lab = Lab::new().await;
    let owner = lab.owner().await;

    let temporary = lab.accounts.reset_password_from_cli("ada").await.unwrap();

    assert_eq!(
        lab.authenticate(&owner).await.err(),
        Some(ErrorCode::Unauthenticated)
    );
    let browser = lab.sign_in("ada", &temporary, &home()).await.unwrap();
    assert!(browser.signed_in.me.must_change_password);
    let reset = &lab.entries(AuditAction::UserPasswordReset).await[0];
    assert_eq!(reset.actor_kind, ActorKind::Cli);
}

#[tokio::test]
async fn a_denied_change_is_audited_but_a_denied_read_is_not() {
    let lab = Lab::new().await;
    let owner = lab.owner().await;
    let grace = lab.user(&owner, "grace", P::CONSOLE_VIEW).await;
    let grace = lab.caller(&grace).await;

    let read = lab
        .accounts
        .authorize(&grace, P::AUDIT_VIEW, "GET", "/api/audit")
        .await;
    let change = lab
        .accounts
        .authorize(&grace, P::USERS_MANAGE, "POST", "/api/users")
        .await;
    lab.accounts
        .authorize(&grace, P::CONSOLE_VIEW, "GET", "/api/console")
        .await
        .unwrap();

    let crate::Error::Refused(refusal) = read.err().unwrap() else {
        unreachable!()
    };
    assert_eq!(refusal.code, ErrorCode::Forbidden);
    assert_eq!(refusal.params["permission"], json!("auditView"));
    assert_eq!(code(change), ErrorCode::Forbidden);
    let denied = lab.entries(AuditAction::PermissionDenied).await;
    assert_eq!(denied.len(), 1);
    assert_eq!(denied[0].outcome, Outcome::Denied);
    assert_eq!(denied[0].target.as_deref(), Some("POST /api/users"));
    assert_eq!(denied[0].details["permission"], json!("usersManage"));
}

#[tokio::test]
async fn sensitive_actions_need_the_password_within_the_last_ten_minutes() {
    let lab = Lab::new().await;
    let owner = lab.owner().await;
    lab.accounts
        .require_reauthentication(&lab.caller(&owner).await)
        .unwrap();

    lab.clock.advance(SignedDuration::from_mins(10));
    let caller = lab.caller(&owner).await;
    assert_eq!(
        lab.accounts
            .require_reauthentication(&caller)
            .err()
            .unwrap()
            .code(),
        Some(ErrorCode::ReauthRequired)
    );
    assert_eq!(
        code(
            lab.accounts
                .reauthenticate(&caller, "not the password".to_owned())
                .await
        ),
        ErrorCode::WrongPassword
    );

    lab.pause();
    lab.accounts
        .reauthenticate(&caller, OWNER_PASSWORD.to_owned())
        .await
        .unwrap();
    lab.accounts
        .require_reauthentication(&lab.caller(&owner).await)
        .unwrap();
}

#[tokio::test]
async fn a_user_lists_and_ends_only_their_own_sessions() {
    let lab = Lab::new().await;
    let owner = lab.owner().await;
    let phone = lab.sign_in("ada", OWNER_PASSWORD, &office()).await.unwrap();
    let grace = lab.user(&owner, "grace", P::NONE).await;
    let ada = lab.caller(&owner).await;

    let sessions = lab.accounts.sessions(&ada).await.unwrap();
    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions.iter().filter(|session| session.current).count(), 1);
    let phone_id = lab.caller(&phone).await.session_id;
    let grace = lab.caller(&grace).await;

    assert_eq!(
        code(lab.accounts.revoke_session(&grace, phone_id).await),
        ErrorCode::NotFound
    );
    lab.accounts.revoke_session(&ada, phone_id).await.unwrap();
    assert_eq!(
        lab.authenticate(&phone).await.err(),
        Some(ErrorCode::Unauthenticated)
    );
    lab.caller(&owner).await;
}

#[tokio::test]
async fn audit_entries_are_kept_for_the_retention_in_force_when_written() {
    let lab = Lab::new().await;
    let owner = lab.owner().await;
    let caller = lab.caller(&owner).await;
    lab.accounts.logout(&caller).await.unwrap();

    lab.set_retention_days(30).await;
    lab.clock.advance(SignedDuration::from_hours(24));
    lab.sign_in("ada", OWNER_PASSWORD, &home()).await.unwrap();

    // The floor is 90 days: the login is kept that long, the logout for the default year.
    lab.clock.advance(SignedDuration::from_hours(24 * 90));
    assert_eq!(lab.audit.purge().await.unwrap(), 0);
    lab.clock.advance(SignedDuration::from_mins(1));
    assert_eq!(lab.audit.purge().await.unwrap(), 1);

    assert_eq!(lab.entries(AuditAction::LoginSucceeded).await, []);
    assert_eq!(lab.entries(AuditAction::Logout).await.len(), 1);
    assert_eq!(lab.entries(AuditAction::SetupCompleted).await.len(), 1);
}

#[tokio::test]
async fn the_purge_runs_when_observer_reads_the_present_a_little_ahead_of_sqlite() {
    let lab = Lab::new().await;
    let ahead = Timestamp::now() + SignedDuration::from_millis(300);
    lab.clock.advance(ahead.duration_since(lab.clock.now()));

    lab.audit.purge().await.unwrap();
}

#[tokio::test]
async fn audit_pages_are_capped_and_point_to_the_next_one() {
    let lab = Lab::new().await;
    lab.owner().await;
    for _ in 0..3 {
        lab.pause();
        lab.sign_in("ada", OWNER_PASSWORD, &home()).await.unwrap();
    }

    let first = lab
        .audit
        .page(AuditFilter {
            limit: Some(2),
            ..AuditFilter::default()
        })
        .await
        .unwrap();
    let rest = lab
        .audit
        .page(AuditFilter {
            before: first.next,
            limit: Some(500),
            ..AuditFilter::default()
        })
        .await
        .unwrap();

    assert_eq!(first.entries.len(), 2);
    assert!(first.next.is_some());
    // The setup link, the setup and three logins.
    assert_eq!(rest.entries.len(), 3);
    assert_eq!(rest.next, None);
}

#[tokio::test]
async fn repeated_refusals_of_one_action_are_written_once_a_window_with_their_count() {
    let lab = Lab::new().await;
    let owner = lab.owner().await;
    let grace = lab.user(&owner, "grace", P::CONSOLE_VIEW).await;
    let grace = lab.caller(&grace).await;

    for _ in 0..3 {
        lab.accounts
            .authorize(&grace, P::USERS_MANAGE, "DELETE", "/api/users/{id}")
            .await
            .unwrap_err();
    }
    lab.accounts
        .authorize(&grace, P::USERS_MANAGE, "POST", "/api/users")
        .await
        .unwrap_err();
    assert_eq!(lab.entries(AuditAction::PermissionDenied).await.len(), 2);

    lab.clock.advance(SignedDuration::from_mins(15));
    lab.accounts.sweep().await.unwrap();

    let denied = lab.entries(AuditAction::PermissionDenied).await;
    assert_eq!(denied.len(), 3);
    assert_eq!(denied[0].target.as_deref(), Some("DELETE /api/users/{id}"));
    assert_eq!(denied[0].details["repeated"], json!(2));
    lab.accounts.sweep().await.unwrap();
    assert_eq!(lab.entries(AuditAction::PermissionDenied).await.len(), 3);
}

#[tokio::test]
async fn a_change_the_delegation_rules_refuse_is_audited_with_its_reason() {
    let lab = Lab::new().await;
    let owner = lab.owner().await;
    let manager = lab.user(&owner, "grace", P::USERS_MANAGE | P::CONSOLE_VIEW).await;
    let operator = lab
        .user(&owner, "linus", P::CONSOLE_VIEW | P::SERVER_CONTROL)
        .await;
    let operator_id = lab.caller(&operator).await.user_id;
    let manager = lab.caller(&manager).await;
    let owner_id = lab.caller(&owner).await.user_id;

    assert!(
        lab.accounts
            .create_user(&manager, "eve", P::SETTINGS_MANAGE)
            .await
            .is_err()
    );
    lab.accounts.delete_user(&manager, operator_id).await.unwrap_err();
    lab.accounts.reset_password(&manager, owner_id).await.unwrap_err();
    lab.accounts
        .update_user(&manager, manager.user_id, P::AUDIT_VIEW, 1)
        .await
        .unwrap_err();

    let denied = lab.entries(AuditAction::PermissionDenied).await;
    let seen: Vec<_> = denied
        .iter()
        .rev()
        .map(|entry| {
            (
                entry.target.as_deref(),
                entry.details["attempted"].as_str(),
                entry.details["refusal"].as_str(),
            )
        })
        .collect();
    assert_eq!(
        seen,
        [
            (Some("eve"), Some("user_created"), Some("escalation")),
            (Some("linus"), Some("user_deleted"), Some("hierarchy")),
            (Some("ada"), Some("user_password_reset"), Some("owner_protected")),
            (Some("grace"), Some("user_permissions_changed"), Some("self_edit")),
        ]
    );
    assert!(denied.iter().all(|entry| entry.outcome == Outcome::Denied));
    assert_eq!(denied[3].details["permissions"], json!("600"));
    assert_eq!(denied[2].details["userId"], json!(operator_id));
    assert_eq!(denied[0].actor_name.as_deref(), Some("grace"));
}

#[tokio::test]
async fn a_username_naming_no_account_stays_out_of_the_audit_log() {
    let lab = Lab::new().await;
    lab.owner().await;
    lab.sign_in("My owner password 123!", "", &office())
        .await
        .unwrap_err();
    lab.pause();
    lab.sign_in("ADA", "guess guess guess", &home())
        .await
        .unwrap_err();

    lab.clock.advance(SignedDuration::from_mins(15));
    lab.accounts.sweep().await.unwrap();

    let failed = lab.entries(AuditAction::LoginFailed).await;
    let unknown = failed
        .iter()
        .find(|entry| entry.ip.as_deref() == Some("198.51.100.20"))
        .unwrap();
    assert_eq!(unknown.target, None);
    assert_eq!(unknown.details.get("username"), None);
    assert_eq!(unknown.details["failures"], json!(1));
    let known = failed
        .iter()
        .find(|entry| entry.ip.as_deref() == Some("203.0.113.7"))
        .unwrap();
    assert_eq!(known.target.as_deref(), Some("ada"));
    assert_eq!(known.details["username"], json!("ada"));
}

#[tokio::test]
async fn a_password_reset_lifts_the_login_locks_on_the_account() {
    let lab = Lab::new().await;
    let owner = lab.owner().await;
    let grace = lab.user(&owner, "grace", P::CONSOLE_VIEW).await;
    let grace_id = lab.caller(&grace).await.user_id;
    for _ in 0..5 {
        lab.pause();
        lab.sign_in("grace", "guess guess guess", &office())
            .await
            .unwrap_err();
    }
    lab.pause();

    let from_cli = lab.accounts.reset_password_from_cli("grace").await.unwrap();
    lab.sign_in("grace", &from_cli, &office()).await.unwrap();

    for _ in 0..5 {
        lab.pause();
        lab.sign_in("grace", "guess guess guess", &office())
            .await
            .unwrap_err();
    }
    lab.pause();
    let admin = lab.caller(&owner).await;
    let from_panel = lab.accounts.reset_password(&admin, grace_id).await.unwrap();
    lab.sign_in("grace", &from_panel, &office()).await.unwrap();

    let failed = lab.entries(AuditAction::LoginFailed).await;
    assert_eq!(failed.len(), 2);
    assert!(failed.iter().all(|entry| entry.details["failures"] == json!(5)));
}

#[tokio::test]
async fn a_signed_in_user_confirming_their_password_is_audited_as_themselves() {
    let lab = Lab::new().await;
    let owner = lab.owner().await;
    let grace = lab.user(&owner, "grace", P::CONSOLE_VIEW).await;
    let grace = lab.caller(&grace).await;

    for _ in 0..3 {
        let wrong = lab
            .accounts
            .reauthenticate(&grace, "not the password".to_owned())
            .await;
        assert_eq!(code(wrong), ErrorCode::WrongPassword);
        lab.pause();
    }
    for _ in 0..2 {
        let wrong = lab
            .accounts
            .change_password(
                &grace,
                "not the password".to_owned(),
                "a brand new passphrase".to_owned(),
            )
            .await;
        assert_eq!(code(wrong), ErrorCode::WrongPassword);
        lab.pause();
    }
    // Throttled like a login all the same.
    let locked = lab.accounts.reauthenticate(&grace, password_of("grace")).await;
    assert_eq!(code(locked), ErrorCode::TooManyAttempts);
    lab.clock.advance(SignedDuration::from_mins(1));
    lab.accounts
        .reauthenticate(&grace, password_of("grace"))
        .await
        .unwrap();

    lab.clock.advance(SignedDuration::from_mins(15));
    lab.accounts.sweep().await.unwrap();
    assert_eq!(lab.entries(AuditAction::LoginFailed).await, []);
    let confirmations = lab.entries(AuditAction::Reauthenticated).await;
    assert_eq!(confirmations.len(), 4);
    assert_eq!(confirmations[0].outcome, Outcome::Success);
    assert!(
        confirmations[1..]
            .iter()
            .all(|entry| entry.outcome == Outcome::Failure && entry.actor_name.as_deref() == Some("grace"))
    );
    let changes = lab.entries(AuditAction::PasswordChanged).await;
    assert_eq!(
        changes
            .iter()
            .filter(|entry| entry.outcome == Outcome::Failure)
            .count(),
        2
    );
}

#[tokio::test]
async fn strangers_filling_the_hash_queue_keep_neither_a_known_network_nor_a_signed_in_user_out() {
    let lab = Lab::new().await;
    let owner = lab.owner().await;
    let ada = lab.caller(&owner).await;
    let ada = Caller {
        client: office(),
        ..ada
    };
    lab.pause();

    let held = lab.accounts.passwords.hold().await;
    let strangers: Vec<_> = (0..24)
        .map(|network| {
            let accounts = lab.accounts.clone();
            let client = Client::new(std::net::IpAddr::from([10, 0, network, 1]), None);
            tokio::spawn(async move {
                accounts
                    .login("ada", "guess guess guess".to_owned(), &client)
                    .await
            })
        })
        .collect();
    while lab.accounts.passwords.waiting() < 18 {
        tokio::task::yield_now().await;
    }
    let from_home = tokio::spawn({
        let accounts = lab.accounts.clone();
        async move { accounts.login("ada", OWNER_PASSWORD.to_owned(), &home()).await }
    });
    let reauthentication = tokio::spawn({
        let accounts = lab.accounts.clone();
        async move { accounts.reauthenticate(&ada, OWNER_PASSWORD.to_owned()).await }
    });
    for _ in 0..100 {
        tokio::task::yield_now().await;
    }
    drop(held);

    from_home.await.unwrap().unwrap();
    reauthentication.await.unwrap().unwrap();
    for stranger in strangers {
        let refused = code(stranger.await.unwrap());
        assert!(matches!(refused, ErrorCode::InvalidCredentials | ErrorCode::Busy));
    }
}

/// A folder of the lab's host, as its platform writes paths.
fn folder(name: &str) -> String {
    if cfg!(windows) {
        format!(r"C:\FXServer\{name}")
    } else {
        format!("/srv/fxserver/{name}")
    }
}

fn launch_settings() -> LaunchSettings {
    LaunchSettings {
        build_path: folder("build"),
        data_path: folder("server-data"),
        cfg_file: "server.cfg".to_owned(),
        onesync: OneSync::On,
        auto_start: false,
        txadmin_events: false,
        extra_arguments: Vec::new(),
    }
}

#[tokio::test]
async fn launch_settings_are_stored_once_checked_with_what_changed_in_the_audit_log() {
    let lab = Lab::new().await;
    let owner = lab.caller(&lab.owner().await).await;
    assert_eq!(lab.servers.launch_settings(Servers::DEFAULT).await.unwrap(), None);

    let first = lab
        .servers
        .change_launch_settings(&owner, Servers::DEFAULT, launch_settings(), None)
        .await
        .unwrap();
    let edited = LaunchSettings {
        onesync: OneSync::Legacy,
        extra_arguments: vec![ExtraArgument {
            name: "sv_enforceGameBuild".to_owned(),
            value: "3258".to_owned(),
        }],
        ..launch_settings()
    };
    let second = lab
        .servers
        .change_launch_settings(&owner, Servers::DEFAULT, edited.clone(), Some(1))
        .await
        .unwrap();
    let again = lab
        .servers
        .change_launch_settings(&owner, Servers::DEFAULT, edited.clone(), Some(2))
        .await
        .unwrap();

    assert_eq!((first.version, second.version, again.version), (1, 2, 2));
    assert_eq!(
        lab.servers.launch_settings(Servers::DEFAULT).await.unwrap(),
        Some(StoredLaunch {
            settings: edited,
            version: 2,
        })
    );
    let changes = lab.entries(AuditAction::LaunchSettingsChanged).await;
    assert_eq!(changes.len(), 2, "saving the same settings again records nothing");
    assert_eq!(changes[0].target.as_deref(), Some("default"));
    assert_eq!(changes[0].actor_name.as_deref(), Some("ada"));
    assert_eq!(
        changes[0].details,
        json!({
            "onesync": { "before": "on", "after": "legacy" },
            "extraArguments": { "set": ["sv_enforceGameBuild"], "removed": [] },
        })
    );
    assert_eq!(
        changes[1].details,
        json!({
            "buildPath": true,
            "dataPath": true,
            "cfgFile": true,
            "onesync": { "before": null, "after": "on" },
            "autoStart": { "before": null, "after": false },
            "txadminEvents": { "before": null, "after": false },
        })
    );
}

#[tokio::test]
async fn a_launch_change_names_the_arguments_set_and_removed_never_their_values() {
    let lab = Lab::new().await;
    let owner = lab.caller(&lab.owner().await).await;
    let argument = |name: &str, value: &str| ExtraArgument {
        name: name.to_owned(),
        value: value.to_owned(),
    };
    let first = LaunchSettings {
        extra_arguments: vec![
            argument("sv_hostname", "Lab"),
            argument("sv_maxclients", "48"),
            argument("sv_projectName", "Observer"),
        ],
        ..launch_settings()
    };
    let second = LaunchSettings {
        data_path: folder("other-data"),
        extra_arguments: vec![
            argument("sv_maxclients", "48"),
            argument("sv_hostname", "Lab 2"),
            argument("sv_enforceGameBuild", "3258"),
        ],
        ..launch_settings()
    };
    for (settings, version) in [(first, None), (second, Some(1))] {
        lab.servers
            .change_launch_settings(&owner, Servers::DEFAULT, settings, version)
            .await
            .unwrap();
    }

    let changes = lab.entries(AuditAction::LaunchSettingsChanged).await;
    assert_eq!(
        changes[0].details,
        json!({
            "dataPath": true,
            "extraArguments": {
                "set": ["sv_hostname", "sv_enforceGameBuild"],
                "removed": ["sv_projectName"],
            },
        })
    );
}

#[tokio::test]
async fn a_launch_change_made_from_settings_replaced_since_is_refused() {
    let lab = Lab::new().await;
    let owner = lab.caller(&lab.owner().await).await;
    lab.servers
        .change_launch_settings(&owner, Servers::DEFAULT, launch_settings(), None)
        .await
        .unwrap();
    let from_elsewhere = LaunchSettings {
        cfg_file: "configs/live.cfg".to_owned(),
        ..launch_settings()
    };
    lab.servers
        .change_launch_settings(&owner, Servers::DEFAULT, from_elsewhere.clone(), Some(1))
        .await
        .unwrap();

    let stale = lab
        .servers
        .change_launch_settings(&owner, Servers::DEFAULT, launch_settings(), Some(1))
        .await;
    let first_save_again = lab
        .servers
        .change_launch_settings(&owner, Servers::DEFAULT, launch_settings(), None)
        .await;

    assert_eq!(code(stale), ErrorCode::Conflict);
    assert_eq!(code(first_save_again), ErrorCode::Conflict);
    assert_eq!(
        lab.servers.launch_settings(Servers::DEFAULT).await.unwrap(),
        Some(StoredLaunch {
            settings: from_elsewhere,
            version: 2,
        })
    );
    assert_eq!(lab.entries(AuditAction::LaunchSettingsChanged).await.len(), 2);
}

#[tokio::test]
async fn launch_settings_with_an_issue_are_refused_with_every_issue_and_change_nothing() {
    let lab = Lab::new().await;
    let owner = lab.caller(&lab.owner().await).await;
    let broken = LaunchSettings {
        data_path: "server-data".to_owned(),
        extra_arguments: vec![ExtraArgument {
            name: "rcon_password".to_owned(),
            value: "hunter2".to_owned(),
        }],
        ..launch_settings()
    };

    let refused = lab
        .servers
        .change_launch_settings(&owner, Servers::DEFAULT, broken, None)
        .await;

    let crate::Error::Refused(refusal) = refused.err().unwrap() else {
        unreachable!()
    };
    assert_eq!(refusal.code, ErrorCode::InvalidLaunchSettings);
    assert_eq!(
        refusal.params["issues"],
        json!([
            { "field": "dataPath", "index": null, "problem": "notAbsolute" },
            { "field": "argumentName", "index": 0, "problem": "secret" },
        ])
    );
    assert_eq!(lab.servers.launch_settings(Servers::DEFAULT).await.unwrap(), None);
    assert_eq!(lab.entries(AuditAction::LaunchSettingsChanged).await, []);
}

#[tokio::test]
async fn only_the_owner_passes_the_owner_check_and_a_refused_change_is_audited() {
    let lab = Lab::new().await;
    let owner = lab.owner().await;
    let grace = lab.user(&owner, "grace", P::all()).await;
    let (owner, grace) = (lab.caller(&owner).await, lab.caller(&grace).await);

    lab.accounts
        .authorize_owner(&owner, "PUT", "/api/servers/default/launch")
        .await
        .unwrap();
    let read = lab
        .accounts
        .authorize_owner(&grace, "GET", "/api/servers/default/launch")
        .await;
    let change = lab
        .accounts
        .authorize_owner(&grace, "PUT", "/api/servers/default/launch")
        .await;

    assert_eq!(code(read), ErrorCode::OwnerOnly);
    assert_eq!(code(change), ErrorCode::OwnerOnly);
    let denied = lab.entries(AuditAction::PermissionDenied).await;
    assert_eq!(denied.len(), 1, "a refused read is not audited");
    assert_eq!(denied[0].actor_name.as_deref(), Some("grace"));
    assert_eq!(
        denied[0].target.as_deref(),
        Some("PUT /api/servers/default/launch")
    );
    assert_eq!(denied[0].details, json!({ "ownerOnly": true }));
}

impl Lab {
    async fn redeem(&self, ticket: &str, browser: &Browser) -> Result<Caller, ErrorCode> {
        self.accounts
            .redeem_ticket(ticket, &browser.signed_in.token, browser.client.clone())
            .await
            .map_err(|error| error.code().unwrap())
    }
}

#[tokio::test]
async fn a_websocket_ticket_works_once_for_its_own_session() {
    let lab = Lab::new().await;
    let laptop = lab.owner().await;
    let phone = lab.sign_in("ada", OWNER_PASSWORD, &office()).await.unwrap();
    let caller = lab.caller(&laptop).await;

    let ticket = lab.accounts.issue_ticket(&caller).unwrap();
    let opened = lab.redeem(&ticket, &laptop).await.unwrap();
    let again = lab.redeem(&ticket, &laptop).await;

    assert_eq!(opened.session_id, caller.session_id);
    assert_eq!(opened.username, "ada");
    assert_eq!(again.err(), Some(ErrorCode::Unauthenticated));

    // The same account's other session is another session: its cookie does not go with the
    // ticket, which is spent by the attempt.
    let ticket = lab.accounts.issue_ticket(&caller).unwrap();
    assert_eq!(
        lab.redeem(&ticket, &phone).await.err(),
        Some(ErrorCode::Unauthenticated)
    );
    assert_eq!(
        lab.redeem(&ticket, &laptop).await.err(),
        Some(ErrorCode::Unauthenticated)
    );
    assert_eq!(
        lab.redeem("not a ticket", &laptop).await.err(),
        Some(ErrorCode::Unauthenticated)
    );
}

#[tokio::test]
async fn a_websocket_ticket_expires_after_thirty_seconds_and_dies_with_its_session() {
    let lab = Lab::new().await;
    let owner = lab.owner().await;
    let caller = lab.caller(&owner).await;

    let late = lab.accounts.issue_ticket(&caller).unwrap();
    lab.clock.advance(SignedDuration::from_secs(29));
    let in_time = lab.accounts.issue_ticket(&caller).unwrap();
    lab.clock.advance(SignedDuration::from_secs(2));
    assert_eq!(
        lab.redeem(&late, &owner).await.err(),
        Some(ErrorCode::Unauthenticated)
    );
    lab.redeem(&in_time, &owner).await.unwrap();

    let orphan = lab.accounts.issue_ticket(&caller).unwrap();
    lab.accounts.logout(&caller).await.unwrap();
    assert_eq!(
        lab.redeem(&orphan, &owner).await.err(),
        Some(ErrorCode::Unauthenticated)
    );
}

#[tokio::test]
async fn a_websocket_ticket_does_not_open_before_a_temporary_password_is_replaced() {
    let lab = Lab::new().await;
    let owner = lab.caller(&lab.owner().await).await;
    let created = lab
        .accounts
        .create_user(&owner, "grace", P::DASHBOARD_VIEW)
        .await
        .unwrap();
    let grace = lab
        .sign_in("grace", &created.temporary_password, &office())
        .await
        .unwrap();
    let ticket = lab.accounts.issue_ticket(&lab.caller(&grace).await).unwrap();

    assert_eq!(
        lab.redeem(&ticket, &grace).await.err(),
        Some(ErrorCode::PasswordChangeRequired)
    );
}

#[tokio::test]
async fn a_recheck_follows_the_session_and_is_announced_when_sessions_change() {
    let lab = Lab::new().await;
    let owner = lab.owner().await;
    let grace = lab.user(&owner, "grace", P::DASHBOARD_VIEW | P::AUDIT_VIEW).await;
    let (admin, grace) = (lab.caller(&owner).await, lab.caller(&grace).await);
    let mut changes = lab.accounts.session_changes();
    changes.mark_unchanged();

    let live = lab.accounts.recheck(&grace).await.unwrap().unwrap();
    assert_eq!(live.permissions, P::DASHBOARD_VIEW | P::AUDIT_VIEW);
    assert!(!changes.has_changed().unwrap());

    let version = lab.accounts.users().await.unwrap()[1].version;
    lab.accounts
        .update_user(&admin, grace.user_id, P::DASHBOARD_VIEW, version)
        .await
        .unwrap();
    assert!(changes.has_changed().unwrap());
    assert!(lab.accounts.recheck(&grace).await.unwrap().is_none());
    changes.mark_unchanged();

    // Watching is not using: a session nobody sends a request through ends after twelve hours.
    lab.clock.advance(SignedDuration::from_hours(12));
    assert!(lab.accounts.recheck(&admin).await.unwrap().is_none());
    lab.accounts.logout(&admin).await.unwrap();
    assert!(changes.has_changed().unwrap());
}

#[tokio::test]
async fn followers_get_each_new_entry_once_in_order_from_where_the_log_ended() {
    let lab = Lab::new().await;
    lab.owner().await;
    let mut early = lab.audit.follow();
    assert_eq!(lab.audit.publish_new().await.unwrap(), 0, "setup happened before");

    lab.sign_in("ada", OWNER_PASSWORD, &office()).await.unwrap();
    // Following starts at the next publication, not at the call: nothing written meanwhile is lost.
    let mut late = lab.audit.follow();
    lab.accounts.reset_password_from_cli("ada").await.unwrap();

    assert_eq!(lab.audit.publish_new().await.unwrap(), 2);
    assert_eq!(lab.audit.publish_new().await.unwrap(), 0);
    let actions = |receiver: &mut tokio::sync::broadcast::Receiver<_>| {
        std::iter::from_fn(|| receiver.try_recv().ok())
            .map(|entry: std::sync::Arc<AuditEntry>| entry.action)
            .collect::<Vec<_>>()
    };
    let expected = [AuditAction::LoginSucceeded, AuditAction::UserPasswordReset];
    assert_eq!(actions(&mut early), expected);
    assert_eq!(actions(&mut late), expected);
    assert_eq!(
        lab.audit.latest_id().await.unwrap(),
        Some(lab.entries(AuditAction::UserPasswordReset).await[0].id)
    );
}

#[tokio::test]
async fn a_follower_too_far_behind_is_told_it_missed_entries() {
    let lab = Lab::new().await;
    let owner = lab.owner().await;
    lab.audit.publish_new().await.unwrap();
    let mut slow = lab.audit.follow();
    let caller = lab.caller(&owner).await;

    for _ in 0..300 {
        lab.accounts
            .reauthenticate(&caller, OWNER_PASSWORD.to_owned())
            .await
            .unwrap();
        lab.pause();
    }
    assert_eq!(lab.audit.publish_new().await.unwrap(), 300);

    assert!(matches!(
        slow.try_recv(),
        Err(tokio::sync::broadcast::error::TryRecvError::Lagged(44))
    ));
}
