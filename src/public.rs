use axum::{Json, Router, extract::State, routing::get};
use serde_json::{Value, json};

use crate::{error::ApiResult, state::AppState};

pub fn router(_state: AppState) -> Router<AppState> {
    Router::new().route("/settings", get(public_settings))
}

async fn public_settings(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let mail_configured = crate::mail::is_configured(&state).await?;
    let values: Vec<(String, String)> = sqlx::query_as(
        "SELECT key, value FROM app_settings WHERE key IN \
         ('site_name', 'site_subtitle', 'site_logo', 'registration_enabled', \
          'email_verification_enabled', 'password_reset_enabled', \
          'channel_monitor_enabled', 'turnstile_enabled', \
          'turnstile_site_key', 'default_theme')",
    )
    .fetch_all(&state.pool)
    .await?;
    let value = |key: &str| {
        values
            .iter()
            .find(|row| row.0 == key)
            .map(|row| row.1.as_str())
    };
    let flag = |key: &str, default: bool| {
        value(key)
            .and_then(|value| value.parse::<bool>().ok())
            .unwrap_or(default)
    };
    let site_logo = value("site_logo")
        .filter(|value| valid_public_logo(value))
        .unwrap_or("/logo.svg");
    Ok(Json(json!({"data": {
        "site_name": value("site_name").unwrap_or("Sub2API Mini"),
        "site_subtitle": value("site_subtitle").unwrap_or("个人 AI API 网关"),
        "site_logo": site_logo,
        "default_theme": normalize_theme(value("default_theme")),
        "version": env!("CARGO_PKG_VERSION"),
        "registration_enabled": flag("registration_enabled", false),
        "email_verification_enabled": flag("email_verification_enabled", false),
        "password_reset_enabled": flag("password_reset_enabled", true),
        "mail_configured": mail_configured,
        "channel_monitor_enabled": flag("channel_monitor_enabled", true),
        "turnstile_enabled": flag("turnstile_enabled", false),
        "turnstile_site_key": value("turnstile_site_key").unwrap_or("")
    }})))
}

pub(crate) fn normalize_theme(value: Option<&str>) -> &'static str {
    match value {
        Some("dark") => "dark",
        _ => "light",
    }
}

fn valid_public_link(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 2048 {
        return None;
    }
    let parsed = url::Url::parse(value).ok()?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return None;
    }
    Some(parsed.to_string())
}

fn valid_public_logo(value: &str) -> bool {
    let value = value.trim();
    if value.len() > 256 * 1024 {
        return false;
    }
    (value.starts_with('/') && !value.starts_with("//"))
        || valid_public_link(value).is_some()
        || [
            "data:image/png;base64,",
            "data:image/jpeg;base64,",
            "data:image/webp;base64,",
            "data:image/gif;base64,",
        ]
        .iter()
        .any(|prefix| value.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    use crate::test_support;

    #[tokio::test]
    async fn public_settings_normalize_the_default_theme() {
        let (_directory, state) = test_support::state().await;
        let Json(initial) = public_settings(State(state.clone())).await.unwrap();
        assert_eq!(initial["data"]["default_theme"], "light");
        assert!(initial["data"].get("home_content").is_none());

        sqlx::query("INSERT INTO app_settings (key, value) VALUES ('default_theme', 'dark')")
            .execute(&state.pool)
            .await
            .unwrap();
        let Json(dark) = public_settings(State(state.clone())).await.unwrap();
        assert_eq!(dark["data"]["default_theme"], "dark");

        sqlx::query("UPDATE app_settings SET value = 'unsupported' WHERE key = 'default_theme'")
            .execute(&state.pool)
            .await
            .unwrap();
        let Json(fallback) = public_settings(State(state)).await.unwrap();
        assert_eq!(fallback["data"]["default_theme"], "light");
    }

    #[tokio::test]
    async fn removed_public_routes_return_not_found() {
        let (_directory, state) = test_support::state().await;
        let app = Router::new()
            .nest("/api/public", router(state.clone()))
            .with_state(state);

        for (method, path) in [
            ("POST", "/api/public/key-usage"),
            ("GET", "/api/public/announcements"),
            ("GET", "/api/public/pages"),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(path)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
        }
    }
}
