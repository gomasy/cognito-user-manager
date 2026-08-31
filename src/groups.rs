//! Groups: the pool's groups, who is in one, and the calls that change that.
//!
//! Every Cognito call that names a group goes through here, so the one
//! confusing thing about them — that a missing group is reported as a missing
//! user pool — is answered in a single place. Membership is edited from two
//! screens, the group's member list and the checkboxes on a user, and both end
//! up in the same pair of calls.

use aws_sdk_cognitoidentityprovider::error::{ProvideErrorMetadata, SdkError};
use aws_sdk_cognitoidentityprovider::operation::get_group::GetGroupError;
use aws_sdk_cognitoidentityprovider::types::GroupType;
use rust_i18n::t;
use serde::Serialize;

use crate::error::{ApiError, ApiResult, cognito, cognito_or_missing};
use crate::state::AppState;
use crate::users::{self, UserPage};

/// The most groups ListGroups will return in one page.
const LIST_LIMIT: i32 = 60;

/// A group-scoped call whose `ResourceNotFoundException` means the group, not
/// the pool. Every write below goes through it, so the wording is settled once.
fn failed<E, R>(error: SdkError<E, R>, lang: &str) -> ApiError
where
    SdkError<E, R>: ProvideErrorMetadata + std::fmt::Debug,
{
    cognito_or_missing(error, "error_group_not_found", lang)
}

fn not_found(lang: &str) -> ApiError {
    ApiError::not_found(t!("error_group_not_found", locale = lang))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupInfo {
    pub name: String,
    pub description: Option<String>,
    /// Lower wins when a user is in several groups; Cognito uses it to order
    /// the `cognito:groups` claim.
    pub precedence: Option<i32>,
    pub role_arn: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

/// `None` for a group Cognito returned without a name, which cannot be acted
/// on and would only show as an empty row.
fn to_info(group: &GroupType) -> Option<GroupInfo> {
    Some(GroupInfo {
        name: group.group_name()?.to_string(),
        description: group.description().map(str::to_string),
        precedence: group.precedence(),
        role_arn: group.role_arn().map(str::to_string),
        created_at: users::timestamp(group.creation_date()),
        updated_at: users::timestamp(group.last_modified_date()),
    })
}

/// Every group in the pool, following pagination and sorted by name.
pub async fn list(state: &AppState, lang: &str) -> ApiResult<Vec<GroupInfo>> {
    let mut groups = Vec::new();
    let mut next_token: Option<String> = None;
    loop {
        let response = state
            .cognito
            .list_groups()
            .user_pool_id(&state.config.user_pool_id)
            .limit(LIST_LIMIT)
            .set_next_token(next_token)
            .send()
            .await
            .map_err(|error| cognito(error, lang))?;

        groups.extend(response.groups().iter().filter_map(to_info));
        next_token = response.next_token().map(str::to_string);
        if next_token.is_none() {
            break;
        }
    }
    groups.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(groups)
}

/// Just the names, for the membership checkboxes on the user screens.
pub async fn names(state: &AppState, lang: &str) -> ApiResult<Vec<String>> {
    Ok(list(state, lang)
        .await?
        .into_iter()
        .map(|group| group.name)
        .collect())
}

/// `None` when no such group exists, so the caller can answer 404 with its own
/// wording rather than the pool-level one a bare ResourceNotFound would get.
pub async fn get(state: &AppState, name: &str, lang: &str) -> ApiResult<Option<GroupInfo>> {
    match state
        .cognito
        .get_group()
        .user_pool_id(&state.config.user_pool_id)
        .group_name(name)
        .send()
        .await
    {
        Ok(response) => Ok(response.group().and_then(to_info)),
        Err(SdkError::ServiceError(error))
            if matches!(error.err(), GetGroupError::ResourceNotFoundException(_)) =>
        {
            Ok(None)
        }
        Err(error) => Err(cognito(error, lang)),
    }
}

/// The same as `get`, for the callers that have nothing to do without the
/// group and would only repeat the same 404 themselves.
pub async fn require(state: &AppState, name: &str, lang: &str) -> ApiResult<GroupInfo> {
    get(state, name, lang).await?.ok_or_else(|| not_found(lang))
}

/// Adds a user to a group. Shared by the group's member list and the group
/// checkboxes on the user screen, which are the same two Cognito calls.
pub async fn add_user(state: &AppState, username: &str, group: &str, lang: &str) -> ApiResult<()> {
    state
        .cognito
        .admin_add_user_to_group()
        .user_pool_id(&state.config.user_pool_id)
        .username(username)
        .group_name(group)
        .send()
        .await
        .map_err(|error| failed(error, lang))?;
    Ok(())
}

pub async fn remove_user(
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
        .map_err(|error| failed(error, lang))?;
    Ok(())
}

pub async fn create(
    state: &AppState,
    name: &str,
    description: Option<String>,
    precedence: Option<i32>,
    lang: &str,
) -> ApiResult<()> {
    state
        .cognito
        .create_group()
        .user_pool_id(&state.config.user_pool_id)
        .group_name(name)
        .set_description(description)
        .set_precedence(precedence)
        .send()
        .await
        // Nothing here names an existing group, so a missing resource is the
        // pool itself and gets the shared wording.
        .map_err(|error| cognito(error, lang))?;
    Ok(())
}

/// Removes the group itself. Its members keep their accounts.
pub async fn delete(state: &AppState, name: &str, lang: &str) -> ApiResult<()> {
    state
        .cognito
        .delete_group()
        .user_pool_id(&state.config.user_pool_id)
        .group_name(name)
        .send()
        .await
        .map_err(|error| failed(error, lang))?;
    Ok(())
}

/// One page of the group's members, in the same shape as the user list.
pub async fn members(
    state: &AppState,
    group: &str,
    limit: i32,
    token: Option<String>,
    lang: &str,
) -> ApiResult<UserPage> {
    let response = state
        .cognito
        .list_users_in_group()
        .user_pool_id(&state.config.user_pool_id)
        .group_name(group)
        .limit(limit)
        .set_next_token(token.filter(|value| !value.is_empty()))
        .send()
        .await
        .map_err(|error| failed(error, lang))?;

    Ok(users::page(response.users(), response.next_token()))
}
