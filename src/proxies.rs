use std::time::Instant;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use futures_util::future::join_all;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::FromRow;

use crate::{
    error::{ApiError, ApiResult},
    state::{AppState, build_http_client},
};

pub fn admin_router() -> Router<AppState> {
    Router::new()
        .route("/proxies", get(list).post(create))
        .route("/proxies/batch", post(batch_create))
        .route("/proxies/batch-delete", post(batch_delete))
        .route("/proxies/data", get(export_data).post(import_data))
        .route("/proxies/{id}", get(get_proxy).put(update).delete(delete))
        .route("/proxies/{id}/test", post(test))
        .route("/proxies/{id}/quality-check", post(quality_check))
        .route("/proxies/{id}/accounts", get(accounts))
        .route("/proxies/{id}/stats", get(stats))
}

#[derive(Debug, Clone, FromRow)]
struct ProxyRow {
    id: i64,
    name: String,
    encrypted_url: String,
    enabled: bool,
    fallback_mode: String,
    backup_proxy_id: Option<i64>,
    backup_proxy_name: Option<String>,
    expiry_warn_days: i64,
    expires_at: Option<String>,
    last_tested_at: Option<String>,
    last_latency_ms: Option<i64>,
    last_error: Option<String>,
    last_ip_address: Option<String>,
    last_country: Option<String>,
    last_country_code: Option<String>,
    last_region: Option<String>,
    last_city: Option<String>,
    quality_score: Option<i64>,
    quality_grade: Option<String>,
    quality_summary: Option<String>,
    quality_checked_at: Option<String>,
    created_at: String,
    updated_at: String,
    account_count: i64,
}

#[derive(Debug, Serialize)]
struct ProxyPublic {
    id: i64,
    name: String,
    protocol: String,
    host: String,
    port: u16,
    username: Option<String>,
    has_password: bool,
    address: String,
    enabled: bool,
    status: &'static str,
    fallback_mode: String,
    backup_proxy_id: Option<i64>,
    backup_proxy_name: Option<String>,
    expiry_warn_days: i64,
    expires_at: Option<String>,
    last_tested_at: Option<String>,
    last_latency_ms: Option<i64>,
    last_error: Option<String>,
    ip_address: Option<String>,
    country: Option<String>,
    country_code: Option<String>,
    region: Option<String>,
    city: Option<String>,
    quality_score: Option<i64>,
    quality_grade: Option<String>,
    quality_summary: Option<String>,
    quality_checked_at: Option<String>,
    account_count: i64,
    created_at: String,
    updated_at: String,
}

impl ProxyRow {
    fn public(&self, state: &AppState) -> ApiResult<ProxyPublic> {
        let raw = String::from_utf8(state.crypto.decrypt(&self.encrypted_url)?)
            .map_err(|_| ApiError::internal("stored proxy URL is malformed"))?;
        let url = validate_proxy_url(&raw)?;
        let host = url
            .host_str()
            .ok_or_else(|| ApiError::internal("stored proxy host is missing"))?
            .to_string();
        let port = url
            .port()
            .ok_or_else(|| ApiError::internal("stored proxy port is missing"))?;
        let expired = self
            .expires_at
            .as_deref()
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
            .is_some_and(|value| value <= Utc::now());
        let status = if expired {
            "expired"
        } else if self.enabled {
            "active"
        } else {
            "inactive"
        };
        let username = (!url.username().is_empty()).then(|| url.username().to_string());
        let address = if username.is_some() {
            format!(
                "{}://{}:***@{}:{}",
                url.scheme(),
                url.username(),
                host,
                port
            )
        } else {
            format!("{}://{}:{}", url.scheme(), host, port)
        };
        Ok(ProxyPublic {
            id: self.id,
            name: self.name.clone(),
            protocol: url.scheme().to_string(),
            host,
            port,
            username,
            has_password: url.password().is_some(),
            address,
            enabled: self.enabled,
            status,
            fallback_mode: self.fallback_mode.clone(),
            backup_proxy_id: self.backup_proxy_id,
            backup_proxy_name: self.backup_proxy_name.clone(),
            expiry_warn_days: self.expiry_warn_days,
            expires_at: self.expires_at.clone(),
            last_tested_at: self.last_tested_at.clone(),
            last_latency_ms: self.last_latency_ms,
            last_error: self.last_error.clone(),
            ip_address: self.last_ip_address.clone(),
            country: self.last_country.clone(),
            country_code: self.last_country_code.clone(),
            region: self.last_region.clone(),
            city: self.last_city.clone(),
            quality_score: self.quality_score,
            quality_grade: self.quality_grade.clone(),
            quality_summary: self.quality_summary.clone(),
            quality_checked_at: self.quality_checked_at.clone(),
            account_count: self.account_count,
            created_at: self.created_at.clone(),
            updated_at: self.updated_at.clone(),
        })
    }
}

