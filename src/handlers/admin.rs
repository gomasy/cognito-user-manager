use aws_sdk_cognitoidentityprovider::types::{DeliveryMediumType, MessageActionType};
use axum::Json;
use axum::extract::{Path, Query, State};
use rust_i18n::t;
use serde::Deserialize;
use serde_json::Value;

use crate::attributes::{self, Patch, Values};
use crate::error::{ApiError, ApiResult, cognito};
use crate::extract::{AdminSession, Lang};
use crate::password;
use crate::session::Session;
use crate::state::AppState;
use crate::users::{self, UserDetail, UserPage};

use super::message;

const PAGE_SIZE: i32 = 25;

#[derive(Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    q: String,
    #[serde(default)]
    field: Option<String>,
    #[serde(default)]
    token: Option<String>,
}

pub async fn list(
    State(state): State<AppState>,
    Lang(lang): Lang,
    AdminSession(_): AdminSession,
    Query(query): Query<ListQuery>,
) -> ApiResult<Json<UserPage>> {
    let field = users::search_field(query.field.as_deref());

    users::list(&state, &query.q, field, PAGE_SIZE, query.token, &lang)
        .await
        .map(Json)
}

async fn require_user(state: &AppState, username: &str, lang: &str) -> ApiResult<UserDetail> {
    users::detail(state, username, lang)
        .await?
        .ok_or_else(|| ApiError::not_found(t!("error_user_not_found", locale = lang)))
}

