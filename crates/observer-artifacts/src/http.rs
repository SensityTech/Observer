use std::time::Duration;

use jiff::Timestamp;
use reqwest::header::{CONTENT_LENGTH, HeaderMap, LAST_MODIFIED, LOCATION};
use reqwest::redirect::Policy;
use reqwest::{Method, Response, StatusCode, Url};

use crate::error::Error;

pub(crate) const MIB: u64 = 1024 * 1024;
/// Cfx.re's build listings, its support policy and its archives: the only hosts Observer talks to.
const HOSTS: [&str; 3] = ["runtime.fivem.net", "changelogs-live.fivem.net", "docs.fivem.net"];
pub(crate) const MAX_REDIRECTS: usize = 3;
const PAGE_LIMIT: u64 = 4 * MIB;
pub(crate) const ARCHIVE_LIMIT: u64 = 512 * MIB;

/// An HTTP client that reaches only the Cfx.re hosts, over HTTPS and through no proxy, and checks
/// every redirect against the same hosts before following it. It sends no browser headers and asks
/// for no compression, so sizes on the wire are the sizes on disk.
#[derive(Debug, Clone)]
pub struct Http {
    client: reqwest::Client,
    /// A port on 127.0.0.1 let through over plain HTTP. Only tests set it, to serve archives locally.
    local_port: Option<u16>,
}

/// What a `HEAD` request says about a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Head {
    pub length: Option<u64>,
    pub last_modified: Option<Timestamp>,
}

