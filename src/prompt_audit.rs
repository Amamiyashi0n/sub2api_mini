use std::{collections::BTreeMap, time::Instant};

use axum::{
    Json, Router,
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use chrono::{Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, QueryBuilder, Sqlite};

use crate::{
    auth::AuthSession,
    crypto::{random_token, token_hash},
    error::{ApiError, ApiResult},
    models::ApiKeyContext,
    state::AppState,
};

const CONFIG_KEY: &str = "prompt_audit_config";
const DEFAULT_MODEL: &str = "sileader/qwen3guard:0.6b";
const MAX_ENDPOINTS: usize = 16;
const MAX_RESPONSE_BYTES: usize = 64 * 1024;
const SCANNERS: [&str; 9] = [
    "violent",
    "non_violent_illegal_acts",
    "sexual_content_or_sexual_acts",
    "pii",
    "suicide_and_self_harm",
    "unethical_acts",
    "politically_sensitive_topics",
    "copyright_violation",
    "jailbreak",
];

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
struct StoredConfig {
    enabled: bool,
    blocking_enabled: bool,
    store_pass_events: bool,
    strategy: String,
    worker_count: u8,
    queue_capacity: u32,
    scanners: Vec<String>,
    all_groups: bool,
    group_ids: Vec<i64>,
    endpoints: Vec<StoredEndpoint>,
    config_version: i64,
    updated_at: String,
    updated_by: i64,
    change_summary: String,
}

impl Default for StoredConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            blocking_enabled: false,
            store_pass_events: false,
            strategy: "priority".into(),
            worker_count: 2,
            queue_capacity: 1_024,
            scanners: SCANNERS.iter().map(|value| (*value).to_string()).collect(),
            all_groups: true,
            group_ids: Vec::new(),
            endpoints: Vec::new(),
            config_version: 1,
            updated_at: Utc::now().to_rfc3339(),
            updated_by: 0,
            change_summary: "default configuration".into(),
        }
    }
}

impl StoredConfig {
    fn effective_mode(&self) -> &'static str {
        if !self.enabled {
            "off"
        } else if self.blocking_enabled {
            "blocking"
        } else {
            "async_audit"
        }
    }

    fn includes_group(&self, group_id: Option<i64>) -> bool {
        self.all_groups || group_id.is_some_and(|id| self.group_ids.contains(&id))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
struct StoredEndpoint {
    id: String,
    name: String,
    protocol: String,
    base_url: String,
    model: String,
    encrypted_token: String,
    timeout_ms: u64,
    input_limit: usize,
    enabled: bool,
}

impl Default for StoredEndpoint {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            protocol: "openai_compatible".into(),
            base_url: String::new(),
            model: DEFAULT_MODEL.into(),
            encrypted_token: String::new(),
            timeout_ms: 3_000,
            input_limit: 4_000,
            enabled: true,
        }
    }
}

#[derive(Debug, Deserialize)]
struct UpdateConfig {
    expected_config_version: i64,
    enabled: bool,
    blocking_enabled: bool,
    store_pass_events: bool,
    strategy: String,
    worker_count: u8,
    queue_capacity: u32,
    #[serde(default)]
    scanners: Vec<String>,
    all_groups: bool,
    #[serde(default)]
    group_ids: Vec<i64>,
    #[serde(default)]
    endpoints: Vec<UpdateEndpoint>,
}

#[derive(Clone, Debug, Deserialize)]
struct UpdateEndpoint {
    id: String,
    name: String,
    #[serde(default = "default_protocol")]
    protocol: String,
    base_url: String,
    #[serde(default = "default_model")]
    model: String,
    token: Option<String>,
    #[serde(default)]
    clear_token: bool,
    timeout_ms: u64,
    input_limit: usize,
    enabled: bool,
}

fn default_protocol() -> String {
    "openai_compatible".into()
}

fn default_model() -> String {
    DEFAULT_MODEL.into()
}

#[derive(Clone, Debug)]
struct PromptSnapshot {
    request_id: String,
    user_id: Option<i64>,
    username: String,
    user_email: String,
    api_key_id: i64,
    api_key_name: String,
    group_id: Option<i64>,
    group_name: String,
    endpoint: String,
    model: String,
    prompt_hash: String,
    redacted_preview: String,
    prompt_length: usize,
    message_count: usize,
}

#[derive(Clone, Debug)]
struct ScanResult {
    decision: String,
    risk_level: String,
    action: String,
    categories: Vec<String>,
    matched_scanners: Vec<String>,
    scanner_scores: BTreeMap<String, f64>,
    scanner_evidence: BTreeMap<String, String>,
    guard_endpoint_id: String,
    chunk_total: usize,
    latency_ms: i64,
}

pub fn admin_router() -> Router<AppState> {
    Router::new()
        .route("/prompt-audit/config", get(get_config).put(update_config))
        .route("/prompt-audit/endpoints/probe", post(probe_endpoint))
        .route("/prompt-audit/runtime", get(runtime))
        .route("/prompt-audit/events", get(list_events))
        .route(
            "/prompt-audit/events/batch-delete",
            post(batch_delete_events),
        )
        .route("/prompt-audit/events/delete-preview", post(delete_preview))
        .route(
            "/prompt-audit/events/delete-by-filter",
            post(delete_by_filter),
        )
        .route(
            "/prompt-audit/events/{id}",
            get(get_event).delete(delete_event),
        )
}

pub async fn initialize(state: &AppState) -> ApiResult<()> {
    sqlx::query(
        "UPDATE prompt_audit_jobs SET status = 'failed', processed_at = CURRENT_TIMESTAMP, \
         processing_started_at = NULL, last_error_code = 'PROCESS_RESTARTED', \
         last_error_message = 'prompt content was intentionally not persisted and cannot be resumed', \
         updated_at = CURRENT_TIMESTAMP WHERE status IN ('staging', 'queued', 'processing', 'retry')",
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
            .map_err(|_| ApiError::internal("stored prompt audit config is malformed")),
        _ => Ok(StoredConfig::default()),
    }
}

async fn save_config(state: &AppState, config: &StoredConfig) -> ApiResult<()> {
    let value = serde_json::to_string(config)
        .map_err(|_| ApiError::internal("prompt audit config serialization failed"))?;
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

fn public_config(config: &StoredConfig) -> Value {
    json!({
        "enabled": config.enabled,
        "blocking_enabled": config.blocking_enabled,
        "store_pass_events": config.store_pass_events,
        "effective_mode": config.effective_mode(),
        "strategy": config.strategy,
        "worker_count": config.worker_count,
        "queue_capacity": config.queue_capacity,
        "scanners": config.scanners,
        "all_groups": config.all_groups,
        "group_ids": config.group_ids,
        "endpoints": config.endpoints.iter().map(|endpoint| json!({
            "id": endpoint.id,
            "name": endpoint.name,
            "protocol": endpoint.protocol,
            "base_url": endpoint.base_url,
            "model": endpoint.model,
            "timeout_ms": endpoint.timeout_ms,
            "input_limit": endpoint.input_limit,
            "enabled": endpoint.enabled,
            "has_token": !endpoint.encrypted_token.is_empty(),
            "token_status": if endpoint.encrypted_token.is_empty() { "missing" } else { "configured" }
        })).collect::<Vec<_>>(),
        "config_version": config.config_version,
        "updated_at": config.updated_at,
        "updated_by": config.updated_by,
        "change_summary": config.change_summary
    })
}

async fn get_config(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let config = load_config(&state).await?;
    Ok(Json(json!({"data": public_config(&config)})))
}

async fn update_config(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
    Json(input): Json<UpdateConfig>,
) -> ApiResult<Json<Value>> {
    let current = load_config(&state).await?;
    if input.expected_config_version != current.config_version {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "PROMPT_AUDIT_CONFIG_CONFLICT",
            "prompt audit configuration was updated by another administrator",
        ));
    }
    if input.endpoints.len() > MAX_ENDPOINTS {
        return Err(ApiError::bad_request(
            "PROMPT_AUDIT_INVALID_ENDPOINT",
            "at most 16 prompt audit endpoints are supported",
        ));
    }
    let mut endpoints = Vec::with_capacity(input.endpoints.len());
    for endpoint in input.endpoints {
        let id = endpoint.id.trim().to_string();
        let base_url = normalize_base_url(&endpoint.base_url)?;
        let old = current
            .endpoints
            .iter()
            .find(|item| item.id == id && item.base_url == base_url);
        let token = endpoint.token.unwrap_or_default();
        let encrypted_token = if !token.trim().is_empty() {
            state.crypto.encrypt(token.trim().as_bytes())?
        } else if endpoint.clear_token {
            String::new()
        } else {
            old.map(|item| item.encrypted_token.clone())
                .unwrap_or_default()
        };
        endpoints.push(StoredEndpoint {
            id,
            name: endpoint.name.trim().to_string(),
            protocol: endpoint.protocol.trim().to_string(),
            base_url,
            model: endpoint.model.trim().to_string(),
            encrypted_token,
            timeout_ms: endpoint.timeout_ms,
            input_limit: endpoint.input_limit,
            enabled: endpoint.enabled,
        });
    }
    let mut config = StoredConfig {
        enabled: input.enabled,
        blocking_enabled: input.enabled && input.blocking_enabled,
        store_pass_events: input.store_pass_events,
        strategy: input.strategy,
        worker_count: input.worker_count,
        queue_capacity: input.queue_capacity,
        scanners: normalize_scanners(input.scanners)?,
        all_groups: input.all_groups,
        group_ids: normalize_ids(input.group_ids),
        endpoints,
        config_version: current.config_version + 1,
        updated_at: Utc::now().to_rfc3339(),
        updated_by: session.user_id,
        change_summary: String::new(),
    };
    validate_config(&config)?;
    config.change_summary = format!(
        "mode={}, endpoints={}, scanners={}, groups={}",
        config.effective_mode(),
        config.endpoints.len(),
        config.scanners.len(),
        if config.all_groups {
            "all".into()
        } else {
            config.group_ids.len().to_string()
        }
    );
    save_config(&state, &config).await?;
    Ok(Json(json!({"data": public_config(&config)})))
}

