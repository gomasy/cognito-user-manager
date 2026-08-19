use std::sync::RwLock;
use std::time::{Duration, Instant};

use aws_sdk_cognitoidentityprovider::types::{AttributeDataType, SchemaAttributeType};
use serde::Serialize;

use crate::error::{ApiResult, cognito};
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

        let mut fields: Vec<AttributeField> = pool
            .map(|p| p.schema_attributes())
            .unwrap_or_default()
            .iter()
            .filter_map(to_field)
            .filter(|field| !HIDDEN.contains(&field.name.as_str()))
            .collect();
        fields.sort_by(|a, b| a.is_custom.cmp(&b.is_custom).then_with(|| a.name.cmp(&b.name)));

        let info = PoolInfo {
            id: state.config.user_pool_id.clone(),
            name: pool.and_then(|p| p.name()).map(str::to_string),
            fields,
            username_is_email: pool
                .map(|p| p.username_attributes())
                .unwrap_or_default()
                .iter()
                .any(|attribute| attribute.as_str() == "email"),
        };

        if let Ok(mut guard) = self.inner.write() {
            *guard = Some((info.clone(), Instant::now()));
        }
        Ok(info)
    }
}

/// All group names in the user pool, following pagination.
pub async fn list_group_names(state: &AppState, lang: &str) -> ApiResult<Vec<String>> {
    let mut names = Vec::new();
    let mut next_token: Option<String> = None;
    loop {
        let response = state
            .cognito
            .list_groups()
            .user_pool_id(&state.config.user_pool_id)
            .limit(60)
            .set_next_token(next_token)
            .send()
            .await
            .map_err(|error| cognito(error, lang))?;

        names.extend(
            response
                .groups()
                .iter()
                .filter_map(|group| group.group_name().map(str::to_string)),
        );
        next_token = response.next_token().map(str::to_string);
        if next_token.is_none() {
            break;
        }
    }
    names.sort();
    Ok(names)
}
