//! Passkeys: the WebAuthn credentials a user registers on their own account.
//!
//! What the browser is handed, and what it answers with, is JSON that Cognito
//! issues and verifies; neither half is ours to read.

use aws_sdk_cognitoidentityprovider::types::WebAuthnCredentialDescription;
use aws_smithy_types::{Document, Number};
use serde::Serialize;
use serde_json::Value;

/// One registered passkey, as the account screen lists it. The friendly name
/// is Cognito's own; there is no way to pass one in.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Credential {
    pub id: String,
    pub name: String,
    pub relying_party_id: String,
    pub attachment: Option<String>,
    pub transports: Vec<String>,
    pub created_at: Option<String>,
}

impl From<&WebAuthnCredentialDescription> for Credential {
    fn from(credential: &WebAuthnCredentialDescription) -> Self {
        Self {
            id: credential.credential_id().to_string(),
            name: credential.friendly_credential_name().to_string(),
            relying_party_id: credential.relying_party_id().to_string(),
            attachment: credential.authenticator_attachment().map(str::to_string),
            transports: credential.authenticator_transports().to_vec(),
            created_at: crate::users::timestamp(Some(credential.created_at())),
        }
    }
}

/// The registration options on their way to the browser.
pub fn to_json(document: &Document) -> Value {
    match document {
        Document::Object(members) => Value::Object(
            members
                .iter()
                .map(|(key, value)| (key.clone(), to_json(value)))
                .collect(),
        ),
        Document::Array(items) => Value::Array(items.iter().map(to_json).collect()),
        Document::Number(Number::PosInt(number)) => Value::from(*number),
        Document::Number(Number::NegInt(number)) => Value::from(*number),
        // An infinity or a NaN has no JSON spelling and becomes null; nothing
        // in a WebAuthn structure is one.
        Document::Number(Number::Float(number)) => Value::from(*number),
        Document::String(text) => Value::String(text.clone()),
        Document::Bool(flag) => Value::Bool(*flag),
        Document::Null => Value::Null,
    }
}

/// The credential the browser produced, on its way back to Cognito.
pub fn from_json(value: &Value) -> Document {
    match value {
        Value::Object(members) => Document::Object(
            members
                .iter()
                .map(|(key, value)| (key.clone(), from_json(value)))
                .collect(),
        ),
        Value::Array(items) => Document::Array(items.iter().map(from_json).collect()),
        Value::Number(number) => {
            if let Some(positive) = number.as_u64() {
                Document::Number(Number::PosInt(positive))
            } else if let Some(negative) = number.as_i64() {
                Document::Number(Number::NegInt(negative))
            } else {
                Document::Number(Number::Float(number.as_f64().unwrap_or_default()))
            }
        }
        Value::String(text) => Document::String(text.clone()),
        Value::Bool(flag) => Document::Bool(*flag),
        Value::Null => Document::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A member lost or a number widened on the way would leave Cognito with a
    /// signature it cannot verify.
    #[test]
    fn a_credential_survives_the_round_trip() {
        let credential = json!({
            "id": "ZXhhbXBsZQ",
            "type": "public-key",
            "response": {
                "clientDataJSON": "eyJ0eXBlIjoid2ViYXV0aG4uY3JlYXRlIn0",
                "transports": ["internal", "hybrid"],
                "publicKeyAlgorithm": -7,
            },
            "clientExtensionResults": { "credProps": { "rk": true } },
            "authenticatorAttachment": null,
        });

        assert_eq!(to_json(&from_json(&credential)), credential);
    }

    #[test]
    fn both_signs_of_a_number_keep_their_type() {
        // -7 is ES256, the one number in the structure that has to stay negative.
        assert_eq!(to_json(&from_json(&json!(-7))), json!(-7));
        assert_eq!(to_json(&from_json(&json!(60000))), json!(60000));
    }
}
