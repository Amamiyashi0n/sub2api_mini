use axum::{
    Router,
    body::Body,
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header},
    response::Response,
    routing::get,
};
use bytes::Bytes;

const INDEX: Asset = Asset::new(
    include_bytes!("../web/index.html"),
    "text/html; charset=utf-8",
    concat!("\"", env!("SUB2API_MINI_INDEX_ETAG"), "\""),
);
const APP: Asset = Asset::new(
    include_bytes!("../web/app.js"),
    "text/javascript; charset=utf-8",
    concat!("\"", env!("SUB2API_MINI_APP_ETAG"), "\""),
);
const DASHBOARD: Asset = Asset::new(
    include_bytes!("../web/dashboard.js"),
    "text/javascript; charset=utf-8",
    concat!("\"", env!("SUB2API_MINI_DASHBOARD_ETAG"), "\""),
);
const USERS: Asset = Asset::new(
    include_bytes!("../web/users.js"),
    "text/javascript; charset=utf-8",
    concat!("\"", env!("SUB2API_MINI_USERS_ETAG"), "\""),
);
const OPS: Asset = Asset::new(
    include_bytes!("../web/ops.js"),
    "text/javascript; charset=utf-8",
    concat!("\"", env!("SUB2API_MINI_OPS_ETAG"), "\""),
);
const USAGE: Asset = Asset::new(
    include_bytes!("../web/usage.js"),
    "text/javascript; charset=utf-8",
    concat!("\"", env!("SUB2API_MINI_USAGE_ETAG"), "\""),
);
const BATCH_IMAGES: Asset = Asset::new(
    include_bytes!("../web/batch-images.js"),
    "text/javascript; charset=utf-8",
    concat!("\"", env!("SUB2API_MINI_BATCH_IMAGES_ETAG"), "\""),
);
const CONTENT: Asset = Asset::new(
    include_bytes!("../web/content.js"),
    "text/javascript; charset=utf-8",
    concat!("\"", env!("SUB2API_MINI_CONTENT_ETAG"), "\""),
);
const ENGAGEMENT: Asset = Asset::new(
    include_bytes!("../web/engagement.js"),
    "text/javascript; charset=utf-8",
    concat!("\"", env!("SUB2API_MINI_ENGAGEMENT_ETAG"), "\""),
);
const ACCOUNTS_TOOLS: Asset = Asset::new(
    include_bytes!("../web/accounts-tools.js"),
    "text/javascript; charset=utf-8",
    concat!("\"", env!("SUB2API_MINI_ACCOUNTS_TOOLS_ETAG"), "\""),
);
const ACCOUNT_SCHEDULES: Asset = Asset::new(
    include_bytes!("../web/account-schedules.js"),
    "text/javascript; charset=utf-8",
    concat!("\"", env!("SUB2API_MINI_ACCOUNT_SCHEDULES_ETAG"), "\""),
);
const SUBSCRIPTIONS: Asset = Asset::new(
    include_bytes!("../web/subscriptions.js"),
    "text/javascript; charset=utf-8",
    concat!("\"", env!("SUB2API_MINI_SUBSCRIPTIONS_ETAG"), "\""),
);
const CHANNELS: Asset = Asset::new(
    include_bytes!("../web/channels.js"),
    "text/javascript; charset=utf-8",
    concat!("\"", env!("SUB2API_MINI_CHANNELS_ETAG"), "\""),
);
const MONITOR_ADMIN: Asset = Asset::new(
    include_bytes!("../web/monitor-admin.js"),
    "text/javascript; charset=utf-8",
    concat!("\"", env!("SUB2API_MINI_MONITOR_ADMIN_ETAG"), "\""),
);
const TURNSTILE: Asset = Asset::new(
    include_bytes!("../web/turnstile.js"),
    "text/javascript; charset=utf-8",
    concat!("\"", env!("SUB2API_MINI_TURNSTILE_ETAG"), "\""),
);
const STYLES: Asset = Asset::new(
    include_bytes!("../web/styles.css"),
    "text/css; charset=utf-8",
    concat!("\"", env!("SUB2API_MINI_STYLES_ETAG"), "\""),
);
const LOGO: Asset = Asset::new(
    include_bytes!("../web/logo.svg"),
    "image/svg+xml",
    concat!("\"", env!("SUB2API_MINI_LOGO_ETAG"), "\""),
);
const SETUP: Asset = Asset::new(
    include_bytes!("../web/setup.js"),
    "text/javascript; charset=utf-8",
    concat!("\"", env!("SUB2API_MINI_SETUP_ETAG"), "\""),
);