const PROXY_SELECT: &str = "SELECT proxies.id, proxies.name, proxies.encrypted_url, proxies.enabled, \
    proxies.fallback_mode, proxies.backup_proxy_id, backup_proxies.name AS backup_proxy_name, \
    proxies.expiry_warn_days, proxies.expires_at, proxies.last_tested_at, proxies.last_latency_ms, \
    proxies.last_error, proxies.last_ip_address, proxies.last_country, proxies.last_country_code, \
    proxies.last_region, proxies.last_city, proxies.quality_score, proxies.quality_grade, \
    proxies.quality_summary, proxies.quality_checked_at, proxies.created_at, proxies.updated_at, \
    COUNT(accounts.id) AS account_count FROM proxies \
    LEFT JOIN accounts ON accounts.proxy_id = proxies.id \
    LEFT JOIN proxies AS backup_proxies ON backup_proxies.id = proxies.backup_proxy_id";

async fn list(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let rows = sqlx::query_as::<_, ProxyRow>(&format!(
        "{PROXY_SELECT} GROUP BY proxies.id ORDER BY proxies.id DESC"
    ))
    .fetch_all(&state.pool)
    .await?;
    let data = rows
        .iter()
        .map(|row| row.public(&state))
        .collect::<ApiResult<Vec<_>>>()?;
    Ok(Json(json!({"data": data})))
}

async fn get_proxy(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    let row = get_row(&state, id).await?;
    Ok(Json(json!({"data": row.public(&state)?})))
}

#[derive(Debug, Clone, Deserialize)]
struct CreateProxyInput {
    #[serde(default)]
    proxy_key: Option<String>,
    name: String,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    protocol: Option<String>,
    #[serde(default)]
    host: Option<String>,
    #[serde(default)]
    port: Option<u16>,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    password: Option<String>,
    #[serde(default = "enabled_by_default")]
    enabled: bool,
    #[serde(default = "default_fallback_mode")]
    fallback_mode: String,
    #[serde(default)]
    backup_proxy_id: Option<i64>,
    #[serde(default)]
    backup_proxy_key: Option<String>,
    #[serde(default = "default_expiry_warn_days")]
    expiry_warn_days: i64,
    #[serde(default)]
    expires_at: Option<String>,
}

fn enabled_by_default() -> bool {
    true
}

fn default_fallback_mode() -> String {
    "none".into()
}

fn default_expiry_warn_days() -> i64 {
    7
}

async fn create(
    State(state): State<AppState>,
    Json(input): Json<CreateProxyInput>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    validate_name(&input.name)?;
    let url = proxy_url_from_parts(
        input.url.as_deref(),
        input.protocol.as_deref(),
        input.host.as_deref(),
        input.port,
        input.username.as_deref(),
        input.password.as_deref(),
    )?;
    let expires_at = validate_expiry(input.expires_at)?;
    validate_fallback(
        &state,
        None,
        &input.fallback_mode,
        input.backup_proxy_id,
        input.expiry_warn_days,
    )
    .await?;
    let encrypted = state.crypto.encrypt(url.as_str().as_bytes())?;
    let result = sqlx::query(
        "INSERT INTO proxies (name, encrypted_url, enabled, fallback_mode, backup_proxy_id, \
         expiry_warn_days, expires_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(input.name.trim())
    .bind(encrypted)
    .bind(input.enabled)
    .bind(input.fallback_mode)
    .bind(input.backup_proxy_id)
    .bind(input.expiry_warn_days)
    .bind(expires_at)
    .execute(&state.pool)
    .await?;
    let row = get_row(&state, result.last_insert_rowid()).await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"data": row.public(&state)?})),
    ))
}

#[derive(Debug, Deserialize)]
struct UpdateProxyInput {
    name: Option<String>,
    url: Option<String>,
    enabled: Option<bool>,
    fallback_mode: Option<String>,
    #[serde(default, deserialize_with = "crate::models::deserialize_nullable")]
    backup_proxy_id: Option<Option<i64>>,
    expiry_warn_days: Option<i64>,
    #[serde(default)]
    expires_at: Option<Option<String>>,
}