fn validate_config(config: &StoredConfig) -> ApiResult<()> {
    let invalid_endpoint = config.endpoints.iter().any(|endpoint| {
        endpoint.id.is_empty()
            || endpoint.id.len() > 128
            || endpoint.name.is_empty()
            || endpoint.name.chars().count() > 128
            || endpoint.protocol != "openai_compatible"
            || endpoint.model.is_empty()
            || endpoint.model.chars().count() > 255
            || !(100..=30_000).contains(&endpoint.timeout_ms)
            || !(128..=100_000).contains(&endpoint.input_limit)
    });
    let mut ids = config
        .endpoints
        .iter()
        .map(|endpoint| endpoint.id.as_str())
        .collect::<Vec<_>>();
    ids.sort_unstable();
    if config.strategy != "priority"
        || !(1..=32).contains(&config.worker_count)
        || !(1..=100_000).contains(&config.queue_capacity)
        || invalid_endpoint
        || ids.windows(2).any(|pair| pair[0] == pair[1])
        || (config.enabled && !config.endpoints.iter().any(|endpoint| endpoint.enabled))
        || (config.enabled && config.scanners.is_empty())
        || (!config.all_groups && config.group_ids.is_empty())
    {
        return Err(ApiError::bad_request(
            "PROMPT_AUDIT_INVALID_CONFIG",
            "prompt audit settings are invalid",
        ));
    }
    Ok(())
}

fn normalize_base_url(value: &str) -> ApiResult<String> {
    let mut url = url::Url::parse(value.trim()).map_err(|_| {
        ApiError::bad_request(
            "PROMPT_AUDIT_INVALID_BASE_URL",
            "prompt audit endpoint URL is invalid",
        )
    })?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ApiError::bad_request(
            "PROMPT_AUDIT_INVALID_BASE_URL",
            "prompt audit endpoint must be an HTTP(S) base URL without credentials, query, or fragment",
        ));
    }
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.as_str().trim_end_matches('/').to_string())
}

fn normalize_ids(mut values: Vec<i64>) -> Vec<i64> {
    values.retain(|value| *value > 0);
    values.sort_unstable();
    values.dedup();
    values
}

fn normalize_scanners(values: Vec<String>) -> ApiResult<Vec<String>> {
    let mut output = values
        .into_iter()
        .map(|value| normalize_category(&value))
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    output.sort_by_key(|value| {
        SCANNERS
            .iter()
            .position(|item| item == value)
            .unwrap_or(usize::MAX)
    });
    output.dedup();
    if output
        .iter()
        .any(|value| !SCANNERS.contains(&value.as_str()))
    {
        return Err(ApiError::bad_request(
            "PROMPT_AUDIT_SCANNERS_INVALID",
            "prompt audit scanner list contains an unknown category",
        ));
    }
    Ok(output)
}

#[derive(Deserialize)]
struct ProbeInput {
    endpoint: UpdateEndpoint,
}

async fn probe_endpoint(
    State(state): State<AppState>,
    Json(input): Json<ProbeInput>,
) -> ApiResult<Json<Value>> {
    let current = load_config(&state).await?;
    let base_url = normalize_base_url(&input.endpoint.base_url)?;
    let token = if input
        .endpoint
        .token
        .as_deref()
        .is_some_and(|token| !token.trim().is_empty())
    {
        input.endpoint.token.unwrap().trim().to_string()
    } else {
        current
            .endpoints
            .iter()
            .find(|endpoint| endpoint.id == input.endpoint.id && endpoint.base_url == base_url)
            .map(|endpoint| decrypt_endpoint_token(&state, endpoint))
            .transpose()?
            .unwrap_or_default()
    };
    let endpoint = StoredEndpoint {
        id: input.endpoint.id.trim().into(),
        name: input.endpoint.name.trim().into(),
        protocol: input.endpoint.protocol,
        base_url,
        model: input.endpoint.model.trim().into(),
        encrypted_token: String::new(),
        timeout_ms: input.endpoint.timeout_ms,
        input_limit: input.endpoint.input_limit,
        enabled: input.endpoint.enabled,
    };
    let result = run_probe(&state, &endpoint, &token).await;
    store_probe_result(&state, &endpoint.id, &result).await?;
    Ok(Json(json!({"data": result})))
}

async fn run_probe(state: &AppState, endpoint: &StoredEndpoint, token: &str) -> Value {
    let started = Instant::now();
    let models_url = endpoint_url(&endpoint.base_url, "models");
    let mut request = state
        .client
        .get(models_url)
        .timeout(std::time::Duration::from_millis(endpoint.timeout_ms));
    if !token.is_empty() {
        request = request.bearer_auth(token);
    }
    let mut http_status = 0;
    let mut error_code = String::new();
    let mut message: String;
    let mut retryable = false;
    let mut ok = false;
    let mut should_fallback = false;
    match request.send().await {
        Ok(response) => {
            http_status = response.status().as_u16() as i64;
            if response.status().is_success() {
                let value = response.json::<Value>().await.unwrap_or(Value::Null);
                ok = value
                    .get("data")
                    .and_then(Value::as_array)
                    .is_some_and(|items| {
                        endpoint.model.is_empty()
                            || items.iter().any(|item| {
                                item.get("id").and_then(Value::as_str)
                                    == Some(endpoint.model.as_str())
                            })
                    });
                if ok {
                    message = "prompt audit endpoint is healthy".into();
                } else {
                    error_code = "model_not_found".into();
                    message = "configured model was not returned by the endpoint".into();
                    should_fallback = true;
                }
            } else {
                error_code = if matches!(http_status, 401 | 403) {
                    "authentication_failed".into()
                } else {
                    "probe_http_error".into()
                };
                retryable = http_status == 429 || http_status >= 500;
                message = format!("prompt audit endpoint returned HTTP {http_status}");
                should_fallback = matches!(http_status, 404 | 405);
            }
        }
        Err(error) => {
            error_code = if error.is_timeout() {
                "timeout".into()
            } else {
                "connection_failed".into()
            };
            retryable = true;
            message = "could not connect to prompt audit endpoint".into();
        }
    }
    if should_fallback {
        match call_guard_endpoint_with_token(
            state,
            endpoint,
            token,
            "Hello",
            &SCANNERS.map(str::to_string),
        )
        .await
        {
            Ok(_) => {
                ok = true;
                http_status = 200;
                error_code.clear();
                retryable = false;
                message = "prompt audit endpoint model call is healthy".into();
            }
            Err(error) => {
                error_code = error.code.to_lowercase();
                message = error.message;
            }
        }
    }
    json!({
        "ok": ok,
        "status": if ok { "healthy" } else { "failed" },
        "error_code": error_code,
        "message": message,
        "latency_ms": started.elapsed().as_millis() as i64,
        "http_status": http_status,
        "retryable": retryable,
        "checked_at": Utc::now().to_rfc3339(),
        "token_applied": !token.is_empty()
    })
}

