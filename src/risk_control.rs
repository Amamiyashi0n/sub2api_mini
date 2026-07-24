use std::{
    collections::{BTreeMap, HashMap},
    time::Instant,
};

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post},
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, SqlitePool};

use crate::{
    crypto::token_hash,
    error::{ApiError, ApiResult},
    models::ApiKeyContext,
    state::AppState,
};

const CONFIG_KEY: &str = "risk_control_config";
const DEFAULT_BLOCK_MESSAGE: &str = "Request blocked by the risk control policy";
const MAX_AUDIT_TEXT_BYTES: usize = 128 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct StoredConfig {
    enabled: bool,
    mode: String,
    base_url: String,
    model: String,
    encrypted_api_keys: Vec<String>,
    timeout_ms: u64,
    sample_rate: u8,
    all_groups: bool,
    group_ids: Vec<i64>,
    record_non_hits: bool,
    worker_count: u8,
    queue_size: u32,
    block_status: u16,
    block_message: String,
    email_on_hit: bool,
    auto_ban_enabled: bool,
    ban_threshold: u32,
    violation_window_hours: u32,
    retry_count: u8,
    hit_retention_days: u32,
    non_hit_retention_days: u32,
    pre_hash_check_enabled: bool,
    blocked_keywords: Vec<String>,
    keyword_blocking_mode: String,
    model_filter: ModelFilter,
    thresholds: BTreeMap<String, f64>,
    cyber_policy_exclude_from_ban_count: bool,
}

impl Default for StoredConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: "pre_block".into(),
            base_url: "https://api.openai.com".into(),
            model: "omni-moderation-latest".into(),
            encrypted_api_keys: Vec::new(),
            timeout_ms: 3_000,
            sample_rate: 100,
            all_groups: true,
            group_ids: Vec::new(),
            record_non_hits: false,
            worker_count: 2,
            queue_size: 1_024,
            block_status: 403,
            block_message: DEFAULT_BLOCK_MESSAGE.into(),
            email_on_hit: false,
            auto_ban_enabled: true,
            ban_threshold: 10,
            violation_window_hours: 720,
            retry_count: 1,
            hit_retention_days: 180,
            non_hit_retention_days: 3,
            pre_hash_check_enabled: true,
            blocked_keywords: Vec::new(),
            keyword_blocking_mode: "keyword_and_api".into(),
            model_filter: ModelFilter::default(),
            thresholds: default_thresholds(),
            cyber_policy_exclude_from_ban_count: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct ModelFilter {
    #[serde(rename = "type")]
    kind: String,
    models: Vec<String>,
}

impl Default for ModelFilter {
    fn default() -> Self {
        Self {
            kind: "all".into(),
            models: Vec::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct UpdateConfig {
    enabled: Option<bool>,
    mode: Option<String>,
    base_url: Option<String>,
    model: Option<String>,
    api_key: Option<String>,
    api_keys: Option<Vec<String>>,
    api_keys_mode: Option<String>,
    #[serde(default)]
    delete_api_key_hashes: Vec<String>,
    #[serde(default)]
    clear_api_key: bool,
    timeout_ms: Option<u64>,
    sample_rate: Option<u8>,
    all_groups: Option<bool>,
    group_ids: Option<Vec<i64>>,
    record_non_hits: Option<bool>,
    worker_count: Option<u8>,
    queue_size: Option<u32>,
    block_status: Option<u16>,
    block_message: Option<String>,
    email_on_hit: Option<bool>,
    auto_ban_enabled: Option<bool>,
    ban_threshold: Option<u32>,
    violation_window_hours: Option<u32>,
    retry_count: Option<u8>,
    hit_retention_days: Option<u32>,
    non_hit_retention_days: Option<u32>,
    pre_hash_check_enabled: Option<bool>,
    blocked_keywords: Option<Vec<String>>,
    keyword_blocking_mode: Option<String>,
    model_filter: Option<ModelFilter>,
    thresholds: Option<BTreeMap<String, f64>>,
    cyber_policy_exclude_from_ban_count: Option<bool>,
}

#[derive(Debug, Clone)]
struct ModerationResult {
    flagged: bool,
    highest_category: String,
    highest_score: f64,
    matched_keyword: String,
    category_scores: BTreeMap<String, f64>,
    latency_ms: Option<i64>,
    error: String,
}

#[derive(Debug)]
struct ModerationCallError {
    message: String,
    http_status: i64,
}

impl ModerationCallError {
    fn upstream(message: impl Into<String>, http_status: i64) -> Self {
        Self {
            message: message.into(),
            http_status,
        }
    }

    fn into_api_error(self) -> ApiError {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            "MODERATION_UPSTREAM_ERROR",
            self.message,
        )
    }
}

#[derive(Debug, Clone, FromRow)]
struct KeyRuntimeRow {
    key_hash: String,
    masked: String,
    failure_count: i64,
    success_count: i64,
    last_error: String,
    last_checked_at: Option<String>,
    frozen_until: Option<String>,
    last_latency_ms: i64,
    last_http_status: i64,
    last_tested: bool,
    active: i64,
    total: i64,
    successes: i64,
    errors: i64,
    latency_total_ms: i64,
    frozen: bool,
}

struct KeyLoadGuard {
    pool: SqlitePool,
    key_hash: String,
    armed: bool,
}

impl KeyLoadGuard {
    fn new(pool: SqlitePool, key_hash: String) -> Self {
        Self {
            pool,
            key_hash,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for KeyLoadGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let pool = self.pool.clone();
        let key_hash = self.key_hash.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let _ = sqlx::query(
                    "UPDATE risk_control_api_key_runtime SET active = MAX(active - 1, 0), \
                     updated_at = CURRENT_TIMESTAMP WHERE key_hash = ?",
                )
                .bind(key_hash)
                .execute(&pool)
                .await;
            });
        }
    }
}

impl ModerationResult {
    fn pass() -> Self {
        Self {
            flagged: false,
            highest_category: String::new(),
            highest_score: 0.0,
            matched_keyword: String::new(),
            category_scores: BTreeMap::new(),
            latency_ms: None,
            error: String::new(),
        }
    }
}

pub fn admin_router() -> Router<AppState> {
    Router::new()
        .route("/risk-control/config", get(get_config).put(update_config))
        .route("/risk-control/status", get(status))
        .route("/risk-control/api-keys/test", post(test_api_keys))
        .route("/risk-control/logs", get(list_logs))
        .route("/risk-control/users/{id}/unban", post(unban_user))
        .route("/risk-control/hashes", delete(delete_hash))
        .route("/risk-control/hashes/all", delete(clear_hashes))
}

pub async fn initialize(state: &AppState) -> ApiResult<()> {
    sqlx::query(
        "UPDATE risk_control_api_key_runtime SET active = 0, updated_at = CURRENT_TIMESTAMP \
         WHERE active != 0",
    )
    .execute(&state.pool)
    .await?;
    Ok(())
}

async fn load_config(state: &AppState) -> ApiResult<StoredConfig> {
    let value: Option<String> = sqlx::query_scalar("SELECT value FROM app_settings WHERE key = ?")
        .bind(CONFIG_KEY)
        .fetch_optional(&state.pool)
        .await?;
    match value {
        Some(value) if value.trim() != "{}" => serde_json::from_str(&value)
            .map_err(|_| ApiError::internal("stored risk control config is malformed")),
        _ => Ok(StoredConfig::default()),
    }
}

