use std::collections::HashMap;

use aws_sdk_cognitoidentityprovider::operation::admin_initiate_auth::AdminInitiateAuthOutput;
use aws_sdk_cognitoidentityprovider::operation::admin_initiate_auth::builders::AdminInitiateAuthFluentBuilder;
use aws_sdk_cognitoidentityprovider::operation::admin_respond_to_auth_challenge::AdminRespondToAuthChallengeOutput;
use aws_sdk_cognitoidentityprovider::types::{
    AuthFlowType, AuthenticationResultType, ChallengeNameType,
};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use rust_i18n::t;
use serde::{Deserialize, Serialize};
use tower_cookies::Cookies;
use tower_cookies::cookie::time::Duration;

use crate::error::{ApiError, ApiResult, cognito};
use crate::extract::{Lang, SecureCookies};
use crate::session::{self, ACCESS_COOKIE, CHALLENGE_COOKIE};
use crate::state::AppState;

const CHALLENGE_MAX_AGE: Duration = Duration::minutes(15);

/// Challenges this app can answer. Anything else is sent back as unsupported.
const SUPPORTED: [&str; 6] = [
    "NEW_PASSWORD_REQUIRED",
    "SMS_MFA",
    "EMAIL_OTP",
    "SOFTWARE_TOKEN_MFA",
    "SELECT_MFA_TYPE",
    WEB_AUTHN,
];

/// Cognito only offers a passkey to a sign-in that asks for one by name.
const WEB_AUTHN: &str = "WEB_AUTHN";

/// Pending challenge, kept in an httpOnly cookie between requests. The Cognito
/// session string never reaches the browser.
#[derive(Debug, Serialize, Deserialize)]
struct StoredChallenge {
    name: String,
    session: String,
    username: String,
    required_attributes: Vec<String>,
    mfa_options: Vec<String>,
    destination: Option<String>,
    /// Needed on the way out and never again, and a cookie holds 4 KB in
    /// total: this one travels in the response body alone.
    #[serde(skip)]
    credential_request_options: Option<String>,
}

/// What the browser is told about a pending challenge.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChallengeView {
    name: String,
    required_attributes: Vec<String>,
    mfa_options: Vec<String>,
    destination: Option<String>,
    credential_request_options: Option<String>,
}

