use std::io::SeekFrom;
use std::path::Path;
use std::time::Duration;

use reqwest::header::{CONTENT_RANGE, HeaderMap, HeaderValue, IF_RANGE, LAST_MODIFIED, RANGE};
use reqwest::{Method, StatusCode, Url};
use sha2::{Digest, Sha256};
use tokio::fs::File;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};

use crate::error::{Error, io};
use crate::http::{self, ARCHIVE_LIMIT, Http};

/// Three attempts in all. A line that drops a 70 MB download three times will not finish it on the
/// fourth, and the operator learns sooner.
pub(crate) const RETRY_DELAYS: [Duration; 2] = [Duration::from_secs(2), Duration::from_secs(5)];
const ATTEMPT_TIMEOUT: Duration = Duration::from_hours(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Downloaded {
    pub bytes: u64,
    pub sha256: [u8; 32],
}

/// Downloads `url` into a new file at `path`. After a cut, the next attempt asks for the rest with
/// `Range`, guarded by `If-Range` so that a file changed in between is fetched whole again. The
/// server must announce `length`, what its `HEAD` announced, and is then held to it exactly. A
/// failed download leaves no file.
pub(crate) async fn download(
    http: &Http,
    url: &Url,
    length: u64,
    path: &Path,
    retry_delays: &[Duration],
    progress: &mut (dyn FnMut(u64, u64) + Send),
) -> Result<Downloaded, Error> {
    let mut file = File::create_new(path).await.map_err(io(path))?;
    let mut partial = Partial {
        url,
        path,
        hasher: Sha256::new(),
        received: 0,
        expected: length,
        validator: None,
    };
    let result = partial.run(http, &mut file, retry_delays, progress).await;
    drop(file);
    if result.is_err() {
        let _ = tokio::fs::remove_file(path).await;
    }
    result
}

struct Partial<'a> {
    url: &'a Url,
    path: &'a Path,
    /// Holds exactly the bytes written so far, so a resumed download needs no second pass.
    hasher: Sha256,
    received: u64,
    expected: u64,
    validator: Option<HeaderValue>,
}

impl Partial<'_> {
    async fn run(
        &mut self,
        http: &Http,
        file: &mut File,
        retry_delays: &[Duration],
        progress: &mut (dyn FnMut(u64, u64) + Send),
    ) -> Result<Downloaded, Error> {
        let mut delays = retry_delays.iter();
        loop {
            match self.attempt(http, file, progress).await {
                Ok(()) => break,
                Err(error) if error.is_transient() => {
                    let Some(delay) = delays.next() else {
                        return Err(error);
                    };
                    tracing::warn!(url = %self.url, %error, "download interrupted, trying again");
                    tokio::time::sleep(*delay).await;
                }
                Err(error) => return Err(error),
            }
        }
        file.flush().await.map_err(io(self.path))?;
        file.sync_all().await.map_err(io(self.path))?;
        Ok(Downloaded {
            bytes: self.received,
            sha256: self.hasher.clone().finalize().into(),
        })
    }

    async fn attempt(
        &mut self,
        http: &Http,
        file: &mut File,
        progress: &mut (dyn FnMut(u64, u64) + Send),
    ) -> Result<(), Error> {
        let resuming = self.received > 0;
        let mut headers = HeaderMap::new();
        if resuming {
            let range = format!("bytes={}-", self.received);
            headers.insert(
                RANGE,
                HeaderValue::from_str(&range).expect("a byte range is ASCII"),
            );
            if let Some(validator) = &self.validator {
                headers.insert(IF_RANGE, validator.clone());
            }
        }
        let mut response = http.send(Method::GET, self.url, headers, ATTEMPT_TIMEOUT).await?;
        match response.status() {
            StatusCode::OK => {
                if resuming {
                    self.restart(file).await?;
                }
                let length = http::content_length(&response).ok_or_else(|| Error::LengthMissing {
                    url: self.url.to_string(),
                })?;
                if length > ARCHIVE_LIMIT {
                    return Err(http::too_large(self.url, ARCHIVE_LIMIT));
                }
                // A maintenance page answered with 200 must not pass for the archive.
                if length != self.expected {
                    return Err(Error::LengthChanged {
                        url: self.url.to_string(),
                        announced: self.expected,
                        sent: length,
                    });
                }
                self.validator = response.headers().get(LAST_MODIFIED).cloned();
            }
            StatusCode::PARTIAL_CONTENT if resuming => {
                if content_range(response.headers()) != Some((self.received, self.expected)) {
                    self.restart(file).await?;
                    return Err(Error::BadRange {
                        url: self.url.to_string(),
                    });
                }
            }
            status => {
                return Err(Error::Status {
                    url: response.url().to_string(),
                    status: status.as_u16(),
                });
            }
        }

        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| http::network(self.url, &error))?
        {
            let length = chunk.len() as u64;
            if self.received + length > self.expected {
                return Err(Error::Overlong {
                    url: self.url.to_string(),
                });
            }
            file.write_all(&chunk).await.map_err(io(self.path))?;
            self.hasher.update(&chunk);
            self.received += length;
            progress(self.received, self.expected);
        }
        if self.received < self.expected {
            return Err(Error::Truncated {
                url: self.url.to_string(),
                received: self.received,
                expected: self.expected,
            });
        }
        Ok(())
    }

    async fn restart(&mut self, file: &mut File) -> Result<(), Error> {
        file.set_len(0).await.map_err(io(self.path))?;
        file.seek(SeekFrom::Start(0)).await.map_err(io(self.path))?;
        self.hasher = Sha256::new();
        self.received = 0;
        Ok(())
    }
}

