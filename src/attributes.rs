use std::collections::BTreeMap;

use aws_sdk_cognitoidentityprovider::types::AttributeType;
use rust_i18n::t;

use crate::error::ApiError;
use crate::schema::{AttributeField, DataType};

/// Attribute values as stored on a user.
pub type Values = BTreeMap<String, String>;

/// What the client submits: `null` asks for the attribute to be deleted.
pub type Patch = BTreeMap<String, Option<String>>;

pub fn to_values(attributes: &[AttributeType]) -> Values {
    attributes
        .iter()
        .map(|attribute| {
            (
                attribute.name().to_string(),
                attribute.value().unwrap_or_default().to_string(),
            )
        })
        .collect()
}

#[derive(Debug)]
pub struct Changes {
    pub attributes: Vec<AttributeType>,
    pub to_delete: Vec<String>,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        self.attributes.is_empty() && self.to_delete.is_empty()
    }
}

fn invalid(key: &str, attr: &str, limit: i64, lang: &str) -> ApiError {
    ApiError::bad_request(t!(key, locale = lang, attr = attr, limit = limit))
}

fn validate(field: &AttributeField, value: &str, lang: &str) -> Result<(), ApiError> {
    match field.data_type {
        DataType::Number => {
            let numeric: f64 = value.parse().map_err(|_| {
                ApiError::bad_request(t!(
                    "error_number_expected",
                    locale = lang,
                    attr = &field.name
                ))
            })?;
            if let Some(min) = field.min_value
                && numeric < min as f64
            {
                return Err(invalid("error_min_value", &field.name, min, lang));
            }
            if let Some(max) = field.max_value
                && numeric > max as f64
            {
                return Err(invalid("error_max_value", &field.name, max, lang));
            }
        }
        DataType::String => {
            let length = value.chars().count() as i64;
            if let Some(min) = field.min_length
                && length < min
            {
                return Err(invalid("error_min_length", &field.name, min, lang));
            }
            if let Some(max) = field.max_length
                && length > max
            {
                return Err(invalid("error_max_length", &field.name, max, lang));
            }
        }
        // Listed rather than caught by a wildcard, so a new data type has to
        // decide here instead of silently skipping validation.
        DataType::Boolean | DataType::DateTime => {}
    }
    Ok(())
}

