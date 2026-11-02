//! An S3-compatible object store, reached with signed requests of our own rather than a vendor's
//! SDK. A copy goes up in one `PUT` whose payload digest is signed, so the store refuses a body
//! that is not the backup; one `DELETE` takes it away; a `GET` streams it back.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use futures_util::StreamExt;
use jiff::Timestamp;
use observer_domain::backups::{BackupError, Secret};
use reqwest::header::{CONTENT_LENGTH, CONTENT_TYPE, HeaderMap, HeaderValue};
use reqwest::{Body, Method, Response, StatusCode, Url};
use tokio_util::io::ReaderStream;
use tokio_util::sync::CancellationToken;

use super::sigv4::{self, Credentials, EMPTY_SHA256, Request};
use crate::{Failure, redact};

/// The largest object one `PUT` may carry.
pub const MAX_SINGLE_PUT: u64 = 5 * 1024 * 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const READ_TIMEOUT: Duration = Duration::from_secs(60);
const SMALL_TIMEOUT: Duration = Duration::from_secs(60);
/// How much of an error's body is read for its code.
const ERROR_BODY: usize = 16 * 1024;

/// A bucket, as a destination names it.
#[derive(Debug, Clone)]
pub struct Bucket {
    client: reqwest::Client,
    endpoint: Url,
    region: String,
    name: String,
    prefix: String,
    access_key_id: String,
    secret: Secret,
    path_style: bool,
}

/// The fields of an S3 destination.
#[derive(Debug, Clone, Copy)]
pub struct BucketSettings<'a> {
    pub endpoint: &'a str,
    pub region: &'a str,
    pub bucket: &'a str,
    pub prefix: &'a str,
    pub access_key_id: &'a str,
    pub path_style: bool,
}

