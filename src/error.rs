use aws_sdk_cognitoidentityprovider::error::{ProvideErrorMetadata, SdkError};
use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use rust_i18n::t;
use serde_json::json;

/// An error already phrased for the user, in the language they asked for.
#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub message: String,
}

pub type ApiResult<T> = Result<T, ApiError>;

impl ApiError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, message)
    }

    pub fn unauthorized(lang: &str) -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            t!("error_unauthorized", locale = lang),
        )
    }

    pub fn internal(lang: &str) -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            t!("error_unexpected", locale = lang),
        )
    }

    pub fn forbidden(lang: &str) -> Self {
        Self::new(StatusCode::FORBIDDEN, t!("error_forbidden", locale = lang))
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
}

/// How a Cognito error code is answered: the wording to show and the status to
/// send with it. Both come out of the same arm, so a code cannot end up with
/// wording in one list and a status in another.
///
/// Anything the caller's input caused is a 4xx, so the frontend can tell it
/// apart from an outage. `None` means we have no wording of our own.
fn known(code: &str) -> Option<(&'static str, StatusCode)> {
    use StatusCode as Http;

    Some(match code {
        "NotAuthorizedException" => ("error_not_authorized", Http::UNAUTHORIZED),
        "UserNotConfirmedException" => ("error_user_not_confirmed", Http::UNAUTHORIZED),
        "PasswordResetRequiredException" => ("error_password_reset_required", Http::UNAUTHORIZED),
        "UserNotFoundException" => ("error_user_not_found", Http::NOT_FOUND),
        "AccessDeniedException" => ("error_access_denied", Http::FORBIDDEN),
        "TooManyRequestsException" => ("error_too_many_requests", Http::TOO_MANY_REQUESTS),
        "LimitExceededException" => ("error_limit_exceeded", Http::TOO_MANY_REQUESTS),
        "TooManyFailedAttemptsException" => {
            ("error_too_many_failed_attempts", Http::TOO_MANY_REQUESTS)
        }
        "CodeMismatchException" => ("error_code_mismatch", Http::BAD_REQUEST),
        "ExpiredCodeException" => ("error_expired_code", Http::BAD_REQUEST),
        "InvalidPasswordException" => ("error_invalid_password", Http::BAD_REQUEST),
        "InvalidParameterException" => ("error_invalid_parameter", Http::BAD_REQUEST),
        "UsernameExistsException" => ("error_username_exists", Http::BAD_REQUEST),
        "GroupExistsException" => ("error_group_exists", Http::BAD_REQUEST),
        // Both are a code that did not match the enrolled authenticator app.
        "EnableSoftwareTokenMFAException" => ("error_code_mismatch", Http::BAD_REQUEST),
        "SoftwareTokenMFANotFoundException" => ("error_totp_not_found", Http::BAD_REQUEST),
        "AliasExistsException" => ("error_alias_exists", Http::BAD_REQUEST),
        // Our own credentials or app client are wrong, which the caller can do
        // nothing about: their own wording, but still an upstream failure.
        "UnrecognizedClientException" | "InvalidSignatureException" => {
            ("error_bad_credentials", Http::BAD_GATEWAY)
        }
        "ResourceNotFoundException" => ("error_pool_not_found", Http::BAD_GATEWAY),
        _ => return None,
    })
}

/// Turns an SDK error into an ApiError, logging the detail.
pub fn cognito<E, R>(error: SdkError<E, R>, lang: &str) -> ApiError
where
    SdkError<E, R>: ProvideErrorMetadata + std::fmt::Debug,
{
    let code = error.code().unwrap_or_default().to_string();
    tracing::warn!(code = %code, detail = ?error, "cognito call failed");

    match known(&code) {
        Some((key, status)) => ApiError::new(status, t!(key, locale = lang)),
        // An unrecognised failure is not the caller's to fix, and Cognito's own
        // message is the best wording there is.
        None => ApiError::new(
            StatusCode::BAD_GATEWAY,
            error
                .message()
                .map(str::to_string)
                .unwrap_or_else(|| t!("error_unexpected", locale = lang).to_string()),
        ),
    }
}

/// Like `cognito`, but with wording of your own for `ResourceNotFoundException`.
///
/// The shared table has to read that code as a missing user pool, which is
/// what it usually means. In a call that names something else — a group, a
/// registered authenticator app — and whose pool has already answered another
/// call, it is that thing that is missing, and telling the caller to go and
/// check their configuration would send them the wrong way.
pub fn cognito_or_missing<E, R>(error: SdkError<E, R>, key: &str, lang: &str) -> ApiError
where
    SdkError<E, R>: ProvideErrorMetadata + std::fmt::Debug,
{
    match error.code() {
        Some("ResourceNotFoundException") => ApiError::not_found(t!(key, locale = lang)),
        _ => cognito(error, lang),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_known_code_carries_both_wording_and_a_status() {
        assert_eq!(
            known("UserNotFoundException"),
            Some(("error_user_not_found", StatusCode::NOT_FOUND))
        );
        assert_eq!(
            known("UnrecognizedClientException"),
            known("InvalidSignatureException")
        );
    }

    /// The frontend tells a rejected input apart from an outage by the status,
    /// so anything the caller caused has to stay a 4xx.
    #[test]
    fn what_the_caller_caused_stays_a_client_error() {
        for code in [
            "NotAuthorizedException",
            "UserNotFoundException",
            "AccessDeniedException",
            "TooManyRequestsException",
            "CodeMismatchException",
            "InvalidPasswordException",
            "UsernameExistsException",
            "GroupExistsException",
            "SoftwareTokenMFANotFoundException",
        ] {
            let (_, status) = known(code).expect("code should have wording");
            assert!(status.is_client_error(), "{code} answered {status}");
        }
    }

    #[test]
    fn an_upstream_failure_stays_a_bad_gateway() {
        for code in ["UnrecognizedClientException", "ResourceNotFoundException"] {
            let (_, status) = known(code).expect("code should have wording");
            assert_eq!(status, StatusCode::BAD_GATEWAY, "{code}");
        }
    }

    #[test]
    fn an_unknown_code_has_no_wording_of_ours() {
        assert!(known("SomeFutureException").is_none());
        assert!(known("").is_none());
    }
}