async fn save_config(state: &AppState, config: &StoredConfig) -> ApiResult<()> {
    let value = serde_json::to_string(config)
        .map_err(|_| ApiError::internal("risk control config serialization failed"))?;
    sqlx::query(
        "INSERT INTO app_settings (key, value, updated_at) VALUES (?, ?, CURRENT_TIMESTAMP) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = CURRENT_TIMESTAMP",
    )
    .bind(CONFIG_KEY)
    .bind(value)
    .execute(&state.pool)
    .await?;
    Ok(())
}

fn decrypt_keys(state: &AppState, config: &StoredConfig) -> ApiResult<Vec<String>> {
    config
        .encrypted_api_keys
        .iter()
        .map(|value| {
            state.crypto.decrypt(value).and_then(|bytes| {
                String::from_utf8(bytes)
                    .map_err(|_| ApiError::internal("stored moderation API key is malformed"))
            })
        })
        .collect()
}

async fn ensure_key_runtime_rows(state: &AppState, keys: &[String]) -> ApiResult<()> {
    for key in keys {
        sqlx::query(
            "INSERT INTO risk_control_api_key_runtime (key_hash, masked) VALUES (?, ?) \
             ON CONFLICT(key_hash) DO NOTHING",
        )
        .bind(token_hash(key))
        .bind(mask_key(key))
        .execute(&state.pool)
        .await?;
    }
    Ok(())
}

async fn runtime_rows(state: &AppState, keys: &[String]) -> ApiResult<Vec<KeyRuntimeRow>> {
    ensure_key_runtime_rows(state, keys).await?;
    let hashes = keys.iter().map(|key| token_hash(key)).collect::<Vec<_>>();
    let rows: Vec<KeyRuntimeRow> = sqlx::query_as(
        "SELECT key_hash, masked, failure_count, success_count, last_error, last_checked_at, \
         frozen_until, last_latency_ms, last_http_status, last_tested, active, total, successes, \
         errors, latency_total_ms, CASE WHEN frozen_until IS NOT NULL AND \
         datetime(frozen_until) > CURRENT_TIMESTAMP THEN 1 ELSE 0 END AS frozen \
         FROM risk_control_api_key_runtime",
    )
    .fetch_all(&state.pool)
    .await?;
    let mut by_hash = rows
        .into_iter()
        .filter(|row| hashes.contains(&row.key_hash))
        .map(|row| (row.key_hash.clone(), row))
        .collect::<HashMap<_, _>>();
    Ok(keys
        .iter()
        .filter_map(|key| by_hash.remove(&token_hash(key)))
        .collect())
}

async fn key_statuses(state: &AppState, config: &StoredConfig) -> ApiResult<Vec<Value>> {
    let keys = decrypt_keys(state, config)?;
    let rows = runtime_rows(state, &keys).await?;
    Ok(rows
        .iter()
        .enumerate()
        .map(|(index, row)| key_status(index, row))
        .collect())
}

fn key_status(index: usize, row: &KeyRuntimeRow) -> Value {
    let state = if row.frozen {
        "frozen"
    } else if !row.last_tested {
        "unknown"
    } else if row.last_error.is_empty() {
        "ok"
    } else {
        "error"
    };
    json!({
        "index": index,
        "key_hash": row.key_hash,
        "masked": row.masked,
        "status": state,
        "failure_count": row.failure_count,
        "success_count": row.success_count,
        "last_error": row.last_error,
        "last_checked_at": row.last_checked_at,
        "frozen_until": row.frozen_until,
        "last_latency_ms": row.last_latency_ms,
        "last_http_status": row.last_http_status,
        "last_tested": row.last_tested,
        "configured": true
    })
}

fn transient_key_status(
    index: usize,
    key: &str,
    result: Result<&ModerationResult, &ModerationCallError>,
    latency_ms: i64,
) -> Value {
    let (status, error, http_status) = match result {
        Ok(_) => ("ok", String::new(), 200),
        Err(error) => ("error", error.message.clone(), error.http_status),
    };
    json!({
        "index": index, "key_hash": token_hash(key), "masked": mask_key(key),
        "status": status, "failure_count": i64::from(status == "error"),
        "success_count": i64::from(status == "ok"), "last_error": error,
        "last_checked_at": Utc::now().to_rfc3339(), "frozen_until": Value::Null,
        "last_latency_ms": latency_ms, "last_http_status": http_status,
        "last_tested": true, "configured": false
    })
}

fn mask_key(key: &str) -> String {
    let suffix: String = key
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("****{suffix}")
}

async fn public_config(state: &AppState, config: &StoredConfig) -> ApiResult<Value> {
    let statuses = key_statuses(state, config).await?;
    let mail_configured = crate::mail::is_configured(state).await?;
    let masks = statuses
        .iter()
        .filter_map(|item| item.get("masked").cloned())
        .collect::<Vec<_>>();
    Ok(json!({
        "enabled": config.enabled,
        "mode": config.mode,
        "base_url": config.base_url,
        "model": config.model,
        "api_key_configured": !config.encrypted_api_keys.is_empty(),
        "api_key_masked": masks.first().cloned().unwrap_or(Value::String(String::new())),
        "api_key_count": config.encrypted_api_keys.len(),
        "api_key_masks": masks,
        "api_key_statuses": statuses,
        "timeout_ms": config.timeout_ms,
        "sample_rate": config.sample_rate,
        "all_groups": config.all_groups,
        "group_ids": config.group_ids,
        "record_non_hits": config.record_non_hits,
        "worker_count": config.worker_count,
        "queue_size": config.queue_size,
        "block_status": config.block_status,
        "block_message": config.block_message,
        "email_on_hit": config.email_on_hit,
        "auto_ban_enabled": config.auto_ban_enabled,
        "ban_threshold": config.ban_threshold,
        "violation_window_hours": config.violation_window_hours,
        "retry_count": config.retry_count,
        "hit_retention_days": config.hit_retention_days,
        "non_hit_retention_days": config.non_hit_retention_days,
        "pre_hash_check_enabled": config.pre_hash_check_enabled,
        "blocked_keywords": config.blocked_keywords,
        "keyword_blocking_mode": config.keyword_blocking_mode,
        "model_filter": config.model_filter,
        "thresholds": config.thresholds,
        "cyber_policy_exclude_from_ban_count": config.cyber_policy_exclude_from_ban_count,
        "mail_configured": mail_configured
    }))
}

async fn get_config(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let config = load_config(&state).await?;
    Ok(Json(json!({"data": public_config(&state, &config).await?})))
}

