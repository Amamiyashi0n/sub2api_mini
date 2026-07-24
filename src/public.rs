use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use chrono::{Duration, NaiveDate, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::FromRow;

use crate::{
    crypto::token_hash,
    error::{ApiError, ApiResult},
    key_policy,
    state::AppState,
};

pub fn router(_state: AppState) -> Router<AppState> {
    Router::new()
        .route("/key-usage", post(key_usage))
        .route("/settings", get(public_settings))
        .merge(crate::content::public_router())
}

async fn public_settings(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let mail_configured = crate::mail::is_configured(&state).await?;
    let values: Vec<(String, String)> = sqlx::query_as(
        "SELECT key, value FROM app_settings WHERE key IN \
         ('site_name', 'site_subtitle', 'site_logo', 'contact_info', 'doc_url', \
          'home_content', 'registration_enabled', 'email_verification_enabled', \
          'password_reset_enabled', \
          'channel_monitor_enabled', 'turnstile_enabled', \
          'turnstile_site_key')",
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
    let oauth_providers = crate::external_auth::public_provider_summary(&state).await?;
    let home_content = value("home_content").unwrap_or("");
    let home_content_url = crate::content::safe_iframe_url(home_content);
    let home_content_html = if home_content_url.is_none() {
        crate::content::render_markdown(home_content)
    } else {
        String::new()
    };
    let site_logo = value("site_logo")
        .filter(|value| valid_public_logo(value))
        .unwrap_or("/logo.svg");
    let doc_url = value("doc_url")
        .and_then(valid_public_link)
        .unwrap_or_default();
    Ok(Json(json!({"data": {
        "site_name": value("site_name").unwrap_or("Sub2API Mini"),
        "site_subtitle": value("site_subtitle").unwrap_or("个人 AI API 网关"),
        "site_logo": site_logo,
        "contact_info": value("contact_info").unwrap_or(""),
        "doc_url": doc_url,
        "home_content": home_content,
        "home_content_url": home_content_url,
        "home_content_html": home_content_html,
        "version": env!("CARGO_PKG_VERSION"),
        "registration_enabled": flag("registration_enabled", false),
        "email_verification_enabled": flag("email_verification_enabled", false),
        "password_reset_enabled": flag("password_reset_enabled", true),
        "mail_configured": mail_configured
        ,"channel_monitor_enabled": flag("channel_monitor_enabled", true)
        ,"turnstile_enabled": flag("turnstile_enabled", false)
        ,"turnstile_site_key": value("turnstile_site_key").unwrap_or("")
        ,"oauth_providers": oauth_providers
    }})))
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

#[derive(Deserialize)]
struct KeyUsageInput {
    api_key: String,
    #[serde(default = "default_range")]
    range: String,
    start_date: Option<String>,
    end_date: Option<String>,
}

fn default_range() -> String {
    "7d".into()
}

async fn key_usage(
    State(state): State<AppState>,
    Json(input): Json<KeyUsageInput>,
) -> ApiResult<Json<Value>> {
    let api_key = input.api_key.trim();
    if !api_key.starts_with("sk-") || api_key.len() < 20 {
        return Err(ApiError::bad_request(
            "INVALID_API_KEY",
            "API key is invalid",
        ));
    }
    let key_id: Option<i64> = sqlx::query_scalar("SELECT id FROM api_keys WHERE token_hash = ?")
        .bind(token_hash(api_key))
        .fetch_optional(&state.pool)
        .await?;
    let key_id = key_id.ok_or_else(|| ApiError::not_found("API key not found"))?;
    let policy = key_policy::get_key(&state.pool, key_id).await?;
    let (start, end) = date_bounds(&input)?;

    let stats: PublicUsageStats = sqlx::query_as(
        "SELECT COUNT(*) AS requests, \
         COALESCE(SUM(CASE WHEN status_code < 400 THEN 1 ELSE 0 END), 0) AS successful_requests, \
         COALESCE(SUM(CASE WHEN status_code >= 400 THEN 1 ELSE 0 END), 0) AS failed_requests, \
         COALESCE(SUM(input_tokens), 0) AS input_tokens, \
         COALESCE(SUM(output_tokens), 0) AS output_tokens, \
         COALESCE(SUM(total_tokens), 0) AS total_tokens, \
         COALESCE(SUM(cached_input_tokens), 0) AS cached_input_tokens, \
         COALESCE(SUM(reasoning_tokens), 0) AS reasoning_tokens, \
         COALESCE(SUM(cost_microusd), 0) AS cost_microusd, \
         CAST(COALESCE(AVG(duration_ms), 0) AS REAL) AS average_duration_ms \
         FROM usage_logs WHERE api_key_id = ? \
         AND (? IS NULL OR datetime(created_at) >= datetime(?)) \
         AND (? IS NULL OR datetime(created_at) < datetime(?))",
    )
    .bind(key_id)
    .bind(&start)
    .bind(&start)
    .bind(&end)
    .bind(&end)
    .fetch_one(&state.pool)
    .await?;

    let models: Vec<(String, i64, i64, i64, i64)> = sqlx::query_as(
        "SELECT COALESCE(NULLIF(model, ''), 'unknown'), COUNT(*), \
         COALESCE(SUM(total_tokens), 0), COALESCE(SUM(cached_input_tokens), 0), \
         COALESCE(SUM(cost_microusd), 0) \
         FROM usage_logs WHERE api_key_id = ? \
         AND (? IS NULL OR datetime(created_at) >= datetime(?)) \
         AND (? IS NULL OR datetime(created_at) < datetime(?)) \
         GROUP BY COALESCE(NULLIF(model, ''), 'unknown') ORDER BY COUNT(*) DESC LIMIT 20",
    )
    .bind(key_id)
    .bind(&start)
    .bind(&start)
    .bind(&end)
    .bind(&end)
    .fetch_all(&state.pool)
    .await?;

    let trend: Vec<(String, i64, i64, i64)> = sqlx::query_as(
        "SELECT date(created_at), COUNT(*), COALESCE(SUM(total_tokens), 0), \
         COALESCE(SUM(cost_microusd), 0) \
         FROM usage_logs WHERE api_key_id = ? \
         AND (? IS NULL OR datetime(created_at) >= datetime(?)) \
         AND (? IS NULL OR datetime(created_at) < datetime(?)) \
         GROUP BY date(created_at) ORDER BY date(created_at) DESC LIMIT 90",
    )
    .bind(key_id)
    .bind(&start)
    .bind(&start)
    .bind(&end)
    .bind(&end)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(json!({"data": {
        "key": {
            "name": policy["name"],
            "token_prefix": policy["token_prefix"],
            "enabled": policy["enabled"],
            "status": policy["status"],
            "last_used_at": policy["last_used_at"],
            "last_used_ip": policy["last_used_ip"],
            "created_at": policy["created_at"],
            "expires_at": policy["expires_at"],
            "quota_tokens": policy["quota_tokens"],
            "quota_cost_microusd": policy["quota_cost_microusd"],
            "used_tokens": policy["used_tokens"],
            "used_cost_microusd": policy["used_cost_microusd"],
            "rate_limit_5h_microusd": policy["rate_limit_5h_microusd"],
            "rate_limit_1d_microusd": policy["rate_limit_1d_microusd"],
            "rate_limit_7d_microusd": policy["rate_limit_7d_microusd"],
            "usage_5h_microusd": policy["usage_5h_microusd"],
            "usage_1d_microusd": policy["usage_1d_microusd"],
            "usage_7d_microusd": policy["usage_7d_microusd"],
            "allowed_model_count": policy["allowed_models"].as_array().map_or(0, Vec::len),
            "ip_whitelist_count": policy["ip_whitelist"].as_array().map_or(0, Vec::len),
            "ip_blacklist_count": policy["ip_blacklist"].as_array().map_or(0, Vec::len),
            "group_name": policy["group_name"]
        },
        "range": {"kind": input.range, "start": start, "end": end},
        "stats": {
            "requests": stats.requests,
            "successful_requests": stats.successful_requests,
            "failed_requests": stats.failed_requests,
            "input_tokens": stats.input_tokens,
            "output_tokens": stats.output_tokens,
            "total_tokens": stats.total_tokens,
            "cached_input_tokens": stats.cached_input_tokens,
            "reasoning_tokens": stats.reasoning_tokens,
            "cost_microusd": stats.cost_microusd,
            "average_duration_ms": stats.average_duration_ms.round() as i64
        },
        "models": models.into_iter().map(|row| json!({
            "model": row.0, "requests": row.1, "tokens": row.2,
            "cached_input_tokens": row.3, "cost_microusd": row.4
        })).collect::<Vec<_>>(),
        "trend": trend.into_iter().map(|row| json!({
            "date": row.0, "requests": row.1, "tokens": row.2, "cost_microusd": row.3
        })).collect::<Vec<_>>()
    }})))
}

#[derive(FromRow)]
struct PublicUsageStats {
    requests: i64,
    successful_requests: i64,
    failed_requests: i64,
    input_tokens: i64,
    output_tokens: i64,
    total_tokens: i64,
    cached_input_tokens: i64,
    reasoning_tokens: i64,
    cost_microusd: i64,
    average_duration_ms: f64,
}

fn date_bounds(input: &KeyUsageInput) -> ApiResult<(Option<String>, Option<String>)> {
    let now = Utc::now();
    let start = match input.range.as_str() {
        "today" => Some(now.date_naive().and_hms_opt(0, 0, 0).unwrap().to_string()),
        "7d" => Some((now - Duration::days(7)).to_rfc3339()),
        "30d" => Some((now - Duration::days(30)).to_rfc3339()),
        "all" => None,
        "custom" => {
            let value = input.start_date.as_deref().ok_or_else(|| {
                ApiError::bad_request("START_DATE_REQUIRED", "start_date is required")
            })?;
            Some(parse_date(value, "start_date")?.to_string())
        }
        _ => {
            return Err(ApiError::bad_request(
                "INVALID_RANGE",
                "range must be today, 7d, 30d, all, or custom",
            ));
        }
    };
    let end = if input.range == "custom" {
        let value = input
            .end_date
            .as_deref()
            .ok_or_else(|| ApiError::bad_request("END_DATE_REQUIRED", "end_date is required"))?;
        let date = parse_date(value, "end_date")?;
        Some(
            date.succ_opt()
                .ok_or_else(|| ApiError::bad_request("INVALID_END_DATE", "end_date is invalid"))?
                .to_string(),
        )
    } else {
        None
    };
    if let (Some(start), Some(end)) = (&start, &end)
        && start >= end
    {
        return Err(ApiError::bad_request(
            "INVALID_DATE_RANGE",
            "start_date must be before or equal to end_date",
        ));
    }
    Ok((start, end))
}

fn parse_date(value: &str, field: &'static str) -> ApiResult<NaiveDate> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| ApiError::bad_request("INVALID_DATE", format!("{field} must use YYYY-MM-DD")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    use crate::{crypto::token_hash, test_support};

    #[test]
    fn validates_custom_date_ranges() {
        let valid = KeyUsageInput {
            api_key: String::new(),
            range: "custom".into(),
            start_date: Some("2026-07-01".into()),
            end_date: Some("2026-07-22".into()),
        };
        let (start, end) = date_bounds(&valid).unwrap();
        assert_eq!(start.as_deref(), Some("2026-07-01"));
        assert_eq!(end.as_deref(), Some("2026-07-23"));

        let invalid = KeyUsageInput {
            end_date: Some("2026-06-30".into()),
            ..valid
        };
        assert!(date_bounds(&invalid).is_err());
    }

    #[tokio::test]
    async fn returns_usage_for_the_requested_key_only() {
        let (_directory, state) = test_support::state().await;
        let token = "sk-mini_public-usage-test-token";
        let key_id = sqlx::query(
            "INSERT INTO api_keys (name, token_prefix, token_hash) VALUES ('public', 'sk-mini_public', ?)",
        )
        .bind(token_hash(token))
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        sqlx::query(
            "INSERT INTO usage_logs \
             (request_id, api_key_id, endpoint, model, status_code, input_tokens, output_tokens, total_tokens, duration_ms) \
             VALUES ('req-ok', ?, '/v1/responses', 'gpt-test', 200, 7, 3, 10, 25), \
                    ('req-fail', ?, '/v1/responses', 'gpt-test', 429, 0, 0, 0, 15)",
        )
        .bind(key_id)
        .bind(key_id)
        .execute(&state.pool)
        .await
        .unwrap();

        let app = Router::new()
            .nest("/api/public", router(state.clone()))
            .with_state(state);
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/public/key-usage")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(
                        r#"{{"api_key":"{token}","range":"all"}}"#
                    )))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["data"]["stats"]["requests"], 2);
        assert_eq!(value["data"]["stats"]["successful_requests"], 1);
        assert_eq!(value["data"]["stats"]["failed_requests"], 1);
        assert_eq!(value["data"]["stats"]["total_tokens"], 10);
        assert_eq!(value["data"]["models"][0]["model"], "gpt-test");
    }
}
