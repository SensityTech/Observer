//! An SFTP server: any host with OpenSSH, Linux or Windows. Observer logs in with a key of its own
//! made for the destination, or a password, and only to a host whose key the owner pinned. A copy
//! is written as `<name>.partial`, renamed once whole, then its size read back.
//!
//! The SSH stack is the one Observer's own SFTP server runs on, with the same algorithm lists:
//! hosts that offer only RSA host keys are refused, since the `rsa` crate is left out on purpose.

use std::borrow::Cow;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use observer_domain::backups::{BackupError, HostKey, Secret, SftpAuth};
use russh::client::{self, AuthResult, Handle};
use russh::keys::{
    Algorithm, EcdsaCurve, HashAlg, PrivateKey, PrivateKeyWithHashAlg, PublicKey, PublicKeyOrCertificate,
};
use russh::{AlgorithmKind, Preferred, cipher, kex, mac};
use russh_sftp::client::SftpSession;
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::protocol::StatusCode;
use ssh_key::LineEnding;
use ssh_key::private::{Ed25519Keypair, KeypairData};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt, ReadBuf};
use tokio_util::sync::CancellationToken;

use crate::{Failure, redact};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const INACTIVITY: Duration = Duration::from_secs(60);
const CHUNK: usize = 256 * 1024;
const KEY_ALGORITHMS: [Algorithm; 3] = [
    Algorithm::Ed25519,
    Algorithm::Ecdsa {
        curve: EcdsaCurve::NistP256,
    },
    Algorithm::Ecdsa {
        curve: EcdsaCurve::NistP384,
    },
];
/// The host key types read from a pinned line, with how their blob starts: a certificate's blob
/// behind a plain label is refused before the parser sees it.
const HOST_KEY_TYPES: [(&str, &str); 3] = [
    ("ssh-ed25519", "AAAAC3NzaC1lZDI1NTE5"),
    ("ecdsa-sha2-nistp256", "AAAAE2VjZHNhLXNoYTItbmlzdHAyNTY"),
    ("ecdsa-sha2-nistp384", "AAAAE2VjZHNhLXNoYTItbmlzdHAzODQ"),
];
const MAX_KEY_LINE: usize = 1024;

/// The fields of an SFTP destination.
#[derive(Debug, Clone, Copy)]
pub struct Remote<'a> {
    pub host: &'a str,
    pub port: u16,
    pub user: &'a str,
    pub folder: &'a str,
    pub auth: SftpAuth,
    pub host_key: Option<&'a str>,
}

/// The copy's path on the server: in `folder`, or the account's home when it is empty.
pub fn location(folder: &str, name: &str) -> String {
    let folder = folder.trim_end_matches('/');
    if folder.is_empty() {
        name.to_owned()
    } else {
        format!("{folder}/{name}")
    }
}

/// A new Ed25519 key for a destination: its private half in OpenSSH form, kept as the
/// destination's secret, and its public half as an `authorized_keys` line.
pub fn new_key() -> Result<(Secret, String), Failure> {
    let mut seed = [0u8; 32];
    getrandom::fill(&mut seed).map_err(|error| Failure::with(BackupError::Io, error.to_string()))?;
    let key = PrivateKey::new(
        KeypairData::Ed25519(Ed25519Keypair::from_seed(&seed)),
        "observer-backups",
    )
    .map_err(|error| Failure::with(BackupError::Io, error.to_string()))?;
    let private = key
        .to_openssh(LineEnding::LF)
        .map_err(|error| Failure::with(BackupError::Io, error.to_string()))?;
    let public = key
        .public_key()
        .to_openssh()
        .map_err(|error| Failure::with(BackupError::Io, error.to_string()))?;
    Ok((Secret::new(private.as_str()), public))
}

/// Reads a pinned host key: `type base64 [comment]`, Ed25519 or ECDSA, never a certificate.
pub fn parse_host_key(line: &str) -> Option<PublicKey> {
    let line = line.trim();
    if line.is_empty() || line.len() > MAX_KEY_LINE || line.contains(['\n', '\r']) {
        return None;
    }
    let mut parts = line.split_whitespace();
    let (label, data) = (parts.next()?, parts.next()?);
    let (_, prefix) = HOST_KEY_TYPES.iter().find(|(known, _)| *known == label)?;
    if !data.starts_with(prefix) {
        return None;
    }
    PublicKey::from_openssh(&format!("{label} {data}")).ok()
}

