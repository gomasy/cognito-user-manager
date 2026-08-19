mod attributes;
mod aws;
mod config;
mod error;
mod extract;
mod handlers;
mod jwks;
mod locale;
mod schema;
mod session;
mod state;
mod static_files;
mod users;

rust_i18n::i18n!("locales", fallback = "en");

pub const VERSION: &str = concat!(
    "v",
    env!("CARGO_PKG_VERSION"),
    "-",
    env!("GIT_HASH"),
    " (",
    env!("BUILD_DATE"),
    ")",
);

use std::process;
use std::sync::Arc;

use axum::Router;
use axum::routing::{get, post, put};
use tower_cookies::CookieManagerLayer;

use crate::config::Config;
use crate::jwks::Jwks;
use crate::schema::SchemaCache;
use crate::state::AppState;

pub fn die(message: impl AsRef<str>) -> ! {
    eprintln!("{}", message.as_ref());
    process::exit(1)
}

fn api_router() -> Router<AppState> {
    Router::new()
        .route("/api/public", get(handlers::meta::public_info))
        .route("/api/session", get(handlers::meta::session))
        .route("/api/pool", get(handlers::meta::pool))
        .route("/api/auth/login", post(handlers::auth::login))
        .route("/api/auth/challenge", post(handlers::auth::challenge))
        .route("/api/auth/logout", post(handlers::auth::logout))
        .route(
            "/api/account",
            get(handlers::account::profile).patch(handlers::account::update),
        )
        .route("/api/account/password", post(handlers::account::change_password))
        .route("/api/account/verify/send", post(handlers::account::send_code))
        .route("/api/account/verify", post(handlers::account::verify))
        .route(
            "/api/admin/users",
            get(handlers::admin::list).post(handlers::admin::create),
        )
        .route(
            "/api/admin/users/{username}",
            get(handlers::admin::detail)
                .patch(handlers::admin::update)
                .delete(handlers::admin::delete),
        )
        .route("/api/admin/users/{username}/groups", put(handlers::admin::set_groups))
        .route("/api/admin/users/{username}/password", post(handlers::admin::set_password))
        .route(
            "/api/admin/users/{username}/password/reset",
            post(handlers::admin::reset_password),
        )
        .route("/api/admin/users/{username}/enabled", post(handlers::admin::set_enabled))
        .route("/api/admin/users/{username}/signout", post(handlers::admin::sign_out))
        .route("/api/admin/users/{username}/invite", post(handlers::admin::resend_invite))
}

#[tokio::main]
async fn main() {
    // Mirrors the usual local workflow; variables already in the environment win.
    let _ = dotenvy::dotenv();

    tracing_subscriber::fmt()
        .with_target(false)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "cognito_user_manager=info,warn".into()),
        )
        .init();

    let config = Arc::new(Config::from_env().unwrap_or_else(|error| die(error)));
    let state = AppState {
        cognito: aws::client(&config).await,
        jwks: Arc::new(Jwks::new(&config)),
        schema: Arc::new(SchemaCache::new()),
        config: config.clone(),
    };

    let app = api_router()
        // Merged after the API so the shell and catalogs stay reachable
        // without a session.
        .merge(static_files::router())
        .layer(CookieManagerLayer::new())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&config.bind)
        .await
        .unwrap_or_else(|error| die(format!("failed to bind {}: {error}", config.bind)));
    tracing::info!(address = %config.bind, version = VERSION, "listening");

    let served = axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await;
    if let Err(error) = served {
        die(format!("server error: {error}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    /// Every screen behind the guards must answer 401 without a session, so a
    /// route added later cannot quietly be public.
    #[tokio::test]
    async fn protected_routes_require_a_session() {
        let config = Arc::new(Config {
            region: "ap-northeast-1".into(),
            user_pool_id: "ap-northeast-1_test".into(),
            client_id: "client".into(),
            client_secret: None,
            admin_group: "admin".into(),
            bind: "127.0.0.1:0".into(),
            secure_cookies: Some(true),
        });
        let state = AppState {
            cognito: aws::client(&config).await,
            jwks: Arc::new(Jwks::new(&config)),
            schema: Arc::new(SchemaCache::new()),
            config,
        };
        let app = api_router()
            .layer(CookieManagerLayer::new())
            .with_state(state);

        for path in [
            "/api/session",
            "/api/pool",
            "/api/account",
            "/api/admin/users",
            "/api/admin/users/someone",
        ] {
            let request = Request::builder()
                .uri(path)
                .body(Body::empty())
                .expect("request");
            let response = app.clone().oneshot(request).await.expect("response");
            assert_eq!(
                response.status(),
                StatusCode::UNAUTHORIZED,
                "{path} should require a session"
            );
        }
    }
}