const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; connect-src 'self' https://challenges.cloudflare.com; \
    img-src 'self' data: http: https:; frame-src http: https:; \
    style-src 'self'; script-src 'self' https://challenges.cloudflare.com; object-src 'none'; base-uri 'none'; \
    frame-ancestors 'none'; form-action 'self'";

struct Asset {
    body: &'static [u8],
    content_type: &'static str,
    etag: &'static str,
}

impl Asset {
    const fn new(body: &'static [u8], content_type: &'static str, etag: &'static str) -> Self {
        Self {
            body,
            content_type,
            etag,
        }
    }
}

pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/", get(index))
        .route("/index.html", get(index))
        .route("/app.js", get(app))
        .route("/dashboard.js", get(dashboard))
        .route("/users.js", get(users))
        .route("/ops.js", get(ops))
        .route("/usage.js", get(usage))
        .route("/batch-images.js", get(batch_images))
        .route("/content.js", get(content))
        .route("/engagement.js", get(engagement))
        .route("/accounts-tools.js", get(accounts_tools))
        .route("/account-schedules.js", get(account_schedules))
        .route("/subscriptions.js", get(subscriptions))
        .route("/channels.js", get(channels))
        .route("/monitor-admin.js", get(monitor_admin))
        .route("/turnstile.js", get(turnstile))
        .route("/styles.css", get(styles))
        .route("/logo.svg", get(logo))
        .route("/setup.js", get(setup))
}

async fn index(headers: HeaderMap) -> Response<Body> {
    asset_response(&headers, &INDEX)
}

async fn app(headers: HeaderMap) -> Response<Body> {
    asset_response(&headers, &APP)
}

async fn dashboard(headers: HeaderMap) -> Response<Body> {
    asset_response(&headers, &DASHBOARD)
}

async fn users(headers: HeaderMap) -> Response<Body> {
    asset_response(&headers, &USERS)
}

async fn ops(headers: HeaderMap) -> Response<Body> {
    asset_response(&headers, &OPS)
}

async fn usage(headers: HeaderMap) -> Response<Body> {
    asset_response(&headers, &USAGE)
}

async fn batch_images(headers: HeaderMap) -> Response<Body> {
    asset_response(&headers, &BATCH_IMAGES)
}

async fn content(headers: HeaderMap) -> Response<Body> {
    asset_response(&headers, &CONTENT)
}

async fn engagement(headers: HeaderMap) -> Response<Body> {
    asset_response(&headers, &ENGAGEMENT)
}

async fn accounts_tools(headers: HeaderMap) -> Response<Body> {
    asset_response(&headers, &ACCOUNTS_TOOLS)
}

async fn account_schedules(headers: HeaderMap) -> Response<Body> {
    asset_response(&headers, &ACCOUNT_SCHEDULES)
}

async fn subscriptions(headers: HeaderMap) -> Response<Body> {
    asset_response(&headers, &SUBSCRIPTIONS)
}

async fn channels(headers: HeaderMap) -> Response<Body> {
    asset_response(&headers, &CHANNELS)
}

async fn monitor_admin(headers: HeaderMap) -> Response<Body> {
    asset_response(&headers, &MONITOR_ADMIN)
}

async fn turnstile(headers: HeaderMap) -> Response<Body> {
    asset_response(&headers, &TURNSTILE)
}

async fn styles(headers: HeaderMap) -> Response<Body> {
    asset_response(&headers, &STYLES)
}

async fn logo(headers: HeaderMap) -> Response<Body> {
    asset_response(&headers, &LOGO)
}

async fn setup(headers: HeaderMap) -> Response<Body> {
    asset_response(&headers, &SETUP)
}

