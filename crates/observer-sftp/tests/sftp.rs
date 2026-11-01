//! The SFTP server end to end: a real listener on loopback, a real database, a server folder in a
//! temporary directory, and `russh`'s own client.
// The helpers below are test code too, though not inside a `#[test]` function.
#![allow(clippy::unwrap_used)]

use std::sync::Arc;

use observer_app::{Accounts, Clock, Files, PasswordCost};
use observer_domain::audit::AuditAction;
use observer_domain::files::{Depth, Effect, Right, Rights, Rule};
use observer_domain::launch::{LaunchSettings, OneSync, StoredLaunch};
use observer_domain::permissions::Permissions;
use observer_store::files::{self as file_store, FileKeyRow};
use observer_store::users::{self, NewUser};
use observer_store::{Database, StoreError, launch};
use russh::keys::ssh_key::HashAlg;
use russh::keys::ssh_key::private::{Ed25519Keypair, KeypairData};
use russh::keys::{PrivateKey, PrivateKeyWithHashAlg};
use russh_sftp::client::SftpSession;
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

struct Lab {
    _dir: tempfile::TempDir,
    root: std::path::PathBuf,
    db: Database,
    address: std::net::SocketAddr,
    stop: CancellationToken,
    key: Arc<PrivateKey>,
    files: Files,
    linus: i64,
}

struct Client;

// The trust check needs no I/O: the test's server is the one it started.
#[allow(clippy::unused_async_trait_impl)]
impl russh::client::Handler for Client {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _key: &russh::keys::PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

fn rule(effect: Effect, folder: &str, names: &[&str], rights: Rights) -> Rule {
    Rule {
        effect,
        folder: folder.to_owned(),
        depth: Depth::Tree,
        names: names.iter().map(|name| (*name).to_owned()).collect(),
        rights,
        max_file_size: Some(1_000_000),
    }
}

/// `linus`, his group, his key, and the launch settings naming the server folder.
async fn seed(db: &Database, fingerprint: String, data_path: String) -> i64 {
    db.write(move |tx| {
        let now = jiff::Timestamp::now();
        let user = users::insert(
            tx,
            &NewUser {
                username: "linus",
                password_hash: "unused",
                is_owner: false,
                permissions: Permissions::NONE,
                must_change_password: false,
                password_expires_at: None,
                created_at: now,
            },
        )?
        .unwrap();
        let rules = vec![
            rule(Effect::Allow, "resources", &[], Rights::FULL),
            rule(
                Effect::Deny,
                "",
                &["*.cfg"],
                Rights::of(&[Right::Read, Right::Modify]),
            ),
        ];
        let group = file_store::insert_group(tx, "default", "Developers", "", &rules, now)?.unwrap();
        let access = file_store::user_access_or_new(tx, user, now)?;
        file_store::set_access_groups(tx, access.id, &[group])?;
        file_store::insert_key(
            tx,
            &FileKeyRow {
                id: 0,
                access_id: access.id,
                algorithm: "ssh-ed25519".to_owned(),
                public_key: Vec::new(),
                fingerprint,
                comment: String::new(),
                created_at: now,
                expires_at: None,
                last_used_at: None,
                last_used_ip: None,
            },
        )?;
        launch::save(
            tx,
            "default",
            &StoredLaunch {
                settings: LaunchSettings {
                    data_path,
                    cfg_file: "server.cfg".to_owned(),
                    onesync: OneSync::On,
                    auto_start: false,
                    txadmin_events: false,
                    extra_arguments: Vec::new(),
                    hang_detection: true,
                    hang_timeout_seconds: 360,
                },
                version: 1,
                legacy_build_path: None,
            },
            now,
        )?;
        Ok::<_, StoreError>(user)
    })
    .await
    .unwrap()
}

impl Lab {
    /// `linus` holds a group that edits `resources` but never a `*.cfg`, and one key.
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("server-data");
        std::fs::create_dir_all(root.join("resources/[maps]/harbour")).unwrap();
        std::fs::write(
            root.join("resources/[maps]/harbour/fxmanifest.lua"),
            "fx_version 'cerulean'",
        )
        .unwrap();
        std::fs::write(root.join("server.cfg"), "sv_licenseKey secret").unwrap();
        let db = Database::open(dir.path().join("observer.sqlite3"), dir.path().join("backups"))
            .await
            .unwrap();
        let clock = Clock::system();
        let accounts = Accounts::new(db.clone(), clock.clone(), PasswordCost::TEST)
            .await
            .unwrap();
        let files = Files::new(db.clone(), clock, accounts);