pub async fn detail(
    State(state): State<AppState>,
    Lang(lang): Lang,
    AdminSession(_): AdminSession,
    Path(username): Path<String>,
) -> ApiResult<Json<UserDetail>> {
    require_user(&state, &username, &lang).await.map(Json)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRequest {
    username: String,
    #[serde(default)]
    attributes: Patch,
    #[serde(default)]
    temporary_password: String,
    #[serde(default)]
    suppress_message: bool,
    #[serde(default)]
    groups: Vec<String>,
}

pub async fn create(
    State(state): State<AppState>,
    Lang(lang): Lang,
    AdminSession(_): AdminSession,
    Json(body): Json<CreateRequest>,
) -> ApiResult<Json<Value>> {
    let username = body.username.trim();
    if username.is_empty() {
        return Err(ApiError::bad_request(t!(
            "error_username_required",
            locale = &lang
        )));
    }

    let pool = state.schema.get(&state, &lang).await?;
    // Immutable attributes can still be set at creation time.
    let changes = attributes::diff(
        &body.attributes,
        &pool.admin_visible(),
        &Values::new(),
        &lang,
    )?;

    // Cognito only makes up a temporary password of its own when the new user
    // has an email address or phone number to deliver it to, so an empty field
    // is filled in here instead of being left out of the request. A passwordless
    // pool is the exception: there the user is meant to be created without one.
    let given = body.temporary_password.trim();
    let generated = match given {
        "" if pool.password_sign_in => {
            Some(password::generate(&pool.password_policy).map_err(|error| {
                tracing::error!(%error, "no system randomness for a temporary password");
                ApiError::internal(&lang)
            })?)
        }
        _ => None,
    };
    let temporary_password = if given.is_empty() {
        generated.clone()
    } else {
        Some(given.to_string())
    };

    let mut request = state
        .cognito
        .admin_create_user()
        .user_pool_id(&state.config.user_pool_id)
        .username(username)
        .set_temporary_password(temporary_password)
        .set_user_attributes(Some(changes.attributes));
    if body.suppress_message {
        request = request.message_action(MessageActionType::Suppress);
    } else {
        request = request.desired_delivery_mediums(DeliveryMediumType::Email);
    }
    request
        .send()
        .await
        .map_err(|error| cognito(error, &lang))?;

    for group in &body.groups {
        add_to_group(&state, username, group, &lang).await?;
    }

    // A suppressed invitation is never delivered, so a password made up here
    // would be known to nobody at all and the account unusable until it was
    // reset. Handing it back is the only way it can reach the new user; when
    // Cognito does mail it out there is nothing to disclose.
    let disclosed = body.suppress_message.then_some(generated).flatten();

    Ok(Json(serde_json::json!({
        "message": t!("msg_user_created", locale = &lang),
        "username": username,
        "temporaryPassword": disclosed,
    })))
}

#[derive(Deserialize)]
pub struct PatchRequest {
    #[serde(default)]
    attributes: Patch,
}

pub async fn update(
    State(state): State<AppState>,
    Lang(lang): Lang,
    AdminSession(_): AdminSession,
    Path(username): Path<String>,
    Json(body): Json<PatchRequest>,
) -> ApiResult<Json<Value>> {
    let user = require_user(&state, &username, &lang).await?;
    let pool = state.schema.get(&state, &lang).await?;
    let changes = attributes::diff(&body.attributes, &pool.editable(), &user.attributes, &lang)?;

    if changes.is_empty() {
        return Ok(message(t!("msg_no_changes", locale = &lang)));
    }

    if !changes.attributes.is_empty() {
        state
            .cognito
            .admin_update_user_attributes()
            .user_pool_id(&state.config.user_pool_id)
            .username(&username)
            .set_user_attributes(Some(changes.attributes))
            .send()
            .await
            .map_err(|error| cognito(error, &lang))?;
    }
    if !changes.to_delete.is_empty() {
        state
            .cognito
            .admin_delete_user_attributes()
            .user_pool_id(&state.config.user_pool_id)
            .username(&username)
            .set_user_attribute_names(Some(changes.to_delete))
            .send()
            .await
            .map_err(|error| cognito(error, &lang))?;
    }

    Ok(message(t!("msg_attributes_updated", locale = &lang)))
}

#[derive(Deserialize)]
pub struct GroupsRequest {
    #[serde(default)]
    groups: Vec<String>,
}

pub async fn set_groups(
    State(state): State<AppState>,
    Lang(lang): Lang,
    AdminSession(session): AdminSession,
    Path(username): Path<String>,
    Json(body): Json<GroupsRequest>,
) -> ApiResult<Json<Value>> {
    let user = require_user(&state, &username, &lang).await?;
    let admin_group = &state.config.admin_group;

    // Losing the admin group would lock the caller out of this screen.
    if session.is_self(&username)
        && user.groups.contains(admin_group)
        && !body.groups.contains(admin_group)
    {
        return Err(ApiError::bad_request(t!(
            "error_self_admin_group",
            locale = &lang,
            group = admin_group
        )));
    }

    for group in body.groups.iter().filter(|g| !user.groups.contains(g)) {
        add_to_group(&state, &username, group, &lang).await?;
    }
    for group in user.groups.iter().filter(|g| !body.groups.contains(g)) {
        remove_from_group(&state, &username, group, &lang).await?;
    }

    Ok(message(t!("msg_groups_updated", locale = &lang)))
}

#[derive(Deserialize)]
pub struct PasswordRequest {
    password: String,
    #[serde(default)]
    permanent: bool,
}

pub async fn set_password(
    State(state): State<AppState>,
    Lang(lang): Lang,
    AdminSession(_): AdminSession,
    Path(username): Path<String>,
    Json(body): Json<PasswordRequest>,
) -> ApiResult<Json<Value>> {
    if body.password.is_empty() {
        return Err(ApiError::bad_request(t!(
            "error_password_required",
            locale = &lang
        )));
    }

    state
        .cognito
        .admin_set_user_password()
        .user_pool_id(&state.config.user_pool_id)
        .username(&username)
        .password(&body.password)
        .permanent(body.permanent)
        .send()
        .await
        .map_err(|error| cognito(error, &lang))?;

    Ok(message(if body.permanent {
        t!("msg_password_set", locale = &lang)
    } else {
        t!("msg_temporary_password_set", locale = &lang)
    }))
}

pub async fn reset_password(
    State(state): State<AppState>,
    Lang(lang): Lang,
    AdminSession(_): AdminSession,
    Path(username): Path<String>,
) -> ApiResult<Json<Value>> {
    state
        .cognito
        .admin_reset_user_password()
        .user_pool_id(&state.config.user_pool_id)
        .username(&username)
        .send()
        .await
        .map_err(|error| cognito(error, &lang))?;

    Ok(message(t!("msg_reset_code_sent", locale = &lang)))
}

#[derive(Deserialize)]
pub struct EnabledRequest {
    enabled: bool,
}

pub async fn set_enabled(
    State(state): State<AppState>,
    Lang(lang): Lang,
    AdminSession(session): AdminSession,
    Path(username): Path<String>,
    Json(body): Json<EnabledRequest>,
) -> ApiResult<Json<Value>> {
    if body.enabled {
        state
            .cognito
            .admin_enable_user()
            .user_pool_id(&state.config.user_pool_id)
            .username(&username)
            .send()
            .await
            .map_err(|error| cognito(error, &lang))?;
        return Ok(message(t!("msg_user_enabled", locale = &lang)));
    }

    deny_self(&session, &username, "error_self_disable", &lang)?;
    state
        .cognito
        .admin_disable_user()
        .user_pool_id(&state.config.user_pool_id)
        .username(&username)
        .send()
        .await
        .map_err(|error| cognito(error, &lang))?;

    Ok(message(t!("msg_user_disabled", locale = &lang)))
}

pub async fn sign_out(
    State(state): State<AppState>,
    Lang(lang): Lang,
    AdminSession(_): AdminSession,
    Path(username): Path<String>,
) -> ApiResult<Json<Value>> {
    state
        .cognito
        .admin_user_global_sign_out()
        .user_pool_id(&state.config.user_pool_id)
        .username(&username)
        .send()
        .await
        .map_err(|error| cognito(error, &lang))?;

    Ok(message(t!("msg_signed_out_user", locale = &lang)))
}

pub async fn resend_invite(
    State(state): State<AppState>,
    Lang(lang): Lang,
    AdminSession(_): AdminSession,
    Path(username): Path<String>,
) -> ApiResult<Json<Value>> {
    state
        .cognito
        .admin_create_user()
        .user_pool_id(&state.config.user_pool_id)
        .username(&username)
        .message_action(MessageActionType::Resend)
        .desired_delivery_mediums(DeliveryMediumType::Email)
        .send()
        .await
        .map_err(|error| cognito(error, &lang))?;

    Ok(message(t!("msg_invite_resent", locale = &lang)))
}

pub async fn delete(
    State(state): State<AppState>,
    Lang(lang): Lang,
    AdminSession(session): AdminSession,
    Path(username): Path<String>,
) -> ApiResult<Json<Value>> {
    deny_self(&session, &username, "error_self_delete", &lang)?;

    state
        .cognito
        .admin_delete_user()
        .user_pool_id(&state.config.user_pool_id)
        .username(&username)
        .send()
        .await
        .map_err(|error| cognito(error, &lang))?;

    Ok(message(t!("msg_user_deleted", locale = &lang)))
}

async fn add_to_group(state: &AppState, username: &str, group: &str, lang: &str) -> ApiResult<()> {
    state
        .cognito
        .admin_add_user_to_group()
        .user_pool_id(&state.config.user_pool_id)
        .username(username)
        .group_name(group)
        .send()
        .await
        .map_err(|error| cognito(error, lang))?;
    Ok(())
}

async fn remove_from_group(
    state: &AppState,
    username: &str,
    group: &str,
    lang: &str,
) -> ApiResult<()> {
    state
        .cognito
        .admin_remove_user_from_group()
        .user_pool_id(&state.config.user_pool_id)
        .username(username)
        .group_name(group)
        .send()
        .await
        .map_err(|error| cognito(error, lang))?;
    Ok(())
}

/// Guard for destructive actions on the signed-in admin, to avoid lockout.
fn deny_self(session: &Session, username: &str, key: &str, lang: &str) -> ApiResult<()> {
    if session.is_self(username) {
        Err(ApiError::bad_request(t!(key, locale = lang)))
    } else {
        Ok(())
    }
}