async fn store_probe_result(state: &AppState, endpoint_id: &str, result: &Value) -> ApiResult<()> {
    sqlx::query(
        "INSERT INTO prompt_audit_probe_results \
         (endpoint_id, ok, status, error_code, message, latency_ms, http_status, retryable, checked_at, token_applied) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(endpoint_id) DO UPDATE SET \
         ok=excluded.ok, status=excluded.status, error_code=excluded.error_code, message=excluded.message, \
         latency_ms=excluded.latency_ms, http_status=excluded.http_status, retryable=excluded.retryable, \
         checked_at=excluded.checked_at, token_applied=excluded.token_applied",
    )
    .bind(endpoint_id)
    .bind(result["ok"].as_bool().unwrap_or(false))
    .bind(result["status"].as_str().unwrap_or("failed"))
    .bind(result["error_code"].as_str().unwrap_or_default())
    .bind(result["message"].as_str().unwrap_or_default())
    .bind(result["latency_ms"].as_i64().unwrap_or(0))
    .bind(result["http_status"].as_i64().unwrap_or(0))
    .bind(result["retryable"].as_bool().unwrap_or(false))
    .bind(result["checked_at"].as_str().unwrap_or_default())
    .bind(result["token_applied"].as_bool().unwrap_or(false))
    .execute(&state.pool)
    .await?;
    Ok(())
}

async fn runtime(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let config = load_config(&state).await?;
    let queue_rows: Vec<(String, i64)> =
        sqlx::query_as("SELECT status, COUNT(*) FROM prompt_audit_jobs GROUP BY status")
            .fetch_all(&state.pool)
            .await?;
    let queue_count = |status: &str| {
        queue_rows
            .iter()
            .find(|row| row.0 == status)
            .map(|row| row.1)
            .unwrap_or(0)
    };
    let processed_total = queue_count("done");
    let failed_total = queue_count("failed");
    let metrics: (i64, i64, i64, i64, f64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(SUM(decision = 'pass'), 0), COALESCE(SUM(decision = 'flag'), 0), \
         COALESCE(SUM(action = 'Block'), 0), CAST(COALESCE(AVG(latency_ms), 0) AS REAL), \
         COALESCE(MAX(latency_ms), 0) FROM prompt_audit_events",
    )
    .fetch_one(&state.pool)
    .await?;
    let latencies: Vec<i64> = sqlx::query_scalar(
        "SELECT latency_ms FROM (SELECT id, latency_ms FROM prompt_audit_events \
         ORDER BY id DESC LIMIT 10000) ORDER BY latency_ms",
    )
    .fetch_all(&state.pool)
    .await?;
    let queue_delays: Vec<i64> = sqlx::query_scalar(
        "SELECT queue_delay_ms FROM (SELECT id, queue_delay_ms FROM prompt_audit_jobs \
         WHERE processed_at IS NOT NULL ORDER BY id DESC LIMIT 10000) ORDER BY queue_delay_ms",
    )
    .fetch_all(&state.pool)
    .await?;
    let runtime_metrics: (f64, i64, f64, i64, i64) = sqlx::query_as(
        "SELECT CAST(COALESCE(AVG(queue_delay_ms), 0) AS REAL), COALESCE(MAX(queue_delay_ms), 0), \
         CAST(COALESCE(AVG(duration_ms), 0) AS REAL), COALESCE(MAX(duration_ms), 0), \
         COALESCE(SUM(datetime(processed_at) >= datetime('now', '-1 minute')), 0) \
         FROM prompt_audit_jobs WHERE processed_at IS NOT NULL",
    )
    .fetch_one(&state.pool)
    .await?;
    let probes: Vec<(
        String,
        bool,
        String,
        String,
        String,
        i64,
        i64,
        bool,
        String,
        bool,
    )> = sqlx::query_as(
        "SELECT endpoint_id, ok, status, error_code, message, latency_ms, http_status, \
             retryable, checked_at, token_applied FROM prompt_audit_probe_results",
    )
    .fetch_all(&state.pool)
    .await?;
    let endpoints = probes
        .into_iter()
        .map(|row| {
            (
                row.0,
                json!({"ok":row.1,"status":row.2,"error_code":row.3,"message":row.4,
                    "latency_ms":row.5,"http_status":row.6,"retryable":row.7,
                    "checked_at":row.8,"token_applied":row.9}),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    let last: Option<(String, String, String)> = sqlx::query_as(
        "SELECT processed_at, last_error_code, last_error_message FROM prompt_audit_jobs \
         WHERE processed_at IS NOT NULL ORDER BY id DESC LIMIT 1",
    )
    .fetch_optional(&state.pool)
    .await?;
    let active = queue_count("processing") + queue_count("queued") + queue_count("retry");
    let queue = json!({
        "staging": queue_count("staging"), "queued": queue_count("queued"),
        "processing": queue_count("processing"), "retry": queue_count("retry"),
        "done": queue_count("done"), "failed": failed_total, "active": active
    });
    let guard_metrics = json!({
        "total": metrics.0, "allowed": metrics.1, "flagged": metrics.2,
        "blocked": metrics.3, "unavailable": failed_total, "invalid": 0,
        "timeouts": 0, "failovers": 0, "bulkhead_full": 0, "record_failed": 0,
        "latency_avg_ms": metrics.4, "latency_p50_ms": percentile(&latencies, 0.50),
        "latency_p95_ms": percentile(&latencies, 0.95),
        "latency_p99_ms": percentile(&latencies, 0.99), "latency_max_ms": metrics.5,
        "latency_sample_size": latencies.len()
    });
    Ok(Json(json!({"data": {
        "process_status": if !config.enabled { "disabled" } else if failed_total > 0 { "degraded" } else { "running" },
        "effective_mode": config.effective_mode(),
        "expected_config_version": config.config_version,
        "active_config_version": config.config_version,
        "config_loaded_at": config.updated_at,
        "config_load_error": "",
        "worker_total": config.worker_count,
        "worker_active": state.prompt_audit_slots.active(),
        "worker_heartbeat_at": last.as_ref().map(|row| row.0.clone()),
        "queue_capacity": config.queue_capacity,
        "queue": queue,
        "processed_total": processed_total,
        "failed_total": failed_total,
        "enqueued_total": processed_total + failed_total + active,
        "dropped_total": 0,
        "throughput_per_minute": runtime_metrics.4,
        "queue_delay_avg_ms": runtime_metrics.0,
        "queue_delay_p50_ms": percentile(&queue_delays, 0.50),
        "queue_delay_p95_ms": percentile(&queue_delays, 0.95),
        "queue_delay_p99_ms": percentile(&queue_delays, 0.99),
        "queue_delay_max_ms": runtime_metrics.1,
        "processing_avg_ms": runtime_metrics.2,
        "processing_max_ms": runtime_metrics.3,
        "last_processed_at": last.as_ref().map(|row| row.0.clone()),
        "last_error_code": last.as_ref().map(|row| row.1.clone()).unwrap_or_default(),
        "last_error_message": last.as_ref().map(|row| row.2.clone()).unwrap_or_default(),
        "database_status": "ok",
        "redis_status": "not_used",
        "endpoints": endpoints,
        "guard_metrics": guard_metrics
    }})))
}

fn percentile(sorted: &[i64], quantile: f64) -> i64 {
    if sorted.is_empty() {
        return 0;
    }
    let index = ((sorted.len() - 1) as f64 * quantile.clamp(0.0, 1.0)).round() as usize;
    sorted[index]
}

fn decrypt_endpoint_token(state: &AppState, endpoint: &StoredEndpoint) -> ApiResult<String> {
    if endpoint.encrypted_token.is_empty() {
        return Ok(String::new());
    }
    let bytes = state.crypto.decrypt(&endpoint.encrypted_token)?;
    String::from_utf8(bytes)
        .map_err(|_| ApiError::internal("stored prompt audit token is malformed"))
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
    if !config.enabled || !config.includes_group(key.group_id) {
        return Ok(());
    }
    let text = extract_prompt_text(value);
    if text.trim().is_empty() {
        return Ok(());
    }
    let snapshot = build_snapshot(
        state,
        key,
        endpoint,
        model.unwrap_or_default(),
        request_id,
        value,
        &text,
    )
    .await?;
    let active: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM prompt_audit_jobs WHERE status IN ('staging','queued','processing','retry')",
    )
    .fetch_one(&state.pool)
    .await?;
    if active >= config.queue_capacity as i64 {
        if config.blocking_enabled {
            return Err(ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "PROMPT_GUARD_UNAVAILABLE",
                "prompt audit queue is full",
            ));
        }
        return Ok(());
    }
    let mode = config.effective_mode();
    let job_id = insert_job(state, &snapshot, &config, mode).await?;
    if !config.blocking_enabled {
        let state = state.clone();
        tokio::spawn(async move {
            if let Err(error) = execute_job(&state, &config, &snapshot, &text, job_id).await {
                tracing::warn!(job_id, %error, "asynchronous prompt audit failed");
            }
        });
        return Ok(());
    }
    let task_state = state.clone();
    let task =
        tokio::spawn(
            async move { execute_job(&task_state, &config, &snapshot, &text, job_id).await },
        );
    match task.await {
        Ok(Ok(result)) if result.action == "Block" => Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "PROMPT_GUARD_BLOCKED",
            "request blocked by prompt audit policy",
        )),
        Ok(Ok(_)) => Ok(()),
        Ok(Err(_)) | Err(_) => Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "PROMPT_GUARD_UNAVAILABLE",
            "prompt audit endpoint is unavailable",
        )),
    }
}

