pub mod account;
pub mod admin;
pub mod auth;
pub mod groups;
pub mod meta;

use axum::Json;
use serde_json::{Value, json};

/// Rows per page on the admin lists. Cognito pages both the user search and a
/// group's members by an opaque token, so the two have to ask for the same
/// size to page alike.
pub const PAGE_SIZE: i32 = 25;

/// The shape every mutating endpoint answers with: a sentence already in the
/// caller's language, ready to show as a toast.
pub fn message(text: impl Into<String>) -> Json<Value> {
    Json(json!({ "message": text.into() }))
}
