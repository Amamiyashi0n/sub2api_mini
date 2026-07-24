use std::time::Instant;

use axum::{
    Json, Router,
    extract::{Extension, Path, Query, State},
    http::{HeaderMap, Method},
    middleware::Next,
    response::Response,
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    auth::AuthSession,
    error::{ApiError, ApiResult},
    state::AppState,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/audit-logs", get(list))
        .route("/audit-logs/clear", post(clear))
        .route("/audit-logs/{id}", get(detail))
}

#[derive(Deserialize)]
struct ClearInput {
    totp_code: String,
}

async fn clear(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
    Json(input): Json<ClearInput>,
) -> ApiResult<Json<Value>> {
    if !crate::totp::verify_code(&state, session.user_id, input.totp_code.trim()).await? {
        return Err(ApiError::forbidden(
            "a valid administrator TOTP code is required",
        ));
    }
    let result = sqlx::query("DELETE FROM audit_logs")
        .execute(&state.pool)
        .await?;
    Ok(Json(json!({"data": {"deleted": result.rows_affected()}})))
}

pub async fn capture(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let headers = request.headers().clone();
    let started = Instant::now();
    let response = next.run(request).await;

    if !matches!(method, Method::GET | Method::HEAD | Method::OPTIONS) {
        let action = action_name(&method, &path);
        let _ = sqlx::query(
            "INSERT INTO audit_logs \
             (user_id, username, action, method, path, status_code, duration_ms, client_ip, user_agent, request_id) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(session.user_id)
        .bind(&session.username)
        .bind(action)
        .bind(method.as_str())
        .bind(&path)
        .bind(response.status().as_u16() as i32)
        .bind(started.elapsed().as_millis() as i64)
        .bind(client_ip(&headers))
        .bind(header_text(&headers, "user-agent", 300))
        .bind(header_text(&headers, "x-request-id", 80))
        .execute(&state.pool)
        .await;
    }
    response
}

fn action_name(method: &Method, path: &str) -> String {
    let resource = path
        .strip_prefix("/api/admin/")
        .unwrap_or(path)
        .trim_start_matches('/')
        .split('/')
        .next()
        .unwrap_or("admin");
    format!("{}.{}", resource, method.as_str().to_ascii_lowercase())
}

fn client_ip(headers: &HeaderMap) -> Option<String> {
    header_text(headers, "x-real-ip", 64).or_else(|| {
        header_text(headers, "x-forwarded-for", 128).and_then(|value| {
            value
                .split(',')
                .next()
                .map(str::trim)
                .map(ToOwned::to_owned)
        })
    })
}

fn header_text(headers: &HeaderMap, name: &str, limit: usize) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.chars().take(limit).collect())
}

#[derive(Deserialize)]
struct AuditQuery {
    #[serde(default = "default_page")]
    page: i64,
    #[serde(default = "default_page_size")]
    page_size: i64,
    q: Option<String>,
    action: Option<String>,
    status_code: Option<i32>,
    start_date: Option<String>,
    end_date: Option<String>,
}

fn default_page() -> i64 {
    1
}
fn default_page_size() -> i64 {
    50
}

