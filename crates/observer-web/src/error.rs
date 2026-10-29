use axum::Json;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use observer_app::Error;
use observer_domain::error::{ApiError, ErrorCode};
use tracing::error;

/// A failed API request, answered as `{ "code": ..., "params": {...} }` with the status the code
/// calls for. Observer's own failures are logged and reach the client as `internal`, without detail.
#[derive(Debug)]
pub(crate) struct Failure(Error);

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        Self(error)
    }
}

impl From<ErrorCode> for Failure {
    fn from(code: ErrorCode) -> Self {
        Self(code.into())
    }
}

impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        let body = match self.0 {
            Error::Refused(refusal) => refusal,
            failure => {
                error!(error = %failure, "an API request failed");
                ApiError::new(ErrorCode::Internal)
            }
        };
        let retry_after = body.params.get("retryAfter").and_then(serde_json::Value::as_u64);

        let mut response = (status(body.code), Json(body)).into_response();
        if let Some(seconds) = retry_after {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from(seconds));
        }
        response
    }
}

fn status(code: ErrorCode) -> StatusCode {
    match code {
        ErrorCode::Unauthenticated | ErrorCode::InvalidCredentials | ErrorCode::TemporaryPasswordExpired => {
            StatusCode::UNAUTHORIZED
        }
        ErrorCode::PasswordChangeRequired
        | ErrorCode::ReauthRequired
        | ErrorCode::WrongPassword
        | ErrorCode::Forbidden
        | ErrorCode::OwnerOnly
        | ErrorCode::SelfEdit
        | ErrorCode::OwnerProtected
        | ErrorCode::Escalation
        | ErrorCode::Hierarchy
        | ErrorCode::InvalidSetupToken
        | ErrorCode::OriginMismatch => StatusCode::FORBIDDEN,
        ErrorCode::NotFound | ErrorCode::CrashNotFound | ErrorCode::CrashFileNotFound => {
            StatusCode::NOT_FOUND
        }
        ErrorCode::Conflict
        | ErrorCode::NameTaken
        | ErrorCode::AlreadySetUp
        | ErrorCode::LaunchNotConfigured
        | ErrorCode::ServerRunning
        | ErrorCode::ServerNotRunning
        | ErrorCode::ServerStopping
        | ErrorCode::NothingToSkip
        | ErrorCode::AlreadySkipped
        | ErrorCode::NothingToRestore
        | ErrorCode::NothingToDelay
        | ErrorCode::NoOneOff => StatusCode::CONFLICT,
        ErrorCode::LaunchFailed | ErrorCode::DelayTooLong | ErrorCode::CrashFileTooLarge => {
            StatusCode::UNPROCESSABLE_ENTITY
        }
        ErrorCode::JsonRequired => StatusCode::UNSUPPORTED_MEDIA_TYPE,
        ErrorCode::UnknownHost => StatusCode::MISDIRECTED_REQUEST,
        ErrorCode::InvalidUsername
        | ErrorCode::PasswordTooShort
        | ErrorCode::PasswordTooLong
        | ErrorCode::PasswordUnchanged
        | ErrorCode::UnknownPermissions
        | ErrorCode::InvalidLaunchSettings
        | ErrorCode::CommandRefused
        | ErrorCode::InvalidRequest
        | ErrorCode::InvalidSchedule => StatusCode::BAD_REQUEST,
        ErrorCode::TooManyAttempts | ErrorCode::TooManySockets | ErrorCode::TooManyCommands => {
            StatusCode::TOO_MANY_REQUESTS
        }
        ErrorCode::Busy | ErrorCode::ShuttingDown => StatusCode::SERVICE_UNAVAILABLE,
        ErrorCode::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
