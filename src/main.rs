mod attributes;
mod aws;
mod config;
mod error;
mod extract;
mod handlers;
mod jwks;
mod locale;
mod password;
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
use tokio::signal::unix::{SignalKind, signal};
use tower_cookies::CookieManagerLayer;

use crate::config::Config;
use crate::jwks::Jwks;
use crate::schema::SchemaCache;
use crate::state::AppState;

pub fn die(message: impl std::fmt::Display) -> ! {
    eprintln!("Error: {message}");
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
        .route(
            "/api/account/password",
            post(handlers::account::change_password),
        )
        .route(
            "/api/account/verify/send",
            post(handlers::account::send_code),
        )
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
        .route(
            "/api/admin/users/{username}/groups",
            put(handlers::admin::set_groups),
        )
        .route(
            "/api/admin/users/{username}/password",
            post(handlers::admin::set_password),
        )
        .route(
            "/api/admin/users/{username}/password/reset",
            post(handlers::admin::reset_password),
        )
        .route(
            "/api/admin/users/{username}/enabled",
            post(handlers::admin::set_enabled),
        )
        .route(
            "/api/admin/users/{username}/signout",
            post(handlers::admin::sign_out),
        )
        .route(
            "/api/admin/users/{username}/invite",
            post(handlers::admin::resend_invite),
        )
}

/// Resolves once the process is asked to stop, so in-flight requests get to
/// finish. systemd, Docker and Kubernetes all send SIGTERM; a terminal sends
/// SIGINT. Lambda uses neither — there the runtime owns the lifecycle.
async fn shutdown_signal() {
    // Registration failure degrades to Ctrl-C rather than taking the process
    // down. `None` then leaves the pattern below unmatched, which disables
    // that branch.
    let mut sigterm = signal(SignalKind::terminate())
        .inspect_err(|error| tracing::warn!(%error, "cannot listen for SIGTERM; Ctrl-C only"))
        .ok();

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        Some(_) = async { Some(sigterm.as_mut()?.recv().await) } => {}
    }
    tracing::info!("shutting down");
}

/// Whether the Lambda runtime is hosting this process. It sets
/// `AWS_LAMBDA_RUNTIME_API` for every function it starts, and nothing else
/// does, so the same binary can tell where it woke up without being told.
fn on_lambda() -> bool {
    std::env::var_os("AWS_LAMBDA_RUNTIME_API").is_some()
}

/// Everything either way of serving needs first. `.env` is read before logging
/// so `RUST_LOG` can live there; variables already in the environment win.
async fn boot() -> (Router, Arc<Config>) {
    let _ = dotenvy::dotenv();

    let logging = tracing_subscriber::fmt()
        .with_target(false)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "cognito_user_manager=info,warn".into()),
        );
    if on_lambda() {
        // CloudWatch stamps every line with its own timestamp and renders no ANSI.
        logging.without_time().with_ansi(false).init();
    } else {
        logging.init();
    }

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

    (app, config)
}

#[tokio::main]
async fn main() {
    let (app, config) = boot().await;

    // The Lambda runtime owns the socket and hands each request to the very
    // same tower service the listener below wraps, so nothing under this
    // function knows where it is running. BIND_ADDR is ignored there.
    #[cfg(feature = "lambda")]
    if on_lambda() {
        tracing::info!(version = VERSION, "serving on lambda");
        if let Err(error) = lambda_http::run(app).await {
            die(format!("the lambda runtime stopped: {error}"));
        }
        return;
    }

    // Without the feature there is no runtime to hand the socket to, and
    // binding a port Lambda never calls would only look like a hung init.
    #[cfg(not(feature = "lambda"))]
    if on_lambda() {
        die("running on Lambda but built without it: rebuild with --features lambda");
    }

    let listener = tokio::net::TcpListener::bind(&config.bind)
        .await
        .unwrap_or_else(|error| die(format!("failed to bind {}: {error}", config.bind)));
    tracing::info!(address = %config.bind, version = VERSION, "listening");

    let served = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await;
    if let Err(error) = served {
        die(format!("serving stopped: {error}"));
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