        let key =
            PrivateKey::new(KeypairData::Ed25519(Ed25519Keypair::from_seed(&[7; 32])), "linus").unwrap();
        let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();
        let data_path = root.to_string_lossy().into_owned();
        let linus = seed(&db, fingerprint, data_path).await;

        let host_key = observer_sftp::host_key(&files).await.unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let stop = CancellationToken::new();
        tokio::spawn(observer_sftp::serve(
            listener,
            files.clone(),
            host_key,
            Vec::new(),
            stop.clone(),
        ));
        Self {
            _dir: dir,
            root,
            db,
            address,
            stop,
            key: Arc::new(key),
            files,
            linus,
        }
    }

    async fn connect(&self, login: &str, key: Arc<PrivateKey>) -> Option<SftpSession> {
        let config = Arc::new(russh::client::Config::default());
        let mut session = russh::client::connect(config, self.address, Client)
            .await
            .unwrap();
        let auth = session
            .authenticate_publickey(login, PrivateKeyWithHashAlg::new(key, None))
            .await
            .unwrap();
        if !auth.success() {
            return None;
        }
        let channel = session.channel_open_session().await.unwrap();
        channel.request_subsystem(true, "sftp").await.unwrap();
        Some(SftpSession::new(channel.into_stream()).await.unwrap())
    }

