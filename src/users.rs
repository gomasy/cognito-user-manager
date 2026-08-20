use aws_sdk_cognitoidentityprovider::error::SdkError;
use aws_sdk_cognitoidentityprovider::operation::admin_get_user::AdminGetUserError;
use aws_smithy_types::DateTime;
use aws_smithy_types::date_time::Format;
use serde::Serialize;

use crate::attributes::{Values, to_values};
use crate::error::{ApiResult, cognito};
use crate::session::Session;
use crate::state::AppState;

/// RFC 3339 so the frontend can format it in the user's locale.
fn timestamp(value: Option<&DateTime>) -> Option<String> {
    value.and_then(|date| date.fmt(Format::DateTime).ok())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserSummary {
    pub username: String,
    pub enabled: bool,
    pub status: Option<String>,
    pub created_at: Option<String>,
    pub attributes: Values,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserDetail {
    pub username: String,
    pub enabled: bool,
    pub status: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub attributes: Values,
    pub groups: Vec<String>,
    pub mfa: Vec<String>,
    pub preferred_mfa: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MyProfile {
    pub username: String,
    pub attributes: Values,
    pub mfa: Vec<String>,
    pub preferred_mfa: Option<String>,
    pub groups: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserPage {
    pub users: Vec<UserSummary>,
    pub next_token: Option<String>,
}

/// Attributes the ListUsers Filter accepts. Custom attributes are not
/// searchable, and an unknown field would make Cognito reject the request.
pub const SEARCH_FIELDS: [&str; 9] = [
    "email",
    "username",
    "name",
    "given_name",
    "family_name",
    "preferred_username",
    "phone_number",
    "sub",
    "cognito:user_status",
];

/// Resolves a client-supplied field name to one of `SEARCH_FIELDS`. Returning
/// `&'static str` is what keeps an arbitrary string out of the filter below.
pub fn search_field(requested: Option<&str>) -> &'static str {
    requested
        .and_then(|name| SEARCH_FIELDS.iter().find(|field| **field == name).copied())
        .unwrap_or(SEARCH_FIELDS[0])
}

/// A filter value is a double-quoted string in which `\` and `"` are escaped.
/// The backslash has to be escaped first, or the one added for a quote would
/// itself be doubled.
fn escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

/// ListUsers only supports prefix matching on a fixed set of attributes,
/// so the search term is applied as a prefix filter on the chosen field.
pub async fn list(
    state: &AppState,
    search: &str,
    field: &'static str,
    limit: i32,
    token: Option<String>,
    lang: &str,
) -> ApiResult<UserPage> {
    let mut request = state
        .cognito
        .list_users()
        .user_pool_id(&state.config.user_pool_id)
        .limit(limit)
        .set_pagination_token(token.filter(|value| !value.is_empty()));

    let search = search.trim();
    if !search.is_empty() {
        request = request.filter(format!("{field} ^= \"{}\"", escape(search)));
    }

    let response = request.send().await.map_err(|error| cognito(error, lang))?;

    Ok(UserPage {
        users: response
            .users()
            .iter()
            .map(|user| UserSummary {
                username: user.username().unwrap_or_default().to_string(),
                enabled: user.enabled(),
                status: user.user_status().map(|status| status.as_str().to_string()),
                created_at: timestamp(user.user_create_date()),
                attributes: to_values(user.attributes()),
            })
            .collect(),
        next_token: response.pagination_token().map(str::to_string),
    })
}

pub async fn detail(state: &AppState, username: &str, lang: &str) -> ApiResult<Option<UserDetail>> {
    let user = match state
        .cognito
        .admin_get_user()
        .user_pool_id(&state.config.user_pool_id)
        .username(username)
        .send()
        .await
    {
        Ok(user) => user,
        Err(SdkError::ServiceError(error))
            if matches!(error.err(), AdminGetUserError::UserNotFoundException(_)) =>
        {
            return Ok(None);
        }
        Err(error) => return Err(cognito(error, lang)),
    };

    let groups = state
        .cognito
        .admin_list_groups_for_user()
        .user_pool_id(&state.config.user_pool_id)
        .username(username)
        .limit(60)
        .send()
        .await
        .map_err(|error| cognito(error, lang))?;

    Ok(Some(UserDetail {
        username: user.username().to_string(),
        enabled: user.enabled(),
        status: user.user_status().map(|status| status.as_str().to_string()),
        created_at: timestamp(user.user_create_date()),
        updated_at: timestamp(user.user_last_modified_date()),
        attributes: to_values(user.user_attributes()),
        groups: groups
            .groups()
            .iter()
            .filter_map(|group| group.group_name().map(str::to_string))
            .collect(),
        mfa: user.user_mfa_setting_list().to_vec(),
        preferred_mfa: user.preferred_mfa_setting().map(str::to_string),
    }))
}

/// The signed-in user's own profile, read with their access token.
pub async fn profile(state: &AppState, session: &Session, lang: &str) -> ApiResult<MyProfile> {
    let response = state
        .cognito
        .get_user()
        .access_token(&session.access_token)
        .send()
        .await
        .map_err(|error| cognito(error, lang))?;

    Ok(MyProfile {
        username: response.username().to_string(),
        attributes: to_values(response.user_attributes()),
        mfa: response.user_mfa_setting_list().to_vec(),
        preferred_mfa: response.preferred_mfa_setting().map(str::to_string),
        groups: session.groups.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_allowed_field_reaches_the_filter() {
        assert_eq!(search_field(Some("phone_number")), "phone_number");
        // An unknown field would make Cognito reject the whole request.
        assert_eq!(search_field(Some("custom:team")), SEARCH_FIELDS[0]);
        assert_eq!(search_field(None), SEARCH_FIELDS[0]);
    }

    /// A term ending in a backslash would otherwise escape the closing quote
    /// and leave Cognito with a filter it cannot parse.
    #[test]
    fn a_search_term_cannot_break_out_of_the_quoted_value() {
        assert_eq!(escape(r#"plain"#), "plain");
        assert_eq!(escape(r#"a"b"#), r#"a\"b"#);
        assert_eq!(escape(r#"trailing\"#), r#"trailing\\"#);
        assert_eq!(escape(r#"\" or sub ^= ""#), r#"\\\" or sub ^= \""#);
    }
}

/// Read-only smoke test against the pool configured in .env.
/// Opt in with `cargo test -- --ignored --nocapture`.
#[cfg(test)]
mod live_tests {
    use super::*;
    use crate::config::Config;
    use crate::schema;
    use std::sync::Arc;

    async fn state() -> AppState {
        let _ = dotenvy::dotenv();
        AppState::new(Arc::new(Config::from_env().expect("config"))).await
    }

    #[tokio::test]
    #[ignore = "requires live AWS credentials"]
    async fn reads_the_pool_schema_and_users() {
        let state = state().await;

        let pool = state
            .schema
            .get(&state, "en")
            .await
            .expect("describe user pool");
        println!(
            "pool={} fields={} custom={} editable={} selfEditable={}",
            pool.name.as_deref().unwrap_or(&pool.id),
            pool.fields.len(),
            pool.fields.iter().filter(|f| f.is_custom).count(),
            pool.editable().len(),
            pool.self_editable().len()
        );
        assert!(!pool.fields.is_empty(), "schema should expose attributes");

        let groups = schema::list_group_names(&state, "en")
            .await
            .expect("list groups");
        println!("groups={groups:?}");

        let page = list(&state, "", "email", 5, None, "en")
            .await
            .expect("list users");
        println!("listed {} user(s)", page.users.len());

        let Some(first) = page.users.first() else {
            println!("pool has no users; detail path not exercised");
            return;
        };

        let found = detail(&state, &first.username, "en")
            .await
            .expect("admin get user")
            .expect("user should exist");
        println!(
            "detail: enabled={} status={:?} attrs={} groups={:?} mfa={:?}",
            found.enabled,
            found.status,
            found.attributes.len(),
            found.groups,
            found.mfa
        );

        assert!(
            detail(&state, "__definitely_missing__", "en")
                .await
                .expect("missing user should not error")
                .is_none()
        );
    }
}