/// Turns a patch into the Cognito calls it implies.
///
/// Only attributes named in the patch are touched, so a screen that shows a
/// subset of the schema cannot clear the rest. Attributes not in `fields` are
/// ignored: that is what keeps a client from writing an immutable or
/// developer-only attribute.
pub fn diff(
    patch: &Patch,
    fields: &[AttributeField],
    current: &Values,
    lang: &str,
) -> Result<Changes, ApiError> {
    let mut attributes = Vec::new();
    let mut to_delete = Vec::new();

    for field in fields {
        let Some(submitted) = patch.get(&field.name) else {
            continue;
        };
        let value = submitted.as_deref().unwrap_or("").trim();

        if value.is_empty() {
            if field.required {
                return Err(ApiError::bad_request(t!(
                    "error_required_attribute",
                    locale = lang,
                    attr = &field.name
                )));
            }
            if current.get(&field.name).is_some_and(|v| !v.is_empty()) {
                to_delete.push(field.name.clone());
            }
            continue;
        }

        validate(field, value, lang)?;
        if current.get(&field.name).map(String::as_str) == Some(value) {
            continue;
        }
        let attribute = AttributeType::builder()
            .name(&field.name)
            .value(value)
            .build()
            .map_err(|_| ApiError::bad_request(t!("error_invalid_parameter", locale = lang)))?;
        attributes.push(attribute);
    }

    Ok(Changes {
        attributes,
        to_delete,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    fn field(name: &str, data_type: DataType, required: bool) -> AttributeField {
        AttributeField {
            name: name.to_string(),
            data_type,
            mutable: true,
            required,
            is_custom: name.starts_with("custom:"),
            developer_only: false,
            min_length: None,
            max_length: None,
            min_value: None,
            max_value: None,
        }
    }

    fn fields() -> Vec<AttributeField> {
        let mut age = field("custom:age", DataType::Number, false);
        age.min_value = Some(0);
        age.max_value = Some(150);
        vec![
            field("email", DataType::String, true),
            field("nickname", DataType::String, false),
            field("email_verified", DataType::Boolean, false),
            age,
        ]
    }

    fn patch(pairs: &[(&str, Option<&str>)]) -> Patch {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.map(str::to_string)))
            .collect()
    }

    fn values(pairs: &[(&str, &str)]) -> Values {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn written(changes: &Changes) -> Vec<(String, String)> {
        changes
            .attributes
            .iter()
            .map(|a| {
                (
                    a.name().to_string(),
                    a.value().unwrap_or_default().to_string(),
                )
            })
            .collect()
    }

    #[test]
    fn writes_only_changed_attributes() {
        let current = values(&[("email", "a@example.com"), ("nickname", "taro")]);
        let changes = diff(
            &patch(&[("email", Some("a@example.com")), ("nickname", Some("jiro"))]),
            &fields(),
            &current,
            "en",
        )
        .expect("diff");
        assert_eq!(written(&changes), vec![("nickname".into(), "jiro".into())]);
    }

    #[test]
    fn null_deletes_an_attribute_that_has_a_value() {
        let current = values(&[("email", "a@example.com"), ("nickname", "taro")]);
        let changes = diff(&patch(&[("nickname", None)]), &fields(), &current, "en").expect("diff");
        assert_eq!(changes.to_delete, vec!["nickname".to_string()]);
        assert!(changes.attributes.is_empty());
    }

    #[test]
    fn null_on_an_unset_attribute_does_nothing() {
        let changes = diff(
            &patch(&[("nickname", None)]),
            &fields(),
            &Values::new(),
            "en",
        )
        .expect("diff");
        assert!(changes.is_empty());
    }

    #[test]
    fn a_required_attribute_cannot_be_cleared() {
        let current = values(&[("email", "a@example.com")]);
        let error = diff(&patch(&[("email", None)]), &fields(), &current, "en")
            .expect_err("required attribute");
        assert_eq!(error.status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn booleans_round_trip_as_strings() {
        let current = values(&[("email_verified", "false")]);
        let changes = diff(
            &patch(&[("email_verified", Some("true"))]),
            &fields(),
            &current,
            "en",
        )
        .expect("diff");
        assert_eq!(
            written(&changes),
            vec![("email_verified".into(), "true".into())]
        );
    }

    #[test]
    fn numbers_are_range_checked() {
        assert!(
            diff(
                &patch(&[("custom:age", Some("200"))]),
                &fields(),
                &Values::new(),
                "en"
            )
            .is_err()
        );
        assert!(
            diff(
                &patch(&[("custom:age", Some("abc"))]),
                &fields(),
                &Values::new(),
                "en"
            )
            .is_err()
        );
        let changes = diff(
            &patch(&[("custom:age", Some("42"))]),
            &fields(),
            &Values::new(),
            "en",
        )
        .expect("diff");
        assert_eq!(written(&changes), vec![("custom:age".into(), "42".into())]);
    }

    #[test]
    fn attributes_outside_the_allowed_set_are_ignored() {
        // A client cannot reach an immutable or developer-only attribute by
        // naming it: only fields the caller was given are considered.
        let allowed = vec![field("nickname", DataType::String, false)];
        let changes = diff(
            &patch(&[
                ("email", Some("evil@example.com")),
                ("nickname", Some("taro")),
            ]),
            &allowed,
            &Values::new(),
            "en",
        )
        .expect("diff");
        assert_eq!(written(&changes), vec![("nickname".into(), "taro".into())]);
    }
}