    async fn entries(&self, action: AuditAction) -> usize {
        let action = action.as_str();
        self.db
            .read(move |conn| {
                conn.query_row(
                    "SELECT count(*) FROM audit_log WHERE action = ?1",
                    [action],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(StoreError::from)
            })
            .await
            .unwrap()
            .try_into()
            .unwrap()
    }
}

#[tokio::test]
async fn a_key_login_reads_writes_and_renames_within_its_rules_and_nothing_else() {
    let lab = Lab::new().await;
    let stranger =
        Arc::new(PrivateKey::new(KeypairData::Ed25519(Ed25519Keypair::from_seed(&[9; 32])), "eve").unwrap());
    assert!(lab.connect("linus", stranger).await.is_none());
    let sftp = lab.connect("linus", Arc::clone(&lab.key)).await.unwrap();

    let top: Vec<String> = sftp
        .read_dir("/")
        .await
        .unwrap()
        .map(|entry| entry.file_name())
        .collect();
    assert_eq!(
        top,
        ["resources"],
        "server.cfg is denied and nothing else leads anywhere"
    );
    let manifest = sftp
        .read("/resources/[maps]/harbour/fxmanifest.lua")
        .await
        .unwrap();
    assert_eq!(manifest, b"fx_version 'cerulean'");
    assert!(sftp.read("/server.cfg").await.is_err());
    assert!(sftp.read("/../server.cfg").await.is_err());

    let mut upload = sftp.create("/resources/[maps]/harbour/client.lua").await.unwrap();
    upload.write_all(b"Citizen.Wait(0)").await.unwrap();
    upload.shutdown().await.unwrap();
    drop(upload);
    assert_eq!(
        std::fs::read_to_string(lab.root.join("resources/[maps]/harbour/client.lua")).unwrap(),
        "Citizen.Wait(0)"
    );
    let left: Vec<_> = std::fs::read_dir(lab.root.join("resources/[maps]/harbour"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    assert!(
        left.iter().all(|name| !name.starts_with(".observer-upload-")),
        "{left:?}"
    );

    sftp.rename(
        "/resources/[maps]/harbour/client.lua",
        "/resources/[maps]/harbour/main.lua",
    )
    .await
    .unwrap();
    assert!(lab.root.join("resources/[maps]/harbour/main.lua").exists());
    // Renaming into a name a deny covers is refused, so is writing outside the rules.
    assert!(
        sftp.rename(
            "/resources/[maps]/harbour/main.lua",
            "/resources/[maps]/harbour/main.cfg"
        )
        .await
        .is_err()
    );
    assert!(sftp.create("/server.cfg").await.is_err());
    assert!(sftp.create("/new.lua").await.is_err());
    assert_eq!(
        std::fs::read_to_string(lab.root.join("server.cfg")).unwrap(),
        "sv_licenseKey secret"
    );

    assert_eq!(lab.entries(AuditAction::FileWritten).await, 1);
    assert_eq!(lab.entries(AuditAction::FileRenamed).await, 1);
    lab.stop.cancel();
}

#[tokio::test]
async fn a_session_ends_as_soon_as_its_groups_are_taken_away() {
    let lab = Lab::new().await;
    let sftp = lab.connect("linus", Arc::clone(&lab.key)).await.unwrap();
    assert!(
        sftp.read("/resources/[maps]/harbour/fxmanifest.lua")
            .await
            .is_ok()
    );
    let owner = observer_app::Caller {
        session_id: 0,
        user_id: 0,
        username: "ada".to_owned(),
        is_owner: true,
        permissions: Permissions::NONE,
        must_change_password: false,
        reauthenticated_at: None,
        client: observer_app::Client {
            ip: "127.0.0.1".parse().unwrap(),
            user_agent: None,
        },
    };

    lab.files
        .set_user_groups(&owner, lab.linus, Vec::new(), None)
        .await
        .unwrap();

    let mut ended = false;
    for _ in 0..50 {
        if sftp
            .read("/resources/[maps]/harbour/fxmanifest.lua")
            .await
            .is_err()
        {
            ended = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(ended, "the session still reads after its groups were taken away");
    assert!(lab.connect("linus", Arc::clone(&lab.key)).await.is_none());
    lab.stop.cancel();
}

#[tokio::test]
async fn a_pipeline_uploads_with_the_key_observer_made_and_showed_once() {
    let lab = Lab::new().await;
    let owner = observer_app::Caller {
        session_id: 0,
        user_id: 0,
        username: "ada".to_owned(),
        is_owner: true,
        permissions: Permissions::NONE,
        must_change_password: false,
        reauthenticated_at: None,
        client: observer_app::Client {
            ip: "127.0.0.1".parse().unwrap(),
            user_agent: None,
        },
    };
    let group = lab.files.groups().await.unwrap().remove(0);

    let created = lab
        .files
        .create_service(
            &owner,
            observer_domain::files::CreateServiceRequest {
                name: "deploy-main".to_owned(),
                groups: vec![group.row.id],
                networks: vec!["127.0.0.0/8".to_owned()],
                expires_in_days: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(created.login, "+deploy-main");
    assert!(created.access.keys[0].expires_at.is_some());
    let key = Arc::new(PrivateKey::from_openssh(&created.private_key).unwrap());

    let sftp = lab.connect("+deploy-main", key).await.unwrap();
    let mut upload = sftp.create("/resources/[maps]/harbour/stream.ytd").await.unwrap();
    upload.write_all(b"texture").await.unwrap();
    upload.shutdown().await.unwrap();
    drop(upload);
    assert_eq!(
        std::fs::read(lab.root.join("resources/[maps]/harbour/stream.ytd")).unwrap(),
        b"texture"
    );
    let actor: String = lab
        .db
        .read(|conn| {
            conn.query_row(
                "SELECT actor_kind || ' ' || actor_name FROM audit_log WHERE action = 'file_written'",
                [],
                |row| row.get(0),
            )
            .map_err(StoreError::from)
        })
        .await
        .unwrap();
    assert_eq!(actor, "service +deploy-main");
    lab.stop.cancel();
}

#[tokio::test]
async fn a_refused_write_never_replaces_the_file_and_cfgs_and_folders_stay_put() {
    let lab = Lab::new().await;
    let sftp = lab.connect("linus", Arc::clone(&lab.key)).await.unwrap();
    let manifest = lab.root.join("resources/[maps]/harbour/fxmanifest.lua");

    // Past the rule's 1 MB, the write is refused and the close commits nothing.
    let mut upload = sftp
        .create("/resources/[maps]/harbour/fxmanifest.lua")
        .await
        .unwrap();
    let _ = upload.write_all(&vec![b'x'; 1_200_000]).await;
    let _ = upload.shutdown().await;
    drop(upload);
    assert_eq!(
        std::fs::read_to_string(&manifest).unwrap(),
        "fx_version 'cerulean'"
    );

    assert!(
        sftp.create("/resources/[maps]/harbour/permissions.cfg")
            .await
            .is_err()
    );
    assert!(sftp.rename("/resources/[maps]", "/resources/maps").await.is_err());
    assert!(lab.root.join("resources/[maps]/harbour").exists());
    let left: Vec<_> = std::fs::read_dir(lab.root.join("resources/[maps]/harbour"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(left, ["fxmanifest.lua"]);
    lab.stop.cancel();
}
