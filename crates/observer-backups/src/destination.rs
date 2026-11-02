//! Where a backup's copies go: a folder of the host, an SFTP server, an S3-compatible store. Each
//! takes the finished file, puts it under its final name only once it is whole, deletes what
//! Observer wrote there and nothing else, and reads a copy back for a download.

pub mod local;
pub mod s3;
pub mod sftp;
pub mod sigv4;

use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use futures_util::TryStreamExt;
use observer_domain::backups::{BackupError, DestinationTarget, DestinationTestResult, Secret};
use tokio::io::AsyncRead;
use tokio_util::io::StreamReader;
use tokio_util::sync::CancellationToken;

use self::s3::{Bucket, BucketSettings};
use self::sftp::Remote;
use crate::Failure;

/// A copy being read back, and its size when the destination says it.
pub type Reader = Pin<Box<dyn AsyncRead + Send>>;

/// The finished file a copy is made of.
#[derive(Debug, Clone, Copy)]
pub struct Upload<'a> {
    pub file: &'a Path,
    pub bytes: u64,
    pub sha256: [u8; 32],
}

/// Where a file named `name` goes at the destination: a path, or an object's key.
pub fn location(target: &DestinationTarget, name: &str) -> String {
    match target {
        DestinationTarget::Local { folder } => local::location(folder, name),
        DestinationTarget::Sftp { folder, .. } => sftp::location(folder, name),
        DestinationTarget::S3 { prefix, .. } => format!("{prefix}{name}"),
    }
}

fn secret(secret: Option<&Secret>) -> Result<&Secret, Failure> {
    secret.ok_or_else(|| {
        Failure::with(
            BackupError::Authentication,
            "no secret is stored for the destination",
        )
    })
}

fn remote(target: &DestinationTarget) -> Option<Remote<'_>> {
    match target {
        DestinationTarget::Sftp {
            host,
            port,
            user,
            folder,
            auth,
            host_key,
        } => Some(Remote {
            host,
            port: *port,
            user,
            folder,
            auth: *auth,
            host_key: host_key.as_deref(),
        }),
        _ => None,
    }
}

fn bucket(target: &DestinationTarget, secret: &Secret) -> Result<Option<Bucket>, Failure> {
    match target {
        DestinationTarget::S3 {
            endpoint,
            region,
            bucket,
            prefix,
            access_key_id,
            path_style,
        } => Bucket::new(
            BucketSettings {
                endpoint,
                region,
                bucket,
                prefix,
                access_key_id,
                path_style: *path_style,
            },
            secret,
        )
        .map(Some),
        _ => Ok(None),
    }
}

/// Copies the file to `location` and returns the bytes the destination holds.
pub async fn upload(
    target: &DestinationTarget,
    stored: Option<&Secret>,
    upload: Upload<'_>,
    location: &str,
    progress: Arc<AtomicU64>,
    cancel: CancellationToken,
) -> Result<u64, Failure> {
    if let DestinationTarget::Local { .. } = target {
        return local::upload(upload.file, location, upload.sha256, progress, cancel).await;
    }
    let secret = secret(stored)?;
    if let Some(remote) = remote(target) {
        return sftp::upload(&remote, secret, upload.file, location, progress, cancel).await;
    }
    let bucket = bucket(target, secret)?.ok_or_else(|| Failure::new(BackupError::Remote))?;
    let digest = sigv4::hex(&upload.sha256);
    bucket
        .put(location, upload.file, upload.bytes, &digest, progress, cancel)
        .await?;
    Ok(upload.bytes)
}

/// Deletes the copy at `location`; one already gone counts as deleted.
pub async fn delete(
    target: &DestinationTarget,
    stored: Option<&Secret>,
    location: &str,
) -> Result<(), Failure> {
    if let DestinationTarget::Local { .. } = target {
        return local::delete(location).await;
    }
    let secret = secret(stored)?;
    if let Some(remote) = remote(target) {
        return sftp::delete(&remote, secret, location).await;
    }
    let bucket = bucket(target, secret)?.ok_or_else(|| Failure::new(BackupError::Remote))?;
    bucket.delete(location).await
}

/// Opens the copy at `location` for reading.
pub async fn open(
    target: &DestinationTarget,
    stored: Option<&Secret>,
    location: &str,
) -> Result<(Reader, Option<u64>), Failure> {
    if let DestinationTarget::Local { .. } = target {
        let (file, bytes) = local::open(location).await?;
        return Ok((Box::pin(file), Some(bytes)));
    }
    let secret = secret(stored)?;
    if let Some(remote) = remote(target) {
        let (download, bytes) = sftp::open(&remote, secret, location).await?;
        return Ok((Box::pin(download), bytes));
    }
    let bucket = bucket(target, secret)?.ok_or_else(|| Failure::new(BackupError::Remote))?;
    let response = bucket.get(location).await?;
    let bytes = response.content_length();
    let stream = response.bytes_stream().map_err(std::io::Error::other);
    Ok((Box::pin(StreamReader::new(stream)), bytes))
}

/// Checks that a copy could be written and deleted there now: the folder made and writable, the
/// SFTP login and the key the host shows, the bucket's keys.
pub async fn test(target: &DestinationTarget, stored: Option<&Secret>) -> DestinationTestResult {
    let (result, host_key) = match target {
        DestinationTarget::Local { folder } => (local::test(Path::new(folder)).await, None),
        DestinationTarget::Sftp { .. } => match (remote(target), secret(stored)) {
            (Some(remote), Ok(secret)) => sftp::test(&remote, secret).await,
            (_, Err(failure)) => (Err(failure), None),
            (None, Ok(_)) => (Err(Failure::new(BackupError::Remote)), None),
        },
        DestinationTarget::S3 { .. } => {
            let tested = async {
                let bucket =
                    bucket(target, secret(stored)?)?.ok_or_else(|| Failure::new(BackupError::Remote))?;
                bucket.test().await
            };
            (tested.await, None)
        }
    };
    let (error, error_detail) = match result {
        Ok(()) => (None, None),
        Err(failure) => (Some(failure.error), failure.detail),
    };
    DestinationTestResult {
        error,
        error_detail,
        host_key,
    }
}
