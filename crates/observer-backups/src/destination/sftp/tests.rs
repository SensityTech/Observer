//! Against an SSH server on the loopback whose SFTP subsystem keeps its files in memory: a password
//! and a key it accepts, a host key of its own.

use std::collections::{HashMap, HashSet};

use russh::server::{Auth, Msg, Server as _, Session as ServerSession};
use russh::{Channel, ChannelId, MethodKind, MethodSet};
use russh_sftp::protocol::{
    Attrs, Data, File, FileAttributes, Handle as FileHandle, Name, OpenFlags, Status, Version,
};
use tokio::net::TcpListener;

use super::*;

const PASSWORD: &str = "hunter2";

#[derive(Default)]
struct Disk {
    files: Mutex<HashMap<String, Vec<u8>>>,
    folders: Mutex<HashSet<String>>,
}

impl Disk {
    fn file(&self, path: &str) -> Option<Vec<u8>> {
        self.files.lock().unwrap().get(path).cloned()
    }
}

#[derive(Clone)]
struct Host {
    disk: Arc<Disk>,
    user_key: PublicKey,
}

impl russh::server::Server for Host {
    type Handler = Login;

    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Login {
        Login {
            host: self.clone(),
            channel: None,
        }
    }
}

struct Login {
    host: Host,
    channel: Option<Channel<Msg>>,
}

