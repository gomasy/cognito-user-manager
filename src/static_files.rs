use axum::Router;
use axum::extract::Request;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{MethodRouter, get};
use tower_http::services::ServeDir;

/// A content-hashed name is never reused, so its bytes can be held forever.
const IMMUTABLE: &str = "public, max-age=31536000, immutable";

/// index.html and the locale catalogs keep their names across deploys.
/// ServeDir answers the revalidation with a 304 from its ETag.
const REVALIDATE: &str = "public, max-age=0, must-revalidate";

/// Where the frontend build writes, relative to the working directory.
const DIST: &str = "front/dist";
const LOCALES: &str = "front/locales";

/// The app shell and the message catalogs. Kept out of the API router so the
/// layer below cannot reach /api.
pub fn router<S: Clone + Send + Sync + 'static>() -> Router<S> {
    // fallback rather than not_found_service: the latter rewrites the status
    // to 404, which would turn the shell into an error page.
    let shell = ServeDir::new(DIST).fallback(spa_shell(format!("{DIST}/index.html")));
    Router::new()
        .nest_service("/locales", ServeDir::new(LOCALES))
        .fallback_service(shell)
        .layer(middleware::from_fn(set_cache_control))
}

/// Client-side routes such as /admin/users/alice have no file behind them, so
/// the shell answers for them.
///
/// The decision reads Accept rather than the path: a username can contain a
/// dot (pools that sign in by email), so an extension-shaped path is not a
/// reliable signal. A navigation asks for text/html; a bundle fetched by
/// <script> or fetch() does not, and stays a 404 so a mistyped asset URL is
/// never served as HTML.
fn spa_shell(index: String) -> MethodRouter<()> {
    get(move |headers: HeaderMap| {
        let index = index.clone();
        async move {
            if !wants_html(&headers) {
                return StatusCode::NOT_FOUND.into_response();
            }
            match tokio::fs::read(&index).await {
                Ok(bytes) => (
                    [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                    bytes,
                )
                    .into_response(),
                Err(error) => {
                    tracing::error!(%index, ?error, "app shell is missing; run the frontend build");
                    StatusCode::NOT_FOUND.into_response()
                }
            }
        }
    })
}

fn wants_html(headers: &HeaderMap) -> bool {
    headers
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|accept| accept.contains("text/html"))
}

async fn set_cache_control(request: Request, next: Next) -> Response {
    let value = cache_control_for(request.uri().path());
    let mut response = next.run(request).await;

    // A 404 under a hashed-looking path must not be pinned in the CDN past the
    // deploy that publishes the file.
    if response.status().is_success() || response.status() == StatusCode::NOT_MODIFIED {
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static(value));
    }
    response
}

fn cache_control_for(path: &str) -> &'static str {
    if is_content_hashed(path) {
        IMMUTABLE
    } else {
        REVALIDATE
    }
}

/// Whether the request path names a content-hashed bundle. Parcel writes eight
/// lowercase hex digits before the extension; demanding at least that keeps a
/// merely dotted name (`vendor.min.js`) from being served as immutable.
fn is_content_hashed(path: &str) -> bool {
    let name = path.rsplit_once('/').map_or(path, |(_, name)| name);
    let Some((stem, _extension)) = name.rsplit_once('.') else {
        return false;
    };
    let Some((_, hash)) = stem.rsplit_once('.') else {
        return false;
    };
    hash.len() >= 8 && hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Uri;
    use tower::ServiceExt;

    async fn get(router: Router<()>, path: &'static str) -> Response {
        let mut request = Request::new(Body::empty());
        *request.uri_mut() = Uri::from_static(path);
        let Ok(response) = router.oneshot(request).await;
        response
    }

    fn cache_control(response: &Response) -> Option<&str> {
        response.headers().get(header::CACHE_CONTROL)?.to_str().ok()
    }

    #[test]
    fn hashed_bundles_are_immutable() {
        assert_eq!(cache_control_for("/index.ade1b22c.js"), IMMUTABLE);
        assert_eq!(cache_control_for("/index.ef82c314.css"), IMMUTABLE);
    }

    #[test]
    fn stable_names_revalidate() {
        assert_eq!(cache_control_for("/"), REVALIDATE);
        assert_eq!(cache_control_for("/index.html"), REVALIDATE);
        assert_eq!(cache_control_for("/locales/ja.json"), REVALIDATE);
    }

    #[test]
    fn only_a_hash_shaped_segment_counts() {
        assert_eq!(cache_control_for("/vendor.min.js"), REVALIDATE);
        assert_eq!(cache_control_for("/app.v2.css"), REVALIDATE);
        assert_eq!(cache_control_for("/index.1234567.js"), REVALIDATE);
        assert_eq!(cache_control_for("/app.zzzzzzzz.js"), REVALIDATE);
        assert_eq!(cache_control_for("/app.4F3A2B1C.js"), REVALIDATE);
        assert_eq!(cache_control_for("/4f3a2b1c/index.html"), REVALIDATE);
    }

    #[tokio::test]
    async fn a_served_catalog_carries_the_header() {
        let response = get(router(), "/locales/ja.json").await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(cache_control(&response), Some(REVALIDATE));
    }

    #[test]
    fn only_navigations_get_the_shell() {
        let mut navigation = HeaderMap::new();
        navigation.insert(
            header::ACCEPT,
            HeaderValue::from_static("text/html,application/xhtml+xml,*/*;q=0.8"),
        );
        assert!(wants_html(&navigation));

        let mut asset = HeaderMap::new();
        asset.insert(header::ACCEPT, HeaderValue::from_static("*/*"));
        assert!(!wants_html(&asset));

        // No Accept at all is treated as an asset, which keeps a 404 a 404.
        assert!(!wants_html(&HeaderMap::new()));
    }

    #[tokio::test]
    async fn a_miss_is_left_uncacheable() {
        // Hash-shaped, so the naive rule would pin this 404 for a year.
        let response = get(router(), "/index.4f3a2b1c.js").await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(cache_control(&response), None);
    }
}