async fn execute_job(
    state: &AppState,
    config: &StoredConfig,
    snapshot: &PromptSnapshot,
    text: &str,
    job_id: i64,
) -> ApiResult<ScanResult> {
    let _permit = state
        .prompt_audit_slots
        .acquire(config.worker_count as usize)
        .await;
    let claimed = sqlx::query(
        "UPDATE prompt_audit_jobs SET status='processing', attempts=attempts+1, \
         queue_delay_ms=MAX(CAST((julianday('now')-julianday(created_at))*86400000 AS INTEGER),0), \
         processing_started_at=strftime('%Y-%m-%dT%H:%M:%fZ','now'), \
         updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') \
         WHERE id=? AND status='queued'",
    )
    .bind(job_id)
    .execute(&state.pool)
    .await?;
    if claimed.rows_affected() != 1 {
        return Err(ApiError::internal("prompt audit job is no longer queued"));
    }
    match process_job(state, config, snapshot, text, job_id).await {
        Ok(result) => Ok(result),
        Err(error) => {
            mark_job_failed(state, job_id, &error).await;
            Err(error)
        }
    }
}

async fn build_snapshot(
    state: &AppState,
    key: &ApiKeyContext,
    endpoint: &str,
    model: &str,
    request_id: &str,
    value: &Value,
    text: &str,
) -> ApiResult<PromptSnapshot> {
    let identity: Option<(String, String, String, String)> = sqlx::query_as(
        "SELECT COALESCE(users.username,''), COALESCE(users.email,''), api_keys.name, \
         COALESCE(groups.name,'') FROM api_keys LEFT JOIN users ON users.id=api_keys.user_id \
         LEFT JOIN groups ON groups.id=api_keys.group_id WHERE api_keys.id=?",
    )
    .bind(key.id)
    .fetch_optional(&state.pool)
    .await?;
    let identity = identity.unwrap_or_default();
    let prompt_length = text.chars().count();
    Ok(PromptSnapshot {
        request_id: request_id.into(),
        user_id: key.user_id,
        username: identity.0,
        user_email: identity.1,
        api_key_id: key.id,
        api_key_name: identity.2,
        group_id: key.group_id,
        group_name: identity.3,
        endpoint: endpoint.into(),
        model: model.into(),
        prompt_hash: hex::encode(Sha256::digest(text.as_bytes())),
        redacted_preview: format!("[prompt content omitted; {prompt_length} characters]"),
        prompt_length,
        message_count: message_count(value),
    })
}

async fn insert_job(
    state: &AppState,
    snapshot: &PromptSnapshot,
    config: &StoredConfig,
    mode: &str,
) -> ApiResult<i64> {
    let result = sqlx::query(
        "INSERT INTO prompt_audit_jobs (request_id,user_id,username_snapshot,user_email_snapshot,api_key_id, \
         api_key_name_snapshot,group_id,group_name,endpoint,model,prompt_hash,redacted_preview,prompt_length, \
         message_count,execution_mode,config_version,status,max_attempts,created_at,updated_at) \
         VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,strftime('%Y-%m-%dT%H:%M:%fZ','now'), \
         strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
    )
    .bind(&snapshot.request_id)
    .bind(snapshot.user_id)
    .bind(&snapshot.username)
    .bind(&snapshot.user_email)
    .bind(snapshot.api_key_id)
    .bind(&snapshot.api_key_name)
    .bind(snapshot.group_id)
    .bind(&snapshot.group_name)
    .bind(&snapshot.endpoint)
    .bind(&snapshot.model)
    .bind(&snapshot.prompt_hash)
    .bind(&snapshot.redacted_preview)
    .bind(snapshot.prompt_length as i64)
    .bind(snapshot.message_count as i64)
    .bind(mode)
    .bind(config.config_version)
    .bind("queued")
    .bind(config.endpoints.len().max(1) as i64)
    .execute(&state.pool)
    .await?;
    Ok(result.last_insert_rowid())
}

async fn process_job(
    state: &AppState,
    config: &StoredConfig,
    snapshot: &PromptSnapshot,
    text: &str,
    job_id: i64,
) -> ApiResult<ScanResult> {
    let result = scan_prompt(state, config, text).await?;
    if result.action != "Allow" || config.store_pass_events {
        insert_event(state, snapshot, config, job_id, &result).await?;
    }
    sqlx::query(
        "UPDATE prompt_audit_jobs SET status='done', \
         processed_at=strftime('%Y-%m-%dT%H:%M:%fZ','now'), \
         duration_ms=MAX(CAST((julianday('now')-julianday(processing_started_at))*86400000 AS INTEGER),0), \
         processing_started_at=NULL, updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?",
    )
    .bind(job_id)
    .execute(&state.pool)
    .await?;
    Ok(result)
}

async fn mark_job_failed(state: &AppState, job_id: i64, error: &ApiError) {
    let _ = sqlx::query(
        "UPDATE prompt_audit_jobs SET status='failed', \
         processed_at=strftime('%Y-%m-%dT%H:%M:%fZ','now'), \
         duration_ms=CASE WHEN processing_started_at IS NULL THEN duration_ms ELSE \
         MAX(CAST((julianday('now')-julianday(processing_started_at))*86400000 AS INTEGER),0) END, \
         processing_started_at=NULL, last_error_code=?, last_error_message=?, \
         updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?",
    )
    .bind(error.code)
    .bind(error.message.chars().take(500).collect::<String>())
    .bind(job_id)
    .execute(&state.pool)
    .await;
}

async fn scan_prompt(state: &AppState, config: &StoredConfig, text: &str) -> ApiResult<ScanResult> {
    let endpoints = config
        .endpoints
        .iter()
        .filter(|endpoint| endpoint.enabled)
        .collect::<Vec<_>>();
    if endpoints.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "PROMPT_GUARD_UNAVAILABLE",
            "no prompt audit endpoint is enabled",
        ));
    }
    let input_limit = endpoints
        .iter()
        .map(|endpoint| endpoint.input_limit)
        .min()
        .unwrap_or(4_000)
        .max(128);
    let chunks = split_chars(text, input_limit);
    let started = Instant::now();
    let mut aggregate = ScanResult {
        decision: "pass".into(),
        risk_level: "low".into(),
        action: "Allow".into(),
        categories: Vec::new(),
        matched_scanners: Vec::new(),
        scanner_scores: BTreeMap::new(),
        scanner_evidence: BTreeMap::new(),
        guard_endpoint_id: String::new(),
        chunk_total: chunks.len(),
        latency_ms: 0,
    };
    for chunk in chunks {
        let mut last_error = None;
        let mut chunk_result = None;
        for endpoint in &endpoints {
            match call_guard_endpoint(state, endpoint, chunk, &config.scanners).await {
                Ok(result) => {
                    chunk_result = Some(result);
                    break;
                }
                Err(error) => last_error = Some(error),
            }
        }
        let result = chunk_result.ok_or_else(|| {
            last_error.unwrap_or_else(|| {
                ApiError::new(
                    StatusCode::BAD_GATEWAY,
                    "PROMPT_GUARD_UNAVAILABLE",
                    "all prompt audit endpoints failed",
                )
            })
        })?;
        merge_scan_result(&mut aggregate, result);
        if aggregate.action == "Block" {
            break;
        }
    }
    aggregate.latency_ms = started.elapsed().as_millis() as i64;
    Ok(aggregate)
}

fn merge_scan_result(target: &mut ScanResult, source: ScanResult) {
    let rank = |action: &str| match action {
        "Block" => 2,
        "Warn" => 1,
        _ => 0,
    };
    if rank(&source.action) > rank(&target.action) {
        target.decision = source.decision.clone();
        target.risk_level = source.risk_level.clone();
        target.action = source.action.clone();
        target.guard_endpoint_id = source.guard_endpoint_id.clone();
    }
    for category in source.categories {
        if !target.categories.contains(&category) {
            target.categories.push(category);
        }
    }
    for scanner in source.matched_scanners {
        if !target.matched_scanners.contains(&scanner) {
            target.matched_scanners.push(scanner);
        }
    }
    target.scanner_scores.extend(source.scanner_scores);
    target.scanner_evidence.extend(source.scanner_evidence);
}

async fn call_guard_endpoint(
    state: &AppState,
    endpoint: &StoredEndpoint,
    chunk: &str,
    scanners: &[String],
) -> ApiResult<ScanResult> {
    let token = decrypt_endpoint_token(state, endpoint)?;
    call_guard_endpoint_with_token(state, endpoint, &token, chunk, scanners).await
}

