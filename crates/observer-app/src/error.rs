use observer_domain::delegation::Refusal;
use observer_domain::error::{ApiError, ErrorCode};
use observer_store::StoreError;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The rules refused the request. The code and its parameters go back to the client as they are.
    #[error("refused: {:?}", .0.code)]
    Refused(ApiError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("password hashing failed: {0}")]
    Hashing(String),
    #[error("the operating system gave no random bytes: {0}")]
    Random(getrandom::Error),
}

impl From<ErrorCode> for Error {
    fn from(code: ErrorCode) -> Self {
        Self::Refused(code.into())
    }
}

impl From<ApiError> for Error {
    fn from(error: ApiError) -> Self {
        Self::Refused(error)
    }
}

impl From<Refusal> for Error {
    fn from(refusal: Refusal) -> Self {
        match refusal {
            Refusal::SelfEdit => ErrorCode::SelfEdit.into(),
            Refusal::OwnerProtected => ErrorCode::OwnerProtected.into(),
            Refusal::Escalation(permissions) => ApiError::new(ErrorCode::Escalation)
                .with("permissions", permissions.to_hex())
                .into(),
            Refusal::Hierarchy => ErrorCode::Hierarchy.into(),
        }
    }
}

impl Error {
    /// The code a refusal carries, or `None` for a failure of Observer itself.
    pub fn code(&self) -> Option<ErrorCode> {
        match self {
            Self::Refused(error) => Some(error.code),
            _ => None,
        }
    }
}