async fn update(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(input): Json<UpdateProxyInput>,
) -> ApiResult<Json<Value>> {
    let row = get_row(&state, id).await?;
    let name = input.name.unwrap_or_else(|| row.name.clone());
    validate_name(&name)?;
    let encrypted_url = match input.url {
        Some(value) if !value.trim().is_empty() => {
            let url = validate_proxy_url(&value)?;
            state.crypto.encrypt(url.as_str().as_bytes())?
        }
        _ => row.encrypted_url,
    };
    let expires_at = match input.expires_at {
        Some(value) => validate_expiry(value)?,
        None => row.expires_at,
    };
    let fallback_mode = input
        .fallback_mode
        .unwrap_or_else(|| row.fallback_mode.clone());
    let backup_proxy_id = input.backup_proxy_id.unwrap_or(row.backup_proxy_id);
    let expiry_warn_days = input.expiry_warn_days.unwrap_or(row.expiry_warn_days);
    validate_fallback(
        &state,
        Some(id),
        &fallback_mode,
        backup_proxy_id,
        expiry_warn_days,
    )
    .await?;
    sqlx::query(
        "UPDATE proxies SET name = ?, encrypted_url = ?, enabled = ?, fallback_mode = ?, \
         backup_proxy_id = ?, expiry_warn_days = ?, expires_at = ?, \
         updated_at = CURRENT_TIMESTAMP WHERE id = ?",
    )
    .bind(name.trim())
    .bind(encrypted_url)
    .bind(input.enabled.unwrap_or(row.enabled))
    .bind(fallback_mode)
    .bind(backup_proxy_id)
    .bind(expiry_warn_days)
    .bind(expires_at)
    .bind(id)
    .execute(&state.pool)
    .await?;
    let row = get_row(&state, id).await?;
    Ok(Json(json!({"data": row.public(&state)?})))
}

async fn delete(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    let usage_count: i64 = sqlx::query_scalar(
        "SELECT (SELECT COUNT(*) FROM accounts WHERE proxy_id = ?) + \
         (SELECT COUNT(*) FROM proxies WHERE backup_proxy_id = ?)",
    )
    .bind(id)
    .bind(id)
    .fetch_one(&state.pool)
    .await?;
    if usage_count > 0 {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "PROXY_IN_USE",
            "remove account and backup-proxy references before deleting it",
        ));
    }
    let result = sqlx::query("DELETE FROM proxies WHERE id = ?")
        .bind(id)
        .execute(&state.pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found("proxy not found"));
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize, Default)]
struct TestInput {
    target: Option<String>,
}

async fn test(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(input): Json<TestInput>,
) -> ApiResult<Json<Value>> {
    let row = get_row(&state, id).await?;
    let proxy_url = String::from_utf8(state.crypto.decrypt(&row.encrypted_url)?)
        .map_err(|_| ApiError::internal("stored proxy URL is malformed"))?;
    validate_proxy_url(&proxy_url)?;
    let target = input.target.as_deref().unwrap_or("https://ipapi.co/json/");
    let target_url = url::Url::parse(target)
        .map_err(|_| ApiError::bad_request("INVALID_TEST_TARGET", "test target is invalid"))?;
    if !matches!(target_url.scheme(), "http" | "https") {
        return Err(ApiError::bad_request(
            "INVALID_TEST_TARGET",
            "test target must use http or https",
        ));
    }
    let client = build_http_client(Some(&proxy_url))?;
    let started = Instant::now();
    match client.get(target_url).send().await {
        Ok(response) => {
            let latency = started.elapsed().as_millis().min(i64::MAX as u128) as i64;
            let http_status = response.status().as_u16();
            let metadata = response.json::<Value>().await.ok();
            let field = |name: &str| {
                metadata
                    .as_ref()
                    .and_then(|value| value.get(name))
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
            };
            let ip = field("ip");
            let country = field("country_name").or_else(|| field("country"));
            let country_code = field("country_code");
            let region = field("region");
            let city = field("city");
            sqlx::query(
                "UPDATE proxies SET last_tested_at = CURRENT_TIMESTAMP, last_latency_ms = ?, \
                 last_error = NULL, last_ip_address = COALESCE(?, last_ip_address), \
                 last_country = COALESCE(?, last_country), \
                 last_country_code = COALESCE(?, last_country_code), \
                 last_region = COALESCE(?, last_region), last_city = COALESCE(?, last_city), \
                 updated_at = CURRENT_TIMESTAMP WHERE id = ?",
            )
            .bind(latency)
            .bind(&ip)
            .bind(&country)
            .bind(&country_code)
            .bind(&region)
            .bind(&city)
            .bind(id)
            .execute(&state.pool)
            .await?;
            Ok(Json(
                json!({"data": {"success": true, "latency_ms": latency,
                "http_status": http_status, "ip_address": ip, "country": country,
                "country_code": country_code, "region": region, "city": city,
                "message": "proxy connection succeeded"}}),
            ))
        }
        Err(error) => {
            let summary = if error.is_timeout() {
                "proxy connection timed out"
            } else if error.is_connect() {
                "proxy connection failed"
            } else {
                "proxy request failed"
            };
            sqlx::query(
                "UPDATE proxies SET last_tested_at = CURRENT_TIMESTAMP, last_latency_ms = NULL, \
                 last_error = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
            )
            .bind(summary)
            .bind(id)
            .execute(&state.pool)
            .await?;
            Ok(Json(
                json!({"data": {"success": false, "message": summary}}),
            ))
        }
    }
}