impl Http {
    pub fn new() -> Result<Self, Error> {
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(30))
            .read_timeout(Duration::from_secs(60))
            .user_agent(concat!("observer/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(Error::Client)?;
        Ok(Self {
            client,
            local_port: None,
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn local(port: u16) -> Self {
        Self {
            local_port: Some(port),
            ..Self::new().expect("the HTTP client starts in tests")
        }
    }

    /// A page or an API answer, at most 4 MiB.
    pub(crate) async fn page(&self, url: &Url, timeout: Duration) -> Result<String, Error> {
        let mut response = self.send(Method::GET, url, HeaderMap::new(), timeout).await?;
        expect_ok(&response)?;
        let url = response.url().clone();
        if content_length(&response).is_some_and(|length| length > PAGE_LIMIT) {
            return Err(too_large(&url, PAGE_LIMIT));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|error| network(&url, &error))? {
            body.extend_from_slice(&chunk);
            if body.len() as u64 > PAGE_LIMIT {
                return Err(too_large(&url, PAGE_LIMIT));
            }
        }
        Ok(String::from_utf8_lossy(&body).into_owned())
    }

    pub(crate) async fn head(&self, url: &Url, timeout: Duration) -> Result<Head, Error> {
        let response = self.send(Method::HEAD, url, HeaderMap::new(), timeout).await?;
        expect_ok(&response)?;
        Ok(Head {
            length: content_length(&response),
            last_modified: response
                .headers()
                .get(LAST_MODIFIED)
                .and_then(|value| value.to_str().ok())
                .and_then(http_date),
        })
    }

    /// Sends a request and follows at most three redirects, each checked like the first URL.
    pub(crate) async fn send(
        &self,
        method: Method,
        url: &Url,
        headers: HeaderMap,
        timeout: Duration,
    ) -> Result<Response, Error> {
        let mut current = url.clone();
        for _ in 0..=MAX_REDIRECTS {
            self.check(&current)?;
            let response = self
                .client
                .request(method.clone(), current.clone())
                .headers(headers.clone())
                .timeout(timeout)
                .send()
                .await
                .map_err(|error| network(&current, &error))?;
            let redirected = matches!(
                response.status(),
                StatusCode::MOVED_PERMANENTLY
                    | StatusCode::FOUND
                    | StatusCode::SEE_OTHER
                    | StatusCode::TEMPORARY_REDIRECT
                    | StatusCode::PERMANENT_REDIRECT
            );
            let location = response
                .headers()
                .get(LOCATION)
                .and_then(|value| value.to_str().ok());
            match location {
                Some(location) if redirected => {
                    let next = current.join(location).map_err(|_| Error::UrlRefused {
                        url: location.to_owned(),
                    })?;
                    tracing::debug!(from = %current, to = %next, "following a redirect");
                    current = next;
                }
                _ => return Ok(response),
            }
        }
        Err(Error::TooManyRedirects { url: url.to_string() })
    }

    fn check(&self, url: &Url) -> Result<(), Error> {
        let local = self.local_port.is_some()
            && url.scheme() == "http"
            && url.host_str() == Some("127.0.0.1")
            && url.port() == self.local_port;
        if local || allowed(url) {
            Ok(())
        } else {
            Err(Error::UrlRefused { url: url.to_string() })
        }
    }
}

/// HTTPS on the default port to one of the Cfx.re hosts, with no credentials in the URL.
fn allowed(url: &Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url.host_str().is_some_and(|host| HOSTS.contains(&host))
}

pub(crate) fn expect_ok(response: &Response) -> Result<(), Error> {
    if response.status() == StatusCode::OK {
        Ok(())
    } else {
        Err(Error::Status {
            url: response.url().to_string(),
            status: response.status().as_u16(),
        })
    }
}

/// Read from the header itself: the body's size hint says nothing for a `HEAD`.
pub(crate) fn content_length(response: &Response) -> Option<u64> {
    response
        .headers()
        .get(CONTENT_LENGTH)?
        .to_str()
        .ok()?
        .parse()
        .ok()
}

/// `Last-Modified` and other HTTP dates, such as `Mon, 26 Oct 2026 12:24:47 GMT`.
pub(crate) fn http_date(text: &str) -> Option<Timestamp> {
    jiff::fmt::rfc2822::DateTimeParser::new()
        .parse_timestamp(text)
        .ok()
}

pub(crate) fn too_large(url: &Url, limit: u64) -> Error {
    Error::TooLarge {
        url: url.to_string(),
        limit_mib: limit / MIB,
    }
}

/// The innermost cause says what went wrong ("connection refused", "operation timed out"); the
/// outer layers only repeat the URL.
pub(crate) fn network(url: &Url, error: &reqwest::Error) -> Error {
    let mut cause: &dyn std::error::Error = error;
    while let Some(source) = cause.source() {
        cause = source;
    }
    let reason = if error.is_timeout() {
        "the request timed out".to_owned()
    } else {
        cause.to_string()
    };
    Error::Network {
        url: url.to_string(),
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_server::{Reply, TestServer};

    const TIMEOUT: Duration = Duration::from_secs(10);

    #[test]
    fn only_https_to_the_cfx_hosts_is_allowed() {
        for url in [
            "https://runtime.fivem.net/artifacts/fivem/build_server_windows/master/",
            "https://changelogs-live.fivem.net/api/changelog/versions/win32/server",
            "https://docs.fivem.net/docs/server-download/?platform=legacy&os=windows",
            "https://RUNTIME.fivem.net:443/artifacts/",
        ] {
            assert!(allowed(&url.parse().unwrap()), "{url}");
        }
        for url in [
            "http://runtime.fivem.net/artifacts/",
            "https://runtime.fivem.net:8443/artifacts/",
            "https://user:pass@runtime.fivem.net/artifacts/",
            "https://runtime.fivem.net.example.com/artifacts/",
            "https://downloads.cfx-services.net/prod/x/cfx-server_win_x64.zip",
            "https://fivem.net/",
            "https://23.55.1.1/artifacts/",
            "ftp://runtime.fivem.net/artifacts/",
        ] {
            assert!(!allowed(&url.parse().unwrap()), "{url}");
        }
    }

    #[test]
    fn http_dates_are_read_in_utc() {
        assert_eq!(
            http_date("Mon, 26 Oct 2026 12:24:47 GMT"),
            Some("2026-10-26T12:24:47Z".parse().unwrap())
        );
        assert_eq!(http_date("yesterday"), None);
    }

    #[tokio::test]
    async fn a_redirect_inside_the_allowed_hosts_is_followed() {
        let server = TestServer::start(|request, _| match request.path.as_str() {
            "/old" => Reply::status(301).header("Location", "/new"),
            _ => Reply::ok(b"moved here".to_vec()),
        })
        .await;
        let http = Http::local(server.port);

        let page = http.page(&server.url("/old"), TIMEOUT).await.unwrap();

        assert_eq!(page, "moved here");
        assert_eq!(server.paths(), ["/old", "/new"]);
    }

    #[tokio::test]
    async fn a_redirect_to_another_host_is_refused_before_it_is_followed() {
        let server =
            TestServer::start(|_, _| Reply::status(302).header("Location", "https://example.com/server.zip"))
                .await;
        let http = Http::local(server.port);

        let error = http.page(&server.url("/server.zip"), TIMEOUT).await.unwrap_err();

        assert!(
            matches!(&error, Error::UrlRefused { url } if url == "https://example.com/server.zip"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn a_redirect_from_https_to_plain_http_is_refused() {
        let server = TestServer::start(|_, _| {
            Reply::status(307).header("Location", "http://runtime.fivem.net/artifacts/")
        })
        .await;
        let http = Http::local(server.port);

        let error = http.page(&server.url("/"), TIMEOUT).await.unwrap_err();

        assert!(matches!(error, Error::UrlRefused { .. }), "{error}");
    }

    #[tokio::test]
    async fn a_fourth_redirect_is_not_followed() {
        let server = TestServer::start(|request, _| {
            let hop: u32 = request.path.trim_start_matches('/').parse().unwrap_or(0);
            Reply::status(302).header("Location", &format!("/{}", hop + 1))
        })
        .await;
        let http = Http::local(server.port);

        let error = http.page(&server.url("/0"), TIMEOUT).await.unwrap_err();

        assert!(matches!(error, Error::TooManyRedirects { .. }), "{error}");
        assert_eq!(server.paths(), ["/0", "/1", "/2", "/3"]);
    }

    #[tokio::test]
    async fn a_page_over_four_mebibytes_is_refused() {
        let body = vec![b'x'; usize::try_from(PAGE_LIMIT).unwrap() + 1];
        let server = TestServer::start(move |_, _| Reply::ok(body.clone()).without_length()).await;
        let http = Http::local(server.port);

        let error = http.page(&server.url("/"), TIMEOUT).await.unwrap_err();

        assert!(matches!(error, Error::TooLarge { limit_mib: 4, .. }), "{error}");
    }
}
