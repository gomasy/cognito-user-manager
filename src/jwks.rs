use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use serde::Deserialize;

use crate::config::Config;

const CACHE_TTL: Duration = Duration::from_secs(60 * 60);

/// A kid the cached set does not hold means the pool rotated its keys, so the
/// set is fetched again rather than waiting out the TTL. Bounding how often
/// that can happen keeps a stream of tokens carrying a made-up kid from
/// turning into one outbound request each.
const REFETCH_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Deserialize)]
struct JwkSet {
    keys: Vec<Jwk>,
}

#[derive(Deserialize)]
struct Jwk {
    kid: String,
    n: String,
    e: String,
}

/// Claims this app reads from a Cognito ID token.
#[derive(Debug, Deserialize)]
pub struct IdClaims {
    pub sub: String,
    #[serde(rename = "cognito:username")]
    pub username: Option<String>,
    #[serde(rename = "cognito:groups", default)]
    pub groups: Vec<String>,
    pub email: Option<String>,
    pub token_use: String,
}

/// Cached signing keys of the user pool.
///
/// Replaces aws-jwt-verify: fetches the pool JWKS, caches it, and validates
/// RS256 signature, expiry, issuer, audience and token_use.
pub struct Jwks {
    url: String,
    issuers: [String; 2],
    client_id: String,
    cache: RwLock<Option<(HashMap<String, DecodingKey>, Instant)>>,
}

impl Jwks {
    pub fn new(config: &Config) -> Self {
        let (region, pool) = (&config.region, &config.user_pool_id);
        let canonical = format!("https://cognito-idp.{region}.amazonaws.com/{pool}");

        Self {
            url: format!("{canonical}/.well-known/jwks.json"),
            // Depending on how a pool is set up, Cognito stamps tokens with
            // either host, and the discovery document advertises only the
            // first even when the tokens carry the second. Both are AWS
            // controlled and name this pool, so accepting both does not widen
            // who is trusted; they also serve the same JWKS.
            issuers: [
                canonical,
                format!("https://issuer-cognito-idp.{region}.amazonaws.com/{pool}"),
            ],
            client_id: config.client_id.clone(),
            cache: RwLock::new(None),
        }
    }

    /// The cached key for `kid`, and whether fetching is worth it when there
    /// is none.
    fn cached(&self, kid: &str) -> (Option<DecodingKey>, bool) {
        let Ok(guard) = self.cache.read() else {
            return (None, true);
        };
        let Some((keys, fetched_at)) = guard.as_ref() else {
            return (None, true);
        };
        let age = fetched_at.elapsed();
        if age >= CACHE_TTL {
            return (None, true);
        }
        match keys.get(kid) {
            Some(key) => (Some(key.clone()), false),
            None => (None, age >= REFETCH_INTERVAL),
        }
    }

    async fn key(&self, kid: &str) -> Result<DecodingKey, String> {
        let (cached, worth_fetching) = self.cached(kid);
        if let Some(key) = cached {
            return Ok(key);
        }
        if !worth_fetching {
            return Err(format!("no signing key for kid {kid}"));
        }

        // ureq is blocking, and this runs rarely: once per TTL, or once per
        // REFETCH_INTERVAL for as long as a kid stays unknown.
        let url = self.url.clone();
        let set: JwkSet = tokio::task::spawn_blocking(move || {
            ureq::get(&url)
                .call()
                .map_err(|error| format!("JWKS fetch failed: {error}"))?
                .body_mut()
                .read_json::<JwkSet>()
                .map_err(|error| format!("JWKS parse failed: {error}"))
        })
        .await
        .map_err(|error| format!("JWKS fetch failed: {error}"))??;

        let mut keys = HashMap::new();
        for jwk in set.keys {
            if let Ok(key) = DecodingKey::from_rsa_components(&jwk.n, &jwk.e) {
                keys.insert(jwk.kid, key);
            }
        }

        let found = keys.get(kid).cloned();
        if let Ok(mut guard) = self.cache.write() {
            *guard = Some((keys, Instant::now()));
        }
        found.ok_or_else(|| format!("no signing key for kid {kid}"))
    }

    pub async fn verify_id_token(&self, token: &str) -> Result<IdClaims, String> {
        let header = decode_header(token).map_err(|error| format!("malformed token: {error}"))?;
        let kid = header.kid.ok_or("token carries no kid")?;
        let key = self.key(&kid).await?;
        verify_with(&key, &validation(&self.issuers, &self.client_id), token).map_err(|error| {
            // The claims are unverified here, which is exactly why they are only
            // ever logged: without them a rejection gives nothing to act on.
            format!(
                "{error}; {} expected iss={:?} aud={}",
                describe(token),
                self.issuers,
                self.client_id
            )
        })
    }
}

