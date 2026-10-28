use crate::catalog::Source;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A source answered, but not in the shape its parser knows. Observer needs an update.
    #[error("{origin} changed format: {reason}")]
    FormatChanged { origin: Source, reason: String },
    #[error("the HTTP client cannot start: {0}")]
    Client(reqwest::Error),
    #[error("{url} is not on a Cfx.re artifact host over HTTPS")]
    UrlRefused { url: String },
    #[error("{url} redirects more than {} times", crate::http::MAX_REDIRECTS)]
    TooManyRedirects { url: String },
    #[error("{url} answered HTTP {status}")]
    Status { url: String, status: u16 },
    #[error("cannot reach {url}: {reason}")]
    Network { url: String, reason: String },
    #[error("{url} is larger than the {limit_mib} MiB Observer accepts")]
    TooLarge { url: String, limit_mib: u64 },
}
