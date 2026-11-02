//! The SFTP server: SSH with Observer's own algorithm lists, logins by key checked by
//! `observer-app`, and each session's requests served from a `ConfinedTree` under its access's
//! rules. The SSH stack stays in this crate.

mod handler;

use std::borrow::Cow;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use observer_app::{Files, Grant, Login};
use observer_platform::ConfinedTree;
use observer_store::files::HostKeyRow;
use russh::keys::ssh_key::private::{Ed25519Keypair, KeypairData};
use russh::keys::ssh_key::{HashAlg, LineEnding};
use russh::keys::{Algorithm, EcdsaCurve, PrivateKey};
use russh::server::{Auth, ChannelOpenHandle, Msg, PublicKeyOffer, Server as _, Session, VerifiedPublicKey};
use russh::{Channel, ChannelId, MethodKind, MethodSet, Preferred, cipher, kex, mac};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

/// How fingerprints are shown, as OpenSSH prints them.
pub const FINGERPRINT: HashAlg = HashAlg::Sha256;

/// The user key algorithms accepted, and the host key's.
const KEY_ALGORITHMS: [Algorithm; 3] = [
    Algorithm::Ed25519,
    Algorithm::Ecdsa {
        curve: EcdsaCurve::NistP256,
    },
    Algorithm::Ecdsa {
        curve: EcdsaCurve::NistP384,
    },
];

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    App(#[from] observer_app::Error),
    #[error("the SFTP host key is unusable: {0}")]
    HostKey(String),
    #[error("the operating system gave no random bytes: {0}")]
    Random(getrandom::Error),
    #[error(transparent)]
    Ssh(#[from] russh::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// The server's Ed25519 key: the one stored, or a new one made from the OS random source at the
/// first start with SFTP on.
pub async fn host_key(files: &Files) -> Result<PrivateKey, Error> {
    let stored = if let Some(stored) = files.host_key().await? {
        stored
    } else {
        let mut seed = [0u8; 32];
        getrandom::fill(&mut seed).map_err(Error::Random)?;
        let key = PrivateKey::new(KeypairData::Ed25519(Ed25519Keypair::from_seed(&seed)), "observer")
            .map_err(|error| Error::HostKey(error.to_string()))?;
        let encoded = key
            .to_openssh(LineEnding::LF)
            .map_err(|error| Error::HostKey(error.to_string()))?;
        files
            .store_host_key(HostKeyRow {
                algorithm: key.algorithm().to_string(),
                private_key: encoded.as_bytes().to_vec(),
                fingerprint: key.fingerprint(HashAlg::Sha256).to_string(),
                created_at: jiff::Timestamp::now(),
            })
            .await?
    };
    let text = String::from_utf8(stored.private_key).map_err(|error| Error::HostKey(error.to_string()))?;
    PrivateKey::from_openssh(text).map_err(|error| Error::HostKey(error.to_string()))
}

/// The SSH configuration: Observer's algorithm lists, keys only, a fixed delay after a refusal.
pub fn config(host_key: PrivateKey) -> russh::server::Config {
    russh::server::Config {
        methods: MethodSet::from(&[MethodKind::PublicKey][..]),
        auth_rejection_time: Duration::from_secs(1),
        auth_rejection_time_initial: Some(Duration::from_secs(1)),
        max_auth_attempts: 10,
        inactivity_timeout: Some(Duration::from_mins(15)),
        keepalive_interval: Some(Duration::from_mins(1)),
        keepalive_max: 3,
        keys: vec![host_key],
        preferred: Preferred {
            kex: Cow::Borrowed(&[
                kex::MLKEM768X25519_SHA256,
                kex::CURVE25519,
                kex::CURVE25519_PRE_RFC_8731,
                kex::EXTENSION_SUPPORT_AS_SERVER,
                kex::EXTENSION_OPENSSH_STRICT_KEX_AS_SERVER,
            ]),
            key: Cow::Owned(KEY_ALGORITHMS.to_vec()),
            cipher: Cow::Borrowed(&[
                cipher::CHACHA20_POLY1305,
                cipher::AES_256_GCM,
                cipher::AES_128_GCM,
            ]),
            mac: Cow::Borrowed(&[mac::HMAC_SHA256_ETM, mac::HMAC_SHA512_ETM]),
            compression: Cow::Borrowed(&[russh::compression::NONE]),
            ..Preferred::default()
        },
        ..Default::default()
    }
}

/// Serves SFTP on `listener` until `stop` is cancelled.
/// Serves SFTP on `listener` until `stop` is cancelled. `refused` are Observer's own folders and
/// files: no session reaches them, wherever the server folder is.
pub async fn serve(
    listener: TcpListener,
    files: Files,
    host_key: PrivateKey,
    refused: Vec<PathBuf>,
    stop: CancellationToken,
) -> std::io::Result<()> {
    let mut server = Server {
        files,
        refused: Arc::new(refused),
    };
    let running = server.run_on_socket(Arc::new(config(host_key)), &listener);
    let shutdown = running.handle();
    tokio::select! {
        result = running => result,
        () = stop.cancelled() => {
            shutdown.shutdown("the SFTP server is closing".to_owned());
            Ok(())
        }
    }
}

#[derive(Clone)]
struct Server {
    files: Files,
    refused: Arc<Vec<PathBuf>>,
}

impl russh::server::Server for Server {
    type Handler = Connection;

    fn new_client(&mut self, peer: Option<SocketAddr>) -> Connection {
        Connection {
            files: self.files.clone(),
            refused: Arc::clone(&self.refused),
            ip: peer.map_or(IpAddr::from([0, 0, 0, 0]), |peer| peer.ip()),
            grant: None,
            channel: None,
        }
    }
}

struct Connection {
    files: Files,
    refused: Arc<Vec<PathBuf>>,
    ip: IpAddr,
    grant: Option<Grant>,
    channel: Option<Channel<Msg>>,
}

// The trait's methods are async; the refusals answer at once.
#[allow(clippy::unused_async_trait_impl)]
impl russh::server::Handler for Connection {
    type Error = Error;

    async fn auth_publickey_offered(&mut self, user: &str, offer: PublicKeyOffer<'_>) -> Result<Auth, Error> {
        if !KEY_ALGORITHMS.contains(offer.algorithm) {
            return Ok(Auth::reject());
        }
        let fingerprint = offer.public_key.fingerprint(HashAlg::Sha256).to_string();
        let known = self.files.key_registered(user.to_owned(), fingerprint).await?;
        Ok(if known { Auth::Accept } else { Auth::reject() })
    }

    async fn auth_publickey(&mut self, user: &str, key: VerifiedPublicKey<'_>) -> Result<Auth, Error> {
        let fingerprint = key.public_key.fingerprint(HashAlg::Sha256).to_string();
        match self.files.login(user.to_owned(), fingerprint, self.ip).await? {
            Login::Granted(grant) => {
                self.grant = Some(grant);
                Ok(Auth::Accept)
            }
            Login::Refused | Login::Failed | Login::Locked | Login::NoServer => Ok(Auth::reject()),
        }
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Error> {
        // One SFTP channel per connection; a second open is refused when `reply` drops.
        if self.channel.is_none() && self.grant.is_some() {
            self.channel = Some(channel);
            reply.accept().await;
        }
        Ok(())
    }

    async fn subsystem_request(
        &mut self,
        id: ChannelId,
        name: &str,
        session: &mut Session,
    ) -> Result<(), Error> {
        let (Some(grant), "sftp") = (self.grant.clone(), name) else {
            refuse(session, id);
            return Ok(());
        };
        let Some(channel) = self.channel.take() else {
            refuse(session, id);
            return Ok(());
        };
        let root = grant.root.clone();
        let refused = Arc::clone(&self.refused);
        let opened = tokio::task::spawn_blocking(move || {
            ConfinedTree::open(Path::new(&root))
                .map(|tree| (tree, observer_app::paths_inside(Path::new(&root), &refused)))
        })
        .await
        .map_err(std::io::Error::other)?;
        let (tree, refused) = match opened {
            Ok((tree, refused)) => (Arc::new(tree), refused),
            Err(error) => {
                tracing::warn!(%error, "the server folder cannot be opened for SFTP");
                refuse(session, id);
                return Ok(());
            }
        };
        session
            .channel_success(id)
            .map_err(|error| Error::Io(std::io::Error::other(error.to_string())))?;
        let watch = CancellationToken::new();
        tokio::spawn(revoke_on_change(
            self.files.clone(),
            grant.clone(),
            self.ip,
            session.handle(),
            watch.clone(),
        ));
        let handler = handler::Sftp::new(
            self.files.clone(),
            grant,
            self.ip.to_string(),
            tree,
            refused,
            watch.drop_guard(),
        );
        russh_sftp::server::run(channel.into_stream(), handler).await;
        Ok(())
    }

    async fn exec_request(
        &mut self,
        id: ChannelId,
        _data: &[u8],
        session: &mut Session,
    ) -> Result<(), Error> {
        refuse(session, id);
        Ok(())
    }

    async fn shell_request(&mut self, id: ChannelId, session: &mut Session) -> Result<(), Error> {
        refuse(session, id);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn pty_request(
        &mut self,
        id: ChannelId,
        _term: &str,
        _columns: u32,
        _rows: u32,
        _width: u32,
        _height: u32,
        _modes: &[(russh::Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Error> {
        refuse(session, id);
        Ok(())
    }

    async fn env_request(
        &mut self,
        id: ChannelId,
        _name: &str,
        _value: &str,
        session: &mut Session,
    ) -> Result<(), Error> {
        refuse(session, id);
        Ok(())
    }
}

/// How often a session checks its access again, for changes made from the command line, which
/// no in-process signal reports.
const RECHECK: Duration = Duration::from_secs(15);

/// Ends the session once its access changes: a group or a rule, a key removed, the access
/// disabled or expired, the server folder moved.
async fn revoke_on_change(
    files: Files,
    grant: Grant,
    ip: IpAddr,
    handle: russh::server::Handle,
    session: CancellationToken,
) {
    let mut changes = files.changes();
    let mut accounts = files.account_changes();
    let mut recheck = tokio::time::interval(RECHECK);
    recheck.tick().await;
    loop {
        tokio::select! {
            () = session.cancelled() => return,
            signal = changes.changed() => {
                if signal.is_err() {
                    return;
                }
            }
            signal = accounts.changed() => {
                if signal.is_err() {
                    return;
                }
            }
            _ = recheck.tick() => {}
        }
        match files.still_granted(&grant, ip).await {
            Ok(true) => {}
            Ok(false) => {
                tracing::info!(login = %grant.login, "an SFTP session ended: its access changed");
                let _ = handle
                    .disconnect(
                        russh::Disconnect::ByApplication,
                        "access changed, reconnect".to_owned(),
                        "en".to_owned(),
                    )
                    .await;
                return;
            }
            Err(error) => tracing::warn!(%error, "an SFTP session's access could not be checked again"),
        }
    }
}

/// Removes the temporary upload files a crash left, each under the root it was made in, and
/// forgets them. Only an exact upload name is removed, never through a link.
pub async fn sweep_uploads(files: &Files) -> Result<(), Error> {
    for upload in files.uploads().await? {
        let name = upload.name.clone();
        let folder = upload.folder.clone();
        let root = upload.root.clone();
        let removed = tokio::task::spawn_blocking(move || {
            if !name.starts_with(observer_domain::files::UPLOAD_PREFIX) {
                return Ok(());
            }
            let tree = ConfinedTree::open(Path::new(&root)).map_err(observer_platform::TreeError::Io)?;
            let folder =
                observer_platform::RelativePath::new(folder.split('/').filter(|part| !part.is_empty()))
                    .map_err(|_| observer_platform::TreeError::Escaped)?;
            match tree.remove_file(&tree.open_folder(&folder)?, &name) {
                Ok(()) | Err(observer_platform::TreeError::NotFound) => Ok(()),
                Err(error) => Err(error),
            }
        })
        .await
        .map_err(std::io::Error::other)?;
        if let Err(error) = removed {
            tracing::warn!(%error, root = %upload.root, "a temporary upload file could not be removed");
        }
        files.forget_upload(upload.id).await?;
    }
    Ok(())
}

/// Answers a channel request with a failure, so that a client never waits for a reply.
fn refuse(session: &mut Session, id: ChannelId) {
    let _ = session.channel_failure(id);
}
