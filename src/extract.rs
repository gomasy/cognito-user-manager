//! Request guards. Putting the session checks in extractors means a handler
//! cannot be written that forgets one.

use std::convert::Infallible;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode, Uri};
use tower_cookies::Cookies;

use crate::error::ApiError;
use crate::locale;
use crate::session::{self, Session};
use crate::state::AppState;

/// Locale advertised by the frontend, already resolved to one we ship.
pub struct Lang(pub String);

/// Shared by the extractor and the guards below, which need the language to
/// phrase their rejection before any extractor has run.
fn lang_of(parts: &Parts) -> String {
    locale::or_default(
        parts
            .headers
            .get("x-app-lang")
            .and_then(|value| value.to_str().ok()),
    )
}

impl<S: Send + Sync> FromRequestParts<S> for Lang {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(Self(lang_of(parts)))
    }
}

/// Whether session cookies should carry the Secure attribute.
///
/// A Secure cookie is silently dropped by the client over plain HTTP, which
/// looks like a sign-in that succeeds and is unauthenticated on the very next
/// request. Deriving it from the request scheme keeps local HTTP working while
/// still marking cookies Secure behind a TLS-terminating proxy. SECURE_COOKIES
/// overrides the guess when a proxy does not forward the scheme.
pub struct SecureCookies(pub bool);

pub fn secure_for(parts: &Parts, state: &AppState) -> bool {
    resolve_secure(state.config.secure_cookies, &parts.headers, &parts.uri)
}

fn resolve_secure(configured: Option<bool>, headers: &HeaderMap, uri: &Uri) -> bool {
    configured.unwrap_or_else(|| request_is_https(headers, uri))
}

fn request_is_https(headers: &HeaderMap, uri: &Uri) -> bool {
    if let Some(proto) = headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
    {
        // A chain of proxies appends, so the client-facing scheme is first.
        return proto
            .split(',')
            .next()
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("https"));
    }
    if let Some(forwarded) = headers.get("forwarded").and_then(|v| v.to_str().ok()) {
        return forwarded.to_ascii_lowercase().contains("proto=https");
    }
    uri.scheme_str() == Some("https")
}

impl FromRequestParts<AppState> for SecureCookies {
    type Rejection = Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        Ok(Self(secure_for(parts, state)))
    }
}

async fn cookies(parts: &mut Parts, state: &AppState) -> Result<Cookies, ApiError> {
    Cookies::from_request_parts(parts, state)
        .await
        .map_err(|_| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "cookie layer missing"))
}

impl FromRequestParts<AppState> for Session {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let lang = lang_of(parts);
        let secure = secure_for(parts, state);
        let jar = cookies(parts, state).await?;
        session::load(state, &jar, secure)
            .await
            .ok_or_else(|| ApiError::unauthorized(&lang))
    }
}

/// A session that also belongs to the admin group.
pub struct AdminSession(pub Session);

impl FromRequestParts<AppState> for AdminSession {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let lang = lang_of(parts);
        let session = Session::from_request_parts(parts, state).await?;
        if session.is_admin {
            Ok(Self(session))
        } else {
            Err(ApiError::forbidden(&lang))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(*name, HeaderValue::from_static(value));
        }
        map
    }

    #[test]
    fn plain_http_is_not_treated_as_secure() {
        // The local development case: a Secure cookie here would be dropped by
        // the client and the session would appear to vanish after sign-in.
        assert!(!request_is_https(
            &HeaderMap::new(),
            &Uri::from_static("/api/session")
        ));
    }

    #[test]
    fn a_terminating_proxy_marks_the_request_secure() {
        assert!(request_is_https(
            &headers(&[("x-forwarded-proto", "https")]),
            &Uri::from_static("/api/session")
        ));
        assert!(request_is_https(
            &headers(&[("x-forwarded-proto", "https, http")]),
            &Uri::from_static("/")
        ));
        assert!(!request_is_https(
            &headers(&[("x-forwarded-proto", "http")]),
            &Uri::from_static("/")
        ));
        assert!(request_is_https(
            &headers(&[("forwarded", "for=203.0.113.1;proto=https")]),
            &Uri::from_static("/")
        ));
    }

    #[test]
    fn the_setting_overrides_the_guess() {
        let plain = HeaderMap::new();
        let uri = Uri::from_static("/api/session");
        assert!(resolve_secure(Some(true), &plain, &uri));
        assert!(!resolve_secure(
            Some(false),
            &headers(&[("x-forwarded-proto", "https")]),
            &uri
        ));
        assert!(!resolve_secure(None, &plain, &uri));
    }

    #[test]
    fn an_absolute_https_uri_counts() {
        assert!(request_is_https(
            &HeaderMap::new(),
            &Uri::from_static("https://example.com/api/session")
        ));
    }
}
