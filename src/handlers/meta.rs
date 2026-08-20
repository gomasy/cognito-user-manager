use axum::Json;
use axum::extract::State;
use serde::Serialize;

use crate::error::ApiResult;
use crate::extract::Lang;
use crate::schema::{self, AttributeField};
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
    /// The subset each screen may edit, resolved server-side so a client
    /// cannot widen it by asking for a different list.
    self_editable: Vec<AttributeField>,
    admin_visible: Vec<AttributeField>,
    editable: Vec<AttributeField>,
    groups: Vec<String>,
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
        self_editable: pool.self_editable(),
        admin_visible: if admin {
            pool.admin_visible()
        } else {
            Vec::new()
        },
        editable: if admin { pool.editable() } else { Vec::new() },
        // Only admins assign groups, and only they may read the list.
        groups: if admin {
            schema::list_group_names(&state, &lang).await?
        } else {
            Vec::new()
        },
        search_fields: if admin { &users::SEARCH_FIELDS } else { &[] },
    }))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicInfo {
    pool_name: Option<String>,
    version: &'static str,
}

/// Pool name for the sign-in screen, before there is a session.
pub async fn public_info(State(state): State<AppState>, Lang(lang): Lang) -> Json<PublicInfo> {
    // Sign-in must stay usable even if the pool cannot be described.
    let pool_name = state
        .schema
        .get(&state, &lang)
        .await
        .ok()
        .and_then(|pool| pool.name);

    Json(PublicInfo {
        pool_name,
        version: crate::VERSION,
    })
}