// The traits' methods are async; the memory answers at once.
#[allow(clippy::unused_async_trait_impl)]
impl russh::server::Handler for Login {
    type Error = russh::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        Ok(if user == "fivem" && password == PASSWORD {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn auth_publickey_offered(
        &mut self,
        user: &str,
        offer: russh::server::PublicKeyOffer<'_>,
    ) -> Result<Auth, Self::Error> {
        Ok(
            if user == "fivem" && offer.public_key.key_data() == self.host.user_key.key_data() {
                Auth::Accept
            } else {
                Auth::reject()
            },
        )
    }

    async fn auth_publickey(
        &mut self,
        _: &str,
        key: russh::server::VerifiedPublicKey<'_>,
    ) -> Result<Auth, Self::Error> {
        Ok(if key.public_key.key_data() == self.host.user_key.key_data() {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: russh::server::ChannelOpenHandle,
        _: &mut ServerSession,
    ) -> Result<(), Self::Error> {
        self.channel = Some(channel);
        reply.accept().await;
        Ok(())
    }

    async fn subsystem_request(
        &mut self,
        id: ChannelId,
        name: &str,
        session: &mut ServerSession,
    ) -> Result<(), Self::Error> {
        let (Some(channel), "sftp") = (self.channel.take(), name) else {
            session.channel_failure(id)?;
            return Ok(());
        };
        session.channel_success(id)?;
        let files = Files {
            disk: Arc::clone(&self.host.disk),
            handles: HashMap::new(),
            next: 0,
        };
        russh_sftp::server::run(channel.into_stream(), files).await;
        Ok(())
    }
}

struct Files {
    disk: Arc<Disk>,
    handles: HashMap<String, String>,
    next: u32,
}

fn ok(id: u32) -> Status {
    Status {
        id,
        status_code: StatusCode::Ok,
        error_message: "Ok".into(),
        language_tag: "en-US".into(),
    }
}

#[allow(clippy::unused_async_trait_impl)]
impl russh_sftp::server::Handler for Files {
    type Error = StatusCode;

    fn unimplemented(&self) -> StatusCode {
        StatusCode::OpUnsupported
    }

    async fn init(&mut self, _: u32, _: HashMap<String, String>) -> Result<Version, StatusCode> {
        Ok(Version::new())
    }

    async fn open(
        &mut self,
        id: u32,
        path: String,
        flags: OpenFlags,
        _: FileAttributes,
    ) -> Result<FileHandle, StatusCode> {
        let mut files = self.disk.files.lock().unwrap();
        if flags.contains(OpenFlags::CREATE) {
            if flags.contains(OpenFlags::TRUNCATE) || !files.contains_key(&path) {
                files.insert(path.clone(), Vec::new());
            }
        } else if !files.contains_key(&path) {
            return Err(StatusCode::NoSuchFile);
        }
        self.next += 1;
        let handle = self.next.to_string();
        self.handles.insert(handle.clone(), path);
        Ok(FileHandle { id, handle })
    }

    async fn close(&mut self, id: u32, handle: String) -> Result<Status, StatusCode> {
        self.handles.remove(&handle);
        Ok(ok(id))
    }

    async fn read(&mut self, id: u32, handle: String, offset: u64, len: u32) -> Result<Data, StatusCode> {
        let path = self.handles.get(&handle).ok_or(StatusCode::Failure)?;
        let files = self.disk.files.lock().unwrap();
        let file = files.get(path).ok_or(StatusCode::NoSuchFile)?;
        let start = usize::try_from(offset).unwrap_or(usize::MAX);
        if start >= file.len() {
            return Err(StatusCode::Eof);
        }
        let end = file.len().min(start + len as usize);
        Ok(Data {
            id,
            data: file[start..end].to_vec(),
        })
    }

    async fn write(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        data: Vec<u8>,
    ) -> Result<Status, StatusCode> {
        let path = self.handles.get(&handle).ok_or(StatusCode::Failure)?;
        let mut files = self.disk.files.lock().unwrap();
        let file = files.get_mut(path).ok_or(StatusCode::NoSuchFile)?;
        let start = usize::try_from(offset).unwrap_or(usize::MAX);
        if file.len() < start + data.len() {
            file.resize(start + data.len(), 0);
        }
        file[start..start + data.len()].copy_from_slice(&data);
        Ok(ok(id))
    }

    async fn fstat(&mut self, id: u32, handle: String) -> Result<Attrs, StatusCode> {
        let path = self.handles.get(&handle).cloned().ok_or(StatusCode::Failure)?;
        self.stat(id, path).await
    }

    async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, StatusCode> {
        if let Some(file) = self.disk.file(&path) {
            return Ok(Attrs {
                id,
                attrs: FileAttributes {
                    size: Some(file.len() as u64),
                    ..FileAttributes::default()
                },
            });
        }
        if self.disk.folders.lock().unwrap().contains(&path) {
            return Ok(Attrs {
                id,
                attrs: FileAttributes::default(),
            });
        }
        Err(StatusCode::NoSuchFile)
    }

    async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, StatusCode> {
        self.stat(id, path).await
    }

    async fn mkdir(&mut self, id: u32, path: String, _: FileAttributes) -> Result<Status, StatusCode> {
        self.disk.folders.lock().unwrap().insert(path);
        Ok(ok(id))
    }

    async fn rename(&mut self, id: u32, from: String, to: String) -> Result<Status, StatusCode> {
        let mut files = self.disk.files.lock().unwrap();
        if files.contains_key(&to) {
            return Err(StatusCode::Failure);
        }
        let file = files.remove(&from).ok_or(StatusCode::NoSuchFile)?;
        files.insert(to, file);
        Ok(ok(id))
    }

    async fn remove(&mut self, id: u32, path: String) -> Result<Status, StatusCode> {
        self.disk
            .files
            .lock()
            .unwrap()
            .remove(&path)
            .map(|_| ok(id))
            .ok_or(StatusCode::NoSuchFile)
    }

    async fn realpath(&mut self, id: u32, path: String) -> Result<Name, StatusCode> {
        Ok(Name {
            id,
            files: vec![File::dummy(path)],
        })
    }
}

struct Started {
    disk: Arc<Disk>,
    port: u16,
    host_key: String,
    user_key: Secret,
}

async fn start() -> Started {
    let (user_key, user_public) = new_key().unwrap();
    let mut seed = [7u8; 32];
    seed[0] = 1;
    let host = PrivateKey::new(
        KeypairData::Ed25519(Ed25519Keypair::from_seed(&seed)),
        "test-host",
    )
    .unwrap();
    let host_key = host_key(host.public_key()).key;
    let config = russh::server::Config {
        methods: MethodSet::from(&[MethodKind::Password, MethodKind::PublicKey][..]),
        auth_rejection_time: Duration::from_millis(10),
        auth_rejection_time_initial: Some(Duration::ZERO),
        keys: vec![host],
        ..russh::server::Config::default()
    };
    let disk = Arc::new(Disk::default());
    let mut server = Host {
        disk: Arc::clone(&disk),
        user_key: PublicKey::from_openssh(&user_public).unwrap(),
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { server.run_on_socket(Arc::new(config), &listener).await });
    Started {
        disk,
        port,
        host_key,
        user_key,
    }
}

fn remote<'a>(started: &Started, auth: SftpAuth, host_key: Option<&'a str>) -> Remote<'a> {
    Remote {
        host: "127.0.0.1",
        port: started.port,
        user: "fivem",
        folder: "backups/es_extended",
        auth,
        host_key,
    }
}

fn backup(dir: &std::path::Path) -> std::path::PathBuf {
    let path = dir.join("es-1.sql.gz");
    std::fs::write(&path, vec![42u8; 300 * 1024]).unwrap();
    path
}

#[tokio::test]
async fn a_copy_goes_up_with_observer_s_key_to_the_pinned_host() {
    let started = start().await;
    let remote = remote(&started, SftpAuth::Key, Some(&started.host_key));
    let dir = tempfile::tempdir().unwrap();
    let location = location(remote.folder, "es-1.sql.gz");
    let progress = Arc::new(AtomicU64::new(0));

    let sent = upload(
        &remote,
        &started.user_key,
        &backup(dir.path()),
        &location,
        Arc::clone(&progress),
        CancellationToken::new(),
    )
    .await
    .unwrap();

    assert_eq!(sent, 300 * 1024);
    assert_eq!(progress.load(Ordering::Relaxed), 300 * 1024);
    assert_eq!(
        started
            .disk
            .file("backups/es_extended/es-1.sql.gz")
            .unwrap()
            .len(),
        300 * 1024
    );
    assert_eq!(started.disk.file("backups/es_extended/es-1.sql.gz.partial"), None);
    assert!(started.disk.folders.lock().unwrap().contains("backups"));

    let (mut download, size) = open(&remote, &started.user_key, &location).await.unwrap();
    let mut back = Vec::new();
    download.read_to_end(&mut back).await.unwrap();
    assert_eq!((back.len(), size), (300 * 1024, Some(300 * 1024)));

    delete(&remote, &started.user_key, &location).await.unwrap();
    delete(&remote, &started.user_key, &location).await.unwrap();
    assert_eq!(started.disk.file("backups/es_extended/es-1.sql.gz"), None);
}

#[tokio::test]
async fn a_password_works_and_a_wrong_one_says_so_without_itself() {
    let started = start().await;
    let remote = remote(&started, SftpAuth::Password, Some(&started.host_key));

    let (passed, _) = test(&remote, &Secret::new(PASSWORD)).await;
    let (refused, _) = test(&remote, &Secret::new("wrong-password")).await;

    assert_eq!(passed, Ok(()));
    let refused = refused.unwrap_err();
    assert_eq!(refused.error, BackupError::Authentication);
    assert!(!refused.detail.unwrap().contains("wrong-password"));
}

#[tokio::test]
async fn nothing_is_sent_to_a_host_whose_key_is_not_pinned() {
    let started = start().await;
    let dir = tempfile::tempdir().unwrap();
    let unpinned = remote(&started, SftpAuth::Password, None);

    let failure = upload(
        &unpinned,
        &Secret::new(PASSWORD),
        &backup(dir.path()),
        "es-1.sql.gz",
        Arc::default(),
        CancellationToken::new(),
    )
    .await
    .unwrap_err();

    assert_eq!(failure.error, BackupError::HostKeyUnknown);
    assert!(started.disk.files.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_host_showing_another_key_is_refused_and_its_key_shown() {
    let started = start().await;
    let (_, other) = new_key().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let changed = remote(&started, SftpAuth::Password, Some(&other));

    let failure = upload(
        &changed,
        &Secret::new(PASSWORD),
        &backup(dir.path()),
        "es-1.sql.gz",
        Arc::default(),
        CancellationToken::new(),
    )
    .await
    .unwrap_err();
    let (tested, shown) = test(&changed, &Secret::new(PASSWORD)).await;

    assert_eq!(failure.error, BackupError::HostKeyMismatch);
    assert_eq!(tested.unwrap_err().error, BackupError::HostKeyMismatch);
    let shown = shown.unwrap();
    assert_eq!(shown.key, started.host_key);
    assert!(shown.fingerprint.starts_with("SHA256:"));
    assert!(started.disk.files.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_test_of_a_new_host_shows_its_key_and_leaves_nothing() {
    let started = start().await;
    let unpinned = remote(&started, SftpAuth::Key, None);

    let (tested, shown) = test(&unpinned, &started.user_key).await;

    assert_eq!(tested, Ok(()));
    assert_eq!(shown.unwrap().key, started.host_key);
    assert!(started.disk.files.lock().unwrap().is_empty());
}

#[test]
fn only_plain_ed25519_and_ecdsa_host_keys_are_pinned() {
    let (_, public) = new_key().unwrap();

    assert!(parse_host_key(&public).is_some());
    assert!(parse_host_key("ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQC").is_none());
    let disguised = public.replacen("ssh-ed25519", "ecdsa-sha2-nistp256", 1);
    assert!(parse_host_key(&disguised).is_none());
    assert!(parse_host_key("ssh-ed25519-cert-v01@openssh.com AAAA").is_none());
}

#[test]
fn locations_sit_in_the_folder_or_the_home() {
    assert_eq!(location("", "a.sql.gz"), "a.sql.gz");
    assert_eq!(location("/srv/backups/", "a.sql.gz"), "/srv/backups/a.sql.gz");
    assert_eq!(location("dumps", "a.sql.gz"), "dumps/a.sql.gz");
}