async fn quality_check(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<Value>> {
    let row = get_row(&state, id).await?;
    let proxy_url = String::from_utf8(state.crypto.decrypt(&row.encrypted_url)?)
        .map_err(|_| ApiError::internal("stored proxy URL is malformed"))?;
    let client = build_http_client(Some(&proxy_url))?;
    let targets = [
        ("OpenAI API", "https://api.openai.com/v1/models"),
        ("OpenAI Auth", "https://auth.openai.com/"),
        (
            "ChatGPT Codex",
            "https://chatgpt.com/backend-api/codex/models",
        ),
    ];
    let checks = targets.into_iter().map(|(target, url)| {
        let client = client.clone();
        async move {
            let started = Instant::now();
            match client.get(url).send().await {
                Ok(response) => {
                    let latency = started.elapsed().as_millis().min(i64::MAX as u128) as i64;
                    let http_status = response.status().as_u16();
                    let cf_ray = response
                        .headers()
                        .get("cf-ray")
                        .and_then(|value| value.to_str().ok())
                        .map(ToOwned::to_owned);
                    let (status, points, message) = if http_status == 403 && cf_ray.is_some() {
                        ("challenge", 30, "Cloudflare challenge detected")
                    } else if http_status >= 500 {
                        ("warn", 60, "target returned a server error")
                    } else {
                        ("pass", 100, "target is reachable")
                    };
                    (
                        points,
                        json!({"target": target, "status": status, "http_status": http_status,
                            "latency_ms": latency, "message": message, "cf_ray": cf_ray}),
                    )
                }
                Err(error) => {
                    let message = if error.is_timeout() {
                        "connection timed out"
                    } else {
                        "connection failed"
                    };
                    (
                        0,
                        json!({"target": target, "status": "fail", "message": message}),
                    )
                }
            }
        }
    });
    let checks = join_all(checks).await;
    let score = checks.iter().map(|item| item.0).sum::<i64>() / checks.len() as i64;
    let grade = match score {
        90..=100 => "A",
        75..=89 => "B",
        60..=74 => "C",
        40..=59 => "D",
        _ => "F",
    };
    let items = checks.into_iter().map(|item| item.1).collect::<Vec<_>>();
    let count = |status: &str| {
        items
            .iter()
            .filter(|item| item.get("status").and_then(Value::as_str) == Some(status))
            .count()
    };
    let passed_count = count("pass");
    let warn_count = count("warn");
    let failed_count = count("fail");
    let challenge_count = count("challenge");
    let summary = format!(
        "{passed_count} passed, {warn_count} warnings, {challenge_count} challenges, {failed_count} failed"
    );
    sqlx::query(
        "UPDATE proxies SET quality_score = ?, quality_grade = ?, quality_summary = ?, \
         quality_checked_at = CURRENT_TIMESTAMP, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
    )
    .bind(score)
    .bind(grade)
    .bind(&summary)
    .bind(id)
    .execute(&state.pool)
    .await?;
    Ok(Json(
        json!({"data": {"proxy_id": id, "score": score, "grade": grade,
        "summary": summary, "passed_count": passed_count, "warn_count": warn_count,
        "failed_count": failed_count, "challenge_count": challenge_count,
        "checked_at": Utc::now().timestamp(), "items": items}}),
    ))
}

async fn accounts(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    get_row(&state, id).await?;
    let rows: Vec<(i64, String, String, bool)> = sqlx::query_as(
        "SELECT id, name, kind, enabled FROM accounts WHERE proxy_id = ? ORDER BY id ASC",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await?;
    let data = rows
        .into_iter()
        .map(|row| json!({"id": row.0, "name": row.1, "kind": row.2, "enabled": row.3}))
        .collect::<Vec<_>>();
    Ok(Json(json!({"data": data})))
}

async fn stats(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    get_row(&state, id).await?;
    let accounts: (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(SUM(CASE WHEN enabled = 1 THEN 1 ELSE 0 END), 0) \
         FROM accounts WHERE proxy_id = ?",
    )
    .bind(id)
    .fetch_one(&state.pool)
    .await?;
    let usage: (i64, i64, f64) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(SUM(CASE WHEN status_code BETWEEN 200 AND 399 THEN 1 ELSE 0 END), 0), \
         CAST(COALESCE(AVG(duration_ms), 0) AS REAL) FROM usage_logs WHERE account_id IN \
         (SELECT id FROM accounts WHERE proxy_id = ?)",
    )
    .bind(id)
    .fetch_one(&state.pool)
    .await?;
    let success_rate = if usage.0 == 0 {
        0.0
    } else {
        usage.1 as f64 * 100.0 / usage.0 as f64
    };
    Ok(Json(
        json!({"data": {"total_accounts": accounts.0, "active_accounts": accounts.1,
        "total_requests": usage.0, "success_rate": success_rate,
        "average_latency": usage.2.round() as i64}}),
    ))
}

#[derive(Debug, Deserialize)]
struct BatchCreateInput {
    proxies: Vec<CreateProxyInput>,
}

async fn batch_create(
    State(state): State<AppState>,
    Json(input): Json<BatchCreateInput>,
) -> ApiResult<Json<Value>> {
    if input.proxies.is_empty() || input.proxies.len() > 500 {
        return Err(ApiError::bad_request(
            "INVALID_PROXY_BATCH",
            "batch must contain 1 to 500 proxies",
        ));
    }
    let rows =
        sqlx::query_as::<_, (i64, String, String)>("SELECT id, name, encrypted_url FROM proxies")
            .fetch_all(&state.pool)
            .await?;
    let mut known_urls = std::collections::HashMap::new();
    let mut keys = std::collections::HashMap::new();
    for (id, name, encrypted_url) in rows {
        if let Ok(value) = state.crypto.decrypt(&encrypted_url)
            && let Ok(url) = String::from_utf8(value)
        {
            known_urls.insert(url, id);
        }
        keys.entry(name).or_insert(id);
    }
    let mut created = 0;
    let mut skipped = 0;
    let mut pending = Vec::new();
    for proxy in input.proxies {
        let key = proxy
            .proxy_key
            .clone()
            .unwrap_or_else(|| proxy.name.trim().to_string());
        if key.is_empty() {
            return Err(ApiError::bad_request(
                "INVALID_PROXY_KEY",
                "proxy import key cannot be empty",
            ));
        }
        let url = proxy_url_from_parts(
            proxy.url.as_deref(),
            proxy.protocol.as_deref(),
            proxy.host.as_deref(),
            proxy.port,
            proxy.username.as_deref(),
            proxy.password.as_deref(),
        )?
        .to_string();
        let id = if let Some(id) = known_urls.get(&url).copied() {
            skipped += 1;
            id
        } else {
            let mut baseline = proxy.clone();
            baseline.fallback_mode = "none".into();
            baseline.backup_proxy_id = None;
            let (_, Json(value)) = create(State(state.clone()), Json(baseline)).await?;
            let id = value["data"]["id"]
                .as_i64()
                .ok_or_else(|| ApiError::internal("created proxy ID is missing"))?;
            known_urls.insert(url, id);
            created += 1;
            id
        };
        if let Some(existing) = keys.insert(key.clone(), id)
            && existing != id
        {
            return Err(ApiError::bad_request(
                "DUPLICATE_PROXY_KEY",
                format!("proxy import key '{key}' is duplicated"),
            ));
        }
        pending.push((
            id,
            proxy.fallback_mode,
            proxy.backup_proxy_id,
            proxy.backup_proxy_key,
            proxy.expiry_warn_days,
        ));
    }
    for (id, mode, backup_id, backup_key, expiry_warn_days) in pending {
        let backup_id = backup_id.or_else(|| backup_key.and_then(|key| keys.get(&key).copied()));
        validate_fallback(&state, Some(id), &mode, backup_id, expiry_warn_days).await?;
        sqlx::query(
            "UPDATE proxies SET fallback_mode = ?, backup_proxy_id = ?, expiry_warn_days = ?, \
             updated_at = CURRENT_TIMESTAMP WHERE id = ?",
        )
        .bind(mode)
        .bind(backup_id)
        .bind(expiry_warn_days)
        .bind(id)
        .execute(&state.pool)
        .await?;
    }
    Ok(Json(
        json!({"data": {"created": created, "skipped": skipped}}),
    ))
}

#[derive(Debug, Deserialize)]
struct BatchDeleteInput {
    ids: Vec<i64>,
}

async fn batch_delete(
    State(state): State<AppState>,
    Json(input): Json<BatchDeleteInput>,
) -> ApiResult<Json<Value>> {
    if input.ids.is_empty() || input.ids.len() > 500 {
        return Err(ApiError::bad_request(
            "INVALID_PROXY_BATCH",
            "batch must contain 1 to 500 proxy IDs",
        ));
    }
    let mut deleted_ids = Vec::new();
    let mut skipped = Vec::new();
    for id in input.ids {
        match delete(State(state.clone()), Path(id)).await {
            Ok(_) => deleted_ids.push(id),
            Err(error) if matches!(error.code, "PROXY_IN_USE" | "NOT_FOUND") => {
                skipped.push(json!({"id": id, "reason": error.message}));
            }
            Err(error) => return Err(error),
        }
    }
    Ok(Json(
        json!({"data": {"deleted_ids": deleted_ids, "skipped": skipped}}),
    ))
}

async fn export_data(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let rows = sqlx::query_as::<_, ProxyRow>(&format!(
        "{PROXY_SELECT} GROUP BY proxies.id ORDER BY proxies.id ASC"
    ))
    .fetch_all(&state.pool)
    .await?;
    let keys = rows
        .iter()
        .map(|row| (row.id, format!("proxy-{}", row.id)))
        .collect::<std::collections::HashMap<_, _>>();
    let mut proxies = Vec::with_capacity(rows.len());
    for row in rows {
        let url = String::from_utf8(state.crypto.decrypt(&row.encrypted_url)?)
            .map_err(|_| ApiError::internal("stored proxy URL is malformed"))?;
        proxies.push(json!({
            "proxy_key": keys.get(&row.id), "name": row.name, "url": url, "enabled": row.enabled,
            "fallback_mode": row.fallback_mode,
            "backup_proxy_key": row.backup_proxy_id.and_then(|id| keys.get(&id)),
            "expiry_warn_days": row.expiry_warn_days, "expires_at": row.expires_at
        }));
    }
    Ok(Json(
        json!({"data": {"type": "sub2api-mini-proxies", "version": 1,
        "exported_at": Utc::now().to_rfc3339(), "proxies": proxies}}),
    ))
}

async fn import_data(
    State(state): State<AppState>,
    Json(input): Json<BatchCreateInput>,
) -> ApiResult<Json<Value>> {
    batch_create(State(state), Json(input)).await
}

async fn get_row(state: &AppState, id: i64) -> ApiResult<ProxyRow> {
    sqlx::query_as::<_, ProxyRow>(&format!(
        "{PROXY_SELECT} WHERE proxies.id = ? GROUP BY proxies.id"
    ))
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| ApiError::not_found("proxy not found"))
}

