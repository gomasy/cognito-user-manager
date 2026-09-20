use std::future::Future;

use aws_config::BehaviorVersion;
use aws_sdk_cognitoidentityprovider::Client;
use aws_smithy_http_client::tls::{Provider, rustls_provider::CryptoMode};

use crate::config::Config;
use crate::error::ApiResult;

/// Credentials come from the default AWS chain: env vars, IAM role, SSO.
pub async fn client(config: &Config) -> Client {
    let http = aws_smithy_http_client::Builder::new()
        .tls_provider(Provider::Rustls(CryptoMode::Ring))
        .build_https();

    let shared = aws_config::defaults(BehaviorVersion::latest())
        .http_client(http)
        .region(aws_config::Region::new(config.region.clone()))
        .load()
        .await;

    Client::new(&shared)
}

/// Follows Cognito's opaque next-token to the end, collecting every row. A
/// list left half-read is a row no screen can act on.
pub async fn every_page<T, F, Fut>(mut read: F) -> ApiResult<Vec<T>>
where
    F: FnMut(Option<String>) -> Fut,
    Fut: Future<Output = ApiResult<(Vec<T>, Option<String>)>>,
{
    let mut rows = Vec::new();
    let mut next = None;
    loop {
        let (page, token) = read(next).await?;
        rows.extend(page);
        if token.is_none() {
            return Ok(rows);
        }
        next = token;
    }
}
