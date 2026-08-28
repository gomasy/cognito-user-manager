//! Second factors: which ones a user has turned on, and the authenticator-app
//! enrolment that has to happen before one of them can be.
//!
//! The same request shape serves the admin screen and the self-service one;
//! only the Cognito call underneath differs, so the rules about what may be
//! preferred live here rather than in either handler.

use aws_sdk_cognitoidentityprovider::types::{
    EmailMfaSettingsType, SmsMfaSettingsType, SoftwareTokenMfaSettingsType,
};
use base64::Engine;
use rust_i18n::t;
use serde::{Deserialize, Serialize};

use crate::error::{ApiError, ApiResult};

/// Factor names exactly as Cognito reports them in `UserMFASettingList`, which
/// is also how the screens name them back to us.
const SMS: &str = "SMS_MFA";
const SOFTWARE_TOKEN: &str = "SOFTWARE_TOKEN_MFA";
const EMAIL: &str = "EMAIL_OTP";

/// The settings blocks one request turns into. A `None` leaves that factor
/// exactly as Cognito already has it.
pub struct Settings {
    pub sms: Option<SmsMfaSettingsType>,
    pub software_token: Option<SoftwareTokenMfaSettingsType>,
    pub email: Option<EmailMfaSettingsType>,
}

/// Which factors to switch on or off, and which of them to prefer.
///
/// A factor left out is not mentioned in the Cognito call at all, so a pool
/// that has never been given, say, email MFA is not asked about it. That
/// matters: naming an unsupported factor is rejected outright, which would
/// otherwise make the whole form unusable.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Preference {
    sms: Option<bool>,
    software_token: Option<bool>,
    email: Option<bool>,
    /// The factor to challenge with when several are on; `None` leaves Cognito
    /// to ask which one at sign-in.
    preferred: Option<String>,
}

impl Preference {
    fn state(&self, factor: &str) -> Option<bool> {
        match factor {
            SMS => self.sms,
            SOFTWARE_TOKEN => self.software_token,
            EMAIL => self.email,
            _ => None,
        }
    }

    /// `(enabled, preferred)` for a factor the request named, or `None` to
    /// leave that factor alone.
    fn flags(&self, factor: &str) -> Option<(bool, bool)> {
        let enabled = self.state(factor)?;
        Some((
            enabled,
            enabled && self.preferred.as_deref() == Some(factor),
        ))
    }

    /// What to send to Cognito, or `None` when the request names no factor and
    /// there is nothing to send.
    ///
    /// The check goes through here rather than being left to each handler, so
    /// a call site cannot send a preference that was never validated.
    pub fn settings(&self, lang: &str) -> ApiResult<Option<Settings>> {
        if self.sms.is_none() && self.software_token.is_none() && self.email.is_none() {
            return Ok(None);
        }
        self.check(lang)?;

        Ok(Some(Settings {
            sms: self.flags(SMS).map(|(enabled, preferred)| {
                SmsMfaSettingsType::builder()
                    .enabled(enabled)
                    .preferred_mfa(preferred)
                    .build()
            }),
            software_token: self.flags(SOFTWARE_TOKEN).map(|(enabled, preferred)| {
                SoftwareTokenMfaSettingsType::builder()
                    .enabled(enabled)
                    .preferred_mfa(preferred)
                    .build()
            }),
            email: self.flags(EMAIL).map(|(enabled, preferred)| {
                EmailMfaSettingsType::builder()
                    .enabled(enabled)
                    .preferred_mfa(preferred)
                    .build()
            }),
        }))
    }

    /// Cognito accepts a preferred factor only if that factor is also on, and
    /// answers a bare `InvalidParameterException` when it is not. Saying so
    /// here keeps the wording specific.
    fn check(&self, lang: &str) -> ApiResult<()> {
        let Some(preferred) = self.preferred.as_deref().filter(|name| !name.is_empty()) else {
            return Ok(());
        };
        match self.state(preferred) {
            Some(true) => Ok(()),
            _ => Err(ApiError::bad_request(t!(
                "error_mfa_preferred",
                locale = lang
            ))),
        }
    }
}

/// The authenticator factor on its own, for the two moments that are not a
/// form submission: enrolment, which turns it on and prefers it, and removal
/// of the registered app, which turns it off. Every other factor is left as
/// it was.
pub fn software_token(enabled: bool) -> SoftwareTokenMfaSettingsType {
    SoftwareTokenMfaSettingsType::builder()
        .enabled(enabled)
        .preferred_mfa(enabled)
        .build()
}

/// What the authenticator app needs to enrol: the shared secret, the URI that
/// carries it, and the same URI as a QR code ready to put in an `<img>`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TotpSetup {
    pub secret_code: String,
    pub otpauth_uri: String,
    pub qr_data_uri: Option<String>,
}