async fn update_config(
    State(state): State<AppState>,
    Json(input): Json<UpdateConfig>,
) -> ApiResult<Json<Value>> {
    let mut config = load_config(&state).await?;
    let previous_keys = decrypt_keys(&state, &config)?;
    if let Some(value) = input.enabled {
        config.enabled = value;
    }
    if let Some(value) = input.mode {
        config.mode = value;
    }
    if let Some(value) = input.base_url {
        config.base_url = normalize_base_url(&value)?;
    }
    if let Some(value) = input.model {
        config.model = value.trim().to_string();
    }
    if let Some(value) = input.timeout_ms {
        config.timeout_ms = value;
    }
    if let Some(value) = input.sample_rate {
        config.sample_rate = value;
    }
    if let Some(value) = input.all_groups {
        config.all_groups = value;
    }
    if let Some(value) = input.group_ids {
        config.group_ids = normalize_ids(value);
    }
    if let Some(value) = input.record_non_hits {
        config.record_non_hits = value;
    }
    if let Some(value) = input.worker_count {
        config.worker_count = value;
    }
    if let Some(value) = input.queue_size {
        config.queue_size = value;
    }
    if let Some(value) = input.block_status {
        config.block_status = value;
    }
    if let Some(value) = input.block_message {
        config.block_message = value.trim().to_string();
    }
    if let Some(value) = input.email_on_hit {
        config.email_on_hit = value;
    }
    if let Some(value) = input.auto_ban_enabled {
        config.auto_ban_enabled = value;
    }
    if let Some(value) = input.ban_threshold {
        config.ban_threshold = value;
    }
    if let Some(value) = input.violation_window_hours {
        config.violation_window_hours = value;
    }
    if let Some(value) = input.retry_count {
        config.retry_count = value;
    }
    if let Some(value) = input.hit_retention_days {
        config.hit_retention_days = value;
    }
    if let Some(value) = input.non_hit_retention_days {
        config.non_hit_retention_days = value;
    }
    if let Some(value) = input.pre_hash_check_enabled {
        config.pre_hash_check_enabled = value;
    }
    if let Some(value) = input.blocked_keywords {
        config.blocked_keywords = normalize_strings(value, 500, 256)?;
    }
    if let Some(value) = input.keyword_blocking_mode {
        config.keyword_blocking_mode = value;
    }
    if let Some(mut value) = input.model_filter {
        value.models = normalize_strings(value.models, 500, 128)?;
        config.model_filter = value;
    }
    if let Some(value) = input.thresholds {
        config.thresholds = value;
    }
    if let Some(value) = input.cyber_policy_exclude_from_ban_count {
        config.cyber_policy_exclude_from_ban_count = value;
    }

    let mut keys = if input.clear_api_key {
        Vec::new()
    } else {
        decrypt_keys(&state, &config)?
    };
    if !input.delete_api_key_hashes.is_empty() {
        keys.retain(|key| {
            !input
                .delete_api_key_hashes
                .iter()
                .any(|hash| hash == &token_hash(key))
        });
    }
    let mut incoming = input.api_keys.unwrap_or_default();
    if let Some(key) = input.api_key {
        incoming.push(key);
    }
    incoming = normalize_strings(incoming, 32, 512)?;
    if input.api_keys_mode.as_deref() == Some("replace") {
        keys.clear();
    }
    for key in incoming {
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    if keys.len() > 32 {
        return Err(ApiError::bad_request(
            "TOO_MANY_MODERATION_KEYS",
            "at most 32 moderation API keys are supported",
        ));
    }
    config.encrypted_api_keys = keys
        .iter()
        .map(|key| state.crypto.encrypt(key.as_bytes()))
        .collect::<ApiResult<Vec<_>>>()?;
    validate_config(&config)?;
    save_config(&state, &config).await?;
    ensure_key_runtime_rows(&state, &keys).await?;
    let current_hashes = keys.iter().map(|key| token_hash(key)).collect::<Vec<_>>();
    for key in previous_keys {
        let hash = token_hash(&key);
        if !current_hashes.contains(&hash) {
            sqlx::query("DELETE FROM risk_control_api_key_runtime WHERE key_hash = ?")
                .bind(hash)
                .execute(&state.pool)
                .await?;
        }
    }
    cleanup_logs(&state, &config).await;
    Ok(Json(json!({"data": public_config(&state, &config).await?})))
}

fn validate_config(config: &StoredConfig) -> ApiResult<()> {
    if !matches!(config.mode.as_str(), "off" | "observe" | "pre_block")
        || !matches!(
            config.keyword_blocking_mode.as_str(),
            "keyword_only" | "keyword_and_api" | "api_only"
        )
        || !matches!(
            config.model_filter.kind.as_str(),
            "all" | "include" | "exclude"
        )
        || !(500..=30_000).contains(&config.timeout_ms)
        || config.sample_rate > 100
        || !(1..=32).contains(&config.worker_count)
        || !(100..=100_000).contains(&config.queue_size)
        || !(400..=599).contains(&config.block_status)
        || config.block_message.is_empty()
        || config.block_message.chars().count() > 500
        || !(1..=1_000).contains(&config.ban_threshold)
        || !(1..=8_760).contains(&config.violation_window_hours)
        || config.retry_count > 5
        || !(1..=3_650).contains(&config.hit_retention_days)
        || !(1..=365).contains(&config.non_hit_retention_days)
        || config.model.trim().is_empty()
        || config.model.chars().count() > 128
        || config.thresholds.len() > 100
        || config
            .thresholds
            .values()
            .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
    {
        return Err(ApiError::bad_request(
            "INVALID_RISK_CONFIG",
            "risk control settings are outside the supported range",
        ));
    }
    Ok(())
}

fn normalize_base_url(value: &str) -> ApiResult<String> {
    let mut url = url::Url::parse(value.trim()).map_err(|_| {
        ApiError::bad_request("INVALID_MODERATION_URL", "moderation base URL is invalid")
    })?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(ApiError::bad_request(
            "INVALID_MODERATION_URL",
            "moderation base URL must use http or https",
        ));
    }
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.as_str().trim_end_matches('/').to_string())
}

fn normalize_strings(
    values: Vec<String>,
    max_items: usize,
    max_len: usize,
) -> ApiResult<Vec<String>> {
    let mut output = values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    output.sort_by_key(|value| value.to_lowercase());
    output.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
    if output.len() > max_items || output.iter().any(|value| value.chars().count() > max_len) {
        return Err(ApiError::bad_request(
            "INVALID_RISK_LIST",
            "risk control list is too large",
        ));
    }
    Ok(output)
}

fn normalize_ids(mut values: Vec<i64>) -> Vec<i64> {
    values.retain(|value| *value > 0);
    values.sort_unstable();
    values.dedup();
    values
}

fn default_thresholds() -> BTreeMap<String, f64> {
    [
        ("sexual", 0.7),
        ("hate", 0.7),
        ("harassment", 0.7),
        ("self-harm", 0.7),
        ("violence", 0.7),
        ("illicit", 0.7),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_string(), value))
    .collect()
}

#[derive(Deserialize)]
struct TestKeysInput {
    #[serde(default)]
    api_keys: Vec<String>,
    base_url: Option<String>,
    model: Option<String>,
    timeout_ms: Option<u64>,
    prompt: Option<String>,
    #[serde(default)]
    images: Vec<String>,
}

async fn test_api_keys(
    State(state): State<AppState>,
    Json(input): Json<TestKeysInput>,
) -> ApiResult<Json<Value>> {
    let mut config = load_config(&state).await?;
    if let Some(base_url) = input.base_url {
        config.base_url = normalize_base_url(&base_url)?;
    }
    if let Some(model) = input.model {
        config.model = model;
    }
    if let Some(timeout) = input.timeout_ms {
        config.timeout_ms = timeout;
    }
    validate_config(&config)?;
    let stored_keys = input.api_keys.is_empty();
    let keys = if stored_keys {
        decrypt_keys(&state, &config)?
    } else {
        normalize_strings(input.api_keys, 32, 512)?
    };
    if keys.is_empty() {
        return Err(ApiError::bad_request(
            "MODERATION_KEY_REQUIRED",
            "provide at least one moderation API key",
        ));
    }
    let text = input.prompt.unwrap_or_else(|| "health check".into());
    let mut statuses = Vec::with_capacity(keys.len());
    let mut audit_result = None;
    for (index, key) in keys.iter().enumerate() {
        let started = Instant::now();
        let result = if stored_keys {
            call_moderation_tracked(&state, &config, key, &text, &input.images).await
        } else {
            call_moderation(&state, &config, key, &text, &input.images).await
        };
        let latency = started.elapsed().as_millis() as i64;
        match result {
            Ok(result) => {
                if audit_result.is_none() {
                    audit_result = Some(result_json(&result, &config));
                }
                if !stored_keys {
                    statuses.push(transient_key_status(index, key, Ok(&result), latency));
                }
            }
            Err(error) => {
                if !stored_keys {
                    statuses.push(transient_key_status(index, key, Err(&error), latency));
                }
            }
        }
    }
    if stored_keys {
        statuses = key_statuses(&state, &config).await?;
    }
    Ok(Json(json!({"data": {
        "items": statuses,
        "audit_result": audit_result,
        "image_count": input.images.len()
    }})))
}

