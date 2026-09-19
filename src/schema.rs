use std::sync::RwLock;
use std::time::{Duration, Instant};

use aws_sdk_cognitoidentityprovider::types::{
    AttributeDataType, AuthFactorType, ExplicitAuthFlowsType, SchemaAttributeType,
};
use serde::Serialize;

use crate::error::{ApiResult, cognito};
use crate::password;
use crate::state::AppState;

const CACHE_TTL: Duration = Duration::from_secs(5 * 60);

/// Standard OIDC attributes of a Cognito user pool. Anything else is custom.
/// Labels live in the frontend catalogs, so only the classification is here.
const STANDARD: [&str; 20] = [
    "sub",
    "name",
    "given_name",
    "family_name",
    "middle_name",
    "nickname",
    "preferred_username",
    "profile",
    "picture",
    "website",
    "email",
    "email_verified",
    "gender",
    "birthdate",
    "zoneinfo",
    "locale",
    "phone_number",
    "phone_number_verified",
    "address",
    "updated_at",
];

/// Never shown: `sub` is assigned by Cognito and `updated_at` is bookkeeping.
const HIDDEN: [&str; 2] = ["sub", "updated_at"];

/// Attributes a user cannot set on themselves; they need Cognito's
/// verification flow instead.
const SELF_READONLY: [&str; 2] = ["email_verified", "phone_number_verified"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DataType {
    String,
    Number,
    DateTime,
    Boolean,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttributeField {
    /// Attribute name as Cognito stores it; custom ones keep the "custom:" prefix.
    pub name: String,
    pub data_type: DataType,
    pub mutable: bool,
    pub required: bool,
    pub is_custom: bool,
    #[serde(skip)]
    pub developer_only: bool,
    pub min_length: Option<i64>,
    pub max_length: Option<i64>,
    pub min_value: Option<i64>,
    pub max_value: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PoolInfo {
    pub id: String,
    pub name: Option<String>,
    pub fields: Vec<AttributeField>,
    /// True when the pool signs users in by email rather than a username.
    pub username_is_email: bool,
    /// `OFF`, `ON` or `OPTIONAL`. A pool with MFA off rejects every per-user
    /// preference, so the screens say so rather than offering a form that
    /// cannot work.
    pub mfa_configuration: String,
    /// Server-side only: it drives the generated temporary passwords and no
    /// screen has any use for it.
    #[serde(skip)]
    pub password_policy: password::Policy,
    /// False only for a pool that signs users in without passwords at all,
    /// where Cognito rejects a new user that comes with one.
    #[serde(skip)]
    pub password_sign_in: bool,
    /// `WEB_AUTHN` among the pool's first factors. The account screen shows the
    /// passkey card on this alone: listing and removing credentials are
    /// access-token APIs that work whatever the app client allows, and a user
    /// who has one registered must be able to get rid of it.
    pub passkeys: bool,
    /// The above *and* `ALLOW_USER_AUTH` on the app client, which is what
    /// registering a passkey and signing in with one both need. Cognito refuses
    /// either half-configured, so the screens offer neither without this.
    pub passkeys_usable: bool,
}

impl PoolInfo {
    /// Everything an admin may see, immutable attributes included: the create
    /// screen can still set those.
    pub fn admin_visible(&self) -> Vec<AttributeField> {
        self.filter(|field| !field.developer_only)
    }

    pub fn editable(&self) -> Vec<AttributeField> {
        self.filter(|field| field.mutable && !field.developer_only)
    }

    pub fn self_editable(&self) -> Vec<AttributeField> {
        self.filter(|field| {
            field.mutable && !field.developer_only && !SELF_READONLY.contains(&field.name.as_str())
        })
    }

    fn filter(&self, keep: impl Fn(&AttributeField) -> bool) -> Vec<AttributeField> {
        self.fields.iter().filter(|f| keep(f)).cloned().collect()
    }
}

fn to_field(attribute: &SchemaAttributeType) -> Option<AttributeField> {
    let raw = attribute.name()?;
    let is_custom = !STANDARD.contains(&raw);
    // DescribeUserPool usually returns the prefix already; add it if it does not.
    let name = if !is_custom || raw.starts_with("custom:") || raw.starts_with("dev:") {
        raw.to_string()
    } else {
        format!("custom:{raw}")
    };

    let strings = attribute.string_attribute_constraints();
    let numbers = attribute.number_attribute_constraints();

    Some(AttributeField {
        name,
        data_type: match attribute.attribute_data_type() {
            Some(AttributeDataType::Number) => DataType::Number,
            Some(AttributeDataType::Datetime) => DataType::DateTime,
            Some(AttributeDataType::Boolean) => DataType::Boolean,
            _ => DataType::String,
        },
        mutable: attribute.mutable().unwrap_or(false),
        required: attribute.required().unwrap_or(false),
        is_custom,
        developer_only: attribute.developer_only_attribute().unwrap_or(false),
        min_length: strings.and_then(|c| c.min_length()).and_then(parse),
        max_length: strings.and_then(|c| c.max_length()).and_then(parse),
        min_value: numbers.and_then(|c| c.min_value()).and_then(parse),
        max_value: numbers.and_then(|c| c.max_value()).and_then(parse),
    })
}

fn parse(value: &str) -> Option<i64> {
    value.parse().ok()
}

/// User pool schema, cached for five minutes.
pub struct SchemaCache {
    inner: RwLock<Option<(PoolInfo, Instant)>>,
}

impl SchemaCache {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(None),
        }
    }

    fn cached(&self) -> Option<PoolInfo> {
        let guard = self.inner.read().ok()?;
        let (info, fetched_at) = guard.as_ref()?;
        (fetched_at.elapsed() < CACHE_TTL).then(|| info.clone())
    }

    pub async fn get(&self, state: &AppState, lang: &str) -> ApiResult<PoolInfo> {
        if let Some(info) = self.cached() {
            return Ok(info);
        }

        let response = state
            .cognito
            .describe_user_pool()
            .user_pool_id(&state.config.user_pool_id)
            .send()
            .await
            .map_err(|error| cognito(error, lang))?;
        let pool = response.user_pool();
        let policies = pool.and_then(|p| p.policies());

        let mut fields: Vec<AttributeField> = pool
            .map(|p| p.schema_attributes())
            .unwrap_or_default()
            .iter()
            .filter_map(to_field)
            .filter(|field| !HIDDEN.contains(&field.name.as_str()))
            .collect();
        fields.sort_by(|a, b| {
            a.is_custom
                .cmp(&b.is_custom)
                .then_with(|| a.name.cmp(&b.name))
        });

        // The first factors the pool allows. An empty list — which is also what
        // a pool that never opted into choice-based authentication answers —
        // leaves passwords as the only one.
        let factors = policies
            .and_then(|p| p.sign_in_policy())
            .map(|policy| policy.allowed_first_auth_factors())
            .unwrap_or_default();
        let passkeys = factors.contains(&AuthFactorType::WebAuthn);
        // Asked only where the pool allows passkeys, so a deployment that has
        // none never makes the call.
        let passkeys_usable = passkeys && choice_sign_in(state).await;

        let info = PoolInfo {
            id: state.config.user_pool_id.clone(),
            name: pool.and_then(|p| p.name()).map(str::to_string),
            fields,
            username_is_email: pool
                .map(|p| p.username_attributes())
                .unwrap_or_default()
                .iter()
                .any(|attribute| attribute.as_str() == "email"),
            mfa_configuration: pool
                .and_then(|p| p.mfa_configuration())
                .map(|mfa| mfa.as_str().to_string())
                .unwrap_or_else(|| "OFF".to_string()),
            password_policy: policies
                .and_then(|p| p.password_policy())
                .map(password::Policy::from)
                .unwrap_or_default(),
            password_sign_in: factors.is_empty() || factors.contains(&AuthFactorType::Password),
            passkeys,
            passkeys_usable,
        };

        if let Ok(mut guard) = self.inner.write() {
            *guard = Some((info.clone(), Instant::now()));
        }
        Ok(info)
    }
}

/// `ALLOW_USER_AUTH` on the app client: choice-based authentication, the only
/// flow passkeys exist in. See `PoolInfo::passkeys_usable`.
///
/// Best effort, because reading the client is an IAM action of its own and a
/// deployment whose policy predates it would otherwise lose passkeys
/// altogether: a call that fails for any reason leaves the pool's own setting
/// to decide, as it did before this check existed. Warned about rather than
/// logged quietly, since the guess then stands for the cache lifetime.
async fn choice_sign_in(state: &AppState) -> bool {
    match state
        .cognito
        .describe_user_pool_client()
        .user_pool_id(&state.config.user_pool_id)
        .client_id(&state.config.client_id)
        .send()
        .await
    {
        Ok(response) => response
            .user_pool_client()
            .map(|client| client.explicit_auth_flows())
            .unwrap_or_default()
            .contains(&ExplicitAuthFlowsType::AllowUserAuth),
        Err(error) => {
            tracing::warn!(?error, "app client flows unreadable; assuming passkeys");
            true
        }
    }
}