async fn list(
    State(state): State<AppState>,
    Query(query): Query<AuditQuery>,
) -> ApiResult<Json<Value>> {
    let page = query.page.max(1);
    let page_size = query.page_size.clamp(1, 200);
    let total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_logs WHERE \
         (? IS NULL OR username LIKE '%' || ? || '%' OR path LIKE '%' || ? || '%') \
         AND (? IS NULL OR action LIKE '%' || ? || '%') \
         AND (? IS NULL OR status_code = ?) \
         AND (? IS NULL OR datetime(created_at) >= datetime(?)) \
         AND (? IS NULL OR datetime(created_at) < datetime(?, '+1 day'))",
    )
    .bind(&query.q)
    .bind(&query.q)
    .bind(&query.q)
    .bind(&query.action)
    .bind(&query.action)
    .bind(query.status_code)
    .bind(query.status_code)
    .bind(&query.start_date)
    .bind(&query.start_date)
    .bind(&query.end_date)
    .bind(&query.end_date)
    .fetch_one(&state.pool)
    .await?;
    let rows: Vec<(
        i64,
        Option<i64>,
        String,
        String,
        String,
        String,
        i32,
        i64,
        Option<String>,
        Option<String>,
        Option<String>,
        String,
    )> = sqlx::query_as(
        "SELECT id, user_id, username, action, method, path, status_code, duration_ms, \
         client_ip, user_agent, request_id, created_at FROM audit_logs WHERE \
         (? IS NULL OR username LIKE '%' || ? || '%' OR path LIKE '%' || ? || '%') \
         AND (? IS NULL OR action LIKE '%' || ? || '%') \
         AND (? IS NULL OR status_code = ?) \
         AND (? IS NULL OR datetime(created_at) >= datetime(?)) \
         AND (? IS NULL OR datetime(created_at) < datetime(?, '+1 day')) \
         ORDER BY id DESC LIMIT ? OFFSET ?",
    )
    .bind(&query.q)
    .bind(&query.q)
    .bind(&query.q)
    .bind(&query.action)
    .bind(&query.action)
    .bind(query.status_code)
    .bind(query.status_code)
    .bind(&query.start_date)
    .bind(&query.start_date)
    .bind(&query.end_date)
    .bind(&query.end_date)
    .bind(page_size)
    .bind((page - 1) * page_size)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(
        json!({"data": rows.into_iter().map(audit_value).collect::<Vec<_>>(), "meta": {
            "page": page, "page_size": page_size, "total": total
        }}),
    ))
}

async fn detail(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    let row: Option<(
        i64,
        Option<i64>,
        String,
        String,
        String,
        String,
        i32,
        i64,
        Option<String>,
        Option<String>,
        Option<String>,
        String,
    )> = sqlx::query_as(
        "SELECT id, user_id, username, action, method, path, status_code, duration_ms, \
         client_ip, user_agent, request_id, created_at FROM audit_logs WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await?;
    Ok(Json(
        json!({"data": audit_value(row.ok_or_else(|| ApiError::not_found("audit log not found"))?)}),
    ))
}

fn audit_value(
    row: (
        i64,
        Option<i64>,
        String,
        String,
        String,
        String,
        i32,
        i64,
        Option<String>,
        Option<String>,
        Option<String>,
        String,
    ),
) -> Value {
    json!({
        "id": row.0, "user_id": row.1, "username": row.2, "action": row.3,
        "method": row.4, "path": row.5, "status_code": row.6, "duration_ms": row.7,
        "client_ip": row.8, "user_agent": row.9, "request_id": row.10, "created_at": row.11
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use chrono::{Duration, Utc};
    use tower::ServiceExt;

    use crate::{crypto::token_hash, test_support};

    #[tokio::test]
    async fn records_authenticated_setting_updates() {
        let (_directory, state) = test_support::state().await;
        state.load_runtime_settings().await.unwrap();
        let admin_id: i64 = sqlx::query_scalar("SELECT id FROM users WHERE role = 'admin'")
            .fetch_one(&state.pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO auth_sessions (user_id, token_hash, csrf_hash, expires_at) VALUES (?, ?, ?, ?)",
        )
        .bind(admin_id)
        .bind(token_hash("audit-session"))
        .bind(token_hash("audit-csrf"))
        .bind((Utc::now() + Duration::hours(1)).to_rfc3339())
        .execute(&state.pool)
        .await
        .unwrap();
        let app = Router::new()
            .nest("/api/admin", crate::admin::router(state.clone()))
            .with_state(state.clone());
        let response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/admin/settings")
                    .header("content-type", "application/json")
                    .header("cookie", "mini_session=audit-session")
                    .header("x-csrf-token", "audit-csrf")
                    .body(Body::from(
                        r#"{"site_name":"Mini Test","audit_retention_days":30,"retry_attempts":2,"model_cache_seconds":120,"cooldown_5xx_seconds":10,"cooldown_429_seconds":45}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(state.runtime_settings.read().await.retry_attempts, 2);
        let row: (String, String, i32) = sqlx::query_as(
            "SELECT username, action, status_code FROM audit_logs ORDER BY id DESC LIMIT 1",
        )
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(row.0, "admin");
        assert_eq!(row.1, "settings.put");
        assert_eq!(row.2, 200);
    }
}
