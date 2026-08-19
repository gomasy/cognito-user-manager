use std::env;

/// Runtime configuration, read once at startup.
#[derive(Debug, Clone)]
pub struct Config {
    pub region: String,
    pub user_pool_id: String,
    pub client_id: String,
    /// Only set when the app client is configured with a secret.
    pub client_secret: Option<String>,
    /// Members of this group may use the admin screens.
    pub admin_group: String,
    /// Ignored when the Lambda runtime is hosting the process.
    pub bind: String,
    /// Force the Secure attribute on cookies. `None` derives it per request
    /// from the scheme, so plain-HTTP local use works without configuration.
    pub secure_cookies: Option<bool>,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        Ok(Self {
            region: required("AWS_REGION")?,
            user_pool_id: required("COGNITO_USER_POOL_ID")?,
            client_id: required("COGNITO_CLIENT_ID")?,
            client_secret: env::var("COGNITO_CLIENT_SECRET").ok().filter(|v| !v.is_empty()),
            admin_group: env::var("COGNITO_ADMIN_GROUP")
                .ok()
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| "admin".to_string()),
            bind: env::var("BIND_ADDR")
                .ok()
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| "127.0.0.1:3000".to_string()),
            secure_cookies: env::var("SECURE_COOKIES")
                .ok()
                .filter(|value| !value.is_empty())
                .map(|value| matches!(value.as_str(), "1" | "true" | "yes")),
        })
    }
}

fn required(name: &str) -> Result<String, String> {
    env::var(name)
        .ok()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("Missing environment variable {name}. See .env.example."))
}