fn validate_name(name: &str) -> ApiResult<()> {
    if name.trim().is_empty() || name.chars().count() > 80 {
        return Err(ApiError::bad_request(
            "INVALID_PROXY_NAME",
            "proxy name must contain 1 to 80 characters",
        ));
    }
    Ok(())
}

async fn validate_fallback(
    state: &AppState,
    current_id: Option<i64>,
    mode: &str,
    backup_proxy_id: Option<i64>,
    expiry_warn_days: i64,
) -> ApiResult<()> {
    if !matches!(mode, "none" | "proxy" | "direct") || !(0..=3650).contains(&expiry_warn_days) {
        return Err(ApiError::bad_request(
            "INVALID_PROXY_FALLBACK",
            "fallback mode or expiry warning window is invalid",
        ));
    }
    if mode == "proxy" && backup_proxy_id.is_none() {
        return Err(ApiError::bad_request(
            "BACKUP_PROXY_REQUIRED",
            "proxy fallback requires a backup proxy",
        ));
    }
    if mode != "proxy" && backup_proxy_id.is_some() {
        return Err(ApiError::bad_request(
            "BACKUP_PROXY_NOT_ALLOWED",
            "backup proxy is only valid for proxy fallback",
        ));
    }
    if let Some(backup_id) = backup_proxy_id {
        if current_id == Some(backup_id) {
            return Err(ApiError::bad_request(
                "INVALID_BACKUP_PROXY",
                "a proxy cannot use itself as backup",
            ));
        }
        let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM proxies WHERE id = ?")
            .bind(backup_id)
            .fetch_one(&state.pool)
            .await?;
        if exists == 0 {
            return Err(ApiError::bad_request(
                "INVALID_BACKUP_PROXY",
                "backup proxy does not exist",
            ));
        }
        if let Some(current_id) = current_id {
            let cycle: i64 = sqlx::query_scalar(
                "WITH RECURSIVE chain(id) AS (SELECT ? UNION ALL SELECT proxies.backup_proxy_id \
                 FROM proxies JOIN chain ON proxies.id = chain.id WHERE proxies.backup_proxy_id IS NOT NULL) \
                 SELECT COUNT(*) FROM chain WHERE id = ?",
            )
            .bind(backup_id)
            .bind(current_id)
            .fetch_one(&state.pool)
            .await?;
            if cycle > 0 {
                return Err(ApiError::bad_request(
                    "PROXY_FALLBACK_CYCLE",
                    "proxy fallback chain contains a cycle",
                ));
            }
        }
    }
    Ok(())
}