impl Bucket {
    pub fn new(settings: BucketSettings<'_>, secret: &Secret) -> Result<Self, Failure> {
        let endpoint = Url::parse(settings.endpoint).map_err(|error| {
            Failure::with(
                BackupError::Remote,
                format!("the endpoint is not an address: {error}"),
            )
        })?;
        let client = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(READ_TIMEOUT)
            .user_agent(concat!("observer/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| Failure::with(BackupError::Io, error.to_string()))?;
        Ok(Self {
            client,
            endpoint,
            region: settings.region.to_owned(),
            name: settings.bucket.to_owned(),
            prefix: settings.prefix.to_owned(),
            access_key_id: settings.access_key_id.to_owned(),
            secret: secret.clone(),
            path_style: settings.path_style,
        })
    }

    /// The key a backup named `name` goes under.
    pub fn key(&self, name: &str) -> String {
        format!("{}{name}", self.prefix)
    }

    fn url(&self, key: &str) -> Result<Url, Failure> {
        let mut url = self.endpoint.clone();
        let encoded = sigv4::encode_path(key);
        if self.path_style {
            url.set_path(&format!("/{}/{encoded}", sigv4::encode_path(&self.name)));
        } else {
            let host = url
                .host_str()
                .ok_or_else(|| Failure::with(BackupError::Remote, "the endpoint has no host"))?;
            let host = format!("{}.{host}", self.name);
            url.set_host(Some(&host))
                .map_err(|error| Failure::with(BackupError::Remote, error.to_string()))?;
            url.set_path(&format!("/{encoded}"));
        }
        url.set_query(None);
        Ok(url)
    }

    /// The signed headers of a request to `url`.
    fn signed(&self, method: &Method, url: &Url, payload_sha256: &str) -> Result<HeaderMap, Failure> {
        let host = match (url.host_str(), url.port()) {
            (Some(host), Some(port)) => format!("{host}:{port}"),
            (Some(host), None) => host.to_owned(),
            (None, _) => return Err(Failure::with(BackupError::Remote, "the endpoint has no host")),
        };
        let now = Timestamp::now();
        let date = sigv4::amz_date(now);
        let headers = [
            ("x-amz-content-sha256", payload_sha256),
            ("x-amz-date", date.as_str()),
        ];
        let authorization = sigv4::authorization(
            Credentials {
                access_key_id: &self.access_key_id,
                secret: self.secret.expose(),
                region: &self.region,
            },
            &Request {
                method: method.as_str(),
                path: url.path(),
                host: &host,
                headers: &headers,
                payload_sha256,
                at: now,
            },
        );
        let mut map = HeaderMap::new();
        for (name, value) in headers
            .into_iter()
            .chain([("authorization", authorization.as_str())])
        {
            let value = HeaderValue::from_str(value)
                .map_err(|error| Failure::with(BackupError::Remote, error.to_string()))?;
            map.insert(name, value);
        }
        Ok(map)
    }

    /// Uploads the file at `path` as `key`. `sha256` is its digest, which the store checks the body
    /// against.
    pub async fn put(
        &self,
        key: &str,
        path: &std::path::Path,
        bytes: u64,
        sha256: &str,
        progress: Arc<AtomicU64>,
        cancel: CancellationToken,
    ) -> Result<(), Failure> {
        if bytes > MAX_SINGLE_PUT {
            return Err(Failure::with(
                BackupError::TooLarge,
                format!("{bytes} bytes, more than one upload to S3 takes"),
            ));
        }
        let url = self.url(key)?;
        let mut headers = self.signed(&Method::PUT, &url, sha256)?;
        headers.insert(CONTENT_LENGTH, HeaderValue::from(bytes));
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/gzip"));
        let file = tokio::fs::File::open(path).await?;
        let sent = Arc::new(AtomicU64::new(0));
        let counted = Arc::clone(&sent);
        let body = ReaderStream::with_capacity(file, 256 * 1024).map(move |chunk| {
            if let Ok(chunk) = &chunk {
                let total = counted.fetch_add(chunk.len() as u64, Ordering::Relaxed) + chunk.len() as u64;
                progress.store(total, Ordering::Relaxed);
            }
            chunk
        });
        let request = self
            .client
            .put(url)
            .headers(headers)
            .body(Body::wrap_stream(body))
            .send();
        let response = tokio::select! {
            response = request => response.map_err(|error| self.unreachable(&error))?,
            () = cancel.cancelled() => return Err(Failure::new(BackupError::Cancelled)),
        };
        self.checked(response).await.map(drop)
    }

    /// Deletes `key`; one already gone counts as deleted.
    pub async fn delete(&self, key: &str) -> Result<(), Failure> {
        let url = self.url(key)?;
        let headers = self.signed(&Method::DELETE, &url, EMPTY_SHA256)?;
        let response = self
            .client
            .delete(url)
            .headers(headers)
            .timeout(SMALL_TIMEOUT)
            .send()
            .await
            .map_err(|error| self.unreachable(&error))?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(());
        }
        self.checked(response).await.map(drop)
    }

    /// The object `key`, its body still to read.
    pub async fn get(&self, key: &str) -> Result<Response, Failure> {
        let url = self.url(key)?;
        let headers = self.signed(&Method::GET, &url, EMPTY_SHA256)?;
        let response = self
            .client
            .get(url)
            .headers(headers)
            .send()
            .await
            .map_err(|error| self.unreachable(&error))?;
        self.checked(response).await
    }

    /// Writes then deletes a small object next to where backups go: what a run and its retention
    /// will need, checked at once.
    pub async fn test(&self) -> Result<(), Failure> {
        let mut name = [0u8; 8];
        getrandom::fill(&mut name).map_err(|error| Failure::with(BackupError::Io, error.to_string()))?;
        let key = self.key(&format!(".observer-test-{}", sigv4::hex(&name)));
        let url = self.url(&key)?;
        let payload = b"observer";
        let sha256 = sigv4::hex(&<sha2::Sha256 as sha2::Digest>::digest(payload));
        let headers = self.signed(&Method::PUT, &url, &sha256)?;
        let response = self
            .client
            .put(url)
            .headers(headers)
            .body(payload.as_slice())
            .timeout(SMALL_TIMEOUT)
            .send()
            .await
            .map_err(|error| self.unreachable(&error))?;
        self.checked(response).await?;
        self.delete(&key).await
    }

    fn unreachable(&self, error: &reqwest::Error) -> Failure {
        let code = if error.is_timeout() {
            BackupError::TimedOut
        } else {
            BackupError::Unreachable
        };
        let mut text = error.to_string();
        let mut source = std::error::Error::source(error);
        while let Some(cause) = source {
            text = format!("{text}: {cause}");
            source = cause.source();
        }
        Failure::with(code, redact(&text, &[self.secret.expose()]))
    }

    /// The response when it succeeded, or why the store refused.
    async fn checked(&self, response: Response) -> Result<Response, Failure> {
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let mut body = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(Ok(chunk)) = stream.next().await {
            body.extend_from_slice(&chunk);
            if body.len() >= ERROR_BODY {
                break;
            }
        }
        let body = String::from_utf8_lossy(&body);
        let code = element(&body, "Code").unwrap_or_default();
        let error = match (status, code) {
            (_, "InvalidAccessKeyId" | "SignatureDoesNotMatch" | "AuthorizationHeaderMalformed") => {
                BackupError::Authentication
            }
            (_, "AccessDenied") | (StatusCode::FORBIDDEN, _) => BackupError::PermissionDenied,
            (_, "XAmzContentSHA256Mismatch" | "BadDigest") => BackupError::VerifyFailed,
            (_, "EntityTooLarge") | (StatusCode::PAYLOAD_TOO_LARGE, _) => BackupError::TooLarge,
            _ => BackupError::Remote,
        };
        let detail = match element(&body, "Message") {
            Some(message) => format!("HTTP {} {code}: {message}", status.as_u16()),
            None => format!("HTTP {} {code}", status.as_u16()),
        };
        Err(Failure::with(error, redact(&detail, &[self.secret.expose()])))
    }
}

/// The text of the first `<name>` element of an XML error body.
fn element<'a>(body: &'a str, name: &str) -> Option<&'a str> {
    let start = body.find(&format!("<{name}>"))? + name.len() + 2;
    let end = body[start..].find(&format!("</{name}>"))?;
    Some(body[start..start + end].trim())
}

#[cfg(test)]
mod tests;
