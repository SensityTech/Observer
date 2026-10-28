use serde::Serialize;
use serde_json::{Map, Value};
use ts_rs::TS;

/// Why the API refused a request. The panel turns the code into a message; the API never sends
/// sentences.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ErrorCode {
    /// No valid session: the cookie or the session key is missing, wrong or expired.
    Unauthenticated,
    InvalidCredentials,
    /// The password was right but it was a temporary one, and it has expired.
    TemporaryPasswordExpired,
    /// Params: `retryAfter`, in seconds.
    TooManyAttempts,
    /// Too many passwords are being checked at once. Params: `retryAfter`, in seconds.
    Busy,
    PasswordChangeRequired,
    /// The action needs the password again; `POST /api/auth/reauth`, then retry.
    ReauthRequired,
    WrongPassword,
    /// Params: `permission`, the key of the missing permission.
    Forbidden,
    SelfEdit,
    OwnerProtected,
    /// Params: `permissions`, the bits the editor would grant or revoke without holding them.
    Escalation,
    Hierarchy,
    /// The account changed since it was read; reload it and try again.
    Conflict,
    NotFound,
    NameTaken,
    InvalidUsername,
    /// Params: `min`, in characters.
    PasswordTooShort,
    /// Params: `max`, in bytes.
    PasswordTooLong,
    PasswordUnchanged,
    /// Params: `permissions`, the bits the catalog does not know.
    UnknownPermissions,
    AlreadySetUp,
    InvalidSetupToken,
    OriginMismatch,
    /// The request named a host Observer was not told it is reached by: an address, `localhost`
    /// and the names in `http.hosts` of `observer.toml` are.
    UnknownHost,
    JsonRequired,
    InvalidRequest,
    Internal,
}

/// The body of every error response.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct ApiError {
    pub code: ErrorCode,
    #[ts(type = "Record<string, unknown>")]
    pub params: Map<String, Value>,
}

impl ApiError {
    pub fn new(code: ErrorCode) -> Self {
        Self {
            code,
            params: Map::new(),
        }
    }

    #[must_use]
    pub fn with(mut self, name: &str, value: impl Into<Value>) -> Self {
        self.params.insert(name.to_owned(), value.into());
        self
    }
}

impl From<ErrorCode> for ApiError {
    fn from(code: ErrorCode) -> Self {
        Self::new(code)
    }
}
