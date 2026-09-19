use axum::Json;
use axum::extract::State;
use serde::Serialize;

use crate::error::ApiResult;
use crate::extract::Lang;
use crate::groups;
use crate::schema::AttributeField;
use crate::session::Session;
use crate::state::AppState;
use crate::users;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    username: String,
    email: Option<String>,
    groups: Vec<String>,
    is_admin: bool,
}

/// Who the caller is. The frontend calls this on load to pick its first screen.
pub async fn session(session: Session) -> Json<SessionInfo> {
    Json(SessionInfo {
        username: session.username,
        email: session.email,
        groups: session.groups,
        is_admin: session.is_admin,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PoolResponse {
    id: String,
    name: Option<String>,
    username_is_email: bool,
    /// `OFF`, `ON` or `OPTIONAL`, so the MFA forms can say when the pool
    /// itself has second factors switched off.
    mfa_configuration: String,
    /// Whether the account screen offers to register a passkey.
    passkey_sign_in: bool,
    /// The subset each screen may edit, resolved server-side so a client
    /// cannot widen it by asking for a different list.
    self_editable: Vec<AttributeField>,
    admin_visible: Vec<AttributeField>,
    editable: Vec<AttributeField>,
    groups: Vec<String>,
    /// The group that grants access to the admin console, so the group screens
    /// can mark it and leave out the delete button the server would refuse.
    admin_group: Option<String>,
    /// Attributes the user search may filter on, served rather than mirrored
    /// in the frontend so the two lists cannot drift apart.
    search_fields: &'static [&'static str],
}

/// Pool schema and group list, driving every attribute form.
pub async fn pool(
    State(state): State<AppState>,
    Lang(lang): Lang,
    session: Session,
) -> ApiResult<Json<PoolResponse>> {
    let pool = state.schema.get(&state, &lang).await?;
    let admin = session.is_admin;

    Ok(Json(PoolResponse {
        id: pool.id.clone(),
        name: pool.name.clone(),
        username_is_email: pool.username_is_email,
        mfa_configuration: pool.mfa_configuration.clone(),
        passkey_sign_in: pool.passkey_sign_in,
        self_editable: pool.self_editable(),
        admin_visible: if admin {
            pool.admin_visible()
        } else {
            Vec::new()
        },
        editable: if admin { pool.editable() } else { Vec::new() },
        // Only admins assign groups, and only they may read the list.
        groups: if admin {
            groups::names(&state, &lang).await?
        } else {
            Vec::new()
        },
        admin_group: admin.then(|| state.config.admin_group.clone()),
        search_fields: if admin { &users::SEARCH_FIELDS } else { &[] },
    }))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicInfo {
    pool_name: Option<String>,
    /// Whether to offer the passkey button. Also false when the pool cannot be
    /// described, which leaves no button rather than one that cannot work.
    passkey_sign_in: bool,
    version: &'static str,
}

/// Pool name for the sign-in screen, before there is a session.
pub async fn public_info(State(state): State<AppState>, Lang(lang): Lang) -> Json<PublicInfo> {
    // Sign-in must stay usable even if the pool cannot be described.
    let pool = state.schema.get(&state, &lang).await.ok();

    Json(PublicInfo {
        pool_name: pool.as_ref().and_then(|pool| pool.name.clone()),
        passkey_sign_in: pool.is_some_and(|pool| pool.passkey_sign_in),
        version: crate::VERSION,
    })
}