/// A key as the panel shows it and a destination pins it.
pub fn host_key(key: &PublicKey) -> HostKey {
    let mut plain = key.clone();
    plain.set_comment("");
    HostKey {
        key: plain.to_openssh().unwrap_or_default().trim().to_owned(),
        fingerprint: key.fingerprint(HashAlg::Sha256).to_string(),
    }
}

fn config() -> client::Config {
    client::Config {
        inactivity_timeout: Some(INACTIVITY),
        keepalive_interval: Some(Duration::from_secs(20)),
        keepalive_max: 3,
        nodelay: true,
        preferred: Preferred {
            kex: Cow::Borrowed(&[
                kex::MLKEM768X25519_SHA256,
                kex::CURVE25519,
                kex::CURVE25519_PRE_RFC_8731,
                kex::EXTENSION_SUPPORT_AS_CLIENT,
                kex::EXTENSION_OPENSSH_STRICT_KEX_AS_CLIENT,
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
        ..client::Config::default()
    }
}

/// Accepts the pinned host key only, or any key for a test, and keeps the one shown.
struct Checker {
    pinned: Option<PublicKey>,
    accept_any: bool,
    seen: Arc<Mutex<Option<PublicKey>>>,
}

// The trait's methods are async; the check answers at once.
#[allow(clippy::unused_async_trait_impl)]
impl client::Handler for Checker {
    type Error = russh::Error;

    async fn check_server_key(&mut self, offered: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        let PublicKeyOrCertificate::PublicKey { key, .. } = offered else {
            return Ok(false);
        };
        if let Ok(mut seen) = self.seen.lock() {
            *seen = Some(key.clone());
        }
        Ok(match &self.pinned {
            Some(pinned) => pinned.key_data() == key.key_data() || self.accept_any,
            None => self.accept_any,
        })
    }
}

/// An SFTP session and the SSH connection it runs on, which must outlive it.
struct Session {
    sftp: SftpSession,
    _ssh: Handle<Checker>,
}

/// Connects and logs in. `accept_any` takes whatever host key is shown, for a test; the key shown
/// comes back either way.
async fn connect(
    remote: &Remote<'_>,
    secret: &Secret,
    accept_any: bool,
) -> (Result<Session, Failure>, Option<PublicKey>) {
    let seen = Arc::new(Mutex::new(None));
    let pinned = remote.host_key.and_then(parse_host_key);
    let result = log_in(remote, secret, accept_any, pinned.clone(), Arc::clone(&seen)).await;
    let seen = seen.lock().ok().and_then(|seen| seen.clone());
    let result = result.map_err(|failure| match (failure.error, &pinned, &seen) {
        (BackupError::HostKeyUnknown, Some(pinned), Some(seen)) if pinned.key_data() != seen.key_data() => {
            Failure::new(BackupError::HostKeyMismatch)
        }
        _ => failure,
    });
    (result, seen)
}

async fn log_in(
    remote: &Remote<'_>,
    secret: &Secret,
    accept_any: bool,
    pinned: Option<PublicKey>,
    seen: Arc<Mutex<Option<PublicKey>>>,
) -> Result<Session, Failure> {
    if pinned.is_none() && !accept_any {
        return Err(Failure::new(BackupError::HostKeyUnknown));
    }
    let checker = Checker {
        pinned,
        accept_any,
        seen,
    };
    let connecting = client::connect(Arc::new(config()), (remote.host, remote.port), checker);
    let mut ssh = tokio::time::timeout(CONNECT_TIMEOUT, connecting)
        .await
        .map_err(|_| Failure::new(BackupError::TimedOut))?
        .map_err(|error| ssh_failure(&error, secret))?;
    let authenticated = match remote.auth {
        SftpAuth::Password => ssh.authenticate_password(remote.user, secret.expose()).await,
        SftpAuth::Key => {
            let key = PrivateKey::from_openssh(secret.expose())
                .map_err(|error| Failure::with(BackupError::Authentication, error.to_string()))?;
            ssh.authenticate_publickey(remote.user, PrivateKeyWithHashAlg::new(Arc::new(key), None))
                .await
        }
    }
    .map_err(|error| ssh_failure(&error, secret))?;
    if !matches!(authenticated, AuthResult::Success) {
        return Err(Failure::with(
            BackupError::Authentication,
            format!(
                "the server refused the {} for {}",
                auth_word(remote.auth),
                remote.user
            ),
        ));
    }
    let channel = ssh
        .channel_open_session()
        .await
        .map_err(|error| ssh_failure(&error, secret))?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(|error| ssh_failure(&error, secret))?;
    let sftp = SftpSession::new(channel.into_stream())
        .await
        .map_err(|error| sftp_failure(&error))?;
    Ok(Session { sftp, _ssh: ssh })
}

const fn auth_word(auth: SftpAuth) -> &'static str {
    match auth {
        SftpAuth::Key => "key",
        SftpAuth::Password => "password",
    }
}

fn ssh_failure(error: &russh::Error, secret: &Secret) -> Failure {
    let code = match error {
        russh::Error::UnknownKey => BackupError::HostKeyUnknown,
        russh::Error::NoCommonAlgo {
            kind: AlgorithmKind::Key,
            ..
        } => BackupError::HostKeyAlgorithm,
        russh::Error::ConnectionTimeout | russh::Error::InactivityTimeout => BackupError::TimedOut,
        russh::Error::IO(error) if error.kind() == std::io::ErrorKind::TimedOut => BackupError::TimedOut,
        russh::Error::IO(_) | russh::Error::Disconnect => BackupError::Unreachable,
        _ => BackupError::Remote,
    };
    Failure::with(code, redact(&error.to_string(), &[secret.expose()]))
}

fn sftp_failure(error: &SftpError) -> Failure {
    let code = match error {
        SftpError::Status(status) => match status.status_code {
            StatusCode::PermissionDenied => BackupError::PermissionDenied,
            StatusCode::NoConnection | StatusCode::ConnectionLost => BackupError::Unreachable,
            _ => BackupError::Remote,
        },
        SftpError::Timeout => BackupError::TimedOut,
        _ => BackupError::Remote,
    };
    Failure::with(code, error.to_string())
}

fn missing(error: &SftpError) -> bool {
    matches!(error, SftpError::Status(status) if status.status_code == StatusCode::NoSuchFile)
}

/// Makes `folder` and the folders above it that are missing.
async fn make_folder(sftp: &SftpSession, folder: &str) -> Result<(), Failure> {
    let mut path = String::new();
    if folder.starts_with('/') {
        path.push('/');
    }
    for part in folder.split('/').filter(|part| !part.is_empty()) {
        if !path.is_empty() && !path.ends_with('/') {
            path.push('/');
        }
        path.push_str(part);
        if !sftp
            .try_exists(path.as_str())
            .await
            .map_err(|error| sftp_failure(&error))?
        {
            sftp.create_dir(path.as_str())
                .await
                .map_err(|error| sftp_failure(&error))?;
        }
    }
    Ok(())
}

/// Sends the file at `path` to `location`, and returns its size once the server reports it.
pub async fn upload(
    remote: &Remote<'_>,
    secret: &Secret,
    path: &std::path::Path,
    location: &str,
    progress: Arc<AtomicU64>,
    cancel: CancellationToken,
) -> Result<u64, Failure> {
    let session = connect(remote, secret, false).await.0?;
    let sftp = &session.sftp;
    make_folder(sftp, remote.folder.trim_end_matches('/')).await?;
    let partial = format!("{location}.partial");
    let sent = send(sftp, path, &partial, &progress, &cancel).await;
    let sent = match sent {
        Ok(sent) => sent,
        Err(failure) => {
            let _ = sftp.remove_file(partial.as_str()).await;
            return Err(failure);
        }
    };
    if let Err(error) = sftp.rename(partial.as_str(), location).await {
        let _ = sftp.remove_file(partial.as_str()).await;
        return Err(sftp_failure(&error));
    }
    let size = sftp
        .metadata(location)
        .await
        .map_err(|error| sftp_failure(&error))?
        .size;
    if size != Some(sent) {
        return Err(Failure::with(
            BackupError::VerifyFailed,
            format!("the server holds {} bytes of {sent}", size.unwrap_or_default()),
        ));
    }
    Ok(sent)
}

async fn send(
    sftp: &SftpSession,
    path: &std::path::Path,
    partial: &str,
    progress: &AtomicU64,
    cancel: &CancellationToken,
) -> Result<u64, Failure> {
    let mut source = tokio::fs::File::open(path).await?;
    let mut target = sftp.create(partial).await.map_err(|error| sftp_failure(&error))?;
    let mut buffer = vec![0; CHUNK];
    let mut sent = 0u64;
    loop {
        if cancel.is_cancelled() {
            return Err(Failure::new(BackupError::Cancelled));
        }
        let read = source.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        target
            .write_all(&buffer[..read])
            .await
            .map_err(|error| Failure::with(BackupError::Remote, error.to_string()))?;
        sent += read as u64;
        progress.store(sent, Ordering::Relaxed);
    }
    target
        .shutdown()
        .await
        .map_err(|error| Failure::with(BackupError::Remote, error.to_string()))?;
    Ok(sent)
}

/// Deletes a copy; one already gone counts as deleted.
pub async fn delete(remote: &Remote<'_>, secret: &Secret, location: &str) -> Result<(), Failure> {
    let session = connect(remote, secret, false).await.0?;
    match session.sftp.remove_file(location).await {
        Err(error) if !missing(&error) => Err(sftp_failure(&error)),
        _ => Ok(()),
    }
}

/// A copy being read from the server, with the session it needs.
pub struct Download {
    file: russh_sftp::client::fs::File,
    _session: Session,
}

impl AsyncRead for Download {
    fn poll_read(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().file).poll_read(context, buffer)
    }
}

pub async fn open(
    remote: &Remote<'_>,
    secret: &Secret,
    location: &str,
) -> Result<(Download, Option<u64>), Failure> {
    let session = connect(remote, secret, false).await.0?;
    let file = session
        .sftp
        .open(location)
        .await
        .map_err(|error| sftp_failure(&error))?;
    let size = file.metadata().await.ok().and_then(|metadata| metadata.size);
    Ok((
        Download {
            file,
            _session: session,
        },
        size,
    ))
}

/// Logs in whatever host key is shown, then writes and deletes a small file in the folder: what a
/// run and its retention need, checked at once. The key shown comes back, pinned or not.
pub async fn test(remote: &Remote<'_>, secret: &Secret) -> (Result<(), Failure>, Option<HostKey>) {
    let (session, seen) = connect(remote, secret, true).await;
    let shown = seen.as_ref().map(host_key);
    let pinned = remote.host_key.and_then(parse_host_key);
    if let (Some(pinned), Some(seen)) = (&pinned, &seen)
        && pinned.key_data() != seen.key_data()
    {
        return (Err(Failure::new(BackupError::HostKeyMismatch)), shown);
    }
    let result = match session {
        Ok(session) => write_and_delete(&session.sftp, remote.folder).await,
        Err(failure) => Err(failure),
    };
    (result, shown)
}

async fn write_and_delete(sftp: &SftpSession, folder: &str) -> Result<(), Failure> {
    make_folder(sftp, folder.trim_end_matches('/')).await?;
    let mut name = [0u8; 8];
    getrandom::fill(&mut name).map_err(|error| Failure::with(BackupError::Io, error.to_string()))?;
    let name = format!(".observer-test-{}", super::sigv4::hex(&name));
    let path = location(folder, &name);
    let mut file = sftp
        .create(path.as_str())
        .await
        .map_err(|error| sftp_failure(&error))?;
    file.write_all(b"observer")
        .await
        .map_err(|error| Failure::with(BackupError::Remote, error.to_string()))?;
    file.shutdown()
        .await
        .map_err(|error| Failure::with(BackupError::Remote, error.to_string()))?;
    sftp.remove_file(path.as_str())
        .await
        .map_err(|error| sftp_failure(&error))
}

#[cfg(test)]
mod tests;