/// Unverified summary of a token, for diagnosing a rejection.
fn describe(token: &str) -> String {
    let Some(claims) = token
        .split('.')
        .nth(1)
        .and_then(|payload| {
            base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, payload).ok()
        })
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
    else {
        return "payload unreadable".to_string();
    };

    let field = |name: &str| {
        claims
            .get(name)
            .map(ToString::to_string)
            .unwrap_or_else(|| "-".to_string())
    };
    format!(
        "token iss={} aud={} token_use={} exp={}",
        field("iss"),
        field("aud"),
        field("token_use"),
        field("exp")
    )
}

/// Signature algorithm, expiry, issuer and audience, all as Cognito issues them.
fn validation(issuers: &[String], client_id: &str) -> Validation {
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_issuer(issuers);
    validation.set_audience(&[client_id]);
    validation
}

fn verify_with(
    key: &DecodingKey,
    validation: &Validation,
    token: &str,
) -> Result<IdClaims, String> {
    let data = decode::<IdClaims>(token, key, validation)
        .map_err(|error| format!("token verification failed: {error}"))?;

    // An access token carries the same signature and issuer, so the use must be
    // checked explicitly before its claims are trusted as an identity.
    if data.claims.token_use != "id" {
        return Err("not an ID token".to_string());
    }
    Ok(data.claims)
}