impl From<&StoredChallenge> for ChallengeView {
    fn from(challenge: &StoredChallenge) -> Self {
        Self {
            name: challenge.name.clone(),
            required_attributes: challenge.required_attributes.clone(),
            mfa_options: challenge.mfa_options.clone(),
            destination: challenge.destination.clone(),
            credential_request_options: challenge.credential_request_options.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum AuthOutcome {
    SignedIn,
    Challenge { challenge: ChallengeView },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginRequest {
    username: String,
    password: String,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChallengeRequest {
    #[serde(default)]
    new_password: Option<String>,
    #[serde(default)]
    confirm_password: Option<String>,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    mfa_type: Option<String>,
    #[serde(default)]
    credential: Option<String>,
    #[serde(default)]
    user_attributes: HashMap<String, String>,
}

fn save_challenge(cookies: &Cookies, secure: bool, challenge: &StoredChallenge) {
    if let Ok(value) = serde_json::to_string(challenge) {
        session::set_cookie(cookies, secure, CHALLENGE_COOKIE, value, CHALLENGE_MAX_AGE);
    }
}

fn read_challenge(cookies: &Cookies) -> Option<StoredChallenge> {
    serde_json::from_str(cookies.get(CHALLENGE_COOKIE)?.value()).ok()
}

fn parse_json_array(value: Option<&String>) -> Vec<String> {
    value
        .and_then(|raw| serde_json::from_str::<Vec<String>>(raw).ok())
        .unwrap_or_default()
}

/// The part of an auth call's output that decides what happens next. Both
/// Cognito operations answer with the same four fields under different types.
struct AuthResponse {
    result: Option<AuthenticationResultType>,
    challenge_name: Option<ChallengeNameType>,
    session: Option<String>,
    parameters: HashMap<String, String>,
}

/// The two outputs are separate generated types that happen to carry the same
/// fields, so the conversion is written once and applied to both.
macro_rules! auth_response_from {
    ($($output:ty),+ $(,)?) => {$(
        impl From<$output> for AuthResponse {
            fn from(output: $output) -> Self {
                Self {
                    result: output.authentication_result,
                    challenge_name: output.challenge_name,
                    session: output.session,
                    parameters: output.challenge_parameters.unwrap_or_default(),
                }
            }
        }
    )+};
}

auth_response_from!(AdminInitiateAuthOutput, AdminRespondToAuthChallengeOutput);

/// `AdminInitiateAuth` with what every sign-in needs on it: the pool, the app
/// client, the user, and the SECRET_HASH for a client that has a secret.
fn initiate(
    state: &AppState,
    flow: AuthFlowType,
    username: &str,
) -> AdminInitiateAuthFluentBuilder {
    let request = state
        .cognito
        .admin_initiate_auth()
        .user_pool_id(&state.config.user_pool_id)
        .client_id(&state.config.client_id)
        .auth_flow(flow)
        .auth_parameters("USERNAME", username);

    match state.secret_hash(username) {
        Some(hash) => request.auth_parameters("SECRET_HASH", hash),
        None => request,
    }
}

/// The account an alias sign-in resolved to, which is the name every later
/// call has to use. The password flows report it as `USER_ID_FOR_SRP`;
/// choice-based authentication, where passkeys live, is not SRP and uses
/// `USERNAME`.
fn resolved_username(parameters: &HashMap<String, String>, typed: &str) -> String {
    ["USER_ID_FOR_SRP", "USERNAME"]
        .iter()
        .find_map(|key| parameters.get(*key).cloned())
        .unwrap_or_else(|| typed.to_string())
}

/// Stores tokens on success, otherwise records the next challenge.
fn handle(
    cookies: &Cookies,
    secure: bool,
    response: AuthResponse,
    fallback_username: &str,
    lang: &str,
) -> ApiResult<AuthOutcome> {
    if let Some(result) = response.result {
        session::save_tokens(cookies, secure, &result);
        return Ok(AuthOutcome::SignedIn);
    }

    let (Some(name), Some(challenge_session)) = (response.challenge_name, response.session) else {
        return Err(ApiError::bad_request(t!(
            "error_login_failed",
            locale = lang
        )));
    };
    let name = name.as_str().to_string();
    if !SUPPORTED.contains(&name.as_str()) {
        return Err(ApiError::bad_request(t!(
            "error_unsupported_challenge",
            locale = lang,
            name = &name
        )));
    }

    let challenge = StoredChallenge {
        username: resolved_username(&response.parameters, fallback_username),
        // Sent as ["userAttributes.email"], but answered as plain names.
        required_attributes: parse_json_array(response.parameters.get("requiredAttributes"))
            .into_iter()
            .map(|item| item.trim_start_matches("userAttributes.").to_string())
            .collect(),
        mfa_options: parse_json_array(response.parameters.get("MFAS_CAN_CHOOSE")),
        destination: response
            .parameters
            .get("CODE_DELIVERY_DESTINATION")
            .cloned(),
        credential_request_options: response
            .parameters
            .get("CREDENTIAL_REQUEST_OPTIONS")
            .cloned(),
        name,
        session: challenge_session,
    };

    save_challenge(cookies, secure, &challenge);
    Ok(AuthOutcome::Challenge {
        challenge: ChallengeView::from(&challenge),
    })
}

pub async fn login(
    State(state): State<AppState>,
    Lang(lang): Lang,
    SecureCookies(secure): SecureCookies,
    cookies: Cookies,
    Json(body): Json<LoginRequest>,
) -> ApiResult<Json<AuthOutcome>> {
    let username = body.username.trim();
    if username.is_empty() {
        return Err(ApiError::bad_request(t!(
            "error_username_required",
            locale = &lang
        )));
    }
    if body.password.is_empty() {
        return Err(ApiError::bad_request(t!(
            "error_password_required",
            locale = &lang
        )));
    }

    let response = initiate(&state, AuthFlowType::AdminUserPasswordAuth, username)
        .auth_parameters("PASSWORD", &body.password)
        .send()
        .await
        .map_err(|error| cognito(error, &lang))?;

    handle(&cookies, secure, response.into(), username, &lang).map(Json)
}

#[derive(Deserialize)]
pub struct PasskeyRequest {
    username: String,
}

/// Starts a sign-in the user answers with a passkey instead of a password.
///
/// Passkeys exist only in choice-based authentication, so this asks for that
/// flow and names the challenge it wants.
pub async fn passkey(
    State(state): State<AppState>,
    Lang(lang): Lang,
    SecureCookies(secure): SecureCookies,
    cookies: Cookies,
    Json(body): Json<PasskeyRequest>,
) -> ApiResult<Json<AuthOutcome>> {
    let username = body.username.trim();
    if username.is_empty() {
        return Err(ApiError::bad_request(t!(
            "error_username_required",
            locale = &lang
        )));
    }

    let response: AuthResponse = initiate(&state, AuthFlowType::UserAuth, username)
        .auth_parameters("PREFERRED_CHALLENGE", WEB_AUTHN)
        .send()
        .await
        .map_err(|error| cognito(error, &lang))?
        .into();

    // A user without a passkey is answered with SELECT_CHALLENGE and the
    // factors they do have, none of which this button can go on with.
    let offered = response
        .challenge_name
        .as_ref()
        .map(ChallengeNameType::as_str);
    if response.result.is_none() && offered != Some(WEB_AUTHN) {
        return Err(ApiError::bad_request(t!(
            "error_passkey_unavailable",
            locale = &lang
        )));
    }

    handle(&cookies, secure, response, username, &lang).map(Json)
}

fn responses_for(
    state: &AppState,
    challenge: &StoredChallenge,
    body: &ChallengeRequest,
    lang: &str,
) -> ApiResult<HashMap<String, String>> {
    let mut responses = HashMap::new();
    responses.insert("USERNAME".to_string(), challenge.username.clone());
    if let Some(hash) = state.secret_hash(&challenge.username) {
        responses.insert("SECRET_HASH".to_string(), hash);
    }

    let code = body.code.as_deref().unwrap_or_default().trim().to_string();
    match challenge.name.as_str() {
        "NEW_PASSWORD_REQUIRED" => {
            let password = body.new_password.as_deref().unwrap_or_default();
            if password.is_empty() {
                return Err(ApiError::bad_request(t!(
                    "error_password_required",
                    locale = lang
                )));
            }
            if Some(password) != body.confirm_password.as_deref() {
                return Err(ApiError::bad_request(t!(
                    "error_password_mismatch",
                    locale = lang
                )));
            }
            responses.insert("NEW_PASSWORD".to_string(), password.to_string());
            for attribute in &challenge.required_attributes {
                if let Some(value) = body.user_attributes.get(attribute) {
                    let value = value.trim();
                    if !value.is_empty() {
                        responses.insert(format!("userAttributes.{attribute}"), value.to_string());
                    }
                }
            }
        }
        "SMS_MFA" => {
            responses.insert("SMS_MFA_CODE".to_string(), code);
        }
        "EMAIL_OTP" => {
            responses.insert("EMAIL_OTP_CODE".to_string(), code);
        }
        "SOFTWARE_TOKEN_MFA" => {
            responses.insert("SOFTWARE_TOKEN_MFA_CODE".to_string(), code);
        }
        WEB_AUTHN => {
            let credential = body.credential.as_deref().unwrap_or_default().trim();
            if credential.is_empty() {
                return Err(ApiError::bad_request(t!(
                    "error_passkey_required",
                    locale = lang
                )));
            }
            responses.insert("CREDENTIAL".to_string(), credential.to_string());
        }
        "SELECT_MFA_TYPE" => {
            responses.insert(
                "ANSWER".to_string(),
                body.mfa_type
                    .as_deref()
                    .unwrap_or_default()
                    .trim()
                    .to_string(),
            );
        }
        other => {
            return Err(ApiError::bad_request(t!(
                "error_unsupported_challenge",
                locale = lang,
                name = other
            )));
        }
    }
    Ok(responses)
}

pub async fn challenge(
    State(state): State<AppState>,
    Lang(lang): Lang,
    SecureCookies(secure): SecureCookies,
    cookies: Cookies,
    Json(body): Json<ChallengeRequest>,
) -> ApiResult<Json<AuthOutcome>> {
    let Some(challenge) = read_challenge(&cookies) else {
        return Err(ApiError::bad_request(t!(
            "error_challenge_expired",
            locale = &lang
        )));
    };
    let responses = responses_for(&state, &challenge, &body, &lang)?;

    let response = state
        .cognito
        .admin_respond_to_auth_challenge()
        .user_pool_id(&state.config.user_pool_id)
        .client_id(&state.config.client_id)
        .challenge_name(ChallengeNameType::from(challenge.name.as_str()))
        .session(&challenge.session)
        .set_challenge_responses(Some(responses))
        .send()
        .await
        .map_err(|error| cognito(error, &lang))?;

    // Clear the old challenge first; the answer may bring a new one.
    session::remove_cookie(&cookies, CHALLENGE_COOKIE);
    handle(
        &cookies,
        secure,
        response.into(),
        &challenge.username,
        &lang,
    )
    .map(Json)
}

pub async fn logout(State(state): State<AppState>, cookies: Cookies) -> StatusCode {
    if let Some(cookie) = cookies.get(ACCESS_COOKIE) {
        // Already expired or revoked is fine; the cookies go regardless.
        let _ = state
            .cognito
            .global_sign_out()
            .access_token(cookie.value())
            .send()
            .await;
    }
    session::clear_tokens(&cookies);
    StatusCode::NO_CONTENT
}

#[cfg(test)]
mod tests {
    use super::*;

    fn challenge_of(name: &str) -> StoredChallenge {
        StoredChallenge {
            name: name.to_string(),
            session: "cognito-session".to_string(),
            username: "alice".to_string(),
            required_attributes: Vec::new(),
            mfa_options: Vec::new(),
            destination: None,
            credential_request_options: Some(r#"{"challenge":"ZXhhbXBsZQ"}"#.to_string()),
        }
    }

    /// A cookie over 4 KB is dropped silently, which would look like a sign-in
    /// that forgets itself.
    #[test]
    fn the_stored_challenge_leaves_the_passkey_options_out() {
        let challenge = challenge_of(WEB_AUTHN);
        let stored = serde_json::to_string(&challenge).expect("a challenge serializes");

        assert!(!stored.contains("credential_request_options"), "{stored}");
        assert!(stored.contains("cognito-session"), "{stored}");
        assert!(
            ChallengeView::from(&challenge)
                .credential_request_options
                .is_some()
        );
    }

    #[tokio::test]
    async fn a_passkey_answers_with_the_credential_it_signed() {
        let state = AppState::for_tests().await;
        let challenge = challenge_of(WEB_AUTHN);
        let body = ChallengeRequest {
            credential: Some(r#"{"id":"ZXhhbXBsZQ"}"#.to_string()),
            ..ChallengeRequest::default()
        };

        let responses = responses_for(&state, &challenge, &body, "en").expect("valid");
        assert_eq!(
            responses.get("CREDENTIAL").map(String::as_str),
            Some(r#"{"id":"ZXhhbXBsZQ"}"#)
        );
        assert_eq!(responses.get("USERNAME").map(String::as_str), Some("alice"));
    }

    fn parameters(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    /// A challenge response naming an alias is refused, so whichever parameter
    /// carries the resolved username has to win over what was typed.
    #[test]
    fn the_account_cognito_resolved_wins_over_the_alias_typed() {
        assert_eq!(
            resolved_username(
                &parameters(&[("USER_ID_FOR_SRP", "alice")]),
                "a@example.com"
            ),
            "alice"
        );
        assert_eq!(
            resolved_username(&parameters(&[("USERNAME", "alice")]), "a@example.com"),
            "alice"
        );
        // Both: SRP's own parameter is the documented one.
        assert_eq!(
            resolved_username(
                &parameters(&[("USER_ID_FOR_SRP", "alice"), ("USERNAME", "a@example.com")]),
                "a@example.com"
            ),
            "alice"
        );
        assert_eq!(
            resolved_username(&parameters(&[]), "a@example.com"),
            "a@example.com"
        );
    }

    /// Cognito answers a bare parameter error; saying so here keeps the
    /// wording specific.
    #[tokio::test]
    async fn a_passkey_step_without_a_credential_is_refused() {
        let state = AppState::for_tests().await;
        let challenge = challenge_of(WEB_AUTHN);

        assert!(responses_for(&state, &challenge, &ChallengeRequest::default(), "en").is_err());
    }
}