async fn call_guard_endpoint_with_token(
    state: &AppState,
    endpoint: &StoredEndpoint,
    token: &str,
    chunk: &str,
    scanners: &[String],
) -> ApiResult<ScanResult> {
    let url = endpoint_url(&endpoint.base_url, "chat/completions");
    let mut request = state
        .client
        .post(url)
        .timeout(std::time::Duration::from_millis(endpoint.timeout_ms))
        .json(&json!({
            "model": endpoint.model,
            "messages": [{"role":"user","content":chunk}],
            "temperature": 0,
            "max_tokens": 64,
            "seed": 42
        }));
    if !token.is_empty() {
        request = request.bearer_auth(token);
    }
    let response = request.send().await?;
    let status = response.status();
    if !status.is_success() {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "PROMPT_GUARD_UPSTREAM_ERROR",
            format!("prompt audit endpoint returned {status}"),
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length as usize > MAX_RESPONSE_BYTES)
    {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "PROMPT_GUARD_INVALID_RESPONSE",
            "prompt audit response is too large",
        ));
    }
    let bytes = response.bytes().await?;
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "PROMPT_GUARD_INVALID_RESPONSE",
            "prompt audit response is too large",
        ));
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            "PROMPT_GUARD_INVALID_RESPONSE",
            "prompt audit endpoint returned invalid JSON",
        )
    })?;
    let content = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::BAD_GATEWAY,
                "PROMPT_GUARD_INVALID_RESPONSE",
                "prompt audit response is missing message content",
            )
        })?;
    parse_qwen_guard(content, scanners, &endpoint.id)
}

fn endpoint_url(base_url: &str, suffix: &str) -> String {
    let base = base_url.trim_end_matches('/');
    if base.ends_with("/v1") {
        format!("{base}/{suffix}")
    } else {
        format!("{base}/v1/{suffix}")
    }
}

fn parse_qwen_guard(
    content: &str,
    scanners: &[String],
    endpoint_id: &str,
) -> ApiResult<ScanResult> {
    let lines = content
        .replace("\r\n", "\n")
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    if lines.len() != 2 {
        return Err(invalid_guard_response());
    }
    let mut safety = None;
    let mut categories = None;
    for line in lines {
        let lower = line.to_lowercase();
        if lower.starts_with("safety:") && safety.is_none() {
            safety = Some(line[7..].trim().to_lowercase());
        } else if lower.starts_with("categories:") && categories.is_none() {
            categories = Some(line[11..].trim().to_string());
        } else {
            return Err(invalid_guard_response());
        }
    }
    let safety = safety.ok_or_else(invalid_guard_response)?;
    if !matches!(safety.as_str(), "safe" | "controversial" | "unsafe") {
        return Err(invalid_guard_response());
    }
    let category_text = categories.ok_or_else(invalid_guard_response)?;
    let mut known = Vec::new();
    let mut unknown = Vec::new();
    for item in category_text.split(',') {
        let item = item.trim();
        if item.is_empty() || item.eq_ignore_ascii_case("none") || item.eq_ignore_ascii_case("n/a")
        {
            continue;
        }
        let category = normalize_category(item);
        if SCANNERS.contains(&category.as_str()) {
            if !known.contains(&category) {
                known.push(category);
            }
        } else {
            unknown.push(format!("unknown:{}", &token_hash(&category)[..16]));
        }
    }
    let matched = known
        .iter()
        .filter(|category| scanners.contains(category))
        .cloned()
        .collect::<Vec<_>>();
    let (decision, risk_level, action, score) = match safety.as_str() {
        "safe" => ("pass", "low", "Allow", 0.0),
        "controversial"
            if matched.iter().any(|value| {
                matches!(
                    value.as_str(),
                    "jailbreak" | "pii" | "suicide_and_self_harm"
                )
            }) =>
        {
            ("critical", "critical", "Block", 0.5)
        }
        "controversial" => ("flag", "medium", "Warn", 0.5),
        "unsafe" if !matched.is_empty() || !unknown.is_empty() || known.is_empty() => {
            ("critical", "critical", "Block", 1.0)
        }
        "unsafe" => ("flag", "high", "Warn", 1.0),
        _ => unreachable!(),
    };
    let scores = matched
        .iter()
        .map(|category| (category.clone(), score))
        .collect();
    let evidence = matched
        .iter()
        .map(|category| (category.clone(), scanner_label(category).to_string()))
        .collect();
    known.extend(unknown);
    Ok(ScanResult {
        decision: decision.into(),
        risk_level: risk_level.into(),
        action: action.into(),
        categories: known,
        matched_scanners: matched,
        scanner_scores: scores,
        scanner_evidence: evidence,
        guard_endpoint_id: endpoint_id.into(),
        chunk_total: 1,
        latency_ms: 0,
    })
}

fn invalid_guard_response() -> ApiError {
    ApiError::new(
        StatusCode::BAD_GATEWAY,
        "PROMPT_GUARD_INVALID_RESPONSE",
        "prompt audit endpoint returned an invalid guard result",
    )
}