/// `Content-Range: bytes <first>-<last>/<total>`, as `(first, total)`, when the range runs to the
/// end of the file.
fn content_range(headers: &HeaderMap) -> Option<(u64, u64)> {
    let value = headers.get(CONTENT_RANGE)?.to_str().ok()?;
    let (range, total) = value.strip_prefix("bytes ")?.split_once('/')?;
    let (first, last) = range.split_once('-')?;
    let (first, last, total) = (
        first.parse().ok()?,
        last.parse::<u64>().ok()?,
        total.parse().ok()?,
    );
    (last.checked_add(1)? == total).then_some((first, total))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_server::{Reply, Request, TestServer};

    const LAST_MODIFIED: &str = "Mon, 26 Oct 2026 12:24:47 GMT";
    const NO_WAIT: [Duration; 2] = [Duration::ZERO, Duration::ZERO];

    fn archive() -> Vec<u8> {
        (0..100_000u32).flat_map(u32::to_le_bytes).collect()
    }

    /// Serves `body` honouring `Range` from the given offset, as runtime.fivem.net does.
    fn ranged(body: &[u8], request: &Request) -> Reply {
        let Some(range) = request.header("Range") else {
            return Reply::ok(body.to_vec()).header("Last-Modified", LAST_MODIFIED);
        };
        let first: usize = range
            .trim_start_matches("bytes=")
            .trim_end_matches('-')
            .parse()
            .unwrap();
        let content_range = format!("bytes {first}-{}/{}", body.len() - 1, body.len());
        Reply::status(206)
            .body(body[first..].to_vec())
            .header("Content-Range", &content_range)
            .header("Last-Modified", LAST_MODIFIED)
    }

    /// Downloads what the server sends for `/server.zip`, which its `HEAD` said was `archive()`.
    async fn fetch(server: &TestServer, path: &Path) -> Result<Downloaded, Error> {
        let mut seen = Vec::new();
        let result = download(
            &Http::local(server.port),
            &server.url("/server.zip"),
            archive().len() as u64,
            path,
            &NO_WAIT,
            &mut |received, total| seen.push((received, total)),
        )
        .await;
        if let Ok(downloaded) = &result {
            assert_eq!(seen.last(), Some(&(downloaded.bytes, downloaded.bytes)));
        }
        result
    }

    fn range(value: &str) -> Option<(u64, u64)> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_RANGE, HeaderValue::from_str(value).unwrap());
        content_range(&headers)
    }

    #[test]
    fn a_content_range_is_read_only_when_it_runs_to_the_end_of_the_file() {
        assert_eq!(range("bytes 1000-399999/400000"), Some((1000, 400_000)));
        assert_eq!(range("bytes 1000-1999/400000"), None);
        assert_eq!(range("bytes 1000-18446744073709551615/400000"), None);
        assert_eq!(range("bytes */400000"), None);
        assert_eq!(range("1000-399999/400000"), None);
    }

    #[tokio::test]
    async fn a_cut_download_resumes_with_a_range_request() {
        let body = archive();
        let server = TestServer::start(move |request, index| {
            let reply = ranged(&archive(), request);
            if index == 0 {
                reply.cut_after(150_000)
            } else {
                reply
            }
        })
        .await;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.zip.part");

        let downloaded = fetch(&server, &path).await.unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), body);
        assert_eq!(downloaded.bytes, body.len() as u64);
        assert_eq!(downloaded.sha256, <[u8; 32]>::from(Sha256::digest(&body)));
        let requests = server.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].header("Range"), None);
        assert_eq!(requests[1].header("Range"), Some("bytes=150000-"));
        assert_eq!(requests[1].header("If-Range"), Some(LAST_MODIFIED));
    }

    #[tokio::test]
    async fn a_server_that_ignores_the_range_sends_the_whole_file_again() {
        let body = archive();
        let server = TestServer::start(move |_, index| {
            let reply = Reply::ok(archive());
            if index == 0 {
                reply.cut_after(10_000)
            } else {
                reply
            }
        })
        .await;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.zip.part");

        let downloaded = fetch(&server, &path).await.unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), body);
        assert_eq!(downloaded.sha256, <[u8; 32]>::from(Sha256::digest(&body)));
    }

    #[tokio::test]
    async fn a_download_cut_on_every_attempt_gives_up_after_three_and_leaves_no_file() {
        let body = archive();
        let server = TestServer::start(move |request, _| ranged(&body, request).cut_after(1_000)).await;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.zip.part");

        let error = fetch(&server, &path).await.unwrap_err();

        assert!(
            matches!(error, Error::Truncated { .. } | Error::Network { .. }),
            "{error}"
        );
        assert_eq!(server.requests().len(), 3);
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn a_download_without_a_length_is_refused() {
        let server = TestServer::start(|_, _| Reply::ok(archive()).without_length()).await;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.zip.part");

        let error = fetch(&server, &path).await.unwrap_err();

        assert!(matches!(error, Error::LengthMissing { .. }), "{error}");
        assert_eq!(server.requests().len(), 1);
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn a_resumed_part_longer_than_the_rest_of_the_file_is_refused() {
        let body = archive();
        let server = TestServer::start(move |request, index| {
            if index == 0 {
                return ranged(&body, request).cut_after(1_000);
            }
            let reply = ranged(&body, request);
            let mut longer = body[1_000..].to_vec();
            longer.extend_from_slice(b"appended");
            reply.body(longer)
        })
        .await;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.zip.part");

        let error = fetch(&server, &path).await.unwrap_err();

        assert!(matches!(error, Error::Overlong { .. }), "{error}");
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn an_archive_over_512_mebibytes_is_refused_before_it_is_read() {
        let server =
            TestServer::start(|_, _| Reply::ok(Vec::new()).header("Content-Length", "536870913")).await;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.zip.part");

        let error = fetch(&server, &path).await.unwrap_err();

        assert!(matches!(error, Error::TooLarge { limit_mib: 512, .. }), "{error}");
    }

    #[tokio::test]
    async fn a_page_answered_in_place_of_the_announced_archive_is_refused() {
        let server = TestServer::start(|_, _| Reply::ok(b"<html>maintenance</html>".to_vec())).await;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.zip.part");

        let error = fetch(&server, &path).await.unwrap_err();

        assert!(
            matches!(
                error,
                Error::LengthChanged {
                    announced: 400_000,
                    sent: 24,
                    ..
                }
            ),
            "{error}"
        );
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn a_missing_archive_is_not_retried() {
        let server = TestServer::start(|_, _| Reply::status(404)).await;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.zip.part");

        let error = fetch(&server, &path).await.unwrap_err();

        assert!(matches!(error, Error::Status { status: 404, .. }), "{error}");
        assert_eq!(server.requests().len(), 1);
    }

    #[tokio::test]
    async fn an_existing_file_is_never_overwritten() {
        let server = TestServer::start(|_, _| Reply::ok(archive())).await;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.zip.part");
        std::fs::write(&path, b"someone else's").unwrap();

        let error = fetch(&server, &path).await.unwrap_err();

        assert!(matches!(error, Error::Io { .. }), "{error}");
        assert_eq!(std::fs::read(&path).unwrap(), b"someone else's");
        assert!(server.requests().is_empty());
    }
}
