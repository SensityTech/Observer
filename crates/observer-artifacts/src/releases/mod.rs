//! Observer's own releases on GitHub: the newest stable one, and the download that stages its
//! executable once the release key's signature, the checksums and GitHub's own digest all agree.
//! Only the GitHub hosts are contacted; what to do with a staged executable is the binary's.

#[cfg(any(test, feature = "test-support"))]
pub mod fake;
mod key;
mod stage;
#[cfg(test)]
mod tests;

use std::time::Duration;

use jiff::Timestamp;
use observer_domain::updates::{self, REPOSITORY};
use reqwest::header::{ACCEPT, ETAG, HeaderMap, HeaderValue, IF_NONE_MATCH};
use reqwest::{Method, StatusCode, Url};
use serde::Deserialize;

pub use key::ReleaseKey;
pub use stage::{StageProgress, Staged, version_of};

use crate::error::Error;
use crate::http::{self, Http, MIB};

const API: &str = "https://api.github.com/";
const API_TIMEOUT: Duration = Duration::from_secs(30);
const API_LIMIT: u64 = 4 * MIB;

/// The releases of [`REPOSITORY`], as GitHub's public API lists them, and the key their checksums
/// must be signed with.
#[derive(Debug, Clone)]
pub struct Releases {
    http: Http,
    api: Url,
    key: Option<ReleaseKey>,
}

/// What the newest stable release is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Latest {
    /// The same answer as the one the `ETag` names.
    Unchanged,
    Found {
        release: Published,
        etag: Option<String>,
    },
    /// GitHub lists none: nothing was published yet, or the repository is private.
    None,
}

/// A published release, as Observer reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Published {
    pub version: semver::Version,
    pub name: String,
    pub notes: String,
    pub url: String,
    pub published_at: Option<Timestamp>,
    pub assets: Vec<Asset>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub size: u64,
    pub url: Url,
    /// The SHA-256 GitHub computed as the file was uploaded, when it says.
    pub digest: Option<[u8; 32]>,
}

impl Releases {
    /// GitHub's API, with the release key this build carries, if any.
    pub fn github(key: Option<ReleaseKey>) -> Result<Self, Error> {
        Ok(Self {
            http: Http::github()?,
            api: Url::parse(API).expect("the API's address parses"),
            key,
        })
    }

    /// A releases server on 127.0.0.1:`port` that answers as GitHub's API does.
    #[cfg(any(test, feature = "test-support"))]
    pub fn local(port: u16, key: Option<ReleaseKey>) -> Self {
        Self {
            http: Http::local(port),
            api: format!("http://127.0.0.1:{port}/")
                .parse()
                .expect("a local address parses"),
            key,
        }
    }

    pub fn has_key(&self) -> bool {
        self.key.is_some()
    }

    /// The newest stable release. With the `ETag` of an earlier answer, an unchanged one costs
    /// nothing of GitHub's hourly allowance.
    pub async fn latest(&self, etag: Option<&str>) -> Result<Latest, Error> {
        let url = self
            .api
            .join(&format!("repos/{REPOSITORY}/releases/latest"))
            .expect("the path joins the API's address");
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static("application/vnd.github+json"));
        headers.insert("x-github-api-version", HeaderValue::from_static("2022-11-28"));
        if let Some(etag) = etag.and_then(|etag| HeaderValue::from_str(etag).ok()) {
            headers.insert(IF_NONE_MATCH, etag);
        }
        let response = self.http.send(Method::GET, &url, headers, API_TIMEOUT).await?;
        match response.status() {
            StatusCode::NOT_MODIFIED => return Ok(Latest::Unchanged),
            StatusCode::NOT_FOUND => return Ok(Latest::None),
            StatusCode::OK => {}
            status if limited(status, response.headers()) => {
                return Err(Error::RateLimited {
                    until: reset(response.headers()),
                });
            }
            status => {
                return Err(Error::Status {
                    url: response.url().to_string(),
                    status: status.as_u16(),
                });
            }
        }
        let etag = response
            .headers()
            .get(ETAG)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body = http::body(response, API_LIMIT).await?;
        let wire: WireRelease =
            serde_json::from_slice(&body).map_err(|error| Error::ReleaseFormat(error.to_string()))?;
        Ok(Latest::Found {
            release: wire.read()?,
            etag,
        })
    }
}

impl Published {
    /// The archive of this platform's executable.
    pub fn archive(&self) -> Option<&Asset> {
        self.asset(&updates::archive_name(
            &self.version.to_string(),
            std::env::consts::OS,
            std::env::consts::ARCH,
        ))
    }

    pub fn signed(&self) -> bool {
        self.asset(updates::SIGNATURE).is_some()
    }

    pub fn asset(&self, name: &str) -> Option<&Asset> {
        self.assets.iter().find(|asset| asset.name == name)
    }
}

/// GitHub answers 403 or 429 once an address spent its allowance, with nothing left in
/// `x-ratelimit-remaining`.
fn limited(status: StatusCode, headers: &HeaderMap) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS
        || (status == StatusCode::FORBIDDEN
            && headers
                .get("x-ratelimit-remaining")
                .is_some_and(|value| value.as_bytes() == b"0"))
}

/// When the allowance comes back: `x-ratelimit-reset`, in seconds since the epoch.
fn reset(headers: &HeaderMap) -> Option<Timestamp> {
    let seconds: i64 = headers.get("x-ratelimit-reset")?.to_str().ok()?.parse().ok()?;
    Timestamp::from_second(seconds).ok()
}

#[derive(Debug, Deserialize)]
struct WireRelease {
    tag_name: String,
    name: Option<String>,
    body: Option<String>,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    published_at: Option<Timestamp>,
    #[serde(default)]
    assets: Vec<WireAsset>,
}

#[derive(Debug, Deserialize)]
struct WireAsset {
    name: String,
    size: u64,
    browser_download_url: String,
    state: Option<String>,
    digest: Option<String>,
}

impl WireRelease {
    fn read(self) -> Result<Published, Error> {
        let format = |reason: String| Error::ReleaseFormat(reason);
        if self.draft || self.prerelease {
            return Err(format(format!("{} is a draft or a pre-release", self.tag_name)));
        }
        let version = self
            .tag_name
            .strip_prefix('v')
            .and_then(|version| semver::Version::parse(version).ok())
            .filter(|version| version.pre.is_empty() && version.build.is_empty())
            .ok_or_else(|| format(format!("the tag {:?} names no version", self.tag_name)))?;
        let assets = self
            .assets
            .into_iter()
            .filter(|asset| asset.state.as_deref().is_none_or(|state| state == "uploaded"))
            .map(|asset| {
                let url = Url::parse(&asset.browser_download_url)
                    .map_err(|_| format(format!("{} has no download address", asset.name)))?;
                let digest = match asset.digest.as_deref() {
                    None => None,
                    Some(digest) => Some(
                        digest
                            .strip_prefix("sha256:")
                            .and_then(key::bytes32)
                            .ok_or_else(|| format(format!("{} has an unknown digest", asset.name)))?,
                    ),
                };
                Ok(Asset {
                    name: asset.name,
                    size: asset.size,
                    url,
                    digest,
                })
            })
            .collect::<Result<_, Error>>()?;
        Ok(Published {
            name: self
                .name
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| self.tag_name.clone()),
            version,
            notes: self.body.unwrap_or_default(),
            url: self.html_url,
            published_at: self.published_at,
            assets,
        })
    }
}
