use aws_sdk_cognitoidentityprovider::types::{AuthFlowType, AuthenticationResultType};
use base64::Engine;
use serde_json::Value;
use tower_cookies::cookie::{time::Duration, SameSite};
use tower_cookies::{Cookie, Cookies};

use crate::state::AppState;

pub const ID_COOKIE: &str = "cum_id";
pub const ACCESS_COOKIE: &str = "cum_at";
pub const REFRESH_COOKIE: &str = "cum_rt";
pub const CHALLENGE_COOKIE: &str = "cum_challenge";

const REFRESH_MAX_AGE_DAYS: i64 = 30;

#[derive(Debug, Clone)]
pub struct Session {
    /// Cognito username, usable as the Username of admin APIs.
    pub username: String,
    pub sub: String,
    pub email: Option<String>,
    pub groups: Vec<String>,
    pub is_admin: bool,
    pub access_token: String,
}

impl Session {
    pub fn is_self(&self, username: &str) -> bool {
        self.username == username || self.sub == username
    }
}

pub fn set_cookie(cookies: &Cookies, secure: bool, name: &str, value: String, max_age: Duration) {
    let mut cookie = Cookie::new(name.to_string(), value);
    cookie.set_http_only(true);
    cookie.set_same_site(SameSite::Lax);
    cookie.set_secure(secure);
    cookie.set_path("/");
    cookie.set_max_age(max_age);
    cookies.add(cookie);
}

pub fn remove_cookie(cookies: &Cookies, name: &str) {
    cookies.remove(Cookie::build((name.to_string(), "")).path("/").build());
}

pub fn save_tokens(cookies: &Cookies, secure: bool, result: &AuthenticationResultType) {
    let expires_in = Duration::seconds(result.expires_in().max(60) as i64);
    if let Some(token) = result.id_token() {
        set_cookie(cookies, secure, ID_COOKIE, token.to_string(), expires_in);
    }
    if let Some(token) = result.access_token() {
        set_cookie(cookies, secure, ACCESS_COOKIE, token.to_string(), expires_in);
    }
    if let Some(token) = result.refresh_token() {
        set_cookie(
            cookies,
            secure,
            REFRESH_COOKIE,
            token.to_string(),
            Duration::days(REFRESH_MAX_AGE_DAYS),
        );
    }
    remove_cookie(cookies, CHALLENGE_COOKIE);
}

pub fn clear_tokens(cookies: &Cookies) {
    for name in [ID_COOKIE, ACCESS_COOKIE, REFRESH_COOKIE, CHALLENGE_COOKIE] {
        remove_cookie(cookies, name);
    }
}

/// Reads a JWT payload without verifying it, only to recover a username.
fn decode_unverified(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}

async fn from_tokens(state: &AppState, id_token: &str, access_token: &str) -> Option<Session> {
    let claims = match state.jwks.verify_id_token(id_token).await {
        Ok(claims) => claims,
        Err(reason) => {
            tracing::warn!(%reason, "ID token rejected");
            return None;
        }
    };
    let is_admin = claims.groups.contains(&state.config.admin_group);
    Some(Session {
        username: claims.username.unwrap_or_else(|| claims.sub.clone()),
        sub: claims.sub,
        email: claims.email,
        groups: claims.groups,
        is_admin,
        access_token: access_token.to_string(),
    })
}

/// Current session, refreshing the tokens in place when the ID token expired.
pub async fn load(state: &AppState, cookies: &Cookies, secure: bool) -> Option<Session> {
    let id_token = cookies.get(ID_COOKIE).map(|c| c.value().to_string());
    let access_token = cookies.get(ACCESS_COOKIE).map(|c| c.value().to_string());

    tracing::debug!(
        id = id_token.as_ref().map(String::len),
        access = access_token.as_ref().map(String::len),
        refresh = cookies.get(REFRESH_COOKIE).map(|c| c.value().len()),
        "session cookies received"
    );

    if let (Some(id), Some(access)) = (&id_token, &access_token)
        && let Some(session) = from_tokens(state, id, access).await
    {
        return Some(session);
    }

    let refresh_token = cookies.get(REFRESH_COOKIE)?.value().to_string();
    let Some(result) = refresh(state, &refresh_token, id_token.as_deref()).await else {
        clear_tokens(cookies);
        return None;
    };
    save_tokens(cookies, secure, &result);
    from_tokens(state, result.id_token()?, result.access_token()?).await
}

