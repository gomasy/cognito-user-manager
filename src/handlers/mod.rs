pub mod account;
pub mod admin;
pub mod auth;
pub mod meta;

use axum::Json;
use serde_json::{Value, json};

/// The shape every mutating endpoint answers with: a sentence already in the
/// caller's language, ready to show as a toast.
pub fn message(text: impl Into<String>) -> Json<Value> {
    Json(json!({ "message": text.into() }))
}