fn result_json(result: &ModerationResult, config: &StoredConfig) -> Value {
    json!({
        "flagged": result.flagged,
        "highest_category": result.highest_category,
        "highest_score": result.highest_score,
        "composite_score": result.highest_score,
        "category_scores": result.category_scores,
        "thresholds": config.thresholds
    })
}

async fn status(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let config = load_config(&state).await?;
    let (processed, errors, blocked, allowed): (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(SUM(action = 'error'), 0), COALESCE(SUM(action = 'blocked'), 0), \
         COALESCE(SUM(action = 'pass'), 0) FROM risk_control_logs",
    )
    .fetch_one(&state.pool)
    .await?;
    let flagged_hash_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM risk_control_hashes")
        .fetch_one(&state.pool)
        .await?;
    let keys = decrypt_keys(&state, &config)?;
    let key_rows = runtime_rows(&state, &keys).await?;
    let statuses = key_rows
        .iter()
        .enumerate()
        .map(|(index, row)| key_status(index, row))
        .collect::<Vec<_>>();
    let key_loads = key_rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            json!({
                "index": index, "key_hash": row.key_hash, "masked": row.masked,
                "status": if row.frozen { "frozen" } else if !row.last_tested { "unknown" }
                    else if row.last_error.is_empty() { "ok" } else { "error" },
                "active": row.active, "total": row.total, "success": row.successes,
                "errors": row.errors,
                "avg_latency_ms": if row.total > 0 { row.latency_total_ms / row.total } else { 0 },
                "last_latency_ms": row.last_latency_ms,
                "last_http_status": row.last_http_status
            })
        })
        .collect::<Vec<_>>();
    let key_active = key_rows.iter().map(|row| row.active).sum::<i64>();
    let key_available = key_rows.iter().filter(|row| !row.frozen).count();
    let key_total = key_rows.iter().map(|row| row.total).sum::<i64>();
    Ok(Json(json!({"data": {
        "enabled": config.enabled,
        "risk_control_enabled": config.enabled,
        "mode": config.mode,
        "worker_count": config.worker_count,
        "max_workers": 32,
        "active_workers": 0,
        "idle_workers": if config.enabled && config.mode == "observe" { config.worker_count } else { 0 },
        "queue_size": config.queue_size,
        "queue_length": 0,
        "queue_usage_percent": 0,
        "enqueued": processed,
        "dropped": 0,
        "processed": processed,
        "errors": errors,
        "pre_block_active": 0,
        "pre_block_checked": processed,
        "pre_block_allowed": allowed,
        "pre_block_blocked": blocked,
        "pre_block_errors": errors,
        "pre_block_avg_latency_ms": 0,
        "pre_block_api_key_active": key_active,
        "pre_block_api_key_available_count": key_available,
        "pre_block_api_key_total_calls": key_total,
        "pre_block_api_key_loads": key_loads,
        "api_key_statuses": statuses,
        "flagged_hash_count": flagged_hash_count,
        "last_cleanup_at": Value::Null,
        "last_cleanup_deleted_hit": 0,
        "last_cleanup_deleted_non_hit": 0
    }})))
}

#[derive(Deserialize)]
struct LogQuery {
    page: Option<i64>,
    page_size: Option<i64>,
    result: Option<String>,
    group_id: Option<i64>,
    endpoint: Option<String>,
    search: Option<String>,
    from: Option<String>,
    to: Option<String>,
}

#[derive(FromRow)]
struct RiskLogRow {
    id: i64,
    request_id: String,
    user_id: Option<i64>,
    username: String,
    api_key_id: Option<i64>,
    api_key_name: String,
    group_id: Option<i64>,
    group_name: String,
    endpoint: String,
    model: String,
    mode: String,
    action: String,
    flagged: bool,
    highest_category: String,
    highest_score: f64,
    matched_keyword: String,
    category_scores: String,
    threshold_snapshot: String,
    input_hash: String,
    upstream_latency_ms: Option<i64>,
    error_summary: String,
    violation_count: i64,
    auto_banned: bool,
    email_sent: bool,
    user_status: String,
    created_at: String,
}

async fn list_logs(
    State(state): State<AppState>,
    Query(query): Query<LogQuery>,
) -> ApiResult<Json<Value>> {
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(20).clamp(1, 100);
    let result = query.result.filter(|value| !value.is_empty());
    let search = query.search.filter(|value| !value.is_empty());
    let total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM risk_control_logs AS logs \
         LEFT JOIN users ON users.id = logs.user_id LEFT JOIN api_keys ON api_keys.id = logs.api_key_id \
         WHERE (? IS NULL OR CASE ? WHEN 'hit' THEN logs.flagged = 1 WHEN 'blocked' THEN logs.action = 'blocked' \
         WHEN 'pass' THEN logs.action = 'pass' WHEN 'error' THEN logs.action = 'error' ELSE 1 END) \
         AND (? IS NULL OR logs.group_id = ?) AND (? IS NULL OR logs.endpoint = ?) \
         AND (? IS NULL OR users.username LIKE '%' || ? || '%' OR api_keys.name LIKE '%' || ? || '%' \
              OR logs.model LIKE '%' || ? || '%' OR logs.request_id LIKE '%' || ? || '%') \
         AND (? IS NULL OR datetime(logs.created_at) >= datetime(?)) \
         AND (? IS NULL OR datetime(logs.created_at) <= datetime(?))",
    )
    .bind(&result).bind(&result)
    .bind(query.group_id.filter(|value| *value > 0)).bind(query.group_id.filter(|value| *value > 0))
    .bind(&query.endpoint).bind(&query.endpoint)
    .bind(&search).bind(&search).bind(&search).bind(&search).bind(&search)
    .bind(&query.from).bind(&query.from).bind(&query.to).bind(&query.to)
    .fetch_one(&state.pool).await?;
    let rows: Vec<RiskLogRow> = sqlx::query_as(
        "SELECT logs.id, logs.request_id, logs.user_id, COALESCE(users.username, '') AS username, logs.api_key_id, \
         COALESCE(api_keys.name, '') AS api_key_name, logs.group_id, COALESCE(groups.name, '') AS group_name, logs.endpoint, logs.model, \
         logs.mode, logs.action, logs.flagged, logs.highest_category, logs.highest_score, logs.matched_keyword, \
         logs.category_scores, logs.threshold_snapshot, logs.input_hash, logs.upstream_latency_ms, \
         logs.error_summary, logs.violation_count, \
         logs.auto_banned, logs.email_sent, COALESCE(CASE WHEN users.enabled = 1 THEN 'active' ELSE 'disabled' END, '') AS user_status, \
         logs.created_at FROM risk_control_logs AS logs LEFT JOIN users ON users.id = logs.user_id \
         LEFT JOIN api_keys ON api_keys.id = logs.api_key_id LEFT JOIN groups ON groups.id = logs.group_id \
         WHERE (? IS NULL OR CASE ? WHEN 'hit' THEN logs.flagged = 1 WHEN 'blocked' THEN logs.action = 'blocked' \
         WHEN 'pass' THEN logs.action = 'pass' WHEN 'error' THEN logs.action = 'error' ELSE 1 END) \
         AND (? IS NULL OR logs.group_id = ?) AND (? IS NULL OR logs.endpoint = ?) \
         AND (? IS NULL OR users.username LIKE '%' || ? || '%' OR api_keys.name LIKE '%' || ? || '%' \
              OR logs.model LIKE '%' || ? || '%' OR logs.request_id LIKE '%' || ? || '%') \
         AND (? IS NULL OR datetime(logs.created_at) >= datetime(?)) \
         AND (? IS NULL OR datetime(logs.created_at) <= datetime(?)) \
         ORDER BY logs.id DESC LIMIT ? OFFSET ?",
    )
    .bind(&result).bind(&result)
    .bind(query.group_id.filter(|value| *value > 0)).bind(query.group_id.filter(|value| *value > 0))
    .bind(&query.endpoint).bind(&query.endpoint)
    .bind(&search).bind(&search).bind(&search).bind(&search).bind(&search)
    .bind(&query.from).bind(&query.from).bind(&query.to).bind(&query.to)
    .bind(page_size).bind((page - 1) * page_size)
    .fetch_all(&state.pool).await?;
    let items = rows
        .into_iter()
        .map(|row| {
            let category_scores =
                serde_json::from_str::<Value>(&row.category_scores).unwrap_or_else(|_| json!({}));
            let threshold_snapshot = serde_json::from_str::<Value>(&row.threshold_snapshot)
                .unwrap_or_else(|_| json!({}));
            json!({
                "id": row.id, "request_id": row.request_id, "user_id": row.user_id,
                "user_email": row.username, "api_key_id": row.api_key_id,
                "api_key_name": row.api_key_name, "group_id": row.group_id,
                "group_name": row.group_name, "endpoint": row.endpoint,
                "provider": "local/openai", "model": row.model, "mode": row.mode,
                "action": row.action, "flagged": row.flagged,
                "highest_category": row.highest_category, "highest_score": row.highest_score,
                "matched_keyword": row.matched_keyword, "category_scores": category_scores,
                "threshold_snapshot": threshold_snapshot, "input_excerpt": "",
                "input_hash": row.input_hash, "upstream_latency_ms": row.upstream_latency_ms,
                "error": row.error_summary, "violation_count": row.violation_count,
                "auto_banned": row.auto_banned, "email_sent": row.email_sent,
                "user_status": row.user_status, "queue_delay_ms": Value::Null,
                "created_at": row.created_at
            })
        })
        .collect::<Vec<_>>();
    Ok(Json(json!({"data": {
        "items": items, "total": total, "page": page, "page_size": page_size,
        "pages": ((total + page_size - 1) / page_size).max(1)
    }})))
}

