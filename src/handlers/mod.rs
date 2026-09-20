pub mod account;
pub mod admin;
pub mod auth;
pub mod groups;
pub mod meta;

use axum::Json;
use rust_i18n::t;
use serde_json::{Value, json};

use crate::error::{ApiError, ApiResult};

/// Rows per page on the admin lists. Cognito pages both the user search and a
/// group's members by an opaque token, so the two have to ask for the same
/// size to page alike.
pub const PAGE_SIZE: i32 = 25;

/// The shape every mutating endpoint answers with: a sentence already in the
/// caller's language, ready to show as a toast.
pub fn message(text: impl Into<String>) -> Json<Value> {
    Json(json!({ "message": text.into() }))
}

/// A username as the caller typed it, refused when empty: Cognito answers a
/// blank one with a bare parameter error that names no field.
pub fn username<'a>(raw: &'a str, lang: &str) -> ApiResult<&'a str> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ApiError::bad_request(t!(
            "error_username_required",
            locale = lang
        )));
    }
    Ok(trimmed)
}