fn asset_response(request_headers: &HeaderMap, asset: &Asset) -> Response<Body> {
    let not_modified = request_headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(',')
                .any(|candidate| matches!(candidate.trim(), "*") || candidate.trim() == asset.etag)
        });
    let status = if not_modified {
        StatusCode::NOT_MODIFIED
    } else {
        StatusCode::OK
    };
    let body = if not_modified {
        Body::empty()
    } else {
        Body::from(Bytes::from_static(asset.body))
    };
    let mut response = Response::new(body);
    *response.status_mut() = status;
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(asset.content_type),
    );
    headers.insert(header::ETAG, HeaderValue::from_static(asset.etag));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    headers.insert(
        HeaderName::from_static("content-security-policy"),
        HeaderValue::from_static(CONTENT_SECURITY_POLICY),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    if !not_modified {
        headers.insert(
            header::CONTENT_LENGTH,
            HeaderValue::from_str(&asset.body.len().to_string())
                .expect("embedded asset length is a valid header"),
        );
    }
    response
}

#[cfg(test)]
mod tests {
    use axum::{
        body::{Body, to_bytes},
        http::{Method, Request, StatusCode, header},
    };
    use tower::ServiceExt;

    use super::router;

    #[tokio::test]
    async fn serves_embedded_assets_with_security_headers() {
        let response = router::<()>()
            .oneshot(
                Request::builder()
                    .uri("/app.js")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/javascript; charset=utf-8"
        );
        assert_eq!(
            response
                .headers()
                .get(header::X_CONTENT_TYPE_OPTIONS)
                .unwrap(),
            "nosniff"
        );
        let csp = response
            .headers()
            .get("content-security-policy")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(csp.contains("frame-src http: https:"));
        assert!(csp.contains("img-src 'self' data: http: https:"));
        let body = to_bytes(response.into_body(), 256 * 1024).await.unwrap();
        assert!(String::from_utf8_lossy(&body).contains("Sub2API Mini"));
    }

    #[tokio::test]
    async fn serves_lazy_feature_assets_from_fixed_routes() {
        let app = router::<()>();
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/users.js")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/javascript; charset=utf-8"
        );
        let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        assert!(String::from_utf8_lossy(&body).contains("Sub2MiniUsers"));

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/subscriptions.js")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        assert!(String::from_utf8_lossy(&body).contains("Sub2MiniSubscriptions"));

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/channels.js")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        assert!(String::from_utf8_lossy(&body).contains("Sub2MiniChannels"));

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/monitor-admin.js")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        assert!(String::from_utf8_lossy(&body).contains("Sub2MiniMonitorAdmin"));

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/turnstile.js")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        assert!(String::from_utf8_lossy(&body).contains("Sub2MiniTurnstile"));

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/ops.js")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        assert!(String::from_utf8_lossy(&body).contains("Sub2MiniOps"));

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/usage.js")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        assert!(String::from_utf8_lossy(&body).contains("Sub2MiniUsage"));

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/batch-images.js")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        assert!(String::from_utf8_lossy(&body).contains("Sub2MiniBatchImages"));

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/content.js")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        assert!(String::from_utf8_lossy(&body).contains("Sub2MiniContent"));

        let engagement_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/engagement.js")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(engagement_response.status(), StatusCode::OK);
        let body = to_bytes(engagement_response.into_body(), 64 * 1024)
            .await
            .unwrap();
        assert!(String::from_utf8_lossy(&body).contains("Sub2MiniEngagement"));

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/dashboard.js")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        assert!(String::from_utf8_lossy(&body).contains("Sub2MiniDashboard"));

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/accounts-tools.js")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        assert!(String::from_utf8_lossy(&body).contains("Sub2MiniAccountTools"));

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/account-schedules.js")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        assert!(String::from_utf8_lossy(&body).contains("Sub2MiniAccountSchedules"));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/setup.js")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        assert!(String::from_utf8_lossy(&body).contains("Sub2MiniSetup"));
    }

    #[tokio::test]
    async fn head_returns_headers_without_a_body() {
        let response = router::<()>()
            .oneshot(
                Request::builder()
                    .method(Method::HEAD)
                    .uri("/styles.css")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().contains_key(header::CONTENT_LENGTH));
        assert!(
            to_bytes(response.into_body(), 64 * 1024)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn matching_etag_returns_not_modified() {
        let app = router::<()>();
        let first = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/logo.svg")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let etag = first.headers().get(header::ETAG).unwrap().clone();
        let second = app
            .oneshot(
                Request::builder()
                    .uri("/logo.svg")
                    .header(header::IF_NONE_MATCH, etag)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(second.status(), StatusCode::NOT_MODIFIED);
        assert!(to_bytes(second.into_body(), 1024).await.unwrap().is_empty());
    }
}