/// Read-only smoke test against the pool configured in .env.
/// Opt in with `cargo test -- --ignored --nocapture`.
#[cfg(test)]
mod live_tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires live AWS credentials"]
    async fn the_pool_jwks_loads_and_parses() {
        let _ = dotenvy::dotenv();
        let config = Config::from_env().expect("config");
        let jwks = Jwks::new(&config);
        println!("issuers={:?}", jwks.issuers);
        println!("url={}", jwks.url);

        let url = jwks.url.clone();
        let set: JwkSet = tokio::task::spawn_blocking(move || {
            ureq::get(&url)
                .call()
                .expect("fetch jwks")
                .body_mut()
                .read_json::<JwkSet>()
                .expect("parse jwks")
        })
        .await
        .expect("join");

        assert!(!set.keys.is_empty(), "the pool must publish signing keys");
        for jwk in &set.keys {
            DecodingKey::from_rsa_components(&jwk.n, &jwk.e)
                .unwrap_or_else(|error| panic!("kid {} did not parse: {error}", jwk.kid));
            println!("kid={} ok", jwk.kid);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{EncodingKey, Header, encode};
    use serde::Serialize;
    use std::time::{SystemTime, UNIX_EPOCH};

    const ISSUER: &str = "https://cognito-idp.ap-northeast-1.amazonaws.com/ap-northeast-1_test";
    const ISSUER_ALT: &str =
        "https://issuer-cognito-idp.ap-northeast-1.amazonaws.com/ap-northeast-1_test";
    const CLIENT_ID: &str = "test-client-id";
    /// Generated for the tests only; it signs nothing that exists in AWS.
    const KEY_PEM: &[u8] = include_bytes!("../tests/fixtures/test_signing_key.pem");
    const KEY_JWK: &str = include_str!("../tests/fixtures/test_signing_key.jwk");

    #[derive(Serialize)]
    struct TestClaims {
        sub: &'static str,
        #[serde(rename = "cognito:username")]
        username: &'static str,
        #[serde(rename = "cognito:groups")]
        groups: Vec<&'static str>,
        email: &'static str,
        token_use: &'static str,
        iss: String,
        aud: String,
        exp: u64,
    }

    fn now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    fn accepted() -> Validation {
        validation(&[ISSUER.to_string(), ISSUER_ALT.to_string()], CLIENT_ID)
    }

    fn decoding_key() -> DecodingKey {
        let mut parts = KEY_JWK.split_whitespace();
        let n = parts.next().expect("modulus");
        let e = parts.next().expect("exponent");
        DecodingKey::from_rsa_components(n, e).expect("public key")
    }

    fn sign(claims: TestClaims) -> String {
        let key = EncodingKey::from_rsa_pem(KEY_PEM).expect("private key");
        encode(&Header::new(Algorithm::RS256), &claims, &key).expect("sign")
    }

    fn claims(token_use: &'static str, issuer: &str, audience: &str, exp: u64) -> TestClaims {
        TestClaims {
            sub: "11111111-2222-3333-4444-555555555555",
            username: "taro",
            groups: vec!["admin"],
            email: "taro@example.com",
            token_use,
            iss: issuer.to_string(),
            aud: audience.to_string(),
            exp,
        }
    }

    /// The whole point of the module: an RS256 signature from the pool's key
    /// must actually verify under the rust_crypto backend.
    #[test]
    fn a_valid_id_token_verifies() {
        let token = sign(claims("id", ISSUER, CLIENT_ID, now() + 3600));
        let verified =
            verify_with(&decoding_key(), &accepted(), &token).expect("token should verify");

        assert_eq!(verified.username.as_deref(), Some("taro"));
        assert_eq!(verified.groups, vec!["admin".to_string()]);
        assert_eq!(verified.email.as_deref(), Some("taro@example.com"));
    }

    #[test]
    fn an_access_token_is_rejected_as_an_identity() {
        let token = sign(claims("access", ISSUER, CLIENT_ID, now() + 3600));
        assert!(verify_with(&decoding_key(), &accepted(), &token).is_err());
    }

    #[test]
    fn a_foreign_issuer_or_audience_is_rejected() {
        let wrong_issuer = sign(claims(
            "id",
            "https://evil.example.com",
            CLIENT_ID,
            now() + 3600,
        ));
        assert!(verify_with(&decoding_key(), &accepted(), &wrong_issuer).is_err());

        // Same shape as the alternate host but a different pool.
        let other_pool = sign(claims(
            "id",
            "https://issuer-cognito-idp.ap-northeast-1.amazonaws.com/ap-northeast-1_other",
            CLIENT_ID,
            now() + 3600,
        ));
        assert!(verify_with(&decoding_key(), &accepted(), &other_pool).is_err());

        let wrong_audience = sign(claims("id", ISSUER, "another-client", now() + 3600));
        assert!(verify_with(&decoding_key(), &accepted(), &wrong_audience).is_err());
    }

    /// Cognito stamps tokens with either host; both must be accepted, which is
    /// the bug this pins down: only the first was, so every signed-in request
    /// came back unauthenticated.
    #[test]
    fn both_issuer_hosts_are_accepted() {
        for issuer in [ISSUER, ISSUER_ALT] {
            let token = sign(claims("id", issuer, CLIENT_ID, now() + 3600));
            assert!(
                verify_with(&decoding_key(), &accepted(), &token).is_ok(),
                "{issuer} should verify"
            );
        }
    }

    #[test]
    fn an_expired_token_is_rejected() {
        let token = sign(claims("id", ISSUER, CLIENT_ID, now() - 3600));
        assert!(verify_with(&decoding_key(), &accepted(), &token).is_err());
    }

    /// jsonwebtoken allows 60s of clock skew by default. Pinning it here means
    /// a change to that default shows up as a failing test rather than as a
    /// silently wider acceptance window.
    #[test]
    fn expiry_tolerates_only_a_minute_of_clock_skew() {
        let inside = sign(claims("id", ISSUER, CLIENT_ID, now() - 30));
        assert!(verify_with(&decoding_key(), &accepted(), &inside).is_ok());

        let outside = sign(claims("id", ISSUER, CLIENT_ID, now() - 120));
        assert!(verify_with(&decoding_key(), &accepted(), &outside).is_err());
    }

    #[test]
    fn a_tampered_signature_is_rejected() {
        let token = sign(claims("id", ISSUER, CLIENT_ID, now() + 3600));
        let mut parts: Vec<&str> = token.split('.').collect();
        let other = sign(claims("id", ISSUER, CLIENT_ID, now() + 7200));
        let other_parts: Vec<&str> = other.split('.').collect();
        parts[2] = other_parts[2];
        assert!(verify_with(&decoding_key(), &accepted(), &parts.join(".")).is_err());
    }
}

/// Signs in for real and verifies the resulting ID token, which is the one path
/// a synthetic token cannot cover: only Cognito knows which issuer it stamps.
///
/// Needs TEST_USERNAME and TEST_PASSWORD on top of the usual credentials:
/// `TEST_USERNAME=... TEST_PASSWORD=... cargo test -- --ignored --nocapture`
#[cfg(test)]
mod live_sign_in {
    use super::*;
    use crate::state::AppState;
    use aws_sdk_cognitoidentityprovider::types::AuthFlowType;
    use std::sync::Arc;

    #[tokio::test]
    #[ignore = "requires live AWS credentials and a test user"]
    async fn a_real_id_token_verifies() {
        let _ = dotenvy::dotenv();
        let (Ok(username), Ok(password)) = (
            std::env::var("TEST_USERNAME"),
            std::env::var("TEST_PASSWORD"),
        ) else {
            println!("TEST_USERNAME / TEST_PASSWORD unset; skipping");
            return;
        };

        let config = Arc::new(Config::from_env().expect("config"));
        let state = AppState::new(config.clone()).await;

        let mut request = state
            .cognito
            .admin_initiate_auth()
            .user_pool_id(&config.user_pool_id)
            .client_id(&config.client_id)
            .auth_flow(AuthFlowType::AdminUserPasswordAuth)
            .auth_parameters("USERNAME", &username)
            .auth_parameters("PASSWORD", &password);
        if let Some(hash) = state.secret_hash(&username) {
            request = request.auth_parameters("SECRET_HASH", hash);
        }

        let response = request.send().await.expect("sign in");
        let result = response
            .authentication_result()
            .expect("a challenge would need answering; use a confirmed user");
        let id_token = result.id_token().expect("id token");

        println!("issuers accepted: {:?}", state.jwks.issuers);
        println!("token: {}", describe(id_token));

        let claims = state
            .jwks
            .verify_id_token(id_token)
            .await
            .expect("the pool's own ID token must verify");
        assert_eq!(claims.token_use, "id");
        assert!(!claims.sub.is_empty());
        println!("verified sub={} username={:?}", claims.sub, claims.username);
    }
}
