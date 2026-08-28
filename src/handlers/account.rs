use aws_sdk_cognitoidentityprovider::types::VerifySoftwareTokenResponseType;
use axum::Json;
use axum::extract::State;
use rust_i18n::t;
use serde::Deserialize;
use serde_json::Value;

use crate::attributes::{self, Patch};
use crate::error::{ApiError, ApiResult, cognito};
use crate::extract::Lang;
use crate::mfa::{self, TotpSetup};
use crate::session::Session;
use crate::state::AppState;
use crate::users::{self, MyProfile};

use super::message;

pub async fn profile(
    State(state): State<AppState>,
    Lang(lang): Lang,
    session: Session,
) -> ApiResult<Json<MyProfile>> {
    users::profile(&state, &session, &lang).await.map(Json)
}

#[derive(Deserialize)]
pub struct PatchRequest {
    #[serde(default)]
    attributes: Patch,
}

pub async fn update(
    State(state): State<AppState>,
    Lang(lang): Lang,
    session: Session,
    Json(body): Json<PatchRequest>,
) -> ApiResult<Json<Value>> {
    let pool = state.schema.get(&state, &lang).await?;
    let profile = users::profile(&state, &session, &lang).await?;
    let changes = attributes::diff(
        &body.attributes,
        &pool.self_editable(),
        &profile.attributes,
        &lang,
    )?;

    if changes.is_empty() {
        return Ok(message(t!("msg_no_changes", locale = &lang)));
    }

    if !changes.to_delete.is_empty() {
        state
            .cognito
            .delete_user_attributes()
            .access_token(&session.access_token)
            .set_user_attribute_names(Some(changes.to_delete))
            .send()
            .await
            .map_err(|error| cognito(error, &lang))?;
    }

    if changes.attributes.is_empty() {
        return Ok(message(t!("msg_profile_updated", locale = &lang)));
    }

    let response = state
        .cognito
        .update_user_attributes()
        .access_token(&session.access_token)
        .set_user_attributes(Some(changes.attributes))
        .send()
        .await
        .map_err(|error| cognito(error, &lang))?;

    // Changing email or phone number requires the user to confirm a code.
    let pending: Vec<&str> = response
        .code_delivery_details_list()
        .iter()
        .filter_map(|detail| detail.attribute_name())
        .collect();

    Ok(message(if pending.is_empty() {
        t!("msg_profile_updated", locale = &lang)
    } else {
        t!(
            "msg_profile_updated_pending",
            locale = &lang,
            attributes = &pending.join(" / ")
        )
    }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PasswordRequest {
    current_password: String,
    new_password: String,
    confirm_password: String,
}

pub async fn change_password(
    State(state): State<AppState>,
    Lang(lang): Lang,
    session: Session,
    Json(body): Json<PasswordRequest>,
) -> ApiResult<Json<Value>> {
    if body.current_password.is_empty() || body.new_password.is_empty() {
        return Err(ApiError::bad_request(t!(
            "error_password_required",
            locale = &lang
        )));
    }
    if body.new_password != body.confirm_password {
        return Err(ApiError::bad_request(t!(
            "error_password_mismatch",
            locale = &lang
        )));
    }

    state
        .cognito
        .change_password()
        .access_token(&session.access_token)
        .previous_password(&body.current_password)
        .proposed_password(&body.new_password)
        .send()
        .await
        .map_err(|error| cognito(error, &lang))?;

    Ok(message(t!("msg_password_changed", locale = &lang)))
}

#[derive(Deserialize)]
pub struct SendCodeRequest {
    attribute: String,
}

pub async fn send_code(
    State(state): State<AppState>,
    Lang(lang): Lang,
    session: Session,
    Json(body): Json<SendCodeRequest>,
) -> ApiResult<Json<Value>> {
    let attribute = body.attribute.trim();
    if attribute.is_empty() {
        return Err(ApiError::bad_request(t!(
            "error_verification_input",
            locale = &lang
        )));
    }

    let response = state
        .cognito
        .get_user_attribute_verification_code()
        .access_token(&session.access_token)
        .attribute_name(attribute)
        .send()
        .await
        .map_err(|error| cognito(error, &lang))?;

    Ok(message(
        match response
            .code_delivery_details()
            .and_then(|d| d.destination())
        {
            Some(destination) => t!("msg_code_sent", locale = &lang, destination = destination),
            None => t!("msg_code_sent_generic", locale = &lang),
        },
    ))
}

#[derive(Deserialize)]
pub struct VerifyRequest {
    attribute: String,
    code: String,
}

pub async fn verify(
    State(state): State<AppState>,
    Lang(lang): Lang,
    session: Session,
    Json(body): Json<VerifyRequest>,
) -> ApiResult<Json<Value>> {
    let (attribute, code) = (body.attribute.trim(), body.code.trim());
    if attribute.is_empty() || code.is_empty() {
        return Err(ApiError::bad_request(t!(
            "error_verification_input",
            locale = &lang
        )));
    }

    state
        .cognito
        .verify_user_attribute()
        .access_token(&session.access_token)
        .attribute_name(attribute)
        .code(code)
        .send()
        .await
        .map_err(|error| cognito(error, &lang))?;

    Ok(message(t!("msg_verified", locale = &lang)))
}

/// Turns the caller's own second factors on or off.
///
/// Written with the access token, so this screen structurally cannot change
/// anybody else's factors.
pub async fn set_mfa(
    State(state): State<AppState>,
    Lang(lang): Lang,
    session: Session,
    Json(preference): Json<mfa::Preference>,
) -> ApiResult<Json<Value>> {
    let Some(settings) = preference.settings(&lang)? else {
        return Ok(message(t!("msg_no_changes", locale = &lang)));
    };

    state
        .cognito
        .set_user_mfa_preference()
        .access_token(&session.access_token)
        .set_sms_mfa_settings(settings.sms)
        .set_software_token_mfa_settings(settings.software_token)
        .set_email_mfa_settings(settings.email)
        .send()
        .await
        .map_err(|error| cognito(error, &lang))?;

    Ok(message(t!("msg_mfa_updated", locale = &lang)))
}

/// Hands out a fresh authenticator secret, as a QR code to scan and as the
/// characters to type where a camera is not an option.
///
/// The factor stays off until a code from that secret comes back to
/// `verify_totp`, so a setup that is started and abandoned changes nothing.
pub async fn start_totp(
    State(state): State<AppState>,
    Lang(lang): Lang,
    session: Session,
) -> ApiResult<Json<TotpSetup>> {
    let response = state
        .cognito
        .associate_software_token()
        .access_token(&session.access_token)
        .send()
        .await
        .map_err(|error| cognito(error, &lang))?;

    let Some(secret) = response.secret_code() else {
        tracing::error!("cognito associated a software token without a secret");
        return Err(ApiError::internal(&lang));
    };

    // The label an authenticator app shows: the pool this account belongs to,
    // and the address the user knows themselves by.
    let pool = state.schema.get(&state, &lang).await?;
    let issuer = pool.name.unwrap_or(pool.id);
    let account = session.email.as_deref().unwrap_or(&session.username);

    Ok(Json(TotpSetup::new(secret, &issuer, account)))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TotpRequest {
    code: String,
    #[serde(default)]
    device_name: Option<String>,
}

/// Confirms the authenticator app really holds the secret, then turns the
/// factor on and prefers it — an app that is enrolled but never asked for
/// would be a setup that silently did nothing.
pub async fn verify_totp(
    State(state): State<AppState>,
    Lang(lang): Lang,
    session: Session,
    Json(body): Json<TotpRequest>,
) -> ApiResult<Json<Value>> {
    let code = body.code.trim();
    if code.is_empty() {
        return Err(ApiError::bad_request(t!(
            "error_code_required",
            locale = &lang
        )));
    }

    let response = state
        .cognito
        .verify_software_token()
        .access_token(&session.access_token)
        .user_code(code)
        .set_friendly_device_name(
            body.device_name
                .as_deref()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_string),
        )
        .send()
        .await
        .map_err(|error| cognito(error, &lang))?;

    // A wrong code is usually an error of its own; this covers the pool that
    // answers with a status instead.
    if response.status() != Some(&VerifySoftwareTokenResponseType::Success) {
        return Err(ApiError::bad_request(t!(
            "error_code_mismatch",
            locale = &lang
        )));
    }

    state
        .cognito
        .set_user_mfa_preference()
        .access_token(&session.access_token)
        .software_token_mfa_settings(mfa::software_token(true))
        .send()
        .await
        .map_err(|error| cognito(error, &lang))?;

    Ok(message(t!("msg_totp_registered", locale = &lang)))
}
