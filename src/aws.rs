use aws_config::BehaviorVersion;
use aws_sdk_cognitoidentityprovider::Client;
use aws_smithy_http_client::tls::{rustls_provider::CryptoMode, Provider};

use crate::config::Config;

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