impl TotpSetup {
    pub fn new(secret: &str, issuer: &str, account: &str) -> Self {
        let uri = otpauth_uri(secret, issuer, account);
        Self {
            qr_data_uri: qr_svg(&uri).map(|svg| {
                format!(
                    "data:image/svg+xml;base64,{}",
                    base64::engine::general_purpose::STANDARD.encode(svg)
                )
            }),
            secret_code: secret.to_string(),
            otpauth_uri: uri,
        }
    }
}

/// Everything outside the unreserved set of RFC 3986 is escaped, which is
/// stricter than a URI needs but is what every authenticator app expects of a
/// label that may hold an email address or a pool name with spaces in it.
fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// The URI an authenticator app reads, per the Key Uri Format:
/// <https://github.com/google/google-authenticator/wiki/Key-Uri-Format>
fn otpauth_uri(secret: &str, issuer: &str, account: &str) -> String {
    format!(
        "otpauth://totp/{}:{}?secret={}&issuer={}",
        escape(issuer),
        escape(account),
        escape(secret),
        escape(issuer),
    )
}

/// `None` when the URI is too long to encode, which leaves the secret itself
/// as the way to enrol rather than failing the whole setup.
fn qr_svg(uri: &str) -> Option<String> {
    let code = qrcode::QrCode::new(uri).ok()?;
    Some(
        code.render::<qrcode::render::svg::Color>()
            .min_dimensions(220, 220)
            .quiet_zone(true)
            .build(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Preferring a factor that is being switched off is the one combination
    /// Cognito rejects, and it is worth its own wording.
    #[test]
    fn a_preferred_factor_has_to_be_on() {
        let on = Preference {
            software_token: Some(true),
            preferred: Some(SOFTWARE_TOKEN.to_string()),
            ..Default::default()
        };
        assert!(on.settings("en").is_ok());

        let off = Preference {
            software_token: Some(false),
            preferred: Some(SOFTWARE_TOKEN.to_string()),
            ..Default::default()
        };
        assert!(off.settings("en").is_err());

        // Preferring a factor the request says nothing about is just as wrong:
        // it may not even be enrolled.
        let unnamed = Preference {
            sms: Some(true),
            preferred: Some(SOFTWARE_TOKEN.to_string()),
            ..Default::default()
        };
        assert!(unnamed.settings("en").is_err());
    }

    /// A factor the request leaves out must not reach Cognito, or a pool
    /// without that factor rejects an unrelated change.
    #[test]
    fn an_unnamed_factor_is_left_alone() {
        let preference = Preference {
            sms: Some(true),
            ..Default::default()
        };
        let settings = preference
            .settings("en")
            .expect("valid")
            .expect("a factor was named");
        assert!(settings.sms.is_some());
        assert!(settings.software_token.is_none());
        assert!(settings.email.is_none());
    }

    /// Nothing named is not an error; it is simply nothing to send.
    #[test]
    fn a_request_naming_no_factor_sends_nothing() {
        assert!(
            Preference::default()
                .settings("en")
                .expect("valid")
                .is_none()
        );
    }

    #[test]
    fn only_the_preferred_factor_is_marked_preferred() {
        let preference = Preference {
            sms: Some(true),
            software_token: Some(true),
            preferred: Some(SMS.to_string()),
            ..Default::default()
        };
        let settings = preference
            .settings("en")
            .expect("valid")
            .expect("factors were named");
        let sms = settings.sms.expect("sms named");
        let totp = settings.software_token.expect("totp named");
        assert!(sms.enabled() && sms.preferred_mfa());
        assert!(totp.enabled() && !totp.preferred_mfa());
    }

    /// An app that is enrolled but never asked for would be a setup that
    /// silently did nothing, so enrolment prefers what it just turned on.
    #[test]
    fn enrolment_turns_the_authenticator_on_and_prefers_it() {
        let on = software_token(true);
        assert!(on.enabled() && on.preferred_mfa());

        let off = software_token(false);
        assert!(!off.enabled() && !off.preferred_mfa());
    }

    /// An email address as the label, or a pool name with a space in it, must
    /// not break the URI apart.
    #[test]
    fn a_label_cannot_break_the_uri() {
        let uri = otpauth_uri("ABC234", "Staff Pool", "user@example.com");
        assert_eq!(
            uri,
            "otpauth://totp/Staff%20Pool:user%40example.com?secret=ABC234&issuer=Staff%20Pool"
        );
    }

    #[test]
    fn a_setup_carries_a_scannable_code() {
        let setup = TotpSetup::new("ABC234", "pool", "alice");
        assert!(setup.otpauth_uri.contains("secret=ABC234"));
        let qr = setup.qr_data_uri.expect("short uri should encode");
        assert!(qr.starts_with("data:image/svg+xml;base64,"));
    }
}