async fn unban_user(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    let result =
        sqlx::query("UPDATE users SET enabled = 1, updated_at = CURRENT_TIMESTAMP WHERE id = ?")
            .bind(id)
            .execute(&state.pool)
            .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found("user not found"));
    }
    Ok(Json(json!({"data": {"user_id": id, "status": "active"}})))
}

#[derive(Deserialize)]
struct DeleteHashInput {
    input_hash: String,
}

async fn delete_hash(
    State(state): State<AppState>,
    Json(input): Json<DeleteHashInput>,
) -> ApiResult<Json<Value>> {
    if input.input_hash.len() != 64
        || !input
            .input_hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(ApiError::bad_request(
            "INVALID_INPUT_HASH",
            "input_hash must be a SHA-256 hex digest",
        ));
    }
    let result = sqlx::query("DELETE FROM risk_control_hashes WHERE input_hash = ?")
        .bind(input.input_hash.to_lowercase())
        .execute(&state.pool)
        .await?;
    Ok(Json(
        json!({"data": {"input_hash": input.input_hash, "deleted": result.rows_affected() > 0}}),
    ))
}

async fn clear_hashes(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let result = sqlx::query("DELETE FROM risk_control_hashes")
        .execute(&state.pool)
        .await?;
    Ok(Json(json!({"data": {"deleted": result.rows_affected()}})))
}

pub async fn inspect(
    state: &AppState,
    key: &ApiKeyContext,
    endpoint: &'static str,
    model: Option<&str>,
    value: &Value,
    request_id: &str,
) -> ApiResult<()> {
    let config = load_config(state).await?;
    if !in_scope(&config, key, model, request_id) {
        return Ok(());
    }
    let text = extract_audit_text(value);
    if text.is_empty() {
        return Ok(());
    }
    if config.mode == "observe" {
        let state = state.clone();
        let key = key.clone();
        let model = model.unwrap_or_default().to_string();
        let request_id = request_id.to_string();
        tokio::spawn(async move {
            if let Err(error) =
                run_check(&state, &config, &key, endpoint, &model, &text, &request_id).await
            {
                tracing::warn!(%error, "risk control observation failed");
            }
        });
        return Ok(());
    }
    let outcome = run_check(
        state,
        &config,
        key,
        endpoint,
        model.unwrap_or_default(),
        &text,
        request_id,
    )
    .await?;
    if outcome.flagged {
        let status = StatusCode::from_u16(config.block_status).unwrap_or(StatusCode::FORBIDDEN);
        return Err(ApiError::new(
            status,
            "RISK_CONTROL_BLOCKED",
            config.block_message,
        ));
    }
    Ok(())
}

fn in_scope(
    config: &StoredConfig,
    key: &ApiKeyContext,
    model: Option<&str>,
    request_id: &str,
) -> bool {
    if !config.enabled || config.mode == "off" || config.sample_rate == 0 {
        return false;
    }
    if !config.all_groups
        && key
            .group_id
            .is_none_or(|id| !config.group_ids.contains(&id))
    {
        return false;
    }
    let model = model.unwrap_or_default();
    match config.model_filter.kind.as_str() {
        "include" if !config.model_filter.models.iter().any(|item| item == model) => return false,
        "exclude" if config.model_filter.models.iter().any(|item| item == model) => return false,
        _ => {}
    }
    let sample = Sha256::digest(request_id.as_bytes())[0] % 100;
    sample < config.sample_rate
}

async fn run_check(
    state: &AppState,
    config: &StoredConfig,
    key: &ApiKeyContext,
    endpoint: &str,
    model: &str,
    text: &str,
    request_id: &str,
) -> ApiResult<ModerationResult> {
    let input_hash = hex::encode(Sha256::digest(text.as_bytes()));
    let mut result = ModerationResult::pass();
    if config.pre_hash_check_enabled {
        let known: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM risk_control_hashes WHERE input_hash = ?")
                .bind(&input_hash)
                .fetch_one(&state.pool)
                .await?;
        if known > 0 {
            result.flagged = true;
            result.highest_category = "known_hash".into();
            result.highest_score = 1.0;
        }
    }
    if !result.flagged && config.keyword_blocking_mode != "api_only" {
        let lower = text.to_lowercase();
        if let Some(keyword) = config
            .blocked_keywords
            .iter()
            .find(|keyword| lower.contains(&keyword.to_lowercase()))
        {
            result.flagged = true;
            result.highest_category = "blocked_keyword".into();
            result.highest_score = 1.0;
            result.matched_keyword = keyword.clone();
        }
    }
    if !result.flagged && config.keyword_blocking_mode != "keyword_only" {
        let keys = decrypt_keys(state, config)?;
        if !keys.is_empty() {
            match call_moderation_with_keys(state, config, &keys, text, request_id).await {
                Ok(value) => result = value,
                Err(error) => result.error = error.message,
            }
        } else if config.keyword_blocking_mode == "api_only" {
            result.error = "no moderation API key is configured".into();
        }
    }
    let should_record = result.flagged || !result.error.is_empty() || config.record_non_hits;
    if should_record {
        record_result(
            state,
            config,
            key,
            endpoint,
            model,
            request_id,
            &input_hash,
            &mut result,
        )
        .await?;
    }
    Ok(result)
}

