use std::sync::Arc;

use aws_sdk_cognitoidentityprovider::Client;
use base64::Engine;
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

use crate::aws;
use crate::config::Config;
use crate::jwks::Jwks;
use crate::schema::SchemaCache;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub cognito: Client,
    pub jwks: Arc<Jwks>,
    pub schema: Arc<SchemaCache>,
}

impl AppState {
    pub async fn new(config: Arc<Config>) -> Self {
        Self {
            cognito: aws::client(&config).await,
            jwks: Arc::new(Jwks::new(&config)),
            schema: Arc::new(SchemaCache::new()),
            config,
        }
    }

    /// SECRET_HASH required by app clients that have a secret.
    pub fn secret_hash(&self, username: &str) -> Option<String> {
        let secret = self.config.client_secret.as_ref()?;
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).ok()?;
        mac.update(username.as_bytes());
        mac.update(self.config.client_id.as_bytes());
        Some(base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes()))
    }
}
