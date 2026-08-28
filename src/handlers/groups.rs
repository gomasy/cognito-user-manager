//! Group administration: the pool's groups, and who is in each of them.
//!
//! Membership can also be edited from the user screen; both sides call the
//! same `groups::add_user` / `groups::remove_user`, so the guard against an
//! admin removing their own admin group is repeated on this side too.

use axum::Json;
use axum::extract::{Path, Query, State};
use rust_i18n::t;
use serde::Deserialize;
use serde_json::Value;

use crate::error::{ApiError, ApiResult};
use crate::extract::{AdminSession, Lang};
use crate::groups::{self, GroupInfo};
use crate::state::AppState;
use crate::users::{self, UserPage};

use super::{PAGE_SIZE, message};

pub async fn list(
    State(state): State<AppState>,
    Lang(lang): Lang,
    AdminSession(_): AdminSession,
) -> ApiResult<Json<Vec<GroupInfo>>> {
    groups::list(&state, &lang).await.map(Json)
}

pub async fn detail(
    State(state): State<AppState>,
    Lang(lang): Lang,
    AdminSession(_): AdminSession,
    Path(group): Path<String>,
) -> ApiResult<Json<GroupInfo>> {
    groups::require(&state, &group, &lang).await.map(Json)
}

#[derive(Deserialize)]
pub struct CreateRequest {
    name: String,
    #[serde(default)]
    description: Option<String>,
    /// Lower wins when a user is in several groups. Left out, Cognito assigns
    /// none, which sorts the group last.
    #[serde(default)]
    precedence: Option<i32>,
}

pub async fn create(
    State(state): State<AppState>,
    Lang(lang): Lang,
    AdminSession(_): AdminSession,
    Json(body): Json<CreateRequest>,
) -> ApiResult<Json<Value>> {
    let name = body.name.trim();
    if name.is_empty() {
        return Err(ApiError::bad_request(t!(
            "error_group_name_required",
            locale = &lang
        )));
    }

    let description = body
        .description
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string);
    groups::create(&state, name, description, body.precedence, &lang).await?;

    Ok(Json(serde_json::json!({
        "message": t!("msg_group_created", locale = &lang),
        "name": name,
    })))
}

pub async fn delete(
    State(state): State<AppState>,
    Lang(lang): Lang,
    AdminSession(_): AdminSession,
    Path(group): Path<String>,
) -> ApiResult<Json<Value>> {
    // Deleting the group that grants admin access locks every admin out of
    // this console at once, and nothing in it could put them back.
    if group == state.config.admin_group {
        return Err(ApiError::bad_request(t!(
            "error_delete_admin_group",
            locale = &lang,
            group = &group
        )));
    }

    groups::delete(&state, &group, &lang).await?;

    Ok(message(t!("msg_group_deleted", locale = &lang)))
}

#[derive(Deserialize)]
pub struct MembersQuery {
    #[serde(default)]
    token: Option<String>,
}

pub async fn members(
    State(state): State<AppState>,
    Lang(lang): Lang,
    AdminSession(_): AdminSession,
    Path(group): Path<String>,
    Query(query): Query<MembersQuery>,
) -> ApiResult<Json<UserPage>> {
    groups::members(&state, &group, PAGE_SIZE, query.token, &lang)
        .await
        .map(Json)
}

#[derive(Deserialize)]
pub struct MemberRequest {
    username: String,
}

pub async fn add_member(
    State(state): State<AppState>,
    Lang(lang): Lang,
    AdminSession(_): AdminSession,
    Path(group): Path<String>,
    Json(body): Json<MemberRequest>,
) -> ApiResult<Json<Value>> {
    let username = body.username.trim();
    if username.is_empty() {
        return Err(ApiError::bad_request(t!(
            "error_username_required",
            locale = &lang
        )));
    }

    // Looking the user up first says which of the two names was wrong, and
    // resolves an alias to the username Cognito wants for the membership call.
    let user = users::require(&state, username, &lang).await?;
    groups::add_user(&state, &user.username, &group, &lang).await?;

    Ok(message(t!("msg_group_member_added", locale = &lang)))
}

pub async fn remove_member(
    State(state): State<AppState>,
    Lang(lang): Lang,
    AdminSession(session): AdminSession,
    Path((group, username)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    // Same lockout as on the user screen: an admin who leaves the admin group
    // cannot reach this page again to undo it.
    if group == state.config.admin_group {
        let user = users::require(&state, &username, &lang).await?;
        if users::is_self(&session, &user) {
            return Err(ApiError::bad_request(t!(
                "error_self_admin_group",
                locale = &lang,
                group = &group
            )));
        }
    }

    groups::remove_user(&state, &username, &group, &lang).await?;

    Ok(message(t!("msg_group_member_removed", locale = &lang)))
}