async fn call_moderation_with_keys(
    state: &AppState,
    config: &StoredConfig,
    keys: &[String],
    text: &str,
    request_id: &str,
) -> ApiResult<ModerationResult> {
    let digest = Sha256::digest(request_id.as_bytes());
    let start = u64::from_be_bytes(digest[..8].try_into().unwrap()) as usize % keys.len();
    let rows = runtime_rows(state, keys).await?;
    let mut candidates = keys
        .iter()
        .enumerate()
        .zip(rows.iter())
        .filter(|(_, row)| !row.frozen)
        .map(|((index, key), row)| (index, key, row.active))
        .collect::<Vec<_>>();
    candidates
        .sort_by_key(|(index, _, active)| (*active, (*index + keys.len() - start) % keys.len()));
    if candidates.is_empty() {
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "MODERATION_KEYS_FROZEN",
            "all moderation API keys are temporarily frozen",
        ));
    }
    let attempts = candidates.len().min(config.retry_count as usize + 1);
    let mut last_error = None;
    for (_, key, _) in candidates.into_iter().take(attempts) {
        match call_moderation_tracked(state, config, key, text, &[]).await {
            Ok(result) => return Ok(result),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error
        .map(ModerationCallError::into_api_error)
        .unwrap_or_else(|| {
            ApiError::new(
                StatusCode::BAD_GATEWAY,
                "MODERATION_UPSTREAM_ERROR",
                "all moderation API keys failed",
            )
        }))
}

async fn record_result(
    state: &AppState,
    config: &StoredConfig,
    key: &ApiKeyContext,
    endpoint: &str,
    model: &str,
    request_id: &str,
    input_hash: &str,
    result: &mut ModerationResult,
) -> ApiResult<()> {
    let violation_count = if result.flagged {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) + 1 FROM risk_control_logs WHERE user_id = ? AND flagged = 1 \
             AND datetime(created_at) >= datetime('now', '-' || ? || ' hours')",
        )
        .bind(key.user_id)
        .bind(config.violation_window_hours as i64)
        .fetch_one(&state.pool)
        .await?
    } else {
        0
    };
    let mut auto_banned = false;
    if result.flagged && config.auto_ban_enabled && violation_count >= config.ban_threshold as i64 {
        if let Some(user_id) = key.user_id {
            let role: Option<String> = sqlx::query_scalar("SELECT role FROM users WHERE id = ?")
                .bind(user_id)
                .fetch_optional(&state.pool)
                .await?;
            if role.as_deref() != Some("admin") {
                sqlx::query(
                    "UPDATE users SET enabled = 0, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
                )
                .bind(user_id)
                .execute(&state.pool)
                .await?;
                auto_banned = true;
            }
        }
    }
    let action = if !result.error.is_empty() {
        "error"
    } else if result.flagged && config.mode == "pre_block" {
        "blocked"
    } else if result.flagged {
        "hit"
    } else {
        "pass"
    };
    let scores = serde_json::to_string(&result.category_scores).unwrap_or_else(|_| "{}".into());
    let thresholds = serde_json::to_string(&config.thresholds).unwrap_or_else(|_| "{}".into());
    let inserted = sqlx::query(
        "INSERT INTO risk_control_logs (request_id, user_id, api_key_id, group_id, endpoint, model, mode, \
         action, flagged, highest_category, highest_score, matched_keyword, category_scores, threshold_snapshot, \
         input_hash, upstream_latency_ms, error_summary, violation_count, auto_banned) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(request_id).bind(key.user_id).bind(key.id).bind(key.group_id).bind(endpoint).bind(model)
    .bind(&config.mode).bind(action).bind(result.flagged).bind(&result.highest_category)
    .bind(result.highest_score).bind(&result.matched_keyword).bind(scores).bind(thresholds)
    .bind(input_hash).bind(result.latency_ms).bind(result.error.chars().take(500).collect::<String>())
    .bind(violation_count).bind(auto_banned).execute(&state.pool).await?;
    if result.flagged {
        sqlx::query(
            "INSERT INTO risk_control_hashes (input_hash, first_log_id) VALUES (?, ?) \
             ON CONFLICT(input_hash) DO UPDATE SET hit_count = hit_count + 1, last_seen_at = CURRENT_TIMESTAMP",
        )
        .bind(input_hash).bind(inserted.last_insert_rowid()).execute(&state.pool).await?;
    }
    if result.flagged && (config.email_on_hit || auto_banned) {
        match deliver_risk_email(
            state,
            key.user_id,
            request_id,
            model,
            &result.highest_category,
            result.highest_score,
            violation_count,
            auto_banned,
        )
        .await
        {
            Ok(true) => {
                sqlx::query("UPDATE risk_control_logs SET email_sent = 1 WHERE id = ?")
                    .bind(inserted.last_insert_rowid())
                    .execute(&state.pool)
                    .await?;
            }
            Ok(false) => {}
            Err(error) => tracing::warn!(%error, %request_id, "risk control email delivery failed"),
        }
    }
    Ok(())
}

async fn deliver_risk_email(
    state: &AppState,
    user_id: Option<i64>,
    request_id: &str,
    model: &str,
    category: &str,
    score: f64,
    violation_count: i64,
    auto_banned: bool,
) -> ApiResult<bool> {
    let Some(user_id) = user_id else {
        return Ok(false);
    };
    let user: Option<(Option<String>, String)> =
        sqlx::query_as("SELECT email, display_name FROM users WHERE id = ?")
            .bind(user_id)
            .fetch_optional(&state.pool)
            .await?;
    let Some((Some(recipient), display_name)) = user else {
        return Ok(false);
    };
    if !crate::mail::is_configured(state).await? {
        return Ok(false);
    }
    let title = if auto_banned {
        "Sub2API Mini account disabled by risk control"
    } else {
        "Sub2API Mini risk control notice"
    };
    let message = format!(
        "Request {request_id} triggered category {category} ({score:.4}) on model {model}. Violation count: {violation_count}. Account disabled: {auto_banned}."
    );
    let body = json!({
        "kind": if auto_banned { "risk_control_account_disabled" } else { "risk_control_violation" },
        "to": recipient, "recipient_name": display_name, "site_name": "Sub2API Mini",
        "title": title, "message": message, "severity": if auto_banned { "critical" } else { "warning" },
        "request_id": request_id, "model": model, "category": category,
        "score": score, "violation_count": violation_count, "auto_banned": auto_banned
    });
    let html = format!(
        "<h2>{}</h2><p>{}</p><p>Request ID: <code>{}</code></p>",
        crate::mail::escape_html(title),
        crate::mail::escape_html(&message),
        crate::mail::escape_html(request_id)
    );
    crate::mail::deliver(state, body, &recipient, title, &html).await?;
    Ok(true)
}

