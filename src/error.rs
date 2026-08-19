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
        Self::new(StatusCode::UNAUTHORIZED, t!("error_unauthorized", locale = lang))
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

/// The i18n key for a Cognito error code, or `None` when we have no wording.
fn key_for(code: &str) -> Option<&'static str> {
    Some(match code {
        "NotAuthorizedException" => "error_not_authorized",
        "UserNotFoundException" => "error_user_not_found",
        "UserNotConfirmedException" => "error_user_not_confirmed",
        "PasswordResetRequiredException" => "error_password_reset_required",
        "CodeMismatchException" => "error_code_mismatch",
        "ExpiredCodeException" => "error_expired_code",
        "InvalidPasswordException" => "error_invalid_password",
        "InvalidParameterException" => "error_invalid_parameter",
        "UsernameExistsException" => "error_username_exists",
        "AliasExistsException" => "error_alias_exists",
        "LimitExceededException" => "error_limit_exceeded",
        "TooManyRequestsException" => "error_too_many_requests",
        "TooManyFailedAttemptsException" => "error_too_many_failed_attempts",
        "AccessDeniedException" => "error_access_denied",
        "UnrecognizedClientException" | "InvalidSignatureException" => "error_bad_credentials",
        "ResourceNotFoundException" => "error_pool_not_found",
        _ => return None,
    })
}

/// HTTP status to answer with for a Cognito error code. Anything caused by the
/// caller's input is a 400 so the frontend can tell it apart from an outage.
fn status_for(code: &str) -> StatusCode {
    match code {
        "NotAuthorizedException" | "UserNotConfirmedException" | "PasswordResetRequiredException" => {
            StatusCode::UNAUTHORIZED
        }
        "UserNotFoundException" => StatusCode::NOT_FOUND,
        "AccessDeniedException" => StatusCode::FORBIDDEN,
        "TooManyRequestsException" | "LimitExceededException" | "TooManyFailedAttemptsException" => {
            StatusCode::TOO_MANY_REQUESTS
        }
        "CodeMismatchException"
        | "ExpiredCodeException"
        | "InvalidPasswordException"
        | "InvalidParameterException"
        | "UsernameExistsException"
        | "AliasExistsException" => StatusCode::BAD_REQUEST,
        _ => StatusCode::BAD_GATEWAY,
    }
}

/// Turns an SDK error into an ApiError, logging the detail.
pub fn cognito<E, R>(error: SdkError<E, R>, lang: &str) -> ApiError
where
    SdkError<E, R>: ProvideErrorMetadata + std::fmt::Debug,
{
    let code = error.code().unwrap_or_default().to_string();
    tracing::warn!(code = %code, detail = ?error, "cognito call failed");

    let message = match key_for(&code) {
        Some(key) => t!(key, locale = lang).to_string(),
        None => error
            .message()
            .map(str::to_string)
            .unwrap_or_else(|| t!("error_unexpected", locale = lang).to_string()),
    };
    ApiError::new(status_for(&code), message)
}