fn normalize_category(value: &str) -> String {
    let normalized = value
        .trim()
        .to_lowercase()
        .replace(['_', '-', '/', '&'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    match normalized.as_str() {
        "violence" | "violent" => "violent".into(),
        "non violent illegal acts" => "non_violent_illegal_acts".into(),
        "sexual" | "sexual content or sexual acts" => "sexual_content_or_sexual_acts".into(),
        "pii" | "personal identifying information" | "personal identifiable information" => {
            "pii".into()
        }
        "suicide self harm" | "suicide and self harm" => "suicide_and_self_harm".into(),
        "unethical" | "unethical acts" => "unethical_acts".into(),
        "political" | "politically sensitive topics" => "politically_sensitive_topics".into(),
        "copyright" | "copyright violation" => "copyright_violation".into(),
        "prompt injection" | "jailbreak" => "jailbreak".into(),
        _ => normalized.replace(' ', "_"),
    }
}

fn scanner_label(category: &str) -> &'static str {
    match category {
        "violent" => "Violence or threats of violence",
        "non_violent_illegal_acts" => "Non-violent illegal activity",
        "sexual_content_or_sexual_acts" => "Sexual content or sexual acts",
        "pii" => "Personal identifying information",
        "suicide_and_self_harm" => "Suicide or self-harm",
        "unethical_acts" => "Unethical behavior",
        "politically_sensitive_topics" => "Politically sensitive topics",
        "copyright_violation" => "Copyright infringement",
        "jailbreak" => "Prompt injection or jailbreak attempt",
        _ => "Unknown safety category",
    }
}

fn issue_summaries(result: &ScanResult) -> Vec<Value> {
    result
        .matched_scanners
        .iter()
        .map(|category| {
            let evidence = result
                .scanner_evidence
                .get(category)
                .cloned()
                .unwrap_or_default();
            json!({
                "category": category,
                "scanner_id": category,
                "title": scanner_label(category),
                "description": scanner_label(category),
                "severity": result.risk_level,
                "severity_label": result.risk_level,
                "action": result.action,
                "action_label": result.action,
                "code": format!("prompt_audit_{category}"),
                "score": result.scanner_scores.get(category).copied().unwrap_or(0.0),
                "evidence": evidence,
                "evidence_hash": token_hash(category)
            })
        })
        .collect()
}

async fn insert_event(
    state: &AppState,
    snapshot: &PromptSnapshot,
    config: &StoredConfig,
    job_id: i64,
    result: &ScanResult,
) -> ApiResult<()> {
    sqlx::query(
        "INSERT INTO prompt_audit_events (job_id,request_id,user_id,username_snapshot,user_email_snapshot, \
         api_key_id,api_key_name_snapshot,group_id,group_name,endpoint,model,prompt_hash,redacted_preview, \
         prompt_length,message_count,decision,risk_level,action,categories,matched_scanners,scanner_scores, \
         scanner_evidence,guard_endpoint_id,config_version,chunk_total,latency_ms,issue_summaries) \
         VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(job_id)
    .bind(&snapshot.request_id)
    .bind(snapshot.user_id)
    .bind(&snapshot.username)
    .bind(&snapshot.user_email)
    .bind(snapshot.api_key_id)
    .bind(&snapshot.api_key_name)
    .bind(snapshot.group_id)
    .bind(&snapshot.group_name)
    .bind(&snapshot.endpoint)
    .bind(&snapshot.model)
    .bind(&snapshot.prompt_hash)
    .bind(&snapshot.redacted_preview)
    .bind(snapshot.prompt_length as i64)
    .bind(snapshot.message_count as i64)
    .bind(&result.decision)
    .bind(&result.risk_level)
    .bind(&result.action)
    .bind(serde_json::to_string(&result.categories).unwrap())
    .bind(serde_json::to_string(&result.matched_scanners).unwrap())
    .bind(serde_json::to_string(&result.scanner_scores).unwrap())
    .bind(serde_json::to_string(&result.scanner_evidence).unwrap())
    .bind(&result.guard_endpoint_id)
    .bind(config.config_version)
    .bind(result.chunk_total as i64)
    .bind(result.latency_ms)
    .bind(serde_json::to_string(&issue_summaries(result)).unwrap())
    .execute(&state.pool)
    .await?;
    Ok(())
}

fn extract_prompt_text(value: &Value) -> String {
    fn visit(value: &Value, output: &mut String) {
        match value {
            Value::String(text) => {
                if !output.is_empty() {
                    output.push('\n');
                }
                output.push_str(text);
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

fn message_count(value: &Value) -> usize {
    value
        .get("messages")
        .or_else(|| value.get("input"))
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or_else(|| usize::from(!extract_prompt_text(value).is_empty()))
}

fn split_chars(value: &str, limit: usize) -> Vec<&str> {
    if value.is_empty() {
        return Vec::new();
    }
    let mut output = Vec::new();
    let mut start = 0;
    let mut count = 0;
    for (index, _) in value.char_indices() {
        if count == limit {
            output.push(&value[start..index]);
            start = index;
            count = 0;
        }
        count += 1;
    }
    output.push(&value[start..]);
    output
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct EventFilter {
    decision: Option<String>,
    risk_level: Option<String>,
    endpoint: Option<String>,
    group_id: Option<i64>,
    user_id: Option<i64>,
    api_key_id: Option<i64>,
    request_id: Option<String>,
    prompt_hash: Option<String>,
    keyword: Option<String>,
    start_at: Option<String>,
    end_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EventQuery {
    #[serde(flatten)]
    filter: EventFilter,
    page: Option<i64>,
    page_size: Option<i64>,
}

#[derive(FromRow)]
struct EventRow {
    id: i64,
    job_id: i64,
    request_id: String,
    user_id: Option<i64>,
    username_snapshot: String,
    user_email_snapshot: String,
    api_key_id: Option<i64>,
    api_key_name_snapshot: String,
    group_id: Option<i64>,
    group_name: String,
    provider: String,
    endpoint: String,
    protocol: String,
    model: String,
    prompt_hash: String,
    redacted_preview: String,
    full_prompt: String,
    prompt_length: i64,
    message_count: i64,
    stage: String,
    decision: String,
    risk_level: String,
    action: String,
    categories: String,
    matched_scanners: String,
    scanner_scores: String,
    scanner_evidence: String,
    scanner_backend: String,
    scanner_version: String,
    guard_endpoint_id: String,
    policy_id: String,
    policy_version: i64,
    config_version: i64,
    chunk_total: i64,
    latency_ms: i64,
    issue_summaries: String,
    created_at: String,
}

const EVENT_COLUMNS: &str = "id,job_id,request_id,user_id,username_snapshot,user_email_snapshot,api_key_id,\
api_key_name_snapshot,group_id,group_name,provider,endpoint,protocol,model,prompt_hash,redacted_preview,\
full_prompt,prompt_length,message_count,stage,decision,risk_level,action,categories,matched_scanners,\
scanner_scores,scanner_evidence,scanner_backend,scanner_version,guard_endpoint_id,policy_id,policy_version,\
config_version,chunk_total,latency_ms,issue_summaries,created_at";

fn push_event_filter<'a>(
    builder: &mut QueryBuilder<'a, Sqlite>,
    filter: &'a EventFilter,
    max_id: Option<i64>,
) {
    builder.push(" WHERE 1=1");
    macro_rules! exact {
        ($field:literal, $value:expr) => {
            if let Some(value) = $value.as_ref().filter(|value| !value.trim().is_empty()) {
                builder
                    .push(concat!(" AND ", $field, " = "))
                    .push_bind(value);
            }
        };
    }
    exact!("decision", filter.decision);
    exact!("risk_level", filter.risk_level);
    exact!("endpoint", filter.endpoint);
    exact!("request_id", filter.request_id);
    exact!("prompt_hash", filter.prompt_hash);
    for (field, value) in [
        ("group_id", filter.group_id),
        ("user_id", filter.user_id),
        ("api_key_id", filter.api_key_id),
    ] {
        if let Some(value) = value.filter(|value| *value > 0) {
            builder
                .push(" AND ")
                .push(field)
                .push(" = ")
                .push_bind(value);
        }
    }
    if let Some(keyword) = filter
        .keyword
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        let pattern = format!("%{}%", keyword.trim());
        builder
            .push(" AND (username_snapshot LIKE ")
            .push_bind(pattern.clone())
            .push(" OR api_key_name_snapshot LIKE ")
            .push_bind(pattern.clone())
            .push(" OR model LIKE ")
            .push_bind(pattern.clone())
            .push(" OR categories LIKE ")
            .push_bind(pattern)
            .push(")");
    }
    if let Some(start) = filter
        .start_at
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        builder
            .push(" AND datetime(created_at) >= datetime(")
            .push_bind(start)
            .push(")");
    }
    if let Some(end) = filter
        .end_at
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        builder
            .push(" AND datetime(created_at) <= datetime(")
            .push_bind(end)
            .push(")");
    }
    if let Some(max_id) = max_id {
        builder.push(" AND id <= ").push_bind(max_id);
    }
}

async fn list_events(
    State(state): State<AppState>,
    Query(query): Query<EventQuery>,
) -> ApiResult<Json<Value>> {
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(20).clamp(1, 100);
    let mut count = QueryBuilder::<Sqlite>::new("SELECT COUNT(*) FROM prompt_audit_events");
    push_event_filter(&mut count, &query.filter, None);
    let total: i64 = count.build_query_scalar().fetch_one(&state.pool).await?;
    let mut rows =
        QueryBuilder::<Sqlite>::new(format!("SELECT {EVENT_COLUMNS} FROM prompt_audit_events"));
    push_event_filter(&mut rows, &query.filter, None);
    rows.push(" ORDER BY id DESC LIMIT ")
        .push_bind(page_size)
        .push(" OFFSET ")
        .push_bind((page - 1) * page_size);
    let rows: Vec<EventRow> = rows.build_query_as().fetch_all(&state.pool).await?;
    Ok(Json(json!({"data": {
        "items": rows.iter().map(event_json).collect::<Vec<_>>(),
        "total": total, "page": page, "page_size": page_size,
        "pages": ((total + page_size - 1) / page_size).max(1)
    }})))
}

async fn get_event(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    let row: Option<EventRow> = sqlx::query_as(&format!(
        "SELECT {EVENT_COLUMNS} FROM prompt_audit_events WHERE id=?"
    ))
    .bind(id)
    .fetch_optional(&state.pool)
    .await?;
    let row = row.ok_or_else(|| ApiError::not_found("prompt audit event not found"))?;
    Ok(Json(json!({"data": event_json(&row)})))
}

fn parsed_json(value: &str, fallback: Value) -> Value {
    serde_json::from_str(value).unwrap_or(fallback)
}

fn event_json(row: &EventRow) -> Value {
    json!({
        "id": row.id,
        "job_id": row.job_id,
        "snapshot": {
            "request_id": row.request_id,
            "user_id": row.user_id.unwrap_or(0),
            "username": row.username_snapshot,
            "user_email": row.user_email_snapshot,
            "api_key_id": row.api_key_id.unwrap_or(0),
            "api_key_name": row.api_key_name_snapshot,
            "group_id": row.group_id,
            "group_name": row.group_name,
            "provider": row.provider,
            "endpoint": row.endpoint,
            "protocol": row.protocol,
            "model": row.model,
            "prompt_hash": row.prompt_hash,
            "redacted_preview": row.redacted_preview,
            "full_prompt": row.full_prompt,
            "prompt_length": row.prompt_length,
            "message_count": row.message_count,
            "stage": row.stage
        },
        "decision": row.decision,
        "risk_level": row.risk_level,
        "action": row.action,
        "categories": parsed_json(&row.categories, json!([])),
        "matched_scanners": parsed_json(&row.matched_scanners, json!([])),
        "scanner_scores": parsed_json(&row.scanner_scores, json!({})),
        "scanner_evidence": parsed_json(&row.scanner_evidence, json!({})),
        "scanner_backend": row.scanner_backend,
        "scanner_version": row.scanner_version,
        "guard_endpoint_id": row.guard_endpoint_id,
        "policy_id": row.policy_id,
        "policy_version": row.policy_version,
        "config_version": row.config_version,
        "chunk_total": row.chunk_total,
        "latency_ms": row.latency_ms,
        "issue_summaries": parsed_json(&row.issue_summaries, json!([])),
        "created_at": row.created_at
    })
}

async fn delete_event(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<Value>> {
    delete_event_ids(&state, &[id]).await
}

#[derive(Deserialize)]
struct BatchDeleteInput {
    ids: Vec<i64>,
}

async fn batch_delete_events(
    State(state): State<AppState>,
    Json(input): Json<BatchDeleteInput>,
) -> ApiResult<Json<Value>> {
    if input.ids.is_empty() || input.ids.len() > 500 || input.ids.iter().any(|id| *id <= 0) {
        return Err(ApiError::bad_request(
            "PROMPT_AUDIT_INVALID_DELETE_BATCH",
            "batch delete requires 1-500 positive event IDs",
        ));
    }
    delete_event_ids(&state, &input.ids).await
}

async fn delete_event_ids(state: &AppState, ids: &[i64]) -> ApiResult<Json<Value>> {
    let mut ids = ids.to_vec();
    ids.sort_unstable();
    ids.dedup();
    let mut transaction = state.pool.begin().await?;
    let mut jobs_query = QueryBuilder::<Sqlite>::new(
        "SELECT DISTINCT job_id FROM prompt_audit_events WHERE id IN (",
    );
    {
        let mut separated = jobs_query.separated(",");
        for id in &ids {
            separated.push_bind(id);
        }
    }
    jobs_query.push(")");
    let job_ids: Vec<i64> = jobs_query
        .build_query_scalar()
        .fetch_all(&mut *transaction)
        .await?;
    let mut delete = QueryBuilder::<Sqlite>::new("DELETE FROM prompt_audit_events WHERE id IN (");
    {
        let mut separated = delete.separated(",");
        for id in &ids {
            separated.push_bind(id);
        }
    }
    delete.push(")");
    let deleted_events = delete
        .build()
        .execute(&mut *transaction)
        .await?
        .rows_affected();
    let deleted_jobs = delete_orphan_jobs(&mut transaction, &job_ids).await?;
    transaction.commit().await?;
    if deleted_events == 0 && ids.len() == 1 {
        return Err(ApiError::not_found("prompt audit event not found"));
    }
    Ok(Json(
        json!({"data": {"deleted_events": deleted_events, "deleted_jobs": deleted_jobs}}),
    ))
}

async fn delete_orphan_jobs(
    transaction: &mut sqlx::Transaction<'_, Sqlite>,
    job_ids: &[i64],
) -> ApiResult<u64> {
    if job_ids.is_empty() {
        return Ok(0);
    }
    let mut delete = QueryBuilder::<Sqlite>::new(
        "DELETE FROM prompt_audit_jobs WHERE NOT EXISTS \
         (SELECT 1 FROM prompt_audit_events WHERE prompt_audit_events.job_id=prompt_audit_jobs.id) \
         AND id IN (",
    );
    {
        let mut separated = delete.separated(",");
        for id in job_ids {
            separated.push_bind(id);
        }
    }
    delete.push(")");
    Ok(delete
        .build()
        .execute(&mut **transaction)
        .await?
        .rows_affected())
}

async fn delete_preview(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
    Json(filter): Json<EventFilter>,
) -> ApiResult<Json<Value>> {
    validate_delete_filter(&filter)?;
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT COUNT(*), COALESCE(MAX(id),0) FROM prompt_audit_events",
    );
    push_event_filter(&mut query, &filter, None);
    let (matched_count, snapshot_max_id): (i64, i64) =
        query.build_query_as().fetch_one(&state.pool).await?;
    let filter_json = canonical_filter_json(&filter)?;
    let filter_hash = token_hash(&format!("{filter_json}:{snapshot_max_id}"));
    let token = random_token(32)?;
    let expires_at = Utc::now() + ChronoDuration::minutes(5);
    sqlx::query(
        "DELETE FROM prompt_audit_delete_previews WHERE datetime(expires_at) <= CURRENT_TIMESTAMP",
    )
    .execute(&state.pool)
    .await?;
    sqlx::query(
        "INSERT INTO prompt_audit_delete_previews \
         (token_hash,admin_id,filter_hash,filter_json,snapshot_max_id,expires_at) VALUES (?,?,?,?,?,?)",
    )
    .bind(token_hash(&token))
    .bind(session.user_id)
    .bind(&filter_hash)
    .bind(&filter_json)
    .bind(snapshot_max_id)
    .bind(expires_at.to_rfc3339())
    .execute(&state.pool)
    .await?;
    Ok(Json(json!({"data": {
        "matched_count": matched_count,
        "filter_summary": filter,
        "snapshot_max_id": snapshot_max_id,
        "filter_hash": filter_hash,
        "confirmation_token": token,
        "expires_at": expires_at.to_rfc3339()
    }})))
}

#[derive(Deserialize)]
struct DeleteByFilterInput {
    filter: EventFilter,
    snapshot_max_id: i64,
    filter_hash: String,
    confirmation_token: String,
    confirm: bool,
}

async fn delete_by_filter(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
    Json(input): Json<DeleteByFilterInput>,
) -> ApiResult<Json<Value>> {
    if !input.confirm {
        return Err(ApiError::bad_request(
            "PROMPT_AUDIT_DELETE_CONFIRMATION_INVALID",
            "delete confirmation is required",
        ));
    }
    validate_delete_filter(&input.filter)?;
    let preview: Option<(i64, String, String, i64, String)> = sqlx::query_as(
        "SELECT admin_id,filter_hash,filter_json,snapshot_max_id,expires_at \
         FROM prompt_audit_delete_previews WHERE token_hash=?",
    )
    .bind(token_hash(&input.confirmation_token))
    .fetch_optional(&state.pool)
    .await?;
    let preview = preview.ok_or_else(|| {
        ApiError::bad_request(
            "PROMPT_AUDIT_DELETE_CONFIRMATION_INVALID",
            "delete confirmation is invalid or expired",
        )
    })?;
    let expected_json = canonical_filter_json(&input.filter)?;
    if preview.0 != session.user_id
        || preview.1 != input.filter_hash
        || preview.2 != expected_json
        || preview.3 != input.snapshot_max_id
        || chrono::DateTime::parse_from_rfc3339(&preview.4)
            .ok()
            .is_none_or(|expires| expires <= Utc::now())
    {
        return Err(ApiError::bad_request(
            "PROMPT_AUDIT_DELETE_CONFIRMATION_INVALID",
            "delete confirmation does not match the current filter",
        ));
    }
    let expected_hash = token_hash(&format!("{expected_json}:{}", input.snapshot_max_id));
    if expected_hash != input.filter_hash {
        return Err(ApiError::bad_request(
            "PROMPT_AUDIT_DELETE_CONFIRMATION_INVALID",
            "delete confirmation filter hash is invalid",
        ));
    }
    let mut jobs_query =
        QueryBuilder::<Sqlite>::new("SELECT DISTINCT job_id FROM prompt_audit_events");
    push_event_filter(&mut jobs_query, &input.filter, Some(input.snapshot_max_id));
    let job_ids: Vec<i64> = jobs_query
        .build_query_scalar()
        .fetch_all(&state.pool)
        .await?;
    let mut transaction = state.pool.begin().await?;
    let mut delete = QueryBuilder::<Sqlite>::new("DELETE FROM prompt_audit_events");
    push_event_filter(&mut delete, &input.filter, Some(input.snapshot_max_id));
    let deleted_events = delete
        .build()
        .execute(&mut *transaction)
        .await?
        .rows_affected();
    let deleted_jobs = delete_orphan_jobs(&mut transaction, &job_ids).await?;
    sqlx::query("DELETE FROM prompt_audit_delete_previews WHERE token_hash=?")
        .bind(token_hash(&input.confirmation_token))
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(Json(
        json!({"data": {"deleted_events": deleted_events, "deleted_jobs": deleted_jobs}}),
    ))
}

fn validate_delete_filter(filter: &EventFilter) -> ApiResult<()> {
    let valid = filter
        .start_at
        .as_deref()
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .zip(
            filter
                .end_at
                .as_deref()
                .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok()),
        )
        .is_some_and(|(start, end)| start < end);
    if !valid {
        return Err(ApiError::bad_request(
            "PROMPT_AUDIT_DELETE_PREVIEW_INVALID",
            "filter delete requires a valid explicit time range",
        ));
    }
    Ok(())
}

fn canonical_filter_json(filter: &EventFilter) -> ApiResult<String> {
    serde_json::to_string(filter)
        .map_err(|_| ApiError::internal("prompt audit filter serialization failed"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{crypto::hash_password, test_support};
    use axum::{Json, extract::State as AxumState, routing::post};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[test]
    fn parses_qwen_guard_and_applies_scanner_policy() {
        let scanners = SCANNERS
            .iter()
            .map(|value| (*value).to_string())
            .collect::<Vec<_>>();
        let unsafe_result = parse_qwen_guard(
            "Safety: Unsafe\nCategories: Violence, Jailbreak",
            &scanners,
            "guard-1",
        )
        .unwrap();
        assert_eq!(unsafe_result.action, "Block");
        assert_eq!(unsafe_result.risk_level, "critical");
        assert_eq!(unsafe_result.guard_endpoint_id, "guard-1");

        let ignored = parse_qwen_guard(
            "Safety: Unsafe\nCategories: Violence",
            &["pii".into()],
            "guard-1",
        )
        .unwrap();
        assert_eq!(ignored.action, "Warn");
        assert_eq!(ignored.risk_level, "high");
    }

    #[test]
    fn prompt_extraction_ignores_metadata() {
        let value = json!({
            "model":"gpt-test",
            "messages":[{"role":"user","content":"audit me"}],
            "metadata":{"secret":"ignore me"}
        });
        let text = extract_prompt_text(&value);
        assert!(text.contains("audit me"));
        assert!(!text.contains("ignore me"));
        assert_eq!(message_count(&value), 1);
    }

    #[tokio::test]
    async fn endpoint_tokens_are_encrypted_and_public_config_is_masked() {
        let (_directory, state) = test_support::state().await;
        let config = StoredConfig {
            endpoints: vec![StoredEndpoint {
                id: "guard".into(),
                name: "Guard".into(),
                base_url: "http://127.0.0.1:8000".into(),
                encrypted_token: state.crypto.encrypt(b"prompt-token-canary").unwrap(),
                ..StoredEndpoint::default()
            }],
            ..StoredConfig::default()
        };
        save_config(&state, &config).await.unwrap();
        let stored: String = sqlx::query_scalar("SELECT value FROM app_settings WHERE key=?")
            .bind(CONFIG_KEY)
            .fetch_one(&state.pool)
            .await
            .unwrap();
        assert!(!stored.contains("prompt-token-canary"));
        let public = public_config(&config);
        assert_eq!(public["endpoints"][0]["has_token"], true);
        assert!(public.to_string().find("prompt-token-canary").is_none());
    }

    #[test]
    fn character_chunking_preserves_utf8_boundaries() {
        let chunks = split_chars("ab中文cd", 3);
        assert_eq!(chunks, vec!["ab中", "文cd"]);
    }

    #[test]
    fn runtime_percentiles_use_the_observed_distribution() {
        let values = vec![10, 20, 30, 40, 50, 100, 200, 500, 900, 1_500, 3_000];
        assert_eq!(percentile(&values, 0.50), 100);
        assert_eq!(percentile(&values, 0.95), 3_000);
        assert_eq!(percentile(&values, 0.99), 3_000);
        assert_eq!(percentile(&[], 0.95), 0);
    }

    #[tokio::test]
    async fn asynchronous_jobs_obey_dynamic_worker_slots_and_record_timings() {
        #[derive(Clone)]
        struct Tracker {
            active: Arc<AtomicUsize>,
            maximum: Arc<AtomicUsize>,
        }

        async fn guard(AxumState(tracker): AxumState<Tracker>) -> Json<Value> {
            let active = tracker.active.fetch_add(1, Ordering::SeqCst) + 1;
            tracker.maximum.fetch_max(active, Ordering::SeqCst);
            tokio::time::sleep(std::time::Duration::from_millis(80)).await;
            tracker.active.fetch_sub(1, Ordering::SeqCst);
            Json(json!({"choices":[{"message":{"content":"Safety: Safe\nCategories: None"}}]}))
        }

        let tracker = Tracker {
            active: Arc::new(AtomicUsize::new(0)),
            maximum: Arc::new(AtomicUsize::new(0)),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server_tracker = tracker.clone();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new()
                    .route("/v1/chat/completions", post(guard))
                    .with_state(server_tracker),
            )
            .await
            .unwrap();
        });
        let (_directory, state) = test_support::state().await;
        let user_id = sqlx::query(
            "INSERT INTO users (username,display_name,password_hash,role) \
             VALUES ('prompt-slot-user','Prompt Slot User',?,'user')",
        )
        .bind(hash_password("test-password").unwrap())
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        let key_id = sqlx::query(
            "INSERT INTO api_keys (name,token_prefix,token_hash,user_id) \
             VALUES ('prompt-slot-key','sk-mini-slot','slot-hash',?)",
        )
        .bind(user_id)
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        let config = StoredConfig {
            enabled: true,
            store_pass_events: true,
            worker_count: 1,
            endpoints: vec![StoredEndpoint {
                id: "slot-guard".into(),
                name: "Slot Guard".into(),
                base_url: format!("http://{address}"),
                ..StoredEndpoint::default()
            }],
            ..StoredConfig::default()
        };
        save_config(&state, &config).await.unwrap();
        let key = ApiKeyContext {
            id: key_id,
            user_id: Some(user_id),
            allowed_models: vec![],
            group_id: None,
        };
        for request_id in ["slot-request-1", "slot-request-2"] {
            inspect(
                &state,
                &key,
                "/v1/responses",
                Some("gpt-test"),
                &json!({"input":"safe prompt"}),
                request_id,
            )
            .await
            .unwrap();
        }
        for _ in 0..100 {
            let done: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM prompt_audit_jobs WHERE status = 'done'")
                    .fetch_one(&state.pool)
                    .await
                    .unwrap();
            if done == 2 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert_eq!(tracker.maximum.load(Ordering::SeqCst), 1);
        let timings: Vec<(i64, i64, String)> = sqlx::query_as(
            "SELECT queue_delay_ms,duration_ms,status FROM prompt_audit_jobs ORDER BY id",
        )
        .fetch_all(&state.pool)
        .await
        .unwrap();
        assert_eq!(timings.len(), 2);
        assert!(
            timings.iter().all(|row| row.1 >= 70 && row.2 == "done"),
            "unexpected job timings: {timings:?}"
        );
        assert!(
            timings[1].0 >= timings[0].0 + 50,
            "unexpected queue timings: {timings:?}"
        );
        assert_eq!(state.prompt_audit_slots.active(), 0);
        let Json(runtime) = runtime(State(state.clone())).await.unwrap();
        assert_eq!(runtime["data"]["worker_active"], 0);
        assert_eq!(runtime["data"]["processed_total"], 2);
        assert!(
            runtime["data"]["guard_metrics"]["latency_p95_ms"]
                .as_i64()
                .unwrap()
                >= 70
        );
        assert!(runtime["data"]["queue_delay_p95_ms"].as_i64().unwrap() >= 50);
        server.abort();
    }

    #[tokio::test]
    async fn startup_marks_unrecoverable_jobs_failed_without_prompt_content() {
        let (_directory, state) = test_support::state().await;
        let job_id = sqlx::query(
            "INSERT INTO prompt_audit_jobs (request_id,status) VALUES ('orphan-job','processing')",
        )
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        initialize(&state).await.unwrap();
        let row: (String, String, String) = sqlx::query_as(
            "SELECT status,last_error_code,last_error_message FROM prompt_audit_jobs WHERE id=?",
        )
        .bind(job_id)
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(row.0, "failed");
        assert_eq!(row.1, "PROCESS_RESTARTED");
        assert!(row.2.contains("not persisted"));
    }

    #[tokio::test]
    async fn blocking_audit_records_only_redacted_metadata() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route(
                    "/v1/chat/completions",
                    post(|| async {
                        Json(json!({"choices":[{"message":{"content":"Safety: Unsafe\nCategories: Jailbreak"}}]}))
                    }),
                ),
            )
            .await
            .unwrap();
        });
        let (_directory, state) = test_support::state().await;
        let user_id = sqlx::query(
            "INSERT INTO users (username,display_name,password_hash,role) VALUES ('prompt-user','Prompt User',?,'user')",
        )
        .bind(hash_password("test-password").unwrap())
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        let key_id = sqlx::query(
            "INSERT INTO api_keys (name,token_prefix,token_hash,user_id) VALUES ('prompt-key','sk-mini-prompt','hash',?)",
        )
        .bind(user_id)
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        let config = StoredConfig {
            enabled: true,
            blocking_enabled: true,
            endpoints: vec![StoredEndpoint {
                id: "guard-test".into(),
                name: "Guard Test".into(),
                base_url: format!("http://{address}"),
                ..StoredEndpoint::default()
            }],
            ..StoredConfig::default()
        };
        save_config(&state, &config).await.unwrap();
        let key = ApiKeyContext {
            id: key_id,
            user_id: Some(user_id),
            allowed_models: vec![],
            group_id: None,
        };
        let private_prompt = "unique-private-prompt jailbreak attempt";
        let error = inspect(
            &state,
            &key,
            "/v1/responses",
            Some("gpt-test"),
            &json!({"model":"gpt-test","input":private_prompt}),
            "prompt-request",
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "PROMPT_GUARD_BLOCKED");
        let row: (String, String, String, String) = sqlx::query_as(
            "SELECT full_prompt,redacted_preview,prompt_hash,action FROM prompt_audit_events",
        )
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert!(row.0.is_empty());
        assert!(!row.1.contains(private_prompt));
        assert_eq!(row.2.len(), 64);
        assert_eq!(row.3, "Block");
        server.abort();
    }
}
