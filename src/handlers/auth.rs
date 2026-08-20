use std::collections::HashMap;

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
const SUPPORTED: [&str; 5] = [
    "NEW_PASSWORD_REQUIRED",
    "SMS_MFA",
    "EMAIL_OTP",
    "SOFTWARE_TOKEN_MFA",
    "SELECT_MFA_TYPE",
];

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
}

/// What the browser is told about a pending challenge.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChallengeView {
    name: String,
    required_attributes: Vec<String>,
    mfa_options: Vec<String>,
    destination: Option<String>,
}

impl From<&StoredChallenge> for ChallengeView {
    fn from(challenge: &StoredChallenge) -> Self {
        Self {
            name: challenge.name.clone(),
            required_attributes: challenge.required_attributes.clone(),
            mfa_options: challenge.mfa_options.clone(),
            destination: challenge.destination.clone(),
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

#[derive(Deserialize)]
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

struct AuthResponse {
    result: Option<AuthenticationResultType>,
    challenge_name: Option<ChallengeNameType>,
    session: Option<String>,
    parameters: HashMap<String, String>,
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
        // After an alias sign-in, Cognito returns the real username to use next.
        username: response
            .parameters
            .get("USER_ID_FOR_SRP")
            .cloned()
            .unwrap_or_else(|| fallback_username.to_string()),
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

    let mut request = state
        .cognito
        .admin_initiate_auth()
        .user_pool_id(&state.config.user_pool_id)
        .client_id(&state.config.client_id)
        .auth_flow(AuthFlowType::AdminUserPasswordAuth)
        .auth_parameters("USERNAME", username)
        .auth_parameters("PASSWORD", &body.password);
    if let Some(hash) = state.secret_hash(username) {
        request = request.auth_parameters("SECRET_HASH", hash);
    }

    let response = request
        .send()
        .await
        .map_err(|error| cognito(error, &lang))?;
    handle(
        &cookies,
        secure,
        AuthResponse {
            result: response.authentication_result,
            challenge_name: response.challenge_name,
            session: response.session,
            parameters: response.challenge_parameters.unwrap_or_default(),
        },
        username,
        &lang,
    )
    .map(Json)
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
        AuthResponse {
            result: response.authentication_result,
            challenge_name: response.challenge_name,
            session: response.session,
            parameters: response.challenge_parameters.unwrap_or_default(),
        },
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