/// Usernames to try when deriving SECRET_HASH, in order.
///
/// Depending on the pool, Cognito derives the hash from the username or from
/// the sub, and only one of them is accepted. Without a client secret there is
/// no hash to derive, so a single attempt is all there is to make.
fn secret_hash_candidates(state: &AppState, previous_id_token: Option<&str>) -> Vec<String> {
    if state.config.client_secret.is_none() {
        return vec![String::new()];
    }

    let mut candidates: Vec<String> = Vec::new();
    if let Some(claims) = previous_id_token.and_then(decode_unverified) {
        for key in ["cognito:username", "sub"] {
            if let Some(name) = claims.get(key).and_then(Value::as_str)
                && !candidates.iter().any(|seen| seen == name)
            {
                candidates.push(name.to_string());
            }
        }
    }
    if candidates.is_empty() {
        candidates.push(String::new());
    }
    candidates
}

/// Exchanges the refresh token for new tokens.
async fn refresh(
    state: &AppState,
    refresh_token: &str,
    previous_id_token: Option<&str>,
) -> Option<AuthenticationResultType> {
    for username in secret_hash_candidates(state, previous_id_token) {
        let mut request = state
            .cognito
            .admin_initiate_auth()
            .user_pool_id(&state.config.user_pool_id)
            .client_id(&state.config.client_id)
            .auth_flow(AuthFlowType::RefreshTokenAuth)
            .auth_parameters("REFRESH_TOKEN", refresh_token);
        if let Some(hash) = state.secret_hash(&username) {
            request = request.auth_parameters("SECRET_HASH", hash);
        }

        match request.send().await {
            Ok(response) => {
                if let Some(result) = response.authentication_result {
                    // A refreshed response carries no refresh token; keep the current one.
                    let mut builder = AuthenticationResultType::builder()
                        .expires_in(result.expires_in())
                        .refresh_token(refresh_token);
                    if let Some(token) = result.id_token() {
                        builder = builder.id_token(token);
                    }
                    if let Some(token) = result.access_token() {
                        builder = builder.access_token(token);
                    }
                    return Some(builder.build());
                }
            }
            Err(error) => {
                tracing::debug!(?error, "refresh attempt failed");
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use axum::routing::get;
    use axum::{Router, response::IntoResponse};
    use tower::ServiceExt;
    use tower_cookies::CookieManagerLayer;

    async fn set_cookie_header(secure: bool) -> Vec<String> {
        let router = Router::new()
            .route(
                "/",
                get(move |cookies: Cookies| async move {
                    let result = AuthenticationResultType::builder()
                        .expires_in(3600)
                        .id_token("id-token")
                        .access_token("access-token")
                        .refresh_token("refresh-token")
                        .build();
                    save_tokens(&cookies, secure, &result);
                    StatusCode::OK.into_response()
                }),
            )
            .layer(CookieManagerLayer::new());

        let response = router
            .oneshot(Request::builder().uri("/").body(Body::empty()).expect("request"))
            .await
            .expect("response");

        response
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok().map(str::to_string))
            .collect()
    }

    #[tokio::test]
    async fn sign_in_sets_all_three_token_cookies() {
        let cookies = set_cookie_header(true).await;
        for name in [ID_COOKIE, ACCESS_COOKIE, REFRESH_COOKIE] {
            assert!(
                cookies.iter().any(|c| c.starts_with(&format!("{name}="))),
                "{name} should be set, got {cookies:?}"
            );
        }
        for cookie in &cookies {
            assert!(cookie.contains("HttpOnly"), "{cookie}");
            assert!(cookie.contains("SameSite=Lax"), "{cookie}");
            assert!(cookie.contains("Path=/"), "{cookie}");
        }
    }

    /// Over plain HTTP a Secure cookie is dropped by the client, which shows up
    /// as a sign-in that succeeds and is then immediately unauthenticated.
    #[tokio::test]
    async fn the_secure_attribute_follows_the_setting() {
        assert!(set_cookie_header(true).await.iter().all(|c| c.contains("Secure")));
        assert!(set_cookie_header(false).await.iter().all(|c| !c.contains("Secure")));
    }
}