fn validate_expiry(value: Option<String>) -> ApiResult<Option<String>> {
    let Some(value) = value.map(|value| value.trim().to_string()) else {
        return Ok(None);
    };
    if value.is_empty() {
        return Ok(None);
    }
    DateTime::parse_from_rfc3339(&value).map_err(|_| {
        ApiError::bad_request("INVALID_PROXY_EXPIRY", "expires_at must be RFC 3339")
    })?;
    Ok(Some(value))
}

fn validate_proxy_url(value: &str) -> ApiResult<url::Url> {
    let url = url::Url::parse(value.trim())
        .map_err(|_| ApiError::bad_request("INVALID_PROXY_URL", "proxy URL is invalid"))?;
    if !matches!(url.scheme(), "http" | "https" | "socks5" | "socks5h")
        || url.host_str().is_none()
        || url.port().is_none()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ApiError::bad_request(
            "INVALID_PROXY_URL",
            "proxy URL must use http, https, socks5, or socks5h and include host and port",
        ));
    }
    Ok(url)
}

fn proxy_url_from_parts(
    value: Option<&str>,
    protocol: Option<&str>,
    host: Option<&str>,
    port: Option<u16>,
    username: Option<&str>,
    password: Option<&str>,
) -> ApiResult<url::Url> {
    if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
        return validate_proxy_url(value);
    }
    let protocol = protocol.unwrap_or("http");
    let host = host
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError::bad_request("INVALID_PROXY_URL", "proxy host is required"))?;
    let port =
        port.ok_or_else(|| ApiError::bad_request("INVALID_PROXY_URL", "proxy port is required"))?;
    let host = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    let mut url = validate_proxy_url(&format!("{protocol}://{host}:{port}"))?;
    if let Some(username) = username.filter(|value| !value.is_empty()) {
        url.set_username(username)
            .map_err(|_| ApiError::bad_request("INVALID_PROXY_URL", "proxy username is invalid"))?;
    }
    if let Some(password) = password.filter(|value| !value.is_empty()) {
        url.set_password(Some(password))
            .map_err(|_| ApiError::bad_request("INVALID_PROXY_URL", "proxy password is invalid"))?;
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::test_support;

    #[test]
    fn accepts_supported_proxy_urls_and_rejects_missing_ports() {
        assert!(validate_proxy_url("http://user:secret@127.0.0.1:3128").is_ok());
        assert!(validate_proxy_url("socks5h://127.0.0.1:1080").is_ok());
        assert!(validate_proxy_url("http://127.0.0.1").is_err());
        assert!(validate_proxy_url("ftp://127.0.0.1:21").is_err());
    }

    #[tokio::test]
    async fn stores_proxy_url_encrypted_and_masks_password() {
        let (_directory, state) = test_support::state().await;
        let (status, Json(value)) = create(
            State(state.clone()),
            Json(CreateProxyInput {
                name: "office".into(),
                proxy_key: None,
                url: Some("http://alice:secret@127.0.0.1:3128".into()),
                protocol: None,
                host: None,
                port: None,
                username: None,
                password: None,
                enabled: true,
                fallback_mode: "none".into(),
                backup_proxy_id: None,
                backup_proxy_key: None,
                expiry_warn_days: 7,
                expires_at: None,
            }),
        )
        .await
        .unwrap();
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(value["data"]["address"], "http://alice:***@127.0.0.1:3128");
        let stored: String = sqlx::query_scalar("SELECT encrypted_url FROM proxies LIMIT 1")
            .fetch_one(&state.pool)
            .await
            .unwrap();
        assert!(!stored.contains("secret"));
    }

    #[tokio::test]
    async fn scheduler_never_bypasses_a_disabled_proxy() {
        let (_directory, state) = test_support::state().await;
        let encrypted_url = state.crypto.encrypt(b"http://127.0.0.1:3128").unwrap();
        let proxy_id = sqlx::query(
            "INSERT INTO proxies (name, encrypted_url, enabled) VALUES ('office', ?, 1)",
        )
        .bind(encrypted_url)
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        let credentials = state.crypto.encrypt(br#"{"api_key":"upstream"}"#).unwrap();
        sqlx::query(
            "INSERT INTO accounts (name, kind, base_url, encrypted_credentials, proxy_id) \
             VALUES ('primary', 'api_key', 'https://api.openai.com', ?, ?)",
        )
        .bind(credentials)
        .bind(proxy_id)
        .execute(&state.pool)
        .await
        .unwrap();

        let selected = state
            .scheduler
            .select(&state, &HashSet::new(), None)
            .await
            .unwrap();
        assert_eq!(
            selected.account.proxy_url.as_deref(),
            Some("http://127.0.0.1:3128")
        );
        drop(selected);

        sqlx::query("UPDATE proxies SET enabled = 0 WHERE id = ?")
            .bind(proxy_id)
            .execute(&state.pool)
            .await
            .unwrap();
        let error = match state.scheduler.select(&state, &HashSet::new(), None).await {
            Ok(_) => panic!("disabled proxy account was scheduled"),
            Err(error) => error,
        };
        assert_eq!(error.code, "NO_UPSTREAM_ACCOUNT");
    }

    #[tokio::test]
    async fn scheduler_uses_backup_or_explicit_direct_fallback() {
        let (_directory, state) = test_support::state().await;
        let backup_url = state.crypto.encrypt(b"http://127.0.0.1:4128").unwrap();
        let backup_id =
            sqlx::query("INSERT INTO proxies (name, encrypted_url) VALUES ('backup', ?)")
                .bind(backup_url)
                .execute(&state.pool)
                .await
                .unwrap()
                .last_insert_rowid();
        let primary_url = state.crypto.encrypt(b"http://127.0.0.1:3128").unwrap();
        let primary_id = sqlx::query(
            "INSERT INTO proxies (name, encrypted_url, enabled, fallback_mode, backup_proxy_id) \
             VALUES ('primary', ?, 0, 'proxy', ?)",
        )
        .bind(primary_url)
        .bind(backup_id)
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        let credentials = state.crypto.encrypt(br#"{"api_key":"upstream"}"#).unwrap();
        sqlx::query(
            "INSERT INTO accounts (name, kind, base_url, encrypted_credentials, proxy_id) \
             VALUES ('primary', 'api_key', 'https://api.openai.com', ?, ?)",
        )
        .bind(credentials)
        .bind(primary_id)
        .execute(&state.pool)
        .await
        .unwrap();

        let selected = state
            .scheduler
            .select(&state, &HashSet::new(), None)
            .await
            .unwrap();
        assert_eq!(
            selected.account.proxy_url.as_deref(),
            Some("http://127.0.0.1:4128")
        );
        drop(selected);

        sqlx::query(
            "UPDATE proxies SET fallback_mode = 'direct', backup_proxy_id = NULL WHERE id = ?",
        )
        .bind(primary_id)
        .execute(&state.pool)
        .await
        .unwrap();
        let selected = state
            .scheduler
            .select(&state, &HashSet::new(), None)
            .await
            .unwrap();
        assert!(selected.account.proxy_url.is_none());
        assert!(state.client_for_account(&selected.account).await.is_ok());
    }

    #[tokio::test]
    async fn rejects_fallback_cycles() {
        let (_directory, state) = test_support::state().await;
        let encrypted = state.crypto.encrypt(b"http://127.0.0.1:3128").unwrap();
        let first = sqlx::query("INSERT INTO proxies (name, encrypted_url) VALUES ('one', ?)")
            .bind(&encrypted)
            .execute(&state.pool)
            .await
            .unwrap()
            .last_insert_rowid();
        let second = sqlx::query(
            "INSERT INTO proxies (name, encrypted_url, fallback_mode, backup_proxy_id) \
             VALUES ('two', ?, 'proxy', ?)",
        )
        .bind(encrypted)
        .bind(first)
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        let error = validate_fallback(&state, Some(first), "proxy", Some(second), 7)
            .await
            .unwrap_err();
        assert_eq!(error.code, "PROXY_FALLBACK_CYCLE");
    }

    #[tokio::test]
    async fn export_import_preserves_backup_references_without_database_ids() {
        let (_source_directory, source) = test_support::state().await;
        let backup_url = source.crypto.encrypt(b"http://127.0.0.1:4128").unwrap();
        let backup_id =
            sqlx::query("INSERT INTO proxies (name, encrypted_url) VALUES ('backup', ?)")
                .bind(backup_url)
                .execute(&source.pool)
                .await
                .unwrap()
                .last_insert_rowid();
        let primary_url = source.crypto.encrypt(b"http://127.0.0.1:3128").unwrap();
        sqlx::query(
            "INSERT INTO proxies (name, encrypted_url, fallback_mode, backup_proxy_id) \
             VALUES ('primary', ?, 'proxy', ?)",
        )
        .bind(primary_url)
        .bind(backup_id)
        .execute(&source.pool)
        .await
        .unwrap();
        let Json(exported) = export_data(State(source)).await.unwrap();
        let input: BatchCreateInput = serde_json::from_value(exported["data"].clone()).unwrap();

        let (_target_directory, target) = test_support::state().await;
        let Json(result) = import_data(State(target.clone()), Json(input))
            .await
            .unwrap();
        assert_eq!(result["data"]["created"], 2);
        let backup_name: String = sqlx::query_scalar(
            "SELECT backup.name FROM proxies AS primary_proxy \
             JOIN proxies AS backup ON backup.id = primary_proxy.backup_proxy_id \
             WHERE primary_proxy.name = 'primary'",
        )
        .fetch_one(&target.pool)
        .await
        .unwrap();
        assert_eq!(backup_name, "backup");
    }
}