async fn call_moderation_tracked(
    state: &AppState,
    config: &StoredConfig,
    key: &str,
    text: &str,
    images: &[String],
) -> Result<ModerationResult, ModerationCallError> {
    ensure_key_runtime_rows(state, &[key.to_string()])
        .await
        .map_err(|error| ModerationCallError::upstream(error.message, 0))?;
    let hash = token_hash(key);
    sqlx::query(
        "UPDATE risk_control_api_key_runtime SET active = active + 1, updated_at = CURRENT_TIMESTAMP \
         WHERE key_hash = ?",
    )
    .bind(&hash)
    .execute(&state.pool)
    .await
    .map_err(|_| ModerationCallError::upstream("failed to update moderation key load", 0))?;
    let mut load_guard = KeyLoadGuard::new(state.pool.clone(), hash.clone());
    let started = Instant::now();
    let result = call_moderation(state, config, key, text, images).await;
    let latency_ms = started.elapsed().as_millis() as i64;
    let (success, http_status, error, freeze_seconds) = match &result {
        Ok(_) => (true, 200, String::new(), 0),
        Err(error) => (
            false,
            error.http_status,
            error.message.chars().take(180).collect::<String>(),
            freeze_seconds(error.http_status),
        ),
    };
    let frozen_until = if freeze_seconds > 0 {
        Some((Utc::now() + chrono::Duration::seconds(freeze_seconds)).to_rfc3339())
    } else {
        None
    };
    let update = if success {
        sqlx::query(
            "UPDATE risk_control_api_key_runtime SET active = MAX(active - 1, 0), total = total + 1, \
             successes = successes + 1, success_count = success_count + 1, failure_count = 0, \
             last_error = '', last_checked_at = CURRENT_TIMESTAMP, frozen_until = NULL, \
             last_latency_ms = ?, last_http_status = 200, last_tested = 1, \
             latency_total_ms = latency_total_ms + ?, updated_at = CURRENT_TIMESTAMP WHERE key_hash = ?",
        )
        .bind(latency_ms)
        .bind(latency_ms)
        .bind(&hash)
        .execute(&state.pool)
        .await
    } else {
        sqlx::query(
            "UPDATE risk_control_api_key_runtime SET active = MAX(active - 1, 0), total = total + 1, \
             errors = errors + 1, failure_count = failure_count + CASE WHEN ? > 0 THEN 1 ELSE 0 END, \
             last_error = ?, last_checked_at = CURRENT_TIMESTAMP, frozen_until = ?, \
             last_latency_ms = ?, last_http_status = ?, last_tested = 1, \
             latency_total_ms = latency_total_ms + ?, updated_at = CURRENT_TIMESTAMP WHERE key_hash = ?",
        )
        .bind(freeze_seconds)
        .bind(error)
        .bind(frozen_until)
        .bind(latency_ms)
        .bind(http_status)
        .bind(latency_ms)
        .bind(&hash)
        .execute(&state.pool)
        .await
    };
    if update.is_err() {
        tracing::warn!(key_hash = %hash, "failed to update moderation key outcome");
    } else {
        load_guard.disarm();
    }
    result
}

fn freeze_seconds(http_status: i64) -> i64 {
    match http_status {
        0 | 400 => 0,
        401 | 403 => 600,
        429 | 529 => 60,
        _ => 10,
    }
}

async fn call_moderation(
    state: &AppState,
    config: &StoredConfig,
    key: &str,
    text: &str,
    images: &[String],
) -> Result<ModerationResult, ModerationCallError> {
    let url = if config.base_url.ends_with("/v1") {
        format!("{}/moderations", config.base_url)
    } else {
        format!("{}/v1/moderations", config.base_url)
    };
    let input = if images.is_empty() {
        Value::String(text.to_string())
    } else {
        let mut content = vec![json!({"type": "text", "text": text})];
        content.extend(
            images
                .iter()
                .map(|url| json!({"type": "image_url", "image_url": {"url": url}})),
        );
        json!([{"type": "message", "role": "user", "content": content}])
    };
    let started = Instant::now();
    let response = state
        .client
        .post(url)
        .bearer_auth(key)
        .timeout(std::time::Duration::from_millis(config.timeout_ms))
        .json(&json!({"model": config.model, "input": input}))
        .send()
        .await
        .map_err(|_| ModerationCallError::upstream("moderation upstream request failed", 0))?;
    let status = response.status();
    if !status.is_success() {
        return Err(ModerationCallError::upstream(
            format!("moderation API returned {status}"),
            i64::from(status.as_u16()),
        ));
    }
    let value: Value = response
        .json()
        .await
        .map_err(|_| ModerationCallError::upstream("moderation API returned invalid JSON", 200))?;
    parse_moderation_result(value, config, started.elapsed().as_millis() as i64)
        .map_err(|error| ModerationCallError::upstream(error.message, 200))
}

fn parse_moderation_result(
    value: Value,
    config: &StoredConfig,
    latency_ms: i64,
) -> ApiResult<ModerationResult> {
    let first = value
        .get("results")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::BAD_GATEWAY,
                "INVALID_MODERATION_RESPONSE",
                "moderation API returned an invalid response",
            )
        })?;
    let scores = first
        .get("category_scores")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut category_scores = BTreeMap::new();
    let mut highest_category = String::new();
    let mut highest_score = 0.0_f64;
    let mut threshold_hit = false;
    for (category, score) in scores {
        let score = score.as_f64().unwrap_or(0.0).clamp(0.0, 1.0);
        if score > highest_score {
            highest_score = score;
            highest_category = category.clone();
        }
        if score >= config.thresholds.get(&category).copied().unwrap_or(1.0) {
            threshold_hit = true;
        }
        category_scores.insert(category, score);
    }
    Ok(ModerationResult {
        flagged: threshold_hit
            || first
                .get("flagged")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        highest_category,
        highest_score,
        matched_keyword: String::new(),
        category_scores,
        latency_ms: Some(latency_ms),
        error: String::new(),
    })
}

fn extract_audit_text(value: &Value) -> String {
    fn visit(value: &Value, output: &mut String) {
        if output.len() >= MAX_AUDIT_TEXT_BYTES {
            return;
        }
        match value {
            Value::String(text) => {
                if !output.is_empty() {
                    output.push('\n');
                }
                let remaining = MAX_AUDIT_TEXT_BYTES.saturating_sub(output.len());
                output.extend(text.chars().take(remaining));
            }
            Value::Array(values) => values.iter().for_each(|value| visit(value, output)),
            Value::Object(values) => {
                for key in [
                    "input",
                    "messages",
                    "content",
                    "prompt",
                    "text",
                    "instructions",
                ] {
                    if let Some(value) = values.get(key) {
                        visit(value, output);
                    }
                }
            }
            _ => {}
        }
    }
    let mut output = String::new();
    visit(value, &mut output);
    output
}

