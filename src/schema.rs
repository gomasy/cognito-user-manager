use std::sync::RwLock;
use std::time::{Duration, Instant};

use aws_sdk_cognitoidentityprovider::types::{
    AttributeDataType, AuthFactorType, SchemaAttributeType,
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
    /// Server-side only: it drives the generated temporary passwords and no
    /// screen has any use for it.
    #[serde(skip)]
    pub password_policy: password::Policy,
    /// False only for a pool that signs users in without passwords at all,
    /// where Cognito rejects a new user that comes with one.
    #[serde(skip)]
    pub password_sign_in: bool,
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

        let info = PoolInfo {
            id: state.config.user_pool_id.clone(),
            name: pool.and_then(|p| p.name()).map(str::to_string),
            fields,
            username_is_email: pool
                .map(|p| p.username_attributes())
                .unwrap_or_default()
                .iter()
                .any(|attribute| attribute.as_str() == "email"),
            password_policy: policies
                .and_then(|p| p.password_policy())
                .map(password::Policy::from)
                .unwrap_or_default(),
            password_sign_in: policies
                .and_then(|p| p.sign_in_policy())
                .map(|policy| policy.allowed_first_auth_factors())
                // An empty list means the pool never opted into choice-based
                // authentication, which leaves passwords as the only factor.
                .is_none_or(|factors| {
                    factors.is_empty() || factors.contains(&AuthFactorType::Password)
                }),
        };

        if let Ok(mut guard) = self.inner.write() {
            *guard = Some((info.clone(), Instant::now()));
        }
        Ok(info)
    }
}