async fn cleanup_logs(state: &AppState, config: &StoredConfig) {
    let _ = sqlx::query(
        "DELETE FROM risk_control_logs WHERE (flagged = 1 AND datetime(created_at) < datetime('now', '-' || ? || ' days')) \
         OR (flagged = 0 AND datetime(created_at) < datetime('now', '-' || ? || ' days'))",
    )
    .bind(config.hit_retention_days as i64).bind(config.non_hit_retention_days as i64)
    .execute(&state.pool).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{crypto::hash_password, test_support};
    use axum::{Router, extract::State as AxumState, http::HeaderMap, routing::post};
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[test]
    fn audit_text_only_collects_request_content() {
        let value = json!({
            "model": "secret-model-name",
            "messages": [{"role": "user", "content": "hello"}],
            "metadata": {"password": "do-not-audit"}
        });
        let text = extract_audit_text(&value);
        assert!(text.contains("hello"));
        assert!(!text.contains("secret-model-name"));
        assert!(!text.contains("do-not-audit"));
    }

    #[test]
    fn model_and_sampling_scope_is_deterministic() {
        let mut config = StoredConfig {
            enabled: true,
            sample_rate: 100,
            ..StoredConfig::default()
        };
        let key = ApiKeyContext {
            id: 1,
            user_id: Some(2),
            allowed_models: vec![],
            group_id: Some(3),
        };
        assert!(in_scope(&config, &key, Some("gpt-5"), "request-a"));
        config.model_filter = ModelFilter {
            kind: "include".into(),
            models: vec!["gpt-4".into()],
        };
        assert!(!in_scope(&config, &key, Some("gpt-5"), "request-a"));
    }

    #[test]
    fn moderation_threshold_can_flag_an_upstream_pass() {
        let config = StoredConfig::default();
        let value = json!({"results": [{"flagged": false, "category_scores": {"violence": 0.9}}]});
        let result = parse_moderation_result(value, &config, 8).unwrap();
        assert!(result.flagged);
        assert_eq!(result.highest_category, "violence");
        assert_eq!(result.latency_ms, Some(8));
    }

    #[tokio::test]
    async fn keyword_blocking_hashes_input_and_auto_bans_a_user() {
        let (_directory, state) = test_support::state().await;
        let user_id = sqlx::query(
            "INSERT INTO users (username, display_name, password_hash, role) VALUES ('risk-user', 'Risk User', ?, 'user')",
        )
        .bind(hash_password("test-password").unwrap())
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        let key_id = sqlx::query(
            "INSERT INTO api_keys (name, token_prefix, token_hash, user_id) VALUES ('risk-key', 'sk-mini_test', 'hash', ?)",
        )
        .bind(user_id)
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        let config = StoredConfig {
            enabled: true,
            keyword_blocking_mode: "keyword_only".into(),
            blocked_keywords: vec!["blockedword".into()],
            ban_threshold: 1,
            ..StoredConfig::default()
        };
        save_config(&state, &config).await.unwrap();
        let context = ApiKeyContext {
            id: key_id,
            user_id: Some(user_id),
            allowed_models: vec![],
            group_id: None,
        };
        let value =
            json!({"model": "gpt-test", "input": "private-prefix blockedword private-suffix"});
        let error = inspect(
            &state,
            &context,
            "/v1/responses",
            Some("gpt-test"),
            &value,
            "risk-request",
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "RISK_CONTROL_BLOCKED");
        let enabled: bool = sqlx::query_scalar("SELECT enabled FROM users WHERE id = ?")
            .bind(user_id)
            .fetch_one(&state.pool)
            .await
            .unwrap();
        assert!(!enabled);
        let row: (String, String, String) = sqlx::query_as(
            "SELECT input_hash, matched_keyword, error_summary FROM risk_control_logs",
        )
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(row.0.len(), 64);
        assert_eq!(row.1, "blockedword");
        assert!(!row.2.contains("private-prefix"));
        let stored: String = sqlx::query_scalar("SELECT value FROM app_settings WHERE key = ?")
            .bind(CONFIG_KEY)
            .fetch_one(&state.pool)
            .await
            .unwrap();
        assert!(!stored.contains("private-prefix"));
    }

    #[tokio::test]
    async fn moderation_api_keys_are_encrypted_in_settings() {
        let (_directory, state) = test_support::state().await;
        let mut config = StoredConfig::default();
        config.encrypted_api_keys = vec![state.crypto.encrypt(b"sk-moderation-plaintext").unwrap()];
        save_config(&state, &config).await.unwrap();
        let stored: String = sqlx::query_scalar("SELECT value FROM app_settings WHERE key = ?")
            .bind(CONFIG_KEY)
            .fetch_one(&state.pool)
            .await
            .unwrap();
        assert!(!stored.contains("sk-moderation-plaintext"));
        assert_eq!(
            decrypt_keys(&state, &config).unwrap(),
            vec!["sk-moderation-plaintext"]
        );
    }

    #[tokio::test]
    async fn moderation_key_load_and_freeze_survive_requests() {
        async fn moderate(headers: HeaderMap) -> (StatusCode, Json<Value>) {
            if headers
                .get("authorization")
                .and_then(|value| value.to_str().ok())
                == Some("Bearer sk-rate-limited")
            {
                return (StatusCode::TOO_MANY_REQUESTS, Json(json!({})));
            }
            (
                StatusCode::OK,
                Json(
                    json!({"results": [{"flagged": false, "category_scores": {"violence": 0.1}}]}),
                ),
            )
        }

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route("/v1/moderations", post(moderate)),
            )
            .await
            .unwrap()
        });
        let (_directory, state) = test_support::state().await;
        let keys = vec!["sk-rate-limited".to_string(), "sk-healthy".to_string()];
        ensure_key_runtime_rows(&state, &keys).await.unwrap();
        sqlx::query("UPDATE risk_control_api_key_runtime SET active = 5 WHERE key_hash = ?")
            .bind(token_hash("sk-healthy"))
            .execute(&state.pool)
            .await
            .unwrap();
        let config = StoredConfig {
            base_url: format!("http://{address}"),
            retry_count: 1,
            ..StoredConfig::default()
        };
        let result =
            call_moderation_with_keys(&state, &config, &keys, "safe request", "load-test-request")
                .await
                .unwrap();
        assert!(!result.flagged);
        let rows = runtime_rows(&state, &keys).await.unwrap();
        assert!(rows[0].frozen);
        assert_eq!(rows[0].last_http_status, 429);
        assert_eq!(rows[0].errors, 1);
        assert_eq!(rows[1].successes, 1);
        assert_eq!(rows[1].active, 5);
        initialize(&state).await.unwrap();
        let rows = runtime_rows(&state, &keys).await.unwrap();
        assert_eq!(rows[1].active, 0);
        assert!(rows[0].frozen);
        task.abort();
    }

    #[tokio::test]
    async fn flagged_request_delivers_redacted_email_and_updates_log() {
        async fn capture_mail(
            AxumState(captured): AxumState<Arc<Mutex<Option<Value>>>>,
            Json(body): Json<Value>,
        ) -> StatusCode {
            *captured.lock().await = Some(body);
            StatusCode::NO_CONTENT
        }

        let captured = Arc::new(Mutex::new(None));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server_state = captured.clone();
        let task = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new()
                    .route("/mail", post(capture_mail))
                    .with_state(server_state),
            )
            .await
            .unwrap()
        });
        let (_directory, mut state) = test_support::state().await;
        state.config.mail_webhook_url = Some(format!("http://{address}/mail"));
        let user_id = sqlx::query(
            "INSERT INTO users (username, display_name, password_hash, role, email, email_verified) \
             VALUES ('mail-risk-user', 'Mail Risk User', ?, 'user', 'risk@example.com', 1)",
        )
        .bind(hash_password("test-password").unwrap())
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        let key_id = sqlx::query(
            "INSERT INTO api_keys (name, token_prefix, token_hash, user_id) \
             VALUES ('mail-risk-key', 'sk-mini_mail', 'mail-hash', ?)",
        )
        .bind(user_id)
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        let config = StoredConfig {
            enabled: true,
            keyword_blocking_mode: "keyword_only".into(),
            blocked_keywords: vec!["private-trigger".into()],
            email_on_hit: true,
            auto_ban_enabled: false,
            ..StoredConfig::default()
        };
        save_config(&state, &config).await.unwrap();
        let context = ApiKeyContext {
            id: key_id,
            user_id: Some(user_id),
            allowed_models: vec![],
            group_id: None,
        };
        let body = json!({"model": "gpt-test", "input": "secret-body private-trigger"});
        let error = inspect(
            &state,
            &context,
            "/v1/responses",
            Some("gpt-test"),
            &body,
            "mail-risk-request",
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "RISK_CONTROL_BLOCKED");
        let email_sent: bool =
            sqlx::query_scalar("SELECT email_sent FROM risk_control_logs WHERE request_id = ?")
                .bind("mail-risk-request")
                .fetch_one(&state.pool)
                .await
                .unwrap();
        assert!(email_sent);
        let mail = captured.lock().await.clone().unwrap();
        assert_eq!(mail["to"], "risk@example.com");
        assert_eq!(mail["kind"], "risk_control_violation");
        assert!(!mail.to_string().contains("secret-body"));
        task.abort();
    }
}
