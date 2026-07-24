use std::{
    collections::{HashMap, HashSet},
    io::{BufRead, BufReader, Read, Seek, Write},
    path::{Path as FsPath, PathBuf},
    time::Duration,
};

use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Extension, Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::Response,
    routing::{get, post, put},
};
use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use bytes::Bytes;
use chrono::{Duration as ChronoDuration, Utc};
use futures_util::StreamExt;
use ring::{
    rand::SystemRandom,
    signature::{RSA_PKCS1_SHA256, RsaKeyPair},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{FromRow, Sqlite, Transaction};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

use crate::{
    auth::AuthSession,
    crypto::token_hash,
    error::{ApiError, ApiResult},
    models::ApiKeyContext,
    state::AppState,
};

const PROVIDER_KIND: &str = "gemini_api";
const VERTEX_PROVIDER_KIND: &str = "vertex";
const DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com";
const DEFAULT_GCS_BASE_URL: &str = "https://storage.googleapis.com";
const DEFAULT_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const DEFAULT_GCS_PREFIX: &str = "batch-image/mini/{batch_id}";
const MAX_ITEMS: usize = 200;
const MAX_OUTPUT_IMAGES: usize = 200;
const MAX_PROMPT_CHARS: usize = 8_000;
const MAX_REFERENCE_IMAGES: usize = 100;
const MAX_REFERENCE_BYTES: usize = 32 * 1024 * 1024;
const MAX_REFERENCE_BYTES_EACH: usize = 10 * 1024 * 1024;
const MAX_RESULT_BYTES: u64 = 512 * 1024 * 1024;
const MAX_RESULT_LINE_BYTES: usize = 24 * 1024 * 1024;
const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;
const MAX_ZIP_ITEMS: usize = 200;
const MAX_ZIP_BYTES: u64 = 512 * 1024 * 1024;

const JOB_SELECT: &str = "SELECT jobs.id, jobs.batch_id, jobs.user_id, jobs.api_key_id, \
    jobs.provider_id, providers.name AS provider_name, \
    COALESCE(NULLIF(jobs.provider_kind, ''), providers.provider_type) AS provider_kind, \
    COALESCE(NULLIF(jobs.provider_base_url_snapshot, ''), providers.base_url) AS provider_base_url, \
    providers.encrypted_api_key, providers.encrypted_service_account_json, \
    COALESCE(NULLIF(jobs.provider_project_id, ''), providers.project_id) AS provider_project_id, \
    COALESCE(NULLIF(jobs.provider_location, ''), providers.location) AS provider_location, \
    COALESCE(NULLIF(jobs.provider_gcs_bucket, ''), providers.gcs_bucket) AS provider_gcs_bucket, \
    COALESCE(NULLIF(jobs.provider_gcs_prefix, ''), providers.gcs_prefix) AS provider_gcs_prefix, \
    COALESCE(NULLIF(jobs.provider_gcs_base_url, ''), providers.gcs_base_url) AS provider_gcs_base_url, \
    COALESCE(NULLIF(jobs.provider_token_url, ''), providers.token_url) AS provider_token_url, jobs.task_name, \
    jobs.parent_batch_id, jobs.status, jobs.model, jobs.response_mime_type, jobs.image_size, \
    jobs.item_count, jobs.requested_image_count, jobs.success_count, jobs.fail_count, \
    jobs.generated_image_count, jobs.estimated_cost_cents, jobs.hold_amount_cents, \
    jobs.billable_unit_price_cents, jobs.hold_unit_price_cents, jobs.actual_cost_cents, \
    jobs.provider_job_name, jobs.provider_input_ref, jobs.provider_output_ref, \
    jobs.idempotency_key, jobs.request_hash, jobs.last_error_code, jobs.last_error_message, \
    jobs.retry_count, jobs.output_expires_at, jobs.downloaded_at, jobs.output_deleted_at, \
    jobs.user_deleted_at, jobs.created_at, jobs.updated_at, jobs.submitted_at, jobs.started_at, \
    jobs.finished_at, jobs.settled_at FROM batch_image_jobs jobs \
    JOIN batch_image_providers providers ON providers.id = jobs.provider_id";

const PROVIDER_SELECT: &str = "SELECT id, name, provider_type AS kind, base_url, \
    encrypted_api_key, encrypted_service_account_json, project_id, location, gcs_bucket, \
    gcs_prefix, gcs_base_url, token_url, models, unit_price_cents, batch_discount_bps, \
    hold_bps, priority, concurrency, enabled, last_used_at, last_error, created_at, updated_at \
    FROM batch_image_providers";

pub fn gateway_router() -> Router<AppState> {
    Router::new()
        .route("/images/batches/models", get(gateway_models))
        .route(
            "/images/batches",
            get(gateway_list_jobs).post(gateway_submit),
        )
        .route(
            "/images/batches/{id}",
            get(gateway_get_job).delete(gateway_delete_record),
        )
        .route("/images/batches/{id}/items", get(gateway_list_items))
        .route("/images/batches/{id}/cancel", post(gateway_cancel))
        .route("/images/batches/{id}/download", get(gateway_download))
        .route(
            "/images/batches/{id}/items/{custom_id}/content",
            get(gateway_item_content),
        )
        .route(
            "/images/batches/{id}/outputs",
            axum::routing::delete(gateway_delete_outputs),
        )
        .layer(DefaultBodyLimit::max(48 * 1024 * 1024))
}

pub fn user_router() -> Router<AppState> {
    Router::new()
        .route("/batch-images/bootstrap", get(user_bootstrap))
        .route("/batch-images/jobs", get(user_list_jobs).post(user_submit))
        .route(
            "/batch-images/jobs/{id}",
            get(user_get_job).delete(user_delete_record),
        )
        .route("/batch-images/jobs/{id}/items", get(user_list_items))
        .route("/batch-images/jobs/{id}/cancel", post(user_cancel))
        .route("/batch-images/jobs/{id}/download", get(user_download))
        .route(
            "/batch-images/jobs/{id}/items/{custom_id}/content",
            get(user_item_content),
        )
        .route(
            "/batch-images/jobs/{id}/outputs",
            axum::routing::delete(user_delete_outputs),
        )
        .layer(DefaultBodyLimit::max(48 * 1024 * 1024))
}

pub fn admin_router() -> Router<AppState> {
    Router::new()
        .route(
            "/batch-image-providers",
            get(list_providers).post(create_provider),
        )
        .route(
            "/batch-image-providers/{id}",
            put(update_provider).delete(delete_provider),
        )
        .route("/batch-image-providers/{id}/test", post(test_provider))
}

#[derive(Clone, Debug, FromRow)]
struct ProviderRow {
    id: i64,
    name: String,
    kind: String,
    base_url: String,
    encrypted_api_key: String,
    encrypted_service_account_json: String,
    project_id: String,
    location: String,
    gcs_bucket: String,
    gcs_prefix: String,
    gcs_base_url: String,
    token_url: String,
    models: String,
    unit_price_cents: i64,
    batch_discount_bps: i64,
    hold_bps: i64,
    priority: i64,
    concurrency: i64,
    enabled: bool,
    last_used_at: Option<String>,
    last_error: Option<String>,
    created_at: String,
    updated_at: String,
}

#[allow(dead_code)]
#[derive(Clone, Debug, FromRow)]
struct JobRow {
    id: i64,
    batch_id: String,
    user_id: i64,
    api_key_id: Option<i64>,
    provider_id: i64,
    provider_name: String,
    provider_kind: String,
    provider_base_url: String,
    encrypted_api_key: String,
    encrypted_service_account_json: String,
    provider_project_id: String,
    provider_location: String,
    provider_gcs_bucket: String,
    provider_gcs_prefix: String,
    provider_gcs_base_url: String,
    provider_token_url: String,
    task_name: String,
    parent_batch_id: Option<String>,
    status: String,
    model: String,
    response_mime_type: String,
    image_size: String,
    item_count: i64,
    requested_image_count: i64,
    success_count: i64,
    fail_count: i64,
    generated_image_count: i64,
    estimated_cost_cents: i64,
    hold_amount_cents: i64,
    billable_unit_price_cents: i64,
    hold_unit_price_cents: i64,
    actual_cost_cents: Option<i64>,
    provider_job_name: Option<String>,
    provider_input_ref: Option<String>,
    provider_output_ref: Option<String>,
    idempotency_key: Option<String>,
    request_hash: String,
    last_error_code: Option<String>,
    last_error_message: Option<String>,
    retry_count: i64,
    output_expires_at: Option<String>,
    downloaded_at: Option<String>,
    output_deleted_at: Option<String>,
    user_deleted_at: Option<String>,
    created_at: String,
    updated_at: String,
    submitted_at: Option<String>,
    started_at: Option<String>,
    finished_at: Option<String>,
    settled_at: Option<String>,
}

#[allow(dead_code)]
#[derive(Clone, Debug, FromRow)]
struct ItemRow {
    id: i64,
    job_id: i64,
    custom_id: String,
    status: String,
    output_count: i64,
    prompt_hash: String,
    mime_type: Option<String>,
    file_extension: Option<String>,
    image_count: i64,
    output_files: String,
    error_code: Option<String>,
    error_message: Option<String>,
    created_at: String,
    indexed_at: Option<String>,
}

#[derive(Default, Deserialize)]
struct ProviderInput {
    name: String,
    #[serde(default = "default_provider_kind")]
    kind: String,
    #[serde(default)]
    base_url: String,
    api_key: Option<String>,
    service_account_json: Option<String>,
    #[serde(default)]
    project_id: String,
    #[serde(default)]
    location: String,
    #[serde(default)]
    gcs_bucket: String,
    #[serde(default)]
    gcs_prefix: String,
    #[serde(default)]
    gcs_base_url: String,
    #[serde(default)]
    token_url: String,
    #[serde(default)]
    models: Vec<String>,
    #[serde(default)]
    unit_price_cents: i64,
    #[serde(default = "default_discount_bps")]
    batch_discount_bps: i64,
    #[serde(default = "default_hold_bps")]
    hold_bps: i64,
    #[serde(default = "default_priority")]
    priority: i64,
    #[serde(default = "default_concurrency")]
    concurrency: i64,
    #[serde(default = "default_true")]
    enabled: bool,
}

fn default_provider_kind() -> String {
    PROVIDER_KIND.into()
}
fn default_discount_bps() -> i64 {
    5_000
}
fn default_hold_bps() -> i64 {
    6_000
}
fn default_priority() -> i64 {
    50
}
fn default_concurrency() -> i64 {
    1
}
fn default_true() -> bool {
    true
}

async fn list_providers(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let rows =
        sqlx::query_as::<_, ProviderRow>(&format!("{PROVIDER_SELECT} ORDER BY priority, id"))
            .fetch_all(&state.pool)
            .await?;
    Ok(Json(
        json!({"data": rows.iter().map(provider_public).collect::<Vec<_>>() }),
    ))
}

fn provider_public(row: &ProviderRow) -> Value {
    json!({
        "id": row.id, "name": row.name, "kind": row.kind, "base_url": row.base_url,
        "has_api_key": !row.encrypted_api_key.is_empty(),
        "has_service_account": !row.encrypted_service_account_json.is_empty(),
        "project_id": row.project_id, "location": row.location,
        "gcs_bucket": row.gcs_bucket, "gcs_prefix": row.gcs_prefix,
        "gcs_base_url": row.gcs_base_url, "token_url": row.token_url,
        "models": parse_models(&row.models).unwrap_or_default(),
        "unit_price_cents": row.unit_price_cents,
        "batch_discount_bps": row.batch_discount_bps, "hold_bps": row.hold_bps,
        "priority": row.priority, "concurrency": row.concurrency, "enabled": row.enabled,
        "last_used_at": row.last_used_at, "last_error": row.last_error,
        "created_at": row.created_at, "updated_at": row.updated_at
    })
}

async fn create_provider(
    State(state): State<AppState>,
    Json(input): Json<ProviderInput>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let normalized = normalize_provider_input(input, true)?;
    let encrypted_api_key = if normalized.kind == PROVIDER_KIND {
        state
            .crypto
            .encrypt(normalized.api_key.as_deref().unwrap_or_default().as_bytes())?
    } else {
        String::new()
    };
    let encrypted_service_account = if normalized.kind == VERTEX_PROVIDER_KIND {
        state.crypto.encrypt(
            normalized
                .service_account_json
                .as_deref()
                .unwrap_or_default()
                .as_bytes(),
        )?
    } else {
        String::new()
    };
    let models = serde_json::to_string(&normalized.models)
        .map_err(|_| ApiError::internal("model list serialization failed"))?;
    let result = sqlx::query(
        "INSERT INTO batch_image_providers (name, kind, provider_type, base_url, encrypted_api_key, \
         encrypted_service_account_json, project_id, location, gcs_bucket, gcs_prefix, \
         gcs_base_url, token_url, models, unit_price_cents, batch_discount_bps, hold_bps, \
         priority, concurrency, enabled) VALUES (?, 'gemini_api', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(normalized.name)
    .bind(normalized.kind)
    .bind(normalized.base_url)
    .bind(encrypted_api_key)
    .bind(encrypted_service_account)
    .bind(normalized.project_id)
    .bind(normalized.location)
    .bind(normalized.gcs_bucket)
    .bind(normalized.gcs_prefix)
    .bind(normalized.gcs_base_url)
    .bind(normalized.token_url)
    .bind(models)
    .bind(normalized.unit_price_cents)
    .bind(normalized.batch_discount_bps)
    .bind(normalized.hold_bps)
    .bind(normalized.priority)
    .bind(normalized.concurrency)
    .bind(normalized.enabled)
    .execute(&state.pool)
    .await
    .map_err(map_provider_unique)?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"data": {"id": result.last_insert_rowid()}})),
    ))
}

async fn update_provider(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(input): Json<ProviderInput>,
) -> ApiResult<Json<Value>> {
    let current = get_provider(&state, id).await?;
    let normalized = normalize_provider_input(input, false)?;
    let encrypted_api_key = if normalized.kind == PROVIDER_KIND {
        match normalized.api_key.as_deref() {
            Some(value) => state.crypto.encrypt(value.as_bytes())?,
            None if !current.encrypted_api_key.is_empty() => current.encrypted_api_key,
            None => {
                return Err(ApiError::bad_request(
                    "API_KEY_REQUIRED",
                    "Gemini API key is required",
                ));
            }
        }
    } else {
        current.encrypted_api_key
    };
    let encrypted_service_account = if normalized.kind == VERTEX_PROVIDER_KIND {
        match normalized.service_account_json.as_deref() {
            Some(value) => state.crypto.encrypt(value.as_bytes())?,
            None if !current.encrypted_service_account_json.is_empty() => {
                current.encrypted_service_account_json
            }
            None => {
                return Err(ApiError::bad_request(
                    "SERVICE_ACCOUNT_REQUIRED",
                    "Vertex service account JSON is required",
                ));
            }
        }
    } else {
        current.encrypted_service_account_json
    };
    let models = serde_json::to_string(&normalized.models)
        .map_err(|_| ApiError::internal("model list serialization failed"))?;
    sqlx::query(
        "UPDATE batch_image_providers SET name = ?, provider_type = ?, base_url = ?, \
         encrypted_api_key = ?, encrypted_service_account_json = ?, project_id = ?, location = ?, \
         gcs_bucket = ?, gcs_prefix = ?, gcs_base_url = ?, token_url = ?, models = ?, \
         unit_price_cents = ?, batch_discount_bps = ?, hold_bps = ?, priority = ?, concurrency = ?, \
         enabled = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
    )
    .bind(normalized.name)
    .bind(normalized.kind)
    .bind(normalized.base_url)
    .bind(encrypted_api_key)
    .bind(encrypted_service_account)
    .bind(normalized.project_id)
    .bind(normalized.location)
    .bind(normalized.gcs_bucket)
    .bind(normalized.gcs_prefix)
    .bind(normalized.gcs_base_url)
    .bind(normalized.token_url)
    .bind(models)
    .bind(normalized.unit_price_cents)
    .bind(normalized.batch_discount_bps)
    .bind(normalized.hold_bps)
    .bind(normalized.priority)
    .bind(normalized.concurrency)
    .bind(normalized.enabled)
    .bind(id)
    .execute(&state.pool)
    .await
    .map_err(map_provider_unique)?;
    state.vertex_tokens.lock().await.remove(&id);
    Ok(Json(json!({"data": {"id": id}})))
}

async fn delete_provider(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    let referenced: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM batch_image_jobs WHERE provider_id = ?")
            .bind(id)
            .fetch_one(&state.pool)
            .await?;
    if referenced > 0 {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "BATCH_IMAGE_PROVIDER_IN_USE",
            "disable providers that already have batch jobs",
        ));
    }
    let result = sqlx::query("DELETE FROM batch_image_providers WHERE id = ?")
        .bind(id)
        .execute(&state.pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found("batch image provider not found"));
    }
    state.vertex_tokens.lock().await.remove(&id);
    Ok(StatusCode::NO_CONTENT)
}

async fn test_provider(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<Value>> {
    let provider = get_provider(&state, id).await?;
    let started = std::time::Instant::now();
    let response = if provider.kind == VERTEX_PROVIDER_KIND {
        let access_token = vertex_access_token(&state, &provider).await?;
        state
            .client
            .get(vertex_collection_url(&provider)?)
            .query(&[("pageSize", "1")])
            .bearer_auth(access_token)
            .send()
            .await?
    } else {
        let api_key = decrypt_provider_key(&state, &provider)?;
        state
            .client
            .get(format!("{}/v1beta/models", provider.base_url))
            .header("x-goog-api-key", api_key)
            .send()
            .await?
    };
    if !response.status().is_success() {
        let status = response.status();
        set_provider_error(&state, id, Some(&format!("model probe returned {status}"))).await;
        return Err(provider_http_error(
            &provider.kind,
            status,
            "batch image provider probe failed",
        ));
    }
    set_provider_error(&state, id, None).await;
    Ok(Json(json!({"data": {
        "ok": true, "latency_ms": started.elapsed().as_millis(),
        "models": parse_models(&provider.models).unwrap_or_default()
    }})))
}

fn normalize_provider_input(
    mut input: ProviderInput,
    require_credentials: bool,
) -> ApiResult<ProviderInput> {
    input.name = input.name.trim().to_string();
    input.kind = input.kind.trim().to_ascii_lowercase();
    input.api_key = input
        .api_key
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    input.service_account_json = input
        .service_account_json
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    input.project_id = input.project_id.trim().to_string();
    input.location = input.location.trim().to_ascii_lowercase();
    input.gcs_bucket = input.gcs_bucket.trim().trim_matches('/').to_string();
    input.gcs_prefix = input.gcs_prefix.trim().trim_matches('/').to_string();
    input.models = normalize_models(input.models)?;
    if input.name.is_empty()
        || input.name.chars().count() > 100
        || !matches!(input.kind.as_str(), PROVIDER_KIND | VERTEX_PROVIDER_KIND)
        || !(0..=1_000_000).contains(&input.unit_price_cents)
        || !(0..=10_000).contains(&input.batch_discount_bps)
        || !(0..=10_000).contains(&input.hold_bps)
        || input.hold_bps < input.batch_discount_bps
        || !(0..=10_000).contains(&input.priority)
        || !(1..=16).contains(&input.concurrency)
    {
        return Err(ApiError::bad_request(
            "INVALID_BATCH_IMAGE_PROVIDER",
            "batch image provider settings are invalid",
        ));
    }
    if input.kind == PROVIDER_KIND {
        input.base_url =
            normalize_service_url(&input.base_url, DEFAULT_BASE_URL, "INVALID_PROVIDER_URL")?;
        if require_credentials && input.api_key.is_none() {
            return Err(ApiError::bad_request(
                "API_KEY_REQUIRED",
                "Gemini API key is required",
            ));
        }
        input.service_account_json = None;
        input.project_id.clear();
        input.location = "global".into();
        input.gcs_bucket.clear();
        input.gcs_prefix = DEFAULT_GCS_PREFIX.into();
        input.gcs_base_url = DEFAULT_GCS_BASE_URL.into();
        input.token_url = DEFAULT_TOKEN_URL.into();
        return Ok(input);
    }

    input.base_url = normalize_service_url(&input.base_url, "", "INVALID_PROVIDER_URL")?;
    input.gcs_base_url =
        normalize_service_url(&input.gcs_base_url, DEFAULT_GCS_BASE_URL, "INVALID_GCS_URL")?;
    input.token_url = normalize_token_url(&input.token_url)?;
    if input.location.is_empty() {
        input.location = "global".into();
    }
    if input.gcs_prefix.is_empty() {
        input.gcs_prefix = DEFAULT_GCS_PREFIX.into();
    }
    if let Some(raw) = input.service_account_json.as_deref() {
        let key = parse_service_account(raw)?;
        if input.project_id.is_empty() {
            input.project_id = key.project_id;
        }
    } else if require_credentials {
        return Err(ApiError::bad_request(
            "SERVICE_ACCOUNT_REQUIRED",
            "Vertex service account JSON is required",
        ));
    }
    input.api_key = None;
    if input.project_id.is_empty()
        || input.project_id.len() > 128
        || !input
            .project_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-.:".contains(character))
        || input.location.len() > 63
        || !input.location.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
        || input.gcs_bucket.is_empty()
        || input.gcs_bucket.len() > 222
        || input.gcs_bucket.contains("://")
        || input.gcs_bucket.contains('/')
        || !input
            .gcs_bucket
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || ".-_".contains(character))
        || input.gcs_prefix.len() > 512
        || !input.gcs_prefix.contains("{batch_id}")
        || input.gcs_prefix.contains("..")
        || input.gcs_prefix.contains('\\')
    {
        return Err(ApiError::bad_request(
            "INVALID_VERTEX_PROVIDER",
            "Vertex project, location, or managed GCS settings are invalid",
        ));
    }
    Ok(input)
}

fn normalize_service_url(value: &str, default: &str, code: &'static str) -> ApiResult<String> {
    let value = if value.trim().is_empty() {
        default
    } else {
        value.trim()
    };
    if value.is_empty() {
        return Ok(String::new());
    }
    let mut url = url::Url::parse(value)
        .map_err(|_| ApiError::bad_request(code, "provider URL is invalid"))?;
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "::1"));
    if (url.scheme() != "https" && !(url.scheme() == "http" && loopback))
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(ApiError::bad_request(
            code,
            "provider URL must use HTTPS or loopback HTTP",
        ));
    }
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.as_str().trim_end_matches('/').to_string())
}

fn normalize_token_url(value: &str) -> ApiResult<String> {
    let normalized = normalize_service_url(value, DEFAULT_TOKEN_URL, "INVALID_TOKEN_URL")?;
    let parsed = url::Url::parse(&normalized)
        .map_err(|_| ApiError::bad_request("INVALID_TOKEN_URL", "token URL is invalid"))?;
    let loopback = matches!(parsed.host_str(), Some("localhost" | "127.0.0.1" | "::1"));
    if normalized != DEFAULT_TOKEN_URL && !loopback {
        return Err(ApiError::bad_request(
            "INVALID_TOKEN_URL",
            "Vertex token URL must use the Google endpoint or loopback testing",
        ));
    }
    Ok(normalized)
}

fn normalize_models(values: Vec<String>) -> ApiResult<Vec<String>> {
    let mut seen = HashSet::new();
    let mut output = Vec::new();
    for value in values {
        let value = value.trim().trim_start_matches("models/").to_string();
        if value.is_empty()
            || value.len() > 128
            || !value
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || ".:_-".contains(character))
        {
            return Err(ApiError::bad_request(
                "INVALID_BATCH_IMAGE_MODEL",
                "batch image model names are invalid",
            ));
        }
        if seen.insert(value.clone()) {
            output.push(value);
        }
    }
    if output.is_empty() || output.len() > 100 {
        return Err(ApiError::bad_request(
            "INVALID_BATCH_IMAGE_MODEL",
            "configure 1-100 batch image models",
        ));
    }
    Ok(output)
}

fn parse_models(value: &str) -> ApiResult<Vec<String>> {
    serde_json::from_str(value)
        .map_err(|_| ApiError::internal("stored batch image models are malformed"))
}

fn map_provider_unique(error: sqlx::Error) -> ApiError {
    match error {
        sqlx::Error::Database(ref database) if database.is_unique_violation() => {
            ApiError::bad_request(
                "BATCH_IMAGE_PROVIDER_EXISTS",
                "provider name already exists",
            )
        }
        other => other.into(),
    }
}

async fn get_provider(state: &AppState, id: i64) -> ApiResult<ProviderRow> {
    sqlx::query_as::<_, ProviderRow>(&format!("{PROVIDER_SELECT} WHERE id = ?"))
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| ApiError::not_found("batch image provider not found"))
}

fn decrypt_provider_key(state: &AppState, provider: &ProviderRow) -> ApiResult<String> {
    String::from_utf8(state.crypto.decrypt(&provider.encrypted_api_key)?)
        .map_err(|_| ApiError::internal("stored Gemini API key is malformed"))
}

fn decrypt_service_account(state: &AppState, provider: &ProviderRow) -> ApiResult<String> {
    String::from_utf8(
        state
            .crypto
            .decrypt(&provider.encrypted_service_account_json)?,
    )
    .map_err(|_| ApiError::internal("stored Vertex service account JSON is malformed"))
}

#[derive(Debug, Deserialize)]
struct ServiceAccountKey {
    #[serde(rename = "type")]
    kind: String,
    project_id: String,
    #[serde(default)]
    private_key_id: String,
    private_key: String,
    client_email: String,
}

#[derive(Debug, Deserialize)]
struct VertexTokenResponse {
    access_token: String,
    #[serde(default)]
    expires_in: i64,
}

fn parse_service_account(raw: &str) -> ApiResult<ServiceAccountKey> {
    if raw.len() > 128 * 1024 {
        return Err(ApiError::bad_request(
            "INVALID_SERVICE_ACCOUNT",
            "Vertex service account JSON is too large",
        ));
    }
    let mut key: ServiceAccountKey = serde_json::from_str(raw).map_err(|_| {
        ApiError::bad_request(
            "INVALID_SERVICE_ACCOUNT",
            "Vertex service account JSON is invalid",
        )
    })?;
    key.kind = key.kind.trim().to_string();
    key.project_id = key.project_id.trim().to_string();
    key.private_key_id = key.private_key_id.trim().to_string();
    key.client_email = key.client_email.trim().to_ascii_lowercase();
    if key.kind != "service_account"
        || key.project_id.is_empty()
        || key.project_id.len() > 128
        || key.private_key.is_empty()
        || key.private_key.len() > 32 * 1024
        || key.client_email.len() > 254
        || !key.client_email.contains('@')
    {
        return Err(ApiError::bad_request(
            "INVALID_SERVICE_ACCOUNT",
            "Vertex service account JSON is missing required fields",
        ));
    }
    Ok(key)
}

fn decode_pkcs8_private_key(pem: &str) -> ApiResult<Vec<u8>> {
    let mut inside = false;
    let mut encoded = String::new();
    for line in pem.lines().map(str::trim) {
        match line {
            "-----BEGIN PRIVATE KEY-----" => inside = true,
            "-----END PRIVATE KEY-----" => {
                inside = false;
                break;
            }
            _ if inside => encoded.push_str(line),
            _ => {}
        }
    }
    if inside || encoded.is_empty() {
        return Err(ApiError::bad_request(
            "INVALID_SERVICE_ACCOUNT",
            "Vertex service account private key must be PKCS#8 PEM",
        ));
    }
    STANDARD.decode(encoded).map_err(|_| {
        ApiError::bad_request(
            "INVALID_SERVICE_ACCOUNT",
            "Vertex service account private key is invalid",
        )
    })
}

fn sign_service_account_assertion(
    key: &ServiceAccountKey,
    token_url: &str,
    now: i64,
) -> ApiResult<String> {
    let header = if key.private_key_id.is_empty() {
        json!({"alg":"RS256","typ":"JWT"})
    } else {
        json!({"alg":"RS256","typ":"JWT","kid":key.private_key_id})
    };
    let claims = json!({
        "iss":key.client_email,
        "scope":"https://www.googleapis.com/auth/cloud-platform",
        "aud":token_url,
        "iat":now,
        "exp":now.saturating_add(3600)
    });
    let header = URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&header)
            .map_err(|_| ApiError::internal("Vertex JWT header serialization failed"))?,
    );
    let claims = URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&claims)
            .map_err(|_| ApiError::internal("Vertex JWT claims serialization failed"))?,
    );
    let signing_input = format!("{header}.{claims}");
    let der = decode_pkcs8_private_key(&key.private_key)?;
    let key_pair = RsaKeyPair::from_pkcs8(&der).map_err(|_| {
        ApiError::bad_request(
            "INVALID_SERVICE_ACCOUNT",
            "Vertex service account private key cannot be parsed",
        )
    })?;
    let mut signature = vec![0; key_pair.public().modulus_len()];
    key_pair
        .sign(
            &RSA_PKCS1_SHA256,
            &SystemRandom::new(),
            signing_input.as_bytes(),
            &mut signature,
        )
        .map_err(|_| ApiError::internal("Vertex service account JWT signing failed"))?;
    Ok(format!(
        "{signing_input}.{}",
        URL_SAFE_NO_PAD.encode(signature)
    ))
}

async fn vertex_access_token(state: &AppState, provider: &ProviderRow) -> ApiResult<String> {
    let raw = decrypt_service_account(state, provider)?;
    let key = parse_service_account(&raw)?;
    let fingerprint = token_hash(&format!(
        "{}\0{}\0{}\0{}",
        key.client_email, key.private_key_id, key.private_key, provider.token_url
    ));
    {
        let cache = state.vertex_tokens.lock().await;
        if let Some(token) = cache.get(&provider.id)
            && token.credential_fingerprint == fingerprint
            && token.expires_at > std::time::Instant::now()
        {
            return Ok(token.token.clone());
        }
    }
    let assertion =
        sign_service_account_assertion(&key, &provider.token_url, Utc::now().timestamp())?;
    let form = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer")
        .append_pair("assertion", &assertion)
        .finish();
    let response = state
        .client
        .post(&provider.token_url)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(form)
        .send()
        .await?;
    let status = response.status();
    let bytes = response.bytes().await?;
    if !status.is_success() || bytes.len() > 1024 * 1024 {
        return Err(provider_http_error(
            VERTEX_PROVIDER_KIND,
            status,
            "Vertex service account token exchange failed",
        ));
    }
    let token: VertexTokenResponse = serde_json::from_slice(&bytes).map_err(|_| {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            "VERTEX_AUTH_FAILED",
            "Vertex token response is invalid",
        )
    })?;
    if token.access_token.trim().is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "VERTEX_AUTH_FAILED",
            "Vertex token response has no access token",
        ));
    }
    let ttl = token.expires_in.clamp(360, 86_400).saturating_sub(300) as u64;
    state.vertex_tokens.lock().await.insert(
        provider.id,
        crate::state::CachedVertexToken {
            token: token.access_token.clone(),
            credential_fingerprint: fingerprint,
            expires_at: std::time::Instant::now() + Duration::from_secs(ttl),
        },
    );
    Ok(token.access_token)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct SubmitRequest {
    model: String,
    #[serde(default)]
    task_name: String,
    #[serde(default)]
    parent_batch_id: String,
    #[serde(default)]
    provider: String,
    #[serde(default)]
    response_mime_type: String,
    #[serde(default)]
    image_size: String,
    items: Vec<SubmitItem>,
    #[serde(default)]
    metadata: HashMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct SubmitItem {
    #[serde(default)]
    custom_id: String,
    prompt: String,
    #[serde(default)]
    output_count: usize,
    #[serde(default)]
    reference_images: Vec<ReferenceImage>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct ReferenceImage {
    #[serde(default)]
    id: String,
    #[serde(default, rename = "type")]
    kind: String,
    mime_type: String,
    #[serde(default)]
    data: String,
    #[serde(default)]
    file_uri: String,
}

#[derive(Deserialize)]
struct UserSubmitRequest {
    api_key_id: i64,
    #[serde(default)]
    idempotency_key: String,
    #[serde(flatten)]
    request: SubmitRequest,
}

#[derive(Clone, Copy)]
struct Owner {
    user_id: i64,
    api_key_id: Option<i64>,
}

async fn gateway_submit(
    State(state): State<AppState>,
    Extension(key): Extension<ApiKeyContext>,
    headers: HeaderMap,
    Json(request): Json<SubmitRequest>,
) -> ApiResult<Json<Value>> {
    let _ = require_user_id(&key)?;
    let idempotency = idempotency_header(&headers)?;
    Ok(Json(submit_job(&state, key, request, idempotency).await?))
}

fn require_user_id(key: &ApiKeyContext) -> ApiResult<i64> {
    key.user_id.ok_or_else(|| {
        ApiError::new(
            StatusCode::FORBIDDEN,
            "BATCH_IMAGE_USER_REQUIRED",
            "batch image jobs require a user-owned API key",
        )
    })
}

async fn user_submit(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
    Json(input): Json<UserSubmitRequest>,
) -> ApiResult<Json<Value>> {
    let key = load_user_key(&state, session.user_id, input.api_key_id).await?;
    let idempotency_key = input.idempotency_key.trim();
    let idempotency_key = if idempotency_key.is_empty() {
        None
    } else {
        if idempotency_key.len() > 255 {
            return Err(ApiError::bad_request(
                "INVALID_IDEMPOTENCY_KEY",
                "idempotency key is too long",
            ));
        }
        Some(idempotency_key.to_string())
    };
    Ok(Json(
        submit_job(&state, key, input.request, idempotency_key).await?,
    ))
}

fn idempotency_header(headers: &HeaderMap) -> ApiResult<Option<String>> {
    let value = headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    if value.as_ref().is_some_and(|value| value.len() > 255) {
        return Err(ApiError::bad_request(
            "INVALID_IDEMPOTENCY_KEY",
            "idempotency key is too long",
        ));
    }
    Ok(value)
}

async fn load_user_key(state: &AppState, user_id: i64, key_id: i64) -> ApiResult<ApiKeyContext> {
    let row: Option<(i64, String, Option<i64>)> = sqlx::query_as(
        "SELECT id, allowed_models, group_id FROM api_keys WHERE id = ? AND user_id = ? \
         AND enabled = 1 AND (expires_at IS NULL OR datetime(expires_at) > CURRENT_TIMESTAMP)",
    )
    .bind(key_id)
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await?;
    let (id, models, group_id) = row.ok_or_else(|| {
        ApiError::new(
            StatusCode::FORBIDDEN,
            "BATCH_IMAGE_KEY_INVALID",
            "select an enabled API key owned by this account",
        )
    })?;
    let mut allowed_models = serde_json::from_str::<Vec<String>>(&models)
        .map_err(|_| ApiError::internal("stored API key model policy is malformed"))?;
    if let Some(group_id) = group_id {
        let group: Option<(bool, String)> =
            sqlx::query_as("SELECT enabled, allowed_models FROM groups WHERE id = ?")
                .bind(group_id)
                .fetch_optional(&state.pool)
                .await?;
        let (enabled, models) =
            group.ok_or_else(|| ApiError::forbidden("API key group is missing"))?;
        if !enabled {
            return Err(ApiError::forbidden("API key group is disabled"));
        }
        let group_models = serde_json::from_str::<Vec<String>>(&models)
            .map_err(|_| ApiError::internal("stored group model policy is malformed"))?;
        if !group_models.is_empty() {
            if allowed_models.is_empty() {
                allowed_models = group_models;
            } else {
                allowed_models.retain(|model| group_models.contains(model));
            }
        }
    }
    Ok(ApiKeyContext {
        id,
        user_id: Some(user_id),
        allowed_models,
        group_id,
    })
}

async fn submit_job(
    state: &AppState,
    key: ApiKeyContext,
    request: SubmitRequest,
    idempotency_key: Option<String>,
) -> ApiResult<Value> {
    let user_id = require_user_id(&key)?;
    let request = normalize_submit(request)?;
    if !key.allowed_models.is_empty() && !key.allowed_models.contains(&request.model) {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "MODEL_NOT_ALLOWED",
            "the requested model is not allowed for this API key",
        ));
    }
    let request_json = serde_json::to_value(&request)
        .map_err(|_| ApiError::internal("batch request serialization failed"))?;
    let request_hash = token_hash(
        &serde_json::to_string(&request_json)
            .map_err(|_| ApiError::internal("batch request serialization failed"))?,
    );
    if let Some(idempotency_key) = idempotency_key.as_deref()
        && let Some(existing) = find_idempotent_job(state, key.id, idempotency_key).await?
    {
        if existing.request_hash != request_hash {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "BATCH_IMAGE_IDEMPOTENCY_CONFLICT",
                "idempotency key was reused with a different request",
            ));
        }
        return job_public(&existing);
    }
    crate::risk_control::inspect(
        state,
        &key,
        "/v1/images/batches",
        Some(&request.model),
        &request_json,
        &Uuid::new_v4().to_string(),
    )
    .await?;
    crate::prompt_audit::inspect(
        state,
        &key,
        "/v1/images/batches",
        Some(&request.model),
        &request_json,
        &Uuid::new_v4().to_string(),
    )
    .await?;

    let provider = select_provider(state, &request.model, &request.provider).await?;
    let billable_unit = rounded_bps(provider.unit_price_cents, provider.batch_discount_bps);
    let hold_unit = rounded_bps(provider.unit_price_cents, provider.hold_bps).max(billable_unit);
    let image_count = request.items.len() as i64;
    let estimated = billable_unit.saturating_mul(image_count);
    let hold = hold_unit.saturating_mul(image_count);
    let batch_id = format!("imgbatch_{}", Uuid::new_v4().simple());
    let task_name = if request.task_name.is_empty() {
        Utc::now().format("%Y-%m-%d %H:%M:%S").to_string()
    } else {
        request.task_name.clone()
    };
    let mut transaction = state.pool.begin().await?;
    reserve_balance(&mut transaction, user_id, hold, &batch_id).await?;
    let job_id = sqlx::query(
        "INSERT INTO batch_image_jobs (batch_id, user_id, api_key_id, provider_id, provider_kind, \
         provider_base_url_snapshot, provider_project_id, provider_location, provider_gcs_bucket, \
         provider_gcs_prefix, provider_gcs_base_url, provider_token_url, task_name, parent_batch_id, \
         model, response_mime_type, image_size, item_count, requested_image_count, \
         estimated_cost_cents, hold_amount_cents, billable_unit_price_cents, hold_unit_price_cents, \
         idempotency_key, request_hash) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULLIF(?, ''), \
         ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&batch_id)
    .bind(user_id)
    .bind(key.id)
    .bind(provider.id)
    .bind(&provider.kind)
    .bind(&provider.base_url)
    .bind(&provider.project_id)
    .bind(&provider.location)
    .bind(&provider.gcs_bucket)
    .bind(&provider.gcs_prefix)
    .bind(&provider.gcs_base_url)
    .bind(&provider.token_url)
    .bind(&task_name)
    .bind(&request.parent_batch_id)
    .bind(&request.model)
    .bind(&request.response_mime_type)
    .bind(&request.image_size)
    .bind(image_count)
    .bind(image_count)
    .bind(estimated)
    .bind(hold)
    .bind(billable_unit)
    .bind(hold_unit)
    .bind(&idempotency_key)
    .bind(&request_hash)
    .execute(&mut *transaction)
    .await
    .map_err(|error| match error {
        sqlx::Error::Database(ref database) if database.is_unique_violation() => ApiError::new(
            StatusCode::CONFLICT,
            "BATCH_IMAGE_IDEMPOTENCY_CONFLICT",
            "idempotency key is already in use",
        ),
        other => other.into(),
    })?
    .last_insert_rowid();
    for item in &request.items {
        sqlx::query(
            "INSERT INTO batch_image_items (job_id, custom_id, output_count, prompt_hash) \
             VALUES (?, ?, 1, ?)",
        )
        .bind(job_id)
        .bind(&item.custom_id)
        .bind(token_hash(&item.prompt))
        .execute(&mut *transaction)
        .await?;
    }
    append_event(
        &mut transaction,
        job_id,
        "created",
        json!({"item_count": image_count}),
    )
    .await?;
    transaction.commit().await?;

    let jsonl = build_gemini_jsonl(&request)?;
    let submitted = match provider.kind.as_str() {
        VERTEX_PROVIDER_KIND => {
            vertex_submit(state, &provider, &batch_id, &request.model, jsonl).await
        }
        _ => {
            let api_key = decrypt_provider_key(state, &provider)?;
            gemini_submit(state, &provider, &api_key, &batch_id, &request.model, jsonl).await
        }
    };
    let submitted = match submitted {
        Ok(value) => value,
        Err(error) => {
            fail_job(state, &batch_id, "PROVIDER_SUBMIT_FAILED", &error.message).await?;
            set_provider_error(state, provider.id, Some(&error.message)).await;
            return Err(error);
        }
    };
    sqlx::query(
        "UPDATE batch_image_jobs SET status = 'queued', provider_job_name = ?, \
         provider_input_ref = ?, provider_output_ref = NULLIF(?, ''), submitted_at = CURRENT_TIMESTAMP, \
         updated_at = CURRENT_TIMESTAMP WHERE id = ? AND status = 'created'",
    )
    .bind(&submitted.job_name)
    .bind(&submitted.input_ref)
    .bind(&submitted.output_ref)
    .bind(job_id)
    .execute(&state.pool)
    .await?;
    sqlx::query(
        "INSERT INTO batch_image_events (job_id, event_type, payload) VALUES (?, 'submitted', ?)",
    )
    .bind(job_id)
    .bind(json!({"provider_state": submitted.state}).to_string())
    .execute(&state.pool)
    .await?;
    set_provider_error(state, provider.id, None).await;
    let job = get_job_for_owner(
        state,
        Owner {
            user_id,
            api_key_id: Some(key.id),
        },
        &batch_id,
    )
    .await?;
    job_public(&job)
}

fn normalize_submit(mut request: SubmitRequest) -> ApiResult<SubmitRequest> {
    request.model = request
        .model
        .trim()
        .trim_start_matches("models/")
        .to_string();
    request.task_name = request.task_name.trim().chars().take(255).collect();
    request.parent_batch_id = request.parent_batch_id.trim().to_string();
    request.provider = request.provider.trim().to_ascii_lowercase();
    request.response_mime_type = request.response_mime_type.trim().to_ascii_lowercase();
    request.image_size = request.image_size.trim().to_ascii_uppercase();
    if request.response_mime_type.is_empty() {
        request.response_mime_type = "image/png".into();
    }
    if request.image_size.is_empty() {
        request.image_size = "1K".into();
    }
    if request.model.is_empty()
        || request.model.len() > 128
        || (!request.provider.is_empty()
            && !matches!(
                request.provider.as_str(),
                PROVIDER_KIND | VERTEX_PROVIDER_KIND
            ))
        || !matches!(
            request.response_mime_type.as_str(),
            "image/png" | "image/jpeg" | "image/webp"
        )
        || request.image_size != "1K"
        || request.items.is_empty()
        || request.items.len() > MAX_ITEMS
        || request.parent_batch_id.len() > 64
    {
        return Err(ApiError::bad_request(
            "BATCH_IMAGE_INVALID_ITEMS",
            "batch image request fields are invalid",
        ));
    }
    let mut expanded = Vec::new();
    let mut ids = HashSet::new();
    let mut reference_count = 0usize;
    let mut reference_bytes = 0usize;
    for (index, item) in request.items.into_iter().enumerate() {
        let mut item = item;
        item.custom_id = item.custom_id.trim().to_string();
        if item.custom_id.is_empty() {
            item.custom_id = format!("item_{:06}", index + 1);
        }
        item.prompt = item.prompt.trim().to_string();
        let output_count = if item.output_count == 0 {
            1
        } else {
            item.output_count
        };
        if item.custom_id.len() > 240
            || item.prompt.is_empty()
            || item.prompt.chars().count() > MAX_PROMPT_CHARS
            || !(1..=4).contains(&output_count)
        {
            return Err(ApiError::bad_request(
                "BATCH_IMAGE_INVALID_ITEMS",
                "batch image item is invalid",
            ));
        }
        for reference in &mut item.reference_images {
            reference.mime_type = reference.mime_type.trim().to_ascii_lowercase();
            reference.file_uri = reference.file_uri.trim().to_string();
            if !matches!(
                reference.mime_type.as_str(),
                "image/png" | "image/jpeg" | "image/webp"
            ) || (reference.data.is_empty() == reference.file_uri.is_empty())
            {
                return Err(ApiError::bad_request(
                    "BATCH_IMAGE_INVALID_REFERENCE_IMAGE",
                    "reference image must contain exactly one valid data or file_uri value",
                ));
            }
            if !reference.data.is_empty() {
                let decoded = STANDARD.decode(&reference.data).map_err(|_| {
                    ApiError::bad_request(
                        "BATCH_IMAGE_INVALID_REFERENCE_IMAGE",
                        "reference image data is not valid base64",
                    )
                })?;
                if decoded.len() > MAX_REFERENCE_BYTES_EACH {
                    return Err(ApiError::bad_request(
                        "BATCH_IMAGE_REFERENCE_IMAGES_TOO_LARGE",
                        "a reference image exceeds 10 MiB",
                    ));
                }
                reference_bytes = reference_bytes.saturating_add(decoded.len() * output_count);
            }
            reference_count = reference_count.saturating_add(output_count);
        }
        for repeat in 1..=output_count {
            let mut expanded_item = item.clone();
            expanded_item.output_count = 1;
            if output_count > 1 {
                expanded_item.custom_id = format!("{}_{repeat:02}", item.custom_id);
            }
            if !ids.insert(expanded_item.custom_id.clone()) {
                return Err(ApiError::bad_request(
                    "BATCH_IMAGE_DUPLICATE_CUSTOM_ID",
                    "batch image custom ids must be unique",
                ));
            }
            expanded.push(expanded_item);
        }
    }
    if expanded.len() > MAX_OUTPUT_IMAGES
        || reference_count > MAX_REFERENCE_IMAGES
        || reference_bytes > MAX_REFERENCE_BYTES
    {
        return Err(ApiError::bad_request(
            "BATCH_IMAGE_LIMIT_EXCEEDED",
            "batch image output or reference image limit was exceeded",
        ));
    }
    request.items = expanded;
    request
        .metadata
        .retain(|key, value| !key.trim().is_empty() && key.len() <= 64 && value.len() <= 256);
    Ok(request)
}

fn rounded_bps(value: i64, bps: i64) -> i64 {
    value.saturating_mul(bps).saturating_add(9_999) / 10_000
}

async fn reserve_balance(
    transaction: &mut Transaction<'_, Sqlite>,
    user_id: i64,
    amount: i64,
    batch_id: &str,
) -> ApiResult<()> {
    if amount == 0 {
        return Ok(());
    }
    let result = sqlx::query(
        "UPDATE users SET balance_cents = balance_cents - ?, \
         frozen_balance_cents = frozen_balance_cents + ?, updated_at = CURRENT_TIMESTAMP \
         WHERE id = ? AND enabled = 1 AND deleted_at IS NULL AND balance_cents >= ?",
    )
    .bind(amount)
    .bind(amount)
    .bind(user_id)
    .bind(amount)
    .execute(&mut **transaction)
    .await?;
    if result.rows_affected() != 1 {
        return Err(ApiError::new(
            StatusCode::PAYMENT_REQUIRED,
            "BATCH_IMAGE_INSUFFICIENT_BALANCE",
            "insufficient balance for batch image hold",
        ));
    }
    let balance: i64 = sqlx::query_scalar("SELECT balance_cents FROM users WHERE id = ?")
        .bind(user_id)
        .fetch_one(&mut **transaction)
        .await?;
    sqlx::query(
        "INSERT INTO user_balance_adjustments \
         (user_id, delta_cents, balance_after_cents, reason) VALUES (?, ?, ?, ?)",
    )
    .bind(user_id)
    .bind(-amount)
    .bind(balance)
    .bind(format!("batch image hold {batch_id}"))
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn append_event(
    transaction: &mut Transaction<'_, Sqlite>,
    job_id: i64,
    event_type: &str,
    payload: Value,
) -> ApiResult<()> {
    sqlx::query("INSERT INTO batch_image_events (job_id, event_type, payload) VALUES (?, ?, ?)")
        .bind(job_id)
        .bind(event_type)
        .bind(payload.to_string())
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

async fn select_provider(
    state: &AppState,
    model: &str,
    requested_kind: &str,
) -> ApiResult<ProviderRow> {
    let rows = sqlx::query_as::<_, ProviderRow>(
        "SELECT providers.id, providers.name, providers.provider_type AS kind, providers.base_url, \
         providers.encrypted_api_key, providers.encrypted_service_account_json, \
         providers.project_id, providers.location, providers.gcs_bucket, providers.gcs_prefix, \
         providers.gcs_base_url, providers.token_url, providers.models, providers.unit_price_cents, \
         providers.batch_discount_bps, providers.hold_bps, providers.priority, \
         providers.concurrency, providers.enabled, providers.last_used_at, providers.last_error, \
         providers.created_at, providers.updated_at FROM batch_image_providers providers \
         WHERE providers.enabled = 1 AND (? = '' OR providers.provider_type = ?) \
         AND EXISTS (SELECT 1 FROM json_each(providers.models) WHERE value = ?) \
         AND (SELECT COUNT(*) FROM batch_image_jobs jobs WHERE jobs.provider_id = providers.id \
              AND jobs.status IN ('created','queued','running','indexing','settling')) < providers.concurrency \
         ORDER BY providers.priority, providers.id",
    )
    .bind(requested_kind)
    .bind(requested_kind)
    .bind(model)
    .fetch_all(&state.pool)
    .await?;
    rows.into_iter().next().ok_or_else(|| {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            "BATCH_IMAGE_NO_PROVIDER_AVAILABLE",
            "no enabled batch image provider supports this model or has capacity",
        )
    })
}

#[derive(Debug)]
struct ProviderSubmission {
    job_name: String,
    input_ref: String,
    output_ref: String,
    state: String,
}

fn build_gemini_jsonl(request: &SubmitRequest) -> ApiResult<Vec<u8>> {
    let mut output = Vec::new();
    for item in &request.items {
        let mut parts = vec![json!({"text": item.prompt})];
        for reference in &item.reference_images {
            if !reference.data.is_empty() {
                parts.push(json!({"inlineData": {
                    "mimeType": reference.mime_type, "data": reference.data
                }}));
            } else {
                parts.push(json!({"fileData": {
                    "mimeType": reference.mime_type, "fileUri": reference.file_uri
                }}));
            }
        }
        let line = json!({
            "key": item.custom_id,
            "request": {
                "contents": [{"role":"user", "parts": parts}],
                "generationConfig": {"responseModalities": ["TEXT", "IMAGE"]}
            }
        });
        serde_json::to_writer(&mut output, &line)
            .map_err(|_| ApiError::internal("Gemini JSONL serialization failed"))?;
        output.push(b'\n');
    }
    Ok(output)
}

async fn gemini_submit(
    state: &AppState,
    provider: &ProviderRow,
    api_key: &str,
    batch_id: &str,
    model: &str,
    jsonl: Vec<u8>,
) -> ApiResult<ProviderSubmission> {
    let boundary = format!("sub2api-mini-{}", Uuid::new_v4().simple());
    let mut body = Vec::with_capacity(jsonl.len() + 1024);
    write!(
        body,
        "--{boundary}\r\nContent-Disposition: form-data; name=\"metadata\"\r\nContent-Type: application/json; charset=utf-8\r\n\r\n"
    )
    .map_err(|_| ApiError::internal("multipart creation failed"))?;
    serde_json::to_writer(
        &mut body,
        &json!({"file": {"displayName": batch_id, "mimeType": "application/jsonl"}}),
    )
    .map_err(|_| ApiError::internal("multipart creation failed"))?;
    write!(
        body,
        "\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"batch.jsonl\"\r\nContent-Type: application/jsonl\r\n\r\n"
    )
    .map_err(|_| ApiError::internal("multipart creation failed"))?;
    body.extend_from_slice(&jsonl);
    write!(body, "\r\n--{boundary}--\r\n")
        .map_err(|_| ApiError::internal("multipart creation failed"))?;
    let uploaded = state
        .client
        .post(format!(
            "{}/upload/v1beta/files?uploadType=multipart",
            provider.base_url
        ))
        .header("x-goog-api-key", api_key)
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(body)
        .send()
        .await?;
    let uploaded = checked_json(uploaded, 1024 * 1024, "Gemini file upload failed").await?;
    let input_ref = uploaded
        .pointer("/file/name")
        .or_else(|| uploaded.get("name"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::BAD_GATEWAY,
                "GEMINI_INVALID_RESPONSE",
                "Gemini upload response has no file name",
            )
        })?
        .to_string();
    let encoded_model: String = url::form_urlencoded::byte_serialize(model.as_bytes()).collect();
    let created = state
        .client
        .post(format!(
            "{}/v1beta/models/{encoded_model}:batchGenerateContent",
            provider.base_url
        ))
        .header("x-goog-api-key", api_key)
        .json(&json!({"batch": {
            "displayName": batch_id, "inputConfig": {"fileName": input_ref}
        }}))
        .send()
        .await?;
    let created = checked_json(created, 1024 * 1024, "Gemini batch creation failed").await?;
    let job_name = created
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::BAD_GATEWAY,
                "GEMINI_INVALID_RESPONSE",
                "Gemini batch response has no job name",
            )
        })?
        .to_string();
    Ok(ProviderSubmission {
        job_name,
        input_ref,
        output_ref: gemini_output_ref(&created).unwrap_or_default(),
        state: created
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or("JOB_STATE_PENDING")
            .to_string(),
    })
}

#[derive(Debug)]
struct VertexManagedRefs {
    input_uri: String,
    output_prefix: String,
}

fn encode_path(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

fn vertex_api_base(provider: &ProviderRow) -> String {
    if !provider.base_url.is_empty() {
        return provider.base_url.trim_end_matches('/').to_string();
    }
    if provider.location == "global" {
        "https://aiplatform.googleapis.com".into()
    } else {
        format!("https://{}-aiplatform.googleapis.com", provider.location)
    }
}

fn vertex_collection_url(provider: &ProviderRow) -> ApiResult<String> {
    if provider.project_id.is_empty()
        || provider.location.is_empty()
        || !provider.location.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
    {
        return Err(ApiError::internal(
            "stored Vertex project or location is invalid",
        ));
    }
    Ok(format!(
        "{}/v1/projects/{}/locations/{}/batchPredictionJobs",
        vertex_api_base(provider),
        encode_path(&provider.project_id),
        encode_path(&provider.location)
    ))
}

fn vertex_resource_url(provider: &JobRow, name: &str) -> ApiResult<String> {
    let name = name.trim().trim_start_matches('/');
    if name.is_empty()
        || !name.starts_with("projects/")
        || name.contains("..")
        || name.contains('?')
        || name.contains('#')
    {
        return Err(ApiError::internal(
            "stored Vertex batch job name is invalid",
        ));
    }
    let base = if !provider.provider_base_url.is_empty() {
        provider.provider_base_url.trim_end_matches('/').to_string()
    } else if provider.provider_location == "global" {
        "https://aiplatform.googleapis.com".into()
    } else {
        format!(
            "https://{}-aiplatform.googleapis.com",
            provider.provider_location
        )
    };
    Ok(format!("{base}/v1/{name}"))
}

fn vertex_managed_refs(provider: &ProviderRow, batch_id: &str) -> ApiResult<VertexManagedRefs> {
    if provider.gcs_bucket.is_empty()
        || provider.gcs_bucket.contains('/')
        || provider.gcs_bucket.contains("://")
        || !batch_id.starts_with("imgbatch_")
        || batch_id.len() > 64
    {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "VERTEX_MANAGED_GCS_BUCKET_MISSING",
            "Vertex managed GCS settings are invalid",
        ));
    }
    let prefix = provider
        .gcs_prefix
        .replace("{batch_id}", batch_id)
        .trim_matches('/')
        .to_string();
    if !prefix.contains(batch_id) || prefix.contains("..") || prefix.contains('\\') {
        return Err(ApiError::internal("stored Vertex GCS prefix is unsafe"));
    }
    let base = format!("gs://{}/{}", provider.gcs_bucket, prefix);
    Ok(VertexManagedRefs {
        input_uri: format!("{base}/input/requests.jsonl"),
        output_prefix: format!("{base}/output/"),
    })
}

async fn vertex_submit(
    state: &AppState,
    provider: &ProviderRow,
    batch_id: &str,
    model: &str,
    jsonl: Vec<u8>,
) -> ApiResult<ProviderSubmission> {
    let token = vertex_access_token(state, provider).await?;
    let refs = vertex_managed_refs(provider, batch_id)?;
    vertex_gcs_upload(state, provider, &token, &refs.input_uri, jsonl).await?;
    let model = model.trim().trim_matches('/');
    let model = if model.starts_with("publishers/") || model.starts_with("projects/") {
        model.to_string()
    } else {
        format!("publishers/google/models/{model}")
    };
    let response = state
        .client
        .post(vertex_collection_url(provider)?)
        .bearer_auth(&token)
        .json(&json!({
            "displayName":format!("sub2api-{batch_id}"),
            "model":model,
            "inputConfig":{"instancesFormat":"jsonl","gcsSource":{"uris":[&refs.input_uri]}},
            "outputConfig":{"predictionsFormat":"jsonl","gcsDestination":{"outputUriPrefix":&refs.output_prefix}},
            "instanceConfig":{"keyField":"key"}
        }))
        .send()
        .await?;
    let created = match checked_vertex_json(response, "Vertex batch creation failed").await {
        Ok(value) => value,
        Err(error) => {
            let _ = vertex_gcs_delete_object(state, provider, &token, &refs.input_uri).await;
            return Err(error);
        }
    };
    let job_name = created
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::BAD_GATEWAY,
                "VERTEX_INVALID_RESPONSE",
                "Vertex batch response has no job name",
            )
        })?
        .to_string();
    Ok(ProviderSubmission {
        job_name,
        input_ref: refs.input_uri,
        output_ref: created
            .pointer("/outputConfig/gcsDestination/outputUriPrefix")
            .and_then(Value::as_str)
            .unwrap_or(&refs.output_prefix)
            .to_string(),
        state: created
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or("JOB_STATE_PENDING")
            .to_string(),
    })
}

fn parse_gcs_uri(uri: &str) -> ApiResult<(&str, &str)> {
    let value = uri
        .trim()
        .strip_prefix("gs://")
        .ok_or_else(|| ApiError::internal("stored Vertex GCS reference is invalid"))?;
    let (bucket, object) = value
        .split_once('/')
        .ok_or_else(|| ApiError::internal("stored Vertex GCS reference is invalid"))?;
    if bucket.is_empty() || object.is_empty() || object.contains("..") || object.contains('\\') {
        return Err(ApiError::internal("stored Vertex GCS reference is unsafe"));
    }
    Ok((bucket, object))
}

async fn vertex_gcs_upload(
    state: &AppState,
    provider: &ProviderRow,
    token: &str,
    uri: &str,
    body: Vec<u8>,
) -> ApiResult<()> {
    let (bucket, object) = parse_gcs_uri(uri)?;
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("uploadType", "media")
        .append_pair("name", object)
        .finish();
    let response = state
        .client
        .post(format!(
            "{}/upload/storage/v1/b/{}/o?{query}",
            provider.gcs_base_url,
            encode_path(bucket)
        ))
        .bearer_auth(token)
        .header(header::CONTENT_TYPE, "application/jsonl")
        .body(body)
        .send()
        .await?;
    if !response.status().is_success() {
        return Err(provider_http_error(
            VERTEX_PROVIDER_KIND,
            response.status(),
            "Vertex managed GCS upload failed",
        ));
    }
    Ok(())
}

async fn checked_vertex_json(
    response: reqwest::Response,
    message: &'static str,
) -> ApiResult<Value> {
    let status = response.status();
    let bytes = response.bytes().await?;
    if !status.is_success() {
        return Err(provider_http_error(VERTEX_PROVIDER_KIND, status, message));
    }
    if bytes.len() > 2 * 1024 * 1024 {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "VERTEX_INVALID_RESPONSE",
            "Vertex response is too large",
        ));
    }
    serde_json::from_slice(&bytes).map_err(|_| {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            "VERTEX_INVALID_RESPONSE",
            "Vertex response is not valid JSON",
        )
    })
}

async fn checked_json(
    response: reqwest::Response,
    max_bytes: usize,
    message: &'static str,
) -> ApiResult<Value> {
    let status = response.status();
    let bytes = response.bytes().await?;
    if !status.is_success() {
        return Err(gemini_error(status, message));
    }
    if bytes.len() > max_bytes {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "GEMINI_INVALID_RESPONSE",
            "Gemini response is too large",
        ));
    }
    serde_json::from_slice(&bytes).map_err(|_| {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            "GEMINI_INVALID_RESPONSE",
            "Gemini response is not valid JSON",
        )
    })
}

fn gemini_error(status: reqwest::StatusCode, message: &'static str) -> ApiError {
    let code = match status.as_u16() {
        401 | 403 => "GEMINI_AUTH_FAILED",
        429 => "GEMINI_RATE_LIMITED",
        404 => "GEMINI_RESOURCE_NOT_FOUND",
        _ => "GEMINI_REQUEST_FAILED",
    };
    ApiError::new(StatusCode::BAD_GATEWAY, code, message)
}

fn provider_http_error(kind: &str, status: reqwest::StatusCode, message: &'static str) -> ApiError {
    if kind != VERTEX_PROVIDER_KIND {
        return gemini_error(status, message);
    }
    let code = match status.as_u16() {
        401 => "VERTEX_AUTH_FAILED",
        403 => "VERTEX_PERMISSION_DENIED",
        404 => "VERTEX_RESOURCE_NOT_FOUND",
        429 => "VERTEX_RATE_LIMITED",
        _ => "VERTEX_REQUEST_FAILED",
    };
    ApiError::new(StatusCode::BAD_GATEWAY, code, message)
}

fn gemini_output_ref(value: &Value) -> Option<String> {
    [
        "/dest/fileName",
        "/dest/file_name",
        "/response/responsesFile",
        "/response/responses_file",
    ]
    .into_iter()
    .find_map(|path| {
        value
            .pointer(path)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    })
}

async fn set_provider_error(state: &AppState, id: i64, error: Option<&str>) {
    let _ = sqlx::query(
        "UPDATE batch_image_providers SET last_error = ?, last_used_at = CURRENT_TIMESTAMP, \
         updated_at = CURRENT_TIMESTAMP WHERE id = ?",
    )
    .bind(error.map(|value| value.chars().take(500).collect::<String>()))
    .bind(id)
    .execute(&state.pool)
    .await;
}

async fn find_idempotent_job(
    state: &AppState,
    api_key_id: i64,
    idempotency_key: &str,
) -> ApiResult<Option<JobRow>> {
    sqlx::query_as::<_, JobRow>(&format!(
        "{JOB_SELECT} WHERE jobs.api_key_id = ? AND jobs.idempotency_key = ?"
    ))
    .bind(api_key_id)
    .bind(idempotency_key)
    .fetch_optional(&state.pool)
    .await
    .map_err(Into::into)
}

async fn get_job_by_batch(state: &AppState, batch_id: &str) -> ApiResult<JobRow> {
    sqlx::query_as::<_, JobRow>(&format!("{JOB_SELECT} WHERE jobs.batch_id = ?"))
        .bind(batch_id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(batch_not_found)
}

async fn get_job_for_owner(state: &AppState, owner: Owner, batch_id: &str) -> ApiResult<JobRow> {
    sqlx::query_as::<_, JobRow>(&format!(
        "{JOB_SELECT} WHERE jobs.batch_id = ? AND jobs.user_id = ? \
         AND (? IS NULL OR jobs.api_key_id = ?) AND jobs.user_deleted_at IS NULL"
    ))
    .bind(batch_id)
    .bind(owner.user_id)
    .bind(owner.api_key_id)
    .bind(owner.api_key_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(batch_not_found)
}

fn batch_not_found() -> ApiError {
    ApiError::new(
        StatusCode::NOT_FOUND,
        "BATCH_IMAGE_NOT_FOUND",
        "batch image job not found",
    )
}

fn job_public(job: &JobRow) -> ApiResult<Value> {
    Ok(json!({
        "id": job.batch_id, "object": "image.batch", "task_name": job.task_name,
        "parent_batch_id": job.parent_batch_id, "status": job.status, "model": job.model,
        "provider": job.provider_kind, "provider_name": job.provider_name,
        "item_count": job.item_count, "requested_image_count": job.requested_image_count,
        "success_count": job.success_count, "fail_count": job.fail_count,
        "generated_image_count": job.generated_image_count,
        "estimated_cost": job.estimated_cost_cents as f64 / 100.0,
        "hold_amount": job.hold_amount_cents as f64 / 100.0,
        "actual_cost": job.actual_cost_cents.map(|value| value as f64 / 100.0),
        "estimated_cost_cents": job.estimated_cost_cents,
        "hold_amount_cents": job.hold_amount_cents,
        "actual_cost_cents": job.actual_cost_cents,
        "created_at": timestamp(&job.created_at),
        "submitted_at": job.submitted_at.as_deref().map(timestamp),
        "settled_at": job.settled_at.as_deref().map(timestamp),
        "downloaded_at": job.downloaded_at.as_deref().map(timestamp),
        "output_deleted_at": job.output_deleted_at.as_deref().map(timestamp),
        "error": job.last_error_code.as_ref().map(|code| json!({
            "code": code, "message": job.last_error_message.as_deref().unwrap_or("batch image job failed")
        }))
    }))
}

fn timestamp(value: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|value| value.timestamp())
        .or_else(|_| {
            chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S")
                .map(|value| value.and_utc().timestamp())
        })
        .unwrap_or(0)
}

fn owner_from_key(key: &ApiKeyContext) -> ApiResult<Owner> {
    Ok(Owner {
        user_id: key.user_id.ok_or_else(|| {
            ApiError::new(
                StatusCode::FORBIDDEN,
                "BATCH_IMAGE_USER_REQUIRED",
                "batch image jobs require a user-owned API key",
            )
        })?,
        api_key_id: Some(key.id),
    })
}

#[derive(Default, Deserialize)]
struct ListQuery {
    status: Option<String>,
    task_name: Option<String>,
    downloaded: Option<String>,
    cursor: Option<i64>,
    limit: Option<i64>,
    api_key_id: Option<i64>,
}

async fn gateway_list_jobs(
    State(state): State<AppState>,
    Extension(key): Extension<ApiKeyContext>,
    Query(query): Query<ListQuery>,
) -> ApiResult<Json<Value>> {
    Ok(Json(list_jobs(&state, owner_from_key(&key)?, query).await?))
}

async fn user_list_jobs(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
    Query(query): Query<ListQuery>,
) -> ApiResult<Json<Value>> {
    if let Some(key_id) = query.api_key_id {
        let _ = load_user_key(&state, session.user_id, key_id).await?;
    }
    Ok(Json(
        list_jobs(
            &state,
            Owner {
                user_id: session.user_id,
                api_key_id: query.api_key_id,
            },
            query,
        )
        .await?,
    ))
}

async fn list_jobs(state: &AppState, owner: Owner, query: ListQuery) -> ApiResult<Value> {
    let status = query.status.unwrap_or_default().trim().to_string();
    if !status.is_empty()
        && !matches!(
            status.as_str(),
            "created"
                | "queued"
                | "running"
                | "indexing"
                | "settling"
                | "completed"
                | "failed"
                | "cancelled"
                | "output_deleted"
        )
    {
        return Err(ApiError::bad_request(
            "INVALID_BATCH_IMAGE_STATUS",
            "batch image status filter is invalid",
        ));
    }
    let task_name = query.task_name.unwrap_or_default().trim().to_string();
    let downloaded = query.downloaded.unwrap_or_default();
    if !matches!(downloaded.as_str(), "" | "true" | "false") {
        return Err(ApiError::bad_request(
            "INVALID_BATCH_IMAGE_FILTER",
            "downloaded filter must be true or false",
        ));
    }
    let limit = query.limit.unwrap_or(20).clamp(1, 100);
    let rows = sqlx::query_as::<_, JobRow>(&format!(
        "{JOB_SELECT} WHERE jobs.user_id = ? AND (? IS NULL OR jobs.api_key_id = ?) \
         AND jobs.user_deleted_at IS NULL AND (? = '' OR jobs.status = ?) \
         AND (? = '' OR jobs.task_name LIKE '%' || ? || '%') \
         AND (? = '' OR (? = 'true' AND jobs.downloaded_at IS NOT NULL) \
              OR (? = 'false' AND jobs.downloaded_at IS NULL)) \
         AND (? IS NULL OR jobs.id < ?) ORDER BY jobs.id DESC LIMIT ?"
    ))
    .bind(owner.user_id)
    .bind(owner.api_key_id)
    .bind(owner.api_key_id)
    .bind(&status)
    .bind(&status)
    .bind(&task_name)
    .bind(&task_name)
    .bind(&downloaded)
    .bind(&downloaded)
    .bind(&downloaded)
    .bind(query.cursor)
    .bind(query.cursor)
    .bind(limit + 1)
    .fetch_all(&state.pool)
    .await?;
    let has_more = rows.len() > limit as usize;
    let data = rows
        .iter()
        .take(limit as usize)
        .map(job_public)
        .collect::<ApiResult<Vec<_>>>()?;
    let next_cursor = if has_more {
        rows.get(limit as usize - 1).map(|job| job.id)
    } else {
        None
    };
    Ok(json!({"object":"list", "data":data, "has_more":has_more, "next_cursor":next_cursor}))
}

async fn gateway_get_job(
    State(state): State<AppState>,
    Extension(key): Extension<ApiKeyContext>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(job_public(
        &get_job_for_owner(&state, owner_from_key(&key)?, &id).await?,
    )?))
}

async fn user_get_job(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(job_public(
        &get_job_for_owner(
            &state,
            Owner {
                user_id: session.user_id,
                api_key_id: None,
            },
            &id,
        )
        .await?,
    )?))
}

#[derive(Default, Deserialize)]
struct ItemQuery {
    status: Option<String>,
    cursor: Option<i64>,
    limit: Option<i64>,
    image_index: Option<usize>,
}

async fn gateway_list_items(
    State(state): State<AppState>,
    Extension(key): Extension<ApiKeyContext>,
    Path(id): Path<String>,
    Query(query): Query<ItemQuery>,
) -> ApiResult<Json<Value>> {
    Ok(Json(
        list_items(&state, owner_from_key(&key)?, &id, query).await?,
    ))
}

async fn user_list_items(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
    Path(id): Path<String>,
    Query(query): Query<ItemQuery>,
) -> ApiResult<Json<Value>> {
    Ok(Json(
        list_items(
            &state,
            Owner {
                user_id: session.user_id,
                api_key_id: None,
            },
            &id,
            query,
        )
        .await?,
    ))
}

async fn list_items(
    state: &AppState,
    owner: Owner,
    batch_id: &str,
    query: ItemQuery,
) -> ApiResult<Value> {
    let job = get_job_for_owner(state, owner, batch_id).await?;
    let mut status = query.status.unwrap_or_default().trim().to_string();
    if status == "succeeded" {
        status = "success".into();
    }
    if !matches!(
        status.as_str(),
        "" | "pending" | "success" | "failed" | "cancelled"
    ) {
        return Err(ApiError::bad_request(
            "INVALID_BATCH_IMAGE_STATUS",
            "batch image item status filter is invalid",
        ));
    }
    let limit = query.limit.unwrap_or(200).clamp(1, 500);
    let rows = sqlx::query_as::<_, ItemRow>(
        "SELECT id, job_id, custom_id, status, output_count, prompt_hash, mime_type, \
         file_extension, image_count, output_files, error_code, error_message, created_at, indexed_at \
         FROM batch_image_items WHERE job_id = ? AND (? = '' OR status = ?) \
         AND (? IS NULL OR id > ?) ORDER BY id LIMIT ?",
    )
    .bind(job.id)
    .bind(&status)
    .bind(&status)
    .bind(query.cursor)
    .bind(query.cursor)
    .bind(limit + 1)
    .fetch_all(&state.pool)
    .await?;
    let has_more = rows.len() > limit as usize;
    let data = rows
        .iter()
        .take(limit as usize)
        .map(item_public)
        .collect::<ApiResult<Vec<_>>>()?;
    let next_cursor = if has_more {
        rows.get(limit as usize - 1).map(|item| item.id)
    } else {
        None
    };
    Ok(json!({"object":"list", "data":data, "has_more":has_more, "next_cursor":next_cursor}))
}

fn item_public(item: &ItemRow) -> ApiResult<Value> {
    let status = if item.status == "success" {
        "succeeded"
    } else {
        item.status.as_str()
    };
    let files = parse_output_files(&item.output_files)?;
    Ok(json!({
        "custom_id": item.custom_id, "status": status, "prompt_preview": Value::Null,
        "prompt_hash": item.prompt_hash, "mime_type": item.mime_type,
        "file_extension": item.file_extension, "image_count": item.image_count,
        "output_count": item.output_count, "files": files,
        "error": item.error_code.as_ref().map(|code| json!({
            "code": code, "message": item.error_message.as_deref().unwrap_or("image generation failed"),
            "source": "provider"
        })),
        "created_at": timestamp(&item.created_at),
        "indexed_at": item.indexed_at.as_deref().map(timestamp)
    }))
}

fn parse_output_files(value: &str) -> ApiResult<Vec<String>> {
    serde_json::from_str(value)
        .map_err(|_| ApiError::internal("stored batch image output list is malformed"))
}

async fn gateway_models(
    State(state): State<AppState>,
    Extension(key): Extension<ApiKeyContext>,
) -> ApiResult<Json<Value>> {
    Ok(Json(models_for_key(&state, &key).await?))
}

async fn models_for_key(state: &AppState, key: &ApiKeyContext) -> ApiResult<Value> {
    let rows: Vec<(String, String, String, i64, i64, i64)> = sqlx::query_as(
        "SELECT providers.provider_type, providers.name, model.value, providers.unit_price_cents, \
         providers.batch_discount_bps, providers.hold_bps \
         FROM batch_image_providers providers, json_each(providers.models) model \
         WHERE providers.enabled = 1 ORDER BY providers.priority, providers.id, model.value",
    )
    .fetch_all(&state.pool)
    .await?;
    let data = rows
        .into_iter()
        .filter(|row| key.allowed_models.is_empty() || key.allowed_models.contains(&row.2))
        .map(|row| {
            json!({"id":row.2, "object":"image.batch.model", "provider":row.0,
                "provider_name":row.1, "unit_price_cents":row.3,
                "batch_discount_bps":row.4, "hold_bps":row.5})
        })
        .collect::<Vec<_>>();
    Ok(json!({"object":"list", "data":data}))
}

async fn user_bootstrap(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
) -> ApiResult<Json<Value>> {
    let keys: Vec<(i64, String, String, bool)> = sqlx::query_as(
        "SELECT id, name, token_prefix, enabled FROM api_keys WHERE user_id = ? ORDER BY id DESC",
    )
    .bind(session.user_id)
    .fetch_all(&state.pool)
    .await?;
    let mut models_by_key = Vec::new();
    for row in keys.iter().filter(|row| row.3) {
        let key = load_user_key(&state, session.user_id, row.0).await?;
        let models = models_for_key(&state, &key).await?;
        models_by_key.push(json!({"api_key_id":row.0, "models":models["data"].clone()}));
    }
    let models = models_by_key
        .first()
        .map(|entry| entry["models"].clone())
        .unwrap_or_else(|| json!([]));
    let balance: (i64, i64) =
        sqlx::query_as("SELECT balance_cents, frozen_balance_cents FROM users WHERE id = ?")
            .bind(session.user_id)
            .fetch_one(&state.pool)
            .await?;
    let jobs = list_jobs(
        &state,
        Owner {
            user_id: session.user_id,
            api_key_id: None,
        },
        ListQuery::default(),
    )
    .await?;
    Ok(Json(json!({"data": {
        "keys": keys.into_iter().map(|row| json!({"id":row.0,"name":row.1,
            "token_prefix":row.2,"enabled":row.3})).collect::<Vec<_>>(),
        "models": models, "models_by_key": models_by_key, "jobs": jobs["data"].clone(),
        "has_more": jobs["has_more"].clone(),
        "next_cursor": jobs["next_cursor"].clone(),
        "balance_cents":balance.0, "frozen_balance_cents":balance.1
    }})))
}

pub fn start_scheduler(state: AppState) {
    tokio::spawn(async move {
        loop {
            if let Err(error) = process_pending_jobs(&state).await {
                tracing::warn!(%error, "batch image scheduler pass failed");
            }
            if let Err(error) = cleanup_expired_output(&state).await {
                tracing::warn!(%error, "batch image output cleanup pass failed");
            }
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    });
}

async fn process_pending_jobs(state: &AppState) -> ApiResult<()> {
    let batch_ids: Vec<String> = sqlx::query_scalar(
        "SELECT batch_id FROM batch_image_jobs WHERE status IN \
         ('created','queued','running','indexing','settling') ORDER BY updated_at, id LIMIT 10",
    )
    .fetch_all(&state.pool)
    .await?;
    for batch_id in batch_ids {
        if let Err(error) = process_job(state, &batch_id).await {
            tracing::warn!(batch_id, code = error.code, message = %error.message, "batch image processing failed");
            record_process_error(state, &batch_id, &error).await?;
        }
    }
    Ok(())
}

async fn process_job(state: &AppState, batch_id: &str) -> ApiResult<()> {
    let mut job = get_job_by_batch(state, batch_id).await?;
    if job.status == "created" {
        let age = Utc::now().timestamp() - timestamp(&job.created_at);
        if age > 300 {
            fail_job(
                state,
                batch_id,
                "SUBMIT_RECOVERY_FAILED",
                "provider submission did not complete before restart",
            )
            .await?;
        }
        return Ok(());
    }
    if job.status == "settling" {
        return settle_job(state, &job).await;
    }
    if job.status == "indexing" {
        index_job(state, &job).await?;
        job = get_job_by_batch(state, batch_id).await?;
        return settle_job(state, &job).await;
    }
    if !matches!(job.status.as_str(), "queued" | "running") {
        return Ok(());
    }
    let provider_state = if job.provider_kind == VERTEX_PROVIDER_KIND {
        vertex_get_batch(state, &job).await?
    } else {
        let api_key = decrypt_job_provider_key(state, &job)?;
        gemini_get_batch(state, &job, &api_key).await?
    };
    match provider_state.state.as_str() {
        "JOB_STATE_PENDING" | "JOB_STATE_QUEUED" => {
            sqlx::query(
                "UPDATE batch_image_jobs SET status = 'queued', updated_at = CURRENT_TIMESTAMP, \
                 last_error_code = NULL, last_error_message = NULL WHERE id = ? AND status IN ('queued','running')",
            )
            .bind(job.id)
            .execute(&state.pool)
            .await?;
        }
        "JOB_STATE_RUNNING" => {
            sqlx::query(
                "UPDATE batch_image_jobs SET status = 'running', started_at = COALESCE(started_at, CURRENT_TIMESTAMP), \
                 updated_at = CURRENT_TIMESTAMP, last_error_code = NULL, last_error_message = NULL \
                 WHERE id = ? AND status IN ('queued','running')",
            )
            .bind(job.id)
            .execute(&state.pool)
            .await?;
        }
        "JOB_STATE_SUCCEEDED" => {
            let output_ref = provider_state.output_ref.ok_or_else(|| {
                ApiError::new(
                    StatusCode::BAD_GATEWAY,
                    "BATCH_IMAGE_RESULT_MISSING",
                    "provider batch succeeded without a result reference",
                )
            })?;
            sqlx::query(
                "UPDATE batch_image_jobs SET status = 'indexing', provider_output_ref = ?, \
                 finished_at = COALESCE(finished_at, CURRENT_TIMESTAMP), updated_at = CURRENT_TIMESTAMP, \
                 last_error_code = NULL, last_error_message = NULL WHERE id = ? AND status IN ('queued','running')",
            )
            .bind(output_ref)
            .bind(job.id)
            .execute(&state.pool)
            .await?;
            job = get_job_by_batch(state, batch_id).await?;
            index_job(state, &job).await?;
            job = get_job_by_batch(state, batch_id).await?;
            settle_job(state, &job).await?;
        }
        "JOB_STATE_FAILED" | "JOB_STATE_EXPIRED" => {
            fail_job(
                state,
                batch_id,
                if job.provider_kind == VERTEX_PROVIDER_KIND {
                    "VERTEX_BATCH_FAILED"
                } else {
                    "GEMINI_BATCH_FAILED"
                },
                provider_state
                    .error
                    .as_deref()
                    .unwrap_or("batch image provider failed"),
            )
            .await?;
        }
        "JOB_STATE_CANCELLED" => {
            finish_cancelled(state, batch_id).await?;
        }
        _ => {
            return Err(ApiError::new(
                StatusCode::BAD_GATEWAY,
                "PROVIDER_INVALID_RESPONSE",
                "batch image provider returned an unknown state",
            ));
        }
    }
    set_provider_error(state, job.provider_id, None).await;
    Ok(())
}

struct ProviderState {
    state: String,
    output_ref: Option<String>,
    error: Option<String>,
}

async fn gemini_get_batch(
    state: &AppState,
    job: &JobRow,
    api_key: &str,
) -> ApiResult<ProviderState> {
    let job_name = job
        .provider_job_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError::internal("batch image provider job name is missing"))?;
    let response = state
        .client
        .get(format!(
            "{}/v1beta/{}",
            job.provider_base_url,
            job_name.trim_start_matches('/')
        ))
        .header("x-goog-api-key", api_key)
        .send()
        .await?;
    let value = checked_json(response, 1024 * 1024, "Gemini batch status failed").await?;
    let error = value
        .pointer("/error/message")
        .and_then(Value::as_str)
        .map(|value| value.chars().take(500).collect::<String>());
    Ok(ProviderState {
        state: value
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_ascii_uppercase(),
        output_ref: gemini_output_ref(&value),
        error,
    })
}

async fn vertex_get_batch(state: &AppState, job: &JobRow) -> ApiResult<ProviderState> {
    let job_name = job
        .provider_job_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError::internal("Vertex batch job name is missing"))?;
    let provider = provider_from_job(job);
    let token = vertex_access_token(state, &provider).await?;
    let response = state
        .client
        .get(vertex_resource_url(job, job_name)?)
        .bearer_auth(token)
        .send()
        .await?;
    let value = checked_vertex_json(response, "Vertex batch status failed").await?;
    Ok(ProviderState {
        state: value
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_ascii_uppercase(),
        output_ref: value
            .pointer("/outputConfig/gcsDestination/outputUriPrefix")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .or_else(|| job.provider_output_ref.clone()),
        error: value
            .pointer("/error/message")
            .and_then(Value::as_str)
            .map(|value| value.chars().take(500).collect()),
    })
}

fn provider_from_job(job: &JobRow) -> ProviderRow {
    ProviderRow {
        id: job.provider_id,
        name: job.provider_name.clone(),
        kind: job.provider_kind.clone(),
        base_url: job.provider_base_url.clone(),
        encrypted_api_key: job.encrypted_api_key.clone(),
        encrypted_service_account_json: job.encrypted_service_account_json.clone(),
        project_id: job.provider_project_id.clone(),
        location: job.provider_location.clone(),
        gcs_bucket: job.provider_gcs_bucket.clone(),
        gcs_prefix: job.provider_gcs_prefix.clone(),
        gcs_base_url: job.provider_gcs_base_url.clone(),
        token_url: job.provider_token_url.clone(),
        models: "[]".into(),
        unit_price_cents: 0,
        batch_discount_bps: 0,
        hold_bps: 0,
        priority: 0,
        concurrency: 1,
        enabled: true,
        last_used_at: None,
        last_error: None,
        created_at: String::new(),
        updated_at: String::new(),
    }
}

fn decrypt_job_provider_key(state: &AppState, job: &JobRow) -> ApiResult<String> {
    String::from_utf8(state.crypto.decrypt(&job.encrypted_api_key)?)
        .map_err(|_| ApiError::internal("stored Gemini API key is malformed"))
}

async fn index_job(state: &AppState, job: &JobRow) -> ApiResult<()> {
    if job.status != "indexing" {
        return Ok(());
    }
    let output_ref = job
        .provider_output_ref
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::BAD_GATEWAY,
                "BATCH_IMAGE_RESULT_MISSING",
                "batch image result reference is missing",
            )
        })?;
    let directory = job_directory(state, &job.batch_id)?;
    tokio::fs::create_dir_all(&directory).await?;
    let result_path = directory.join("result.jsonl");
    if !result_path.exists() {
        if job.provider_kind == VERTEX_PROVIDER_KIND {
            download_vertex_results(state, job, output_ref, &result_path).await?;
        } else {
            let api_key = decrypt_job_provider_key(state, job)?;
            download_gemini_result(state, job, &api_key, output_ref, &result_path).await?;
        }
    }
    let expected_rows: Vec<(i64, String)> =
        sqlx::query_as("SELECT id, custom_id FROM batch_image_items WHERE job_id = ? ORDER BY id")
            .bind(job.id)
            .fetch_all(&state.pool)
            .await?;
    let expected = expected_rows
        .iter()
        .map(|row| (row.1.clone(), row.0))
        .collect::<HashMap<_, _>>();
    let path = result_path.clone();
    let output_directory = directory.clone();
    let indexed =
        tokio::task::spawn_blocking(move || index_result_file(&path, &output_directory, &expected))
            .await
            .map_err(|_| ApiError::internal("batch result indexing task failed"))??;
    let mut transaction = state.pool.begin().await?;
    let mut success_count = 0i64;
    let mut fail_count = 0i64;
    let mut generated_count = 0i64;
    for item in indexed {
        if item.status == "success" {
            success_count += 1;
            generated_count += item.files.len() as i64;
        } else {
            fail_count += 1;
        }
        sqlx::query(
            "UPDATE batch_image_items SET status = ?, mime_type = ?, file_extension = ?, \
             image_count = ?, output_files = ?, error_code = ?, error_message = ?, \
             indexed_at = CURRENT_TIMESTAMP WHERE id = ? AND job_id = ?",
        )
        .bind(&item.status)
        .bind(&item.mime_type)
        .bind(&item.extension)
        .bind(item.files.len() as i64)
        .bind(serde_json::to_string(&item.files).unwrap_or_else(|_| "[]".into()))
        .bind(&item.error_code)
        .bind(&item.error_message)
        .bind(item.id)
        .bind(job.id)
        .execute(&mut *transaction)
        .await?;
    }
    sqlx::query(
        "UPDATE batch_image_jobs SET status = 'settling', success_count = ?, fail_count = ?, \
         generated_image_count = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ? AND status = 'indexing'",
    )
    .bind(success_count)
    .bind(fail_count)
    .bind(generated_count)
    .bind(job.id)
    .execute(&mut *transaction)
    .await?;
    append_event(
        &mut transaction,
        job.id,
        "indexed",
        json!({"success_count":success_count,"fail_count":fail_count,"image_count":generated_count}),
    )
    .await?;
    transaction.commit().await?;
    if let Some(input_ref) = job.provider_input_ref.as_deref() {
        if job.provider_kind == VERTEX_PROVIDER_KIND {
            let _ = vertex_delete_managed_input(state, job, input_ref).await;
        } else if let Ok(api_key) = decrypt_job_provider_key(state, job) {
            let _ = gemini_delete_file(state, job, &api_key, input_ref).await;
        }
    }
    Ok(())
}

async fn download_gemini_result(
    state: &AppState,
    job: &JobRow,
    api_key: &str,
    output_ref: &str,
    destination: &FsPath,
) -> ApiResult<()> {
    let metadata = state
        .client
        .get(format!(
            "{}/v1beta/{}",
            job.provider_base_url,
            output_ref.trim_start_matches('/')
        ))
        .header("x-goog-api-key", api_key)
        .send()
        .await?;
    let metadata = checked_json(metadata, 1024 * 1024, "Gemini result metadata failed").await?;
    let download_url = metadata
        .get("downloadUri")
        .or_else(|| metadata.get("download_url"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| {
            format!(
                "{}/v1beta/{}:download",
                job.provider_base_url,
                output_ref.trim_start_matches('/')
            )
        });
    validate_download_url(&download_url, &job.provider_base_url)?;
    let response = state
        .client
        .get(download_url)
        .header("x-goog-api-key", api_key)
        .send()
        .await?;
    if !response.status().is_success() {
        return Err(gemini_error(
            response.status(),
            "Gemini result download failed",
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESULT_BYTES)
    {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "BATCH_IMAGE_RESULT_TOO_LARGE",
            "Gemini result exceeds 512 MiB",
        ));
    }
    let temporary = destination.with_extension("jsonl.part");
    let mut file = tokio::fs::File::create(&temporary).await?;
    let mut total = 0u64;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        total = total.saturating_add(chunk.len() as u64);
        if total > MAX_RESULT_BYTES {
            let _ = tokio::fs::remove_file(&temporary).await;
            return Err(ApiError::new(
                StatusCode::BAD_GATEWAY,
                "BATCH_IMAGE_RESULT_TOO_LARGE",
                "Gemini result exceeds 512 MiB",
            ));
        }
        tokio::io::AsyncWriteExt::write_all(&mut file, &chunk).await?;
    }
    tokio::io::AsyncWriteExt::flush(&mut file).await?;
    drop(file);
    tokio::fs::rename(temporary, destination).await?;
    Ok(())
}

async fn vertex_list_objects(
    state: &AppState,
    provider: &ProviderRow,
    token: &str,
    prefix_uri: &str,
    jsonl_only: bool,
) -> ApiResult<Vec<String>> {
    let (bucket, prefix) = parse_gcs_uri(prefix_uri)?;
    let mut page_token = String::new();
    let mut objects = Vec::new();
    for _ in 0..100 {
        let query = {
            let mut serializer = url::form_urlencoded::Serializer::new(String::new());
            serializer
                .append_pair("prefix", prefix)
                .append_pair("maxResults", "1000");
            if !page_token.is_empty() {
                serializer.append_pair("pageToken", &page_token);
            }
            serializer.finish()
        };
        let response = state
            .client
            .get(format!(
                "{}/storage/v1/b/{}/o?{}",
                provider.gcs_base_url,
                encode_path(bucket),
                query
            ))
            .bearer_auth(token)
            .send()
            .await?;
        let value = checked_vertex_json(response, "Vertex managed GCS list failed").await?;
        if let Some(items) = value.get("items").and_then(Value::as_array) {
            for name in items
                .iter()
                .filter_map(|item| item.get("name").and_then(Value::as_str))
                .map(str::trim)
            {
                if name.starts_with(prefix)
                    && (!jsonl_only || name.ends_with(".jsonl"))
                    && !name.contains("..")
                {
                    objects.push(format!("gs://{bucket}/{name}"));
                    if objects.len() > 10_000 {
                        return Err(ApiError::new(
                            StatusCode::BAD_GATEWAY,
                            "VERTEX_RESULT_TOO_MANY_OBJECTS",
                            "Vertex result contains too many objects",
                        ));
                    }
                }
            }
        }
        page_token = value
            .get("nextPageToken")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if page_token.is_empty() {
            objects.sort();
            objects.dedup();
            return Ok(objects);
        }
    }
    Err(ApiError::new(
        StatusCode::BAD_GATEWAY,
        "VERTEX_RESULT_PAGINATION_LIMIT",
        "Vertex result object pagination exceeded the safety limit",
    ))
}

async fn vertex_open_object(
    state: &AppState,
    provider: &ProviderRow,
    token: &str,
    uri: &str,
) -> ApiResult<reqwest::Response> {
    let (bucket, object) = parse_gcs_uri(uri)?;
    let response = state
        .client
        .get(format!(
            "{}/storage/v1/b/{}/o/{}?alt=media",
            provider.gcs_base_url,
            encode_path(bucket),
            encode_path(object)
        ))
        .bearer_auth(token)
        .send()
        .await?;
    if !response.status().is_success() {
        return Err(provider_http_error(
            VERTEX_PROVIDER_KIND,
            response.status(),
            "Vertex result object download failed",
        ));
    }
    Ok(response)
}

async fn download_vertex_results(
    state: &AppState,
    job: &JobRow,
    output_ref: &str,
    destination: &FsPath,
) -> ApiResult<()> {
    let provider = provider_from_job(job);
    let token = vertex_access_token(state, &provider).await?;
    ensure_safe_vertex_output(job, output_ref)?;
    let objects = vertex_list_objects(state, &provider, &token, output_ref, true).await?;
    if objects.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "VERTEX_RESULT_OBJECTS_MISSING",
            "Vertex result objects are missing",
        ));
    }
    let temporary = destination.with_extension("jsonl.part");
    let mut file = tokio::fs::File::create(&temporary).await?;
    let mut total = 0u64;
    for (index, object) in objects.iter().enumerate() {
        if index > 0 {
            file.write_all(b"\n").await?;
            total += 1;
        }
        let response = vertex_open_object(state, &provider, &token, object).await?;
        if response
            .content_length()
            .is_some_and(|length| total.saturating_add(length) > MAX_RESULT_BYTES)
        {
            let _ = tokio::fs::remove_file(&temporary).await;
            return Err(ApiError::new(
                StatusCode::BAD_GATEWAY,
                "BATCH_IMAGE_RESULT_TOO_LARGE",
                "Vertex result exceeds 512 MiB",
            ));
        }
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            total = total.saturating_add(chunk.len() as u64);
            if total > MAX_RESULT_BYTES {
                let _ = tokio::fs::remove_file(&temporary).await;
                return Err(ApiError::new(
                    StatusCode::BAD_GATEWAY,
                    "BATCH_IMAGE_RESULT_TOO_LARGE",
                    "Vertex result exceeds 512 MiB",
                ));
            }
            file.write_all(&chunk).await?;
        }
    }
    file.flush().await?;
    drop(file);
    tokio::fs::rename(temporary, destination).await?;
    Ok(())
}

fn validate_download_url(download_url: &str, base_url: &str) -> ApiResult<()> {
    let download = url::Url::parse(download_url).map_err(|_| {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            "GEMINI_INVALID_RESPONSE",
            "Gemini download URL is invalid",
        )
    })?;
    let base = url::Url::parse(base_url)
        .map_err(|_| ApiError::internal("stored Gemini base URL is invalid"))?;
    let host = download.host_str().unwrap_or_default().to_ascii_lowercase();
    let same_host = download.host_str() == base.host_str()
        && download.port_or_known_default() == base.port_or_known_default();
    let google = host == "googleapis.com" || host.ends_with(".googleapis.com");
    let loopback = matches!(host.as_str(), "localhost" | "127.0.0.1" | "::1");
    if (download.scheme() == "https" && (same_host || google))
        || (download.scheme() == "http" && same_host && loopback)
    {
        return Ok(());
    }
    Err(ApiError::new(
        StatusCode::BAD_GATEWAY,
        "GEMINI_INVALID_RESPONSE",
        "Gemini download URL host is not allowed",
    ))
}

#[derive(Debug)]
struct IndexedItem {
    id: i64,
    status: String,
    mime_type: Option<String>,
    extension: Option<String>,
    files: Vec<String>,
    error_code: Option<String>,
    error_message: Option<String>,
}

fn index_result_file(
    path: &FsPath,
    directory: &FsPath,
    expected: &HashMap<String, i64>,
) -> ApiResult<Vec<IndexedItem>> {
    let file = std::fs::File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut line = Vec::new();
    let mut seen = HashSet::new();
    let mut indexed = Vec::new();
    loop {
        line.clear();
        let read = reader.read_until(b'\n', &mut line)?;
        if read == 0 {
            break;
        }
        if line.len() > MAX_RESULT_LINE_BYTES {
            return Err(ApiError::new(
                StatusCode::BAD_GATEWAY,
                "BATCH_IMAGE_RESULT_LINE_TOO_LARGE",
                "Gemini result contains an oversized line",
            ));
        }
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let value: Value = serde_json::from_slice(&line).map_err(|_| {
            ApiError::new(
                StatusCode::BAD_GATEWAY,
                "BATCH_IMAGE_RESULT_INVALID",
                "Gemini result JSONL is malformed",
            )
        })?;
        let custom_id = result_custom_id(&value).ok_or_else(|| {
            ApiError::new(
                StatusCode::BAD_GATEWAY,
                "BATCH_IMAGE_RESULT_INVALID",
                "Gemini result line has no custom id",
            )
        })?;
        let Some(id) = expected.get(&custom_id).copied() else {
            continue;
        };
        if !seen.insert(custom_id.clone()) {
            return Err(ApiError::new(
                StatusCode::BAD_GATEWAY,
                "BATCH_IMAGE_DUPLICATE_RESULT",
                "Gemini result contains a duplicate custom id",
            ));
        }
        let images = result_images(&value);
        if let Some((mime_type, data)) = images.into_iter().next() {
            let bytes = STANDARD.decode(data).map_err(|_| {
                ApiError::new(
                    StatusCode::BAD_GATEWAY,
                    "BATCH_IMAGE_RESULT_INVALID",
                    "Gemini image output is not valid base64",
                )
            })?;
            if bytes.len() > MAX_IMAGE_BYTES {
                return Err(ApiError::new(
                    StatusCode::BAD_GATEWAY,
                    "BATCH_IMAGE_RESULT_TOO_LARGE",
                    "a generated image exceeds 20 MiB",
                ));
            }
            let extension = image_extension(&mime_type).ok_or_else(|| {
                ApiError::new(
                    StatusCode::BAD_GATEWAY,
                    "BATCH_IMAGE_RESULT_INVALID",
                    "Gemini returned an unsupported image type",
                )
            })?;
            let filename = output_filename(&custom_id, extension);
            std::fs::write(directory.join(&filename), bytes)?;
            indexed.push(IndexedItem {
                id,
                status: "success".into(),
                mime_type: Some(mime_type),
                extension: Some(extension.into()),
                files: vec![filename],
                error_code: None,
                error_message: None,
            });
        } else {
            let (code, message) = result_error(&value);
            indexed.push(IndexedItem {
                id,
                status: "failed".into(),
                mime_type: None,
                extension: None,
                files: Vec::new(),
                error_code: Some(code),
                error_message: Some(message),
            });
        }
    }
    for (custom_id, id) in expected {
        if !seen.contains(custom_id) {
            indexed.push(IndexedItem {
                id: *id,
                status: "failed".into(),
                mime_type: None,
                extension: None,
                files: Vec::new(),
                error_code: Some("PROVIDER_RESULT_MISSING".into()),
                error_message: Some("provider output did not include this item".into()),
            });
        }
    }
    if indexed.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "BATCH_IMAGE_RESULT_EMPTY",
            "Gemini result contains no submitted items",
        ));
    }
    Ok(indexed)
}

fn result_custom_id(value: &Value) -> Option<String> {
    value
        .get("key")
        .or_else(|| value.get("custom_id"))
        .or_else(|| value.get("customId"))
        .or_else(|| value.pointer("/request/key"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn result_images(value: &Value) -> Vec<(String, String)> {
    let candidates = value
        .pointer("/response/candidates")
        .or_else(|| value.get("candidates"))
        .and_then(Value::as_array);
    let mut images = Vec::new();
    for candidate in candidates.into_iter().flatten() {
        for part in candidate
            .pointer("/content/parts")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let inline = part.get("inlineData").or_else(|| part.get("inline_data"));
            let mime = inline
                .and_then(|value| value.get("mimeType").or_else(|| value.get("mime_type")))
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or("");
            let data = inline
                .and_then(|value| value.get("data"))
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or("");
            if mime.starts_with("image/") && !data.is_empty() {
                images.push((mime.to_ascii_lowercase(), data.to_string()));
            }
        }
    }
    images
}

fn result_error(value: &Value) -> (String, String) {
    let error = value.get("error").or_else(|| value.get("status"));
    let message = error
        .and_then(|value| value.get("message").or_else(|| value.get("details")))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("provider response contained no image output")
        .chars()
        .take(500)
        .collect::<String>();
    let lower = message.to_ascii_lowercase();
    let code = if lower.contains("safety") || lower.contains("policy") || lower.contains("blocked")
    {
        "SAFETY_BLOCKED"
    } else if lower.contains("quota") || lower.contains("rate") {
        "PROVIDER_RATE_LIMITED"
    } else {
        "PROVIDER_ITEM_FAILED"
    };
    (code.into(), message)
}

fn image_extension(mime_type: &str) -> Option<&'static str> {
    match mime_type {
        "image/png" => Some("png"),
        "image/jpeg" | "image/jpg" => Some("jpg"),
        "image/webp" => Some("webp"),
        _ => None,
    }
}

fn output_filename(custom_id: &str, extension: &str) -> String {
    let mut base = custom_id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || "._-".contains(character) {
                character
            } else {
                '_'
            }
        })
        .take(80)
        .collect::<String>();
    base = base.trim_matches('.').to_string();
    if base.is_empty() {
        base = "image".into();
    }
    let hash = token_hash(custom_id);
    format!("{base}-{}.{}", &hash[..8], extension)
}

fn job_directory(state: &AppState, batch_id: &str) -> ApiResult<PathBuf> {
    if !batch_id.starts_with("imgbatch_")
        || batch_id.len() > 64
        || !batch_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        return Err(ApiError::internal("stored batch image id is unsafe"));
    }
    let data_directory = state
        .config
        .database_path
        .parent()
        .ok_or_else(|| ApiError::config("database path has no parent directory"))?;
    Ok(data_directory.join("batch_images").join(batch_id))
}

async fn settle_job(state: &AppState, job: &JobRow) -> ApiResult<()> {
    if job.status != "settling" {
        return Ok(());
    }
    let actual = job
        .generated_image_count
        .saturating_mul(job.billable_unit_price_cents);
    if actual > job.hold_amount_cents {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "BATCH_IMAGE_COST_EXCEEDS_HOLD",
            "batch image cost exceeds held balance",
        ));
    }
    let mut transaction = state.pool.begin().await?;
    let updated = sqlx::query(
        "UPDATE batch_image_jobs SET status = 'completed', actual_cost_cents = ?, \
         settled_at = CURRENT_TIMESTAMP, output_expires_at = ?, updated_at = CURRENT_TIMESTAMP, \
         last_error_code = NULL, last_error_message = NULL WHERE id = ? AND status = 'settling'",
    )
    .bind(actual)
    .bind((Utc::now() + ChronoDuration::hours(72)).to_rfc3339())
    .bind(job.id)
    .execute(&mut *transaction)
    .await?;
    if updated.rows_affected() == 0 {
        return Ok(());
    }
    capture_balance(
        &mut transaction,
        job.user_id,
        job.hold_amount_cents,
        actual,
        &job.batch_id,
    )
    .await?;
    sqlx::query(
        "INSERT INTO usage_logs (request_id, api_key_id, account_id, user_id, endpoint, model, \
         status_code, duration_ms, cost_microusd, request_type, stream) \
         VALUES (?, ?, NULL, ?, '/v1/images/batches', ?, 200, 0, ?, 'sync', 0)",
    )
    .bind(format!("batch_image_capture:{}", job.batch_id))
    .bind(job.api_key_id)
    .bind(job.user_id)
    .bind(&job.model)
    .bind(actual.saturating_mul(10_000))
    .execute(&mut *transaction)
    .await?;
    append_event(
        &mut transaction,
        job.id,
        "settled",
        json!({"actual_cost_cents":actual,"held_cents":job.hold_amount_cents}),
    )
    .await?;
    transaction.commit().await?;
    Ok(())
}

async fn capture_balance(
    transaction: &mut Transaction<'_, Sqlite>,
    user_id: i64,
    hold: i64,
    actual: i64,
    batch_id: &str,
) -> ApiResult<()> {
    if hold == 0 {
        return Ok(());
    }
    let remainder = hold - actual;
    let updated = sqlx::query(
        "UPDATE users SET balance_cents = balance_cents + ?, \
         frozen_balance_cents = frozen_balance_cents - ?, updated_at = CURRENT_TIMESTAMP \
         WHERE id = ? AND frozen_balance_cents >= ?",
    )
    .bind(remainder)
    .bind(hold)
    .bind(user_id)
    .bind(hold)
    .execute(&mut **transaction)
    .await?;
    if updated.rows_affected() != 1 {
        return Err(ApiError::internal(
            "batch image frozen balance is inconsistent",
        ));
    }
    if remainder > 0 {
        let balance: i64 = sqlx::query_scalar("SELECT balance_cents FROM users WHERE id = ?")
            .bind(user_id)
            .fetch_one(&mut **transaction)
            .await?;
        sqlx::query(
            "INSERT INTO user_balance_adjustments \
             (user_id, delta_cents, balance_after_cents, reason) VALUES (?, ?, ?, ?)",
        )
        .bind(user_id)
        .bind(remainder)
        .bind(balance)
        .bind(format!("batch image settlement remainder {batch_id}"))
        .execute(&mut **transaction)
        .await?;
    }
    Ok(())
}

async fn fail_job(state: &AppState, batch_id: &str, code: &str, message: &str) -> ApiResult<()> {
    finish_without_charge(state, batch_id, "failed", code, message).await
}

async fn finish_cancelled(state: &AppState, batch_id: &str) -> ApiResult<()> {
    finish_without_charge(
        state,
        batch_id,
        "cancelled",
        "BATCH_IMAGE_CANCELLED",
        "batch image job was cancelled",
    )
    .await
}

async fn finish_without_charge(
    state: &AppState,
    batch_id: &str,
    status: &str,
    code: &str,
    message: &str,
) -> ApiResult<()> {
    let job = get_job_by_batch(state, batch_id).await?;
    if matches!(
        job.status.as_str(),
        "completed" | "failed" | "cancelled" | "output_deleted"
    ) {
        return Ok(());
    }
    let mut transaction = state.pool.begin().await?;
    let updated = sqlx::query(
        "UPDATE batch_image_jobs SET status = ?, last_error_code = ?, last_error_message = ?, \
         finished_at = CURRENT_TIMESTAMP, updated_at = CURRENT_TIMESTAMP \
         WHERE id = ? AND status NOT IN ('completed','failed','cancelled','output_deleted')",
    )
    .bind(status)
    .bind(code)
    .bind(message.chars().take(500).collect::<String>())
    .bind(job.id)
    .execute(&mut *transaction)
    .await?;
    if updated.rows_affected() == 0 {
        return Ok(());
    }
    release_balance(
        &mut transaction,
        job.user_id,
        job.hold_amount_cents,
        &job.batch_id,
    )
    .await?;
    let item_status = if status == "cancelled" {
        "cancelled"
    } else {
        "failed"
    };
    sqlx::query(
        "UPDATE batch_image_items SET status = ?, error_code = COALESCE(error_code, ?), \
         error_message = COALESCE(error_message, ?), indexed_at = COALESCE(indexed_at, CURRENT_TIMESTAMP) \
         WHERE job_id = ? AND status = 'pending'",
    )
    .bind(item_status)
    .bind(code)
    .bind(message.chars().take(500).collect::<String>())
    .bind(job.id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "UPDATE batch_image_jobs SET \
         success_count = (SELECT COUNT(*) FROM batch_image_items WHERE job_id = ? AND status = 'success'), \
         fail_count = (SELECT COUNT(*) FROM batch_image_items WHERE job_id = ? AND status = 'failed'), \
         generated_image_count = (SELECT COALESCE(SUM(image_count), 0) FROM batch_image_items WHERE job_id = ?) \
         WHERE id = ?",
    )
    .bind(job.id)
    .bind(job.id)
    .bind(job.id)
    .bind(job.id)
    .execute(&mut *transaction)
    .await?;
    append_event(&mut transaction, job.id, status, json!({"code":code})).await?;
    transaction.commit().await?;
    Ok(())
}

async fn release_balance(
    transaction: &mut Transaction<'_, Sqlite>,
    user_id: i64,
    amount: i64,
    batch_id: &str,
) -> ApiResult<()> {
    if amount == 0 {
        return Ok(());
    }
    let updated = sqlx::query(
        "UPDATE users SET balance_cents = balance_cents + ?, \
         frozen_balance_cents = frozen_balance_cents - ?, updated_at = CURRENT_TIMESTAMP \
         WHERE id = ? AND frozen_balance_cents >= ?",
    )
    .bind(amount)
    .bind(amount)
    .bind(user_id)
    .bind(amount)
    .execute(&mut **transaction)
    .await?;
    if updated.rows_affected() != 1 {
        return Err(ApiError::internal(
            "batch image frozen balance is inconsistent",
        ));
    }
    let balance: i64 = sqlx::query_scalar("SELECT balance_cents FROM users WHERE id = ?")
        .bind(user_id)
        .fetch_one(&mut **transaction)
        .await?;
    sqlx::query(
        "INSERT INTO user_balance_adjustments \
         (user_id, delta_cents, balance_after_cents, reason) VALUES (?, ?, ?, ?)",
    )
    .bind(user_id)
    .bind(amount)
    .bind(balance)
    .bind(format!("batch image hold release {batch_id}"))
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn record_process_error(state: &AppState, batch_id: &str, error: &ApiError) -> ApiResult<()> {
    let retry_count: Option<i64> = sqlx::query_scalar(
        "UPDATE batch_image_jobs SET retry_count = retry_count + 1, last_error_code = ?, \
         last_error_message = ?, updated_at = CURRENT_TIMESTAMP WHERE batch_id = ? \
         AND status IN ('queued','running','indexing','settling') RETURNING retry_count",
    )
    .bind(error.code)
    .bind(error.message.chars().take(500).collect::<String>())
    .bind(batch_id)
    .fetch_optional(&state.pool)
    .await?;
    if retry_count.is_some_and(|count| count >= 5) {
        fail_job(
            state,
            batch_id,
            "BATCH_IMAGE_RETRY_EXHAUSTED",
            "batch image processing retry limit reached",
        )
        .await?;
    }
    Ok(())
}

async fn gemini_delete_file(
    state: &AppState,
    job: &JobRow,
    api_key: &str,
    file_ref: &str,
) -> ApiResult<()> {
    if file_ref.trim().is_empty() {
        return Ok(());
    }
    let response = state
        .client
        .delete(format!(
            "{}/v1beta/{}",
            job.provider_base_url,
            file_ref.trim_start_matches('/')
        ))
        .header("x-goog-api-key", api_key)
        .send()
        .await?;
    if response.status().is_success() || response.status() == reqwest::StatusCode::NOT_FOUND {
        Ok(())
    } else {
        Err(gemini_error(
            response.status(),
            "Gemini file cleanup failed",
        ))
    }
}

fn ensure_safe_vertex_input(job: &JobRow, uri: &str) -> ApiResult<()> {
    let provider = provider_from_job(job);
    let refs = vertex_managed_refs(&provider, &job.batch_id)?;
    if uri.trim() != refs.input_uri {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "VERTEX_UNSAFE_CLEANUP_PATH",
            "Vertex input cleanup path is outside this batch",
        ));
    }
    Ok(())
}

fn ensure_safe_vertex_output(job: &JobRow, uri: &str) -> ApiResult<()> {
    let provider = provider_from_job(job);
    let refs = vertex_managed_refs(&provider, &job.batch_id)?;
    if !uri.trim().starts_with(&refs.output_prefix) {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "VERTEX_UNSAFE_CLEANUP_PATH",
            "Vertex output path is outside this batch",
        ));
    }
    Ok(())
}

async fn vertex_gcs_delete_object(
    state: &AppState,
    provider: &ProviderRow,
    token: &str,
    uri: &str,
) -> ApiResult<()> {
    let (bucket, object) = parse_gcs_uri(uri)?;
    let response = state
        .client
        .delete(format!(
            "{}/storage/v1/b/{}/o/{}",
            provider.gcs_base_url,
            encode_path(bucket),
            encode_path(object)
        ))
        .bearer_auth(token)
        .send()
        .await?;
    if response.status().is_success() || response.status() == reqwest::StatusCode::NOT_FOUND {
        Ok(())
    } else {
        Err(provider_http_error(
            VERTEX_PROVIDER_KIND,
            response.status(),
            "Vertex managed GCS cleanup failed",
        ))
    }
}

async fn vertex_delete_managed_input(state: &AppState, job: &JobRow, uri: &str) -> ApiResult<()> {
    ensure_safe_vertex_input(job, uri)?;
    let provider = provider_from_job(job);
    let token = vertex_access_token(state, &provider).await?;
    vertex_gcs_delete_object(state, &provider, &token, uri).await
}

async fn vertex_delete_managed_output(state: &AppState, job: &JobRow, uri: &str) -> ApiResult<()> {
    ensure_safe_vertex_output(job, uri)?;
    let provider = provider_from_job(job);
    let token = vertex_access_token(state, &provider).await?;
    let objects = vertex_list_objects(state, &provider, &token, uri, false).await?;
    for object in objects {
        vertex_gcs_delete_object(state, &provider, &token, &object).await?;
    }
    Ok(())
}

async fn cleanup_expired_output(state: &AppState) -> ApiResult<()> {
    let batch_id: Option<String> = sqlx::query_scalar(
        "SELECT batch_id FROM batch_image_jobs WHERE status = 'completed' \
         AND output_expires_at IS NOT NULL AND datetime(output_expires_at) <= CURRENT_TIMESTAMP \
         ORDER BY output_expires_at LIMIT 1",
    )
    .fetch_optional(&state.pool)
    .await?;
    if let Some(batch_id) = batch_id {
        delete_outputs_internal(state, &batch_id).await?;
    }
    Ok(())
}

async fn gateway_cancel(
    State(state): State<AppState>,
    Extension(key): Extension<ApiKeyContext>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(cancel_job(&state, owner_from_key(&key)?, &id).await?))
}

async fn user_cancel(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(
        cancel_job(
            &state,
            Owner {
                user_id: session.user_id,
                api_key_id: None,
            },
            &id,
        )
        .await?,
    ))
}

async fn cancel_job(state: &AppState, owner: Owner, batch_id: &str) -> ApiResult<Value> {
    let job = get_job_for_owner(state, owner, batch_id).await?;
    if matches!(
        job.status.as_str(),
        "completed" | "failed" | "cancelled" | "output_deleted"
    ) {
        return job_public(&job);
    }
    if let Some(job_name) = job.provider_job_name.as_deref() {
        let response = if job.provider_kind == VERTEX_PROVIDER_KIND {
            let provider = provider_from_job(&job);
            let token = vertex_access_token(state, &provider).await?;
            state
                .client
                .post(format!("{}:cancel", vertex_resource_url(&job, job_name)?))
                .bearer_auth(token)
                .json(&json!({}))
                .send()
                .await?
        } else {
            let api_key = decrypt_job_provider_key(state, &job)?;
            state
                .client
                .post(format!(
                    "{}/v1beta/{}:cancel",
                    job.provider_base_url,
                    job_name.trim_start_matches('/')
                ))
                .header("x-goog-api-key", api_key)
                .send()
                .await?
        };
        if !response.status().is_success() && response.status() != reqwest::StatusCode::NOT_FOUND {
            return Err(provider_http_error(
                &job.provider_kind,
                response.status(),
                "batch image provider cancel failed",
            ));
        }
    }
    finish_cancelled(state, batch_id).await?;
    if job.provider_kind == VERTEX_PROVIDER_KIND
        && let Some(input_ref) = job.provider_input_ref.as_deref()
    {
        let _ = vertex_delete_managed_input(state, &job, input_ref).await;
    }
    job_public(&get_job_for_owner(state, owner, batch_id).await?)
}

async fn gateway_delete_record(
    State(state): State<AppState>,
    Extension(key): Extension<ApiKeyContext>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    delete_record(&state, owner_from_key(&key)?, &id).await
}

async fn user_delete_record(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    delete_record(
        &state,
        Owner {
            user_id: session.user_id,
            api_key_id: None,
        },
        &id,
    )
    .await
}

async fn delete_record(state: &AppState, owner: Owner, batch_id: &str) -> ApiResult<StatusCode> {
    let job = get_job_for_owner(state, owner, batch_id).await?;
    if !matches!(
        job.status.as_str(),
        "completed" | "failed" | "cancelled" | "output_deleted"
    ) {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "BATCH_IMAGE_RECORD_NOT_READY",
            "batch image record can only be deleted after the job finishes",
        ));
    }
    sqlx::query(
        "UPDATE batch_image_jobs SET user_deleted_at = CURRENT_TIMESTAMP, \
         updated_at = CURRENT_TIMESTAMP WHERE id = ?",
    )
    .bind(job.id)
    .execute(&state.pool)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn gateway_delete_outputs(
    State(state): State<AppState>,
    Extension(key): Extension<ApiKeyContext>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let owner = owner_from_key(&key)?;
    let _ = get_job_for_owner(&state, owner, &id).await?;
    delete_outputs_internal(&state, &id).await?;
    Ok(Json(job_public(
        &get_job_for_owner(&state, owner, &id).await?,
    )?))
}

async fn user_delete_outputs(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let owner = Owner {
        user_id: session.user_id,
        api_key_id: None,
    };
    let _ = get_job_for_owner(&state, owner, &id).await?;
    delete_outputs_internal(&state, &id).await?;
    Ok(Json(job_public(
        &get_job_for_owner(&state, owner, &id).await?,
    )?))
}

async fn delete_outputs_internal(state: &AppState, batch_id: &str) -> ApiResult<()> {
    let job = get_job_by_batch(state, batch_id).await?;
    if job.status == "output_deleted" {
        return Ok(());
    }
    if job.status != "completed" {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "BATCH_IMAGE_OUTPUT_DELETE_NOT_READY",
            "batch image output can only be deleted after completion",
        ));
    }
    if let Some(output_ref) = job.provider_output_ref.as_deref() {
        if job.provider_kind == VERTEX_PROVIDER_KIND {
            vertex_delete_managed_output(state, &job, output_ref).await?;
        } else {
            let api_key = decrypt_job_provider_key(state, &job)?;
            gemini_delete_file(state, &job, &api_key, output_ref).await?;
        }
    }
    let directory = job_directory(state, batch_id)?;
    match tokio::fs::remove_dir_all(directory).await {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    sqlx::query(
        "UPDATE batch_image_jobs SET status = 'output_deleted', output_deleted_at = CURRENT_TIMESTAMP, \
         updated_at = CURRENT_TIMESTAMP WHERE id = ? AND status = 'completed'",
    )
    .bind(job.id)
    .execute(&state.pool)
    .await?;
    Ok(())
}

async fn gateway_item_content(
    State(state): State<AppState>,
    Extension(key): Extension<ApiKeyContext>,
    Path((id, custom_id)): Path<(String, String)>,
    Query(query): Query<ItemQuery>,
) -> ApiResult<Response> {
    item_content(
        &state,
        owner_from_key(&key)?,
        &id,
        &custom_id,
        query.image_index.unwrap_or(0),
    )
    .await
}

async fn user_item_content(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
    Path((id, custom_id)): Path<(String, String)>,
    Query(query): Query<ItemQuery>,
) -> ApiResult<Response> {
    item_content(
        &state,
        Owner {
            user_id: session.user_id,
            api_key_id: None,
        },
        &id,
        &custom_id,
        query.image_index.unwrap_or(0),
    )
    .await
}

async fn item_content(
    state: &AppState,
    owner: Owner,
    batch_id: &str,
    custom_id: &str,
    image_index: usize,
) -> ApiResult<Response> {
    let job = get_job_for_owner(state, owner, batch_id).await?;
    if job.status == "output_deleted" {
        return Err(ApiError::new(
            StatusCode::GONE,
            "BATCH_IMAGE_OUTPUT_DELETED",
            "batch image output has been deleted",
        ));
    }
    if job.status != "completed" {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "BATCH_IMAGE_NOT_READY",
            "batch image job is not completed",
        ));
    }
    let item: ItemRow = sqlx::query_as(
        "SELECT id, job_id, custom_id, status, output_count, prompt_hash, mime_type, \
         file_extension, image_count, output_files, error_code, error_message, created_at, indexed_at \
         FROM batch_image_items WHERE job_id = ? AND custom_id = ?",
    )
    .bind(job.id)
    .bind(custom_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| {
        ApiError::new(
            StatusCode::NOT_FOUND,
            "BATCH_IMAGE_ITEM_NOT_FOUND",
            "batch image item not found",
        )
    })?;
    if item.status != "success" {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "BATCH_IMAGE_ITEM_FAILED",
            "batch image item did not succeed",
        ));
    }
    let files = parse_output_files(&item.output_files)?;
    let filename = files.get(image_index).ok_or_else(|| {
        ApiError::bad_request(
            "BATCH_IMAGE_ITEM_IMAGE_INDEX_OUT_OF_RANGE",
            "batch image item image index is out of range",
        )
    })?;
    validate_stored_filename(filename)?;
    mark_downloaded(state, job.id).await?;
    stream_file(
        job_directory(state, batch_id)?.join(filename),
        item.mime_type
            .as_deref()
            .unwrap_or("application/octet-stream"),
        &download_filename(custom_id, item.file_extension.as_deref().unwrap_or("bin")),
    )
    .await
}

async fn gateway_download(
    State(state): State<AppState>,
    Extension(key): Extension<ApiKeyContext>,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    download_zip(&state, owner_from_key(&key)?, &id).await
}

async fn user_download(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    download_zip(
        &state,
        Owner {
            user_id: session.user_id,
            api_key_id: None,
        },
        &id,
    )
    .await
}

async fn download_zip(state: &AppState, owner: Owner, batch_id: &str) -> ApiResult<Response> {
    let job = get_job_for_owner(state, owner, batch_id).await?;
    if job.status == "output_deleted" {
        return Err(ApiError::new(
            StatusCode::GONE,
            "BATCH_IMAGE_OUTPUT_DELETED",
            "batch image output has been deleted",
        ));
    }
    if job.status != "completed" || job.success_count == 0 {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "BATCH_IMAGE_NOT_READY",
            "batch image job has no completed output",
        ));
    }
    let items: Vec<ItemRow> = sqlx::query_as(
        "SELECT id, job_id, custom_id, status, output_count, prompt_hash, mime_type, \
         file_extension, image_count, output_files, error_code, error_message, created_at, indexed_at \
         FROM batch_image_items WHERE job_id = ? ORDER BY id",
    )
    .bind(job.id)
    .fetch_all(&state.pool)
    .await?;
    if items.len() > MAX_ZIP_ITEMS {
        return Err(ApiError::bad_request(
            "BATCH_IMAGE_ZIP_TOO_MANY_ITEMS",
            "batch image ZIP contains too many items",
        ));
    }
    let directory = job_directory(state, batch_id)?;
    let zip_path = directory.join("download.zip");
    let build_job = job.clone();
    let build_items = items.clone();
    let build_directory = directory.clone();
    let build_zip = zip_path.clone();
    tokio::task::spawn_blocking(move || {
        create_job_zip(&build_zip, &build_directory, &build_job, &build_items)
    })
    .await
    .map_err(|_| ApiError::internal("batch image ZIP task failed"))??;
    mark_downloaded(state, job.id).await?;
    stream_file(zip_path, "application/zip", &format!("{batch_id}.zip")).await
}

async fn mark_downloaded(state: &AppState, job_id: i64) -> ApiResult<()> {
    sqlx::query(
        "UPDATE batch_image_jobs SET downloaded_at = COALESCE(downloaded_at, CURRENT_TIMESTAMP), \
         updated_at = CURRENT_TIMESTAMP WHERE id = ?",
    )
    .bind(job_id)
    .execute(&state.pool)
    .await?;
    Ok(())
}

fn validate_stored_filename(filename: &str) -> ApiResult<()> {
    if filename.is_empty()
        || filename.len() > 180
        || filename.contains('/')
        || filename.contains('\\')
        || filename.contains("..")
    {
        return Err(ApiError::internal("stored batch image filename is unsafe"));
    }
    Ok(())
}

fn download_filename(custom_id: &str, extension: &str) -> String {
    let safe = output_filename(custom_id, extension);
    safe.rsplit_once('-')
        .map(|row| format!("{}.{}", row.0, extension))
        .unwrap_or(safe)
}

async fn stream_file(path: PathBuf, content_type: &str, filename: &str) -> ApiResult<Response> {
    let metadata = tokio::fs::metadata(&path).await.map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            ApiError::new(
                StatusCode::GONE,
                "BATCH_IMAGE_OUTPUT_MISSING",
                "batch image output file is missing",
            )
        } else {
            error.into()
        }
    })?;
    let file = tokio::fs::File::open(path).await?;
    let stream = async_stream::stream! {
        let mut file = file;
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            match file.read(&mut buffer).await {
                Ok(0) => break,
                Ok(read) => yield Ok::<Bytes, std::io::Error>(Bytes::copy_from_slice(&buffer[..read])),
                Err(error) => { yield Err(error); break; }
            }
        }
    };
    let mut response = Response::new(Body::from_stream(stream));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(content_type)
            .map_err(|_| ApiError::internal("invalid output content type"))?,
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!(
            "attachment; filename=\"{}\"",
            filename.replace(['"', '\\', '/'], "_")
        ))
        .map_err(|_| ApiError::internal("invalid output filename"))?,
    );
    response.headers_mut().insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&metadata.len().to_string())
            .map_err(|_| ApiError::internal("invalid output length"))?,
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    Ok(response)
}

enum ZipContent {
    File(PathBuf),
    Memory(Vec<u8>),
}

struct ZipInput {
    name: String,
    content: ZipContent,
}

struct ZipInfo {
    name: String,
    crc32: u32,
    size: u32,
    offset: u32,
}

fn create_job_zip(
    destination: &FsPath,
    directory: &FsPath,
    job: &JobRow,
    items: &[ItemRow],
) -> ApiResult<()> {
    let mut inputs = Vec::new();
    let mut errors = Vec::new();
    for item in items {
        if item.status == "success" {
            for filename in parse_output_files(&item.output_files)? {
                validate_stored_filename(&filename)?;
                inputs.push(ZipInput {
                    name: format!("images/{filename}"),
                    content: ZipContent::File(directory.join(filename)),
                });
            }
        } else if item.status == "failed" {
            errors.push(json!({
                "custom_id":item.custom_id, "code":item.error_code,
                "message":item.error_message
            }));
        }
    }
    let manifest = json!({
        "batch_id":job.batch_id, "task_name":job.task_name, "model":job.model,
        "provider":job.provider_kind, "success_count":job.success_count,
        "fail_count":job.fail_count, "generated_image_count":job.generated_image_count,
        "actual_cost_cents":job.actual_cost_cents
    });
    inputs.push(ZipInput {
        name: "manifest.json".into(),
        content: ZipContent::Memory(
            serde_json::to_vec_pretty(&manifest)
                .map_err(|_| ApiError::internal("manifest serialization failed"))?,
        ),
    });
    if !errors.is_empty() {
        inputs.push(ZipInput {
            name: "errors.json".into(),
            content: ZipContent::Memory(
                serde_json::to_vec_pretty(&errors)
                    .map_err(|_| ApiError::internal("error manifest serialization failed"))?,
            ),
        });
    }
    write_store_zip(destination, &inputs)
}

fn write_store_zip(destination: &FsPath, inputs: &[ZipInput]) -> ApiResult<()> {
    let temporary = destination.with_extension("zip.part");
    let mut output = std::fs::File::create(&temporary)?;
    let mut infos = Vec::new();
    let mut total = 0u64;
    for input in inputs {
        let (crc32, size) = content_crc_size(&input.content)?;
        total = total.saturating_add(u64::from(size));
        if total > MAX_ZIP_BYTES {
            return Err(ApiError::bad_request(
                "BATCH_IMAGE_DOWNLOAD_TOO_LARGE",
                "batch image ZIP exceeds 512 MiB",
            ));
        }
        let offset = output.stream_position()?;
        let name = input.name.as_bytes();
        if name.len() > u16::MAX as usize || offset > u32::MAX as u64 {
            return Err(ApiError::internal("batch image ZIP limits were exceeded"));
        }
        write_u32(&mut output, 0x0403_4b50)?;
        write_u16(&mut output, 20)?;
        write_u16(&mut output, 0x0800)?;
        write_u16(&mut output, 0)?;
        write_u16(&mut output, 0)?;
        write_u16(&mut output, 0)?;
        write_u32(&mut output, crc32)?;
        write_u32(&mut output, size)?;
        write_u32(&mut output, size)?;
        write_u16(&mut output, name.len() as u16)?;
        write_u16(&mut output, 0)?;
        output.write_all(name)?;
        write_content(&mut output, &input.content)?;
        infos.push(ZipInfo {
            name: input.name.clone(),
            crc32,
            size,
            offset: offset as u32,
        });
    }
    let central_offset = output.stream_position()?;
    for info in &infos {
        let name = info.name.as_bytes();
        write_u32(&mut output, 0x0201_4b50)?;
        write_u16(&mut output, 20)?;
        write_u16(&mut output, 20)?;
        write_u16(&mut output, 0x0800)?;
        write_u16(&mut output, 0)?;
        write_u16(&mut output, 0)?;
        write_u16(&mut output, 0)?;
        write_u32(&mut output, info.crc32)?;
        write_u32(&mut output, info.size)?;
        write_u32(&mut output, info.size)?;
        write_u16(&mut output, name.len() as u16)?;
        write_u16(&mut output, 0)?;
        write_u16(&mut output, 0)?;
        write_u16(&mut output, 0)?;
        write_u16(&mut output, 0)?;
        write_u32(&mut output, 0)?;
        write_u32(&mut output, info.offset)?;
        output.write_all(name)?;
    }
    let central_end = output.stream_position()?;
    if infos.len() > u16::MAX as usize
        || central_offset > u32::MAX as u64
        || central_end - central_offset > u32::MAX as u64
    {
        return Err(ApiError::internal("batch image ZIP limits were exceeded"));
    }
    write_u32(&mut output, 0x0605_4b50)?;
    write_u16(&mut output, 0)?;
    write_u16(&mut output, 0)?;
    write_u16(&mut output, infos.len() as u16)?;
    write_u16(&mut output, infos.len() as u16)?;
    write_u32(&mut output, (central_end - central_offset) as u32)?;
    write_u32(&mut output, central_offset as u32)?;
    write_u16(&mut output, 0)?;
    output.flush()?;
    drop(output);
    std::fs::rename(temporary, destination)?;
    Ok(())
}

fn content_crc_size(content: &ZipContent) -> ApiResult<(u32, u32)> {
    let mut crc = 0xffff_ffffu32;
    let mut size = 0u64;
    match content {
        ZipContent::File(path) => {
            let mut file = std::fs::File::open(path)?;
            let mut buffer = [0u8; 64 * 1024];
            loop {
                let read = file.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                crc = crc32_update(crc, &buffer[..read]);
                size = size.saturating_add(read as u64);
            }
        }
        ZipContent::Memory(bytes) => {
            crc = crc32_update(crc, bytes);
            size = bytes.len() as u64;
        }
    }
    if size > u32::MAX as u64 {
        return Err(ApiError::bad_request(
            "BATCH_IMAGE_DOWNLOAD_TOO_LARGE",
            "a ZIP entry is too large",
        ));
    }
    Ok((!crc, size as u32))
}

fn crc32_update(mut crc: u32, bytes: &[u8]) -> u32 {
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    crc
}

fn write_content(output: &mut std::fs::File, content: &ZipContent) -> ApiResult<()> {
    match content {
        ZipContent::File(path) => {
            let mut input = std::fs::File::open(path)?;
            std::io::copy(&mut input, output)?;
        }
        ZipContent::Memory(bytes) => output.write_all(bytes)?,
    }
    Ok(())
}

fn write_u16(output: &mut std::fs::File, value: u16) -> std::io::Result<()> {
    output.write_all(&value.to_le_bytes())
}

fn write_u32(output: &mut std::fs::File, value: u32) -> std::io::Result<()> {
    output.write_all(&value.to_le_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        body::{Body, to_bytes},
        routing::{delete, get, post},
    };

    use crate::test_support;

    // Static PKCS#8 fixture used only to sign local Vertex test assertions.
    const TEST_PRIVATE_KEY_PKCS8: &str = r#"
MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQDs+soBifPeJopK
70XMmWcpscvmbPSBxhhlqbirpR3CHv8WkKuCxA4XIzGyONCQD+YAl/aKhWI/Vwqo
4r9fRBfe4Wu821Yv6DORrPyNDFD+T833c5KaEasTQnehf3Lrvec2aTw067HlUa3v
KzRXm3S+UpnGqvqI5ZKMJo2/y8DWgzm+fSNK5yOwKQc68ka+TZW/uBCYSQWchkAb
7WiN0Xwa+fEhmxyIQJaR+mHdPVr7jHzBCe6u7QQGnvyDqxJbtwgXqV9DzEv0PUmU
G38Csx6Bkgf3p67UsEhqk0jADqRdRWkoJX0UZ+Y+aLvsueRdaNMceoW8efOVMYDw
fwi0YmRzAgMBAAECggEAYWKeaQt8ACruYeT6Vh4kWuoJ1OOphzsVA5I/panxFLkQ
MwG4ucA/2hpIbekTLGCcMFpCoqI1wbnPU5/67Pdap+kTEUVBoeZWauMf1gbdseSx
y6Le+BmSqBOEfgWWAHLF9YJBj63cKVTrmYGzvNzRmPTw5MeWtXNCSf39+neNA5mX
Mts0gEI5oyGOWpSaDvpFzhEe0eD4OcVdrtoYRC+AJyb5x25olRD9JaAPvuT5JPsm
Ck6LuCgrhkVYDGSWEZOmNaATHUZZ+UFfm2NzZxvnbTune/1OLUMAk11dRVi0FcqG
KPiLqIdIF1ZaNQqhal//Iv0X8w96u84GtTuhDj1U4QKBgQD+PMtdfdwhn3s2LR7H
zNdzDQKAgmEzCvBqVMcZ8xjUQz6W99hAKEmhTxx0v/EOzA85X8t+kK+antbKP+td
8uJPcYwBK+nUzUBzDjYm8EZa5lVPFLajVhAvoZtCz4Xmm3eaL0icOUqFzsxKt8of
2pgPq0e+YZrT0Bd3dfKEIiJE9wKBgQDun13TrxgRyxGlvvqm8i5EYc8uV2GCi4ZT
0Vdwf4AABx16MUmlx9zuIFTryyoT5RP9jucBwOXQFCoBDE6kkmkLwI5caFMXwnbq
233cSXyiD3psscWkPcmF4oA/tO92VB/l0/S//raPZ+hRndSYxsjqpvV1uI/74l8F
wna1rXOJZQKBgB26IBFTeRzZV//SsMmt8vc56zP5isH8InZcaVdobFvNbREb88Y0
r79Tz8D6/IW9aH5N7C5lXpMWxYiqhqvajYm6fiNY7iN6yHFrlPtiludkDU+M3Xol
wwi+vbfHKiH3xblalAPoUwVoU8zcxp6I4cTbQy1InmDr8QJ/4RaAIz+rAoGAFkh7
kpD/RmoYM8opzf0/pNMdbc5rJK2y1ZDvAWpmoZoIfqirn/eSAgqy43INc94oh70Y
hWlmDJBVe9OSZHvno1lP8gEsAUP/pt7oWfHi2Z9oZ04SjsvWTdJg95IF6p7ge63X
ZTZ8BdhGMZjziXDGwmLk+SFLENKK3RbTzxNrfqECgYEA+h+DdedcEn0O8TvOlbBo
b61jb7JqJhN4FJmHSd7b9VsUH7Jcy3T8i4469eK52Njl/QGHs1Zv8QevY1/edNAn
EzuCVt+HMAzMJRRdHIBT9pim59OsLCVynF3lNyVFJtN3VVqj6fMtxYADGfQkq0kl
aaaWDr0k1K6FwD6X3/HkOZA=
"#;

    fn test_private_key_pem() -> String {
        format!(
            "-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----",
            TEST_PRIVATE_KEY_PKCS8.trim()
        )
    }

    fn request(items: Vec<SubmitItem>) -> SubmitRequest {
        SubmitRequest {
            model: "gemini-3.1-flash-image".into(),
            task_name: "Smoke batch".into(),
            parent_batch_id: String::new(),
            provider: PROVIDER_KIND.into(),
            response_mime_type: "image/png".into(),
            image_size: "1K".into(),
            items,
            metadata: HashMap::new(),
        }
    }

    fn item(custom_id: &str, prompt: &str) -> SubmitItem {
        SubmitItem {
            custom_id: custom_id.into(),
            prompt: prompt.into(),
            output_count: 1,
            reference_images: Vec::new(),
        }
    }

    async fn configure_test_provider(state: &AppState, base_url: &str) -> i64 {
        let (_, Json(created)) = create_provider(
            State(state.clone()),
            Json(ProviderInput {
                name: "Test Gemini".into(),
                kind: PROVIDER_KIND.into(),
                base_url: base_url.into(),
                api_key: Some("gemini-secret".into()),
                models: vec!["gemini-3.1-flash-image".into()],
                unit_price_cents: 100,
                batch_discount_bps: 5_000,
                hold_bps: 6_000,
                priority: 1,
                concurrency: 2,
                enabled: true,
                ..ProviderInput::default()
            }),
        )
        .await
        .unwrap();
        created["data"]["id"].as_i64().unwrap()
    }

    async fn configure_vertex_provider(state: &AppState, base_url: &str) -> i64 {
        let service_account = json!({
            "type":"service_account", "project_id":"test-project",
            "private_key_id":"test-kid", "private_key":test_private_key_pem(),
            "client_email":"batch-test@test-project.iam.gserviceaccount.com"
        })
        .to_string();
        let (_, Json(created)) = create_provider(
            State(state.clone()),
            Json(ProviderInput {
                name: "Test Vertex".into(),
                kind: VERTEX_PROVIDER_KIND.into(),
                base_url: base_url.into(),
                service_account_json: Some(service_account),
                project_id: "test-project".into(),
                location: "global".into(),
                gcs_bucket: "test-bucket".into(),
                gcs_prefix: DEFAULT_GCS_PREFIX.into(),
                gcs_base_url: base_url.into(),
                token_url: format!("{base_url}/token"),
                models: vec!["gemini-3.1-flash-image".into()],
                unit_price_cents: 100,
                batch_discount_bps: 5_000,
                hold_bps: 6_000,
                priority: 1,
                concurrency: 2,
                enabled: true,
                ..ProviderInput::default()
            }),
        )
        .await
        .unwrap();
        created["data"]["id"].as_i64().unwrap()
    }

    async fn user_key(state: &AppState) -> (i64, ApiKeyContext) {
        let user_id: i64 = sqlx::query_scalar("SELECT id FROM users WHERE role = 'admin'")
            .fetch_one(&state.pool)
            .await
            .unwrap();
        sqlx::query("UPDATE users SET balance_cents = 1000 WHERE id = ?")
            .bind(user_id)
            .execute(&state.pool)
            .await
            .unwrap();
        let key_id = sqlx::query(
            "INSERT INTO api_keys (user_id, name, token_prefix, token_hash) \
             VALUES (?, 'batch-test', 'sk-mini-batch', 'batch-test-hash')",
        )
        .bind(user_id)
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        (
            user_id,
            ApiKeyContext {
                id: key_id,
                user_id: Some(user_id),
                allowed_models: Vec::new(),
                group_id: None,
            },
        )
    }

    #[test]
    fn normalizes_output_expansion_and_crc32() {
        let normalized = normalize_submit(request(vec![SubmitItem {
            custom_id: "cover".into(),
            prompt: " draw a cover ".into(),
            output_count: 3,
            reference_images: Vec::new(),
        }]))
        .unwrap();
        assert_eq!(normalized.items.len(), 3);
        assert_eq!(normalized.items[0].custom_id, "cover_01");
        assert_eq!(normalized.items[2].custom_id, "cover_03");
        assert_eq!(!crc32_update(0xffff_ffff, b"123456789"), 0xcbf4_3926);
    }

    #[tokio::test]
    async fn gemini_batch_flow_is_persistent_idempotent_and_settles_by_success() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let result_base = base_url.clone();
        let mock = Router::new()
            .route(
                "/upload/v1beta/files",
                post(|| async { Json(json!({"file":{"name":"files/input"}})) }),
            )
            .route(
                "/v1beta/models/gemini-3.1-flash-image:batchGenerateContent",
                post(|| async {
                    Json(json!({"name":"batches/test-job","state":"JOB_STATE_PENDING"}))
                }),
            )
            .route(
                "/v1beta/batches/test-job",
                get(|| async {
                    Json(json!({"name":"batches/test-job","state":"JOB_STATE_SUCCEEDED",
                        "response":{"responsesFile":"files/output"}}))
                }),
            )
            .route(
                "/v1beta/batches/test-job:cancel",
                post(|| async { StatusCode::NO_CONTENT }),
            )
            .route(
                "/v1beta/files/output",
                get(move || {
                    let result_base = result_base.clone();
                    async move { Json(json!({"downloadUri":format!("{result_base}/download/output")})) }
                })
                .delete(|| async { StatusCode::NO_CONTENT }),
            )
            .route(
                "/v1beta/files/input",
                delete(|| async { StatusCode::NO_CONTENT }),
            )
            .route(
                "/download/output",
                get(|| async {
                    Body::from(concat!(
                        "{\"key\":\"ok\",\"response\":{\"candidates\":[{\"content\":{\"parts\":[{\"inlineData\":{\"mimeType\":\"image/png\",\"data\":\"c21va2UtcG5n\"}}]}}]}}\n",
                        "{\"key\":\"bad\",\"status\":{\"code\":3,\"message\":\"blocked by safety policy\"}}\n"
                    ))
                }),
            )
            .route("/v1beta/models", get(|| async { Json(json!({"models":[]})) }));
        let server = tokio::spawn(async move { axum::serve(listener, mock).await });

        let (_directory, state) = test_support::state().await;
        let provider_id = configure_test_provider(&state, &base_url).await;
        let encrypted: String =
            sqlx::query_scalar("SELECT encrypted_api_key FROM batch_image_providers WHERE id = ?")
                .bind(provider_id)
                .fetch_one(&state.pool)
                .await
                .unwrap();
        assert!(!encrypted.contains("gemini-secret"));
        let (user_id, key) = user_key(&state).await;
        let original = request(vec![
            item("ok", "draw success"),
            item("bad", "draw blocked"),
        ]);
        let jsonl = build_gemini_jsonl(&original).unwrap();
        let jsonl = String::from_utf8(jsonl).unwrap();
        assert!(jsonl.contains("draw success"));
        assert!(jsonl.contains("draw blocked"));
        let submitted = submit_job(
            &state,
            key.clone(),
            original.clone(),
            Some("same-request".into()),
        )
        .await
        .unwrap();
        assert_eq!(submitted["status"], "queued");
        assert_eq!(submitted["item_count"], 2);
        let batch_id = submitted["id"].as_str().unwrap().to_string();
        let balance: (i64, i64) =
            sqlx::query_as("SELECT balance_cents, frozen_balance_cents FROM users WHERE id = ?")
                .bind(user_id)
                .fetch_one(&state.pool)
                .await
                .unwrap();
        assert_eq!(balance, (880, 120));
        let stored_prompt: String =
            sqlx::query_scalar("SELECT prompt_hash FROM batch_image_items ORDER BY id LIMIT 1")
                .fetch_one(&state.pool)
                .await
                .unwrap();
        assert_ne!(stored_prompt, "draw success");

        let replay = submit_job(
            &state,
            key.clone(),
            original.clone(),
            Some("same-request".into()),
        )
        .await
        .unwrap();
        assert_eq!(replay["id"], batch_id);
        let conflict = submit_job(
            &state,
            key.clone(),
            request(vec![item("other", "different")]),
            Some("same-request".into()),
        )
        .await
        .unwrap_err();
        assert_eq!(conflict.code, "BATCH_IMAGE_IDEMPOTENCY_CONFLICT");

        process_job(&state, &batch_id).await.unwrap();
        let completed = get_job_by_batch(&state, &batch_id).await.unwrap();
        assert_eq!(completed.status, "completed");
        assert_eq!(completed.success_count, 1);
        assert_eq!(completed.fail_count, 1);
        assert_eq!(completed.generated_image_count, 1);
        assert_eq!(completed.actual_cost_cents, Some(50));
        let balance: (i64, i64) =
            sqlx::query_as("SELECT balance_cents, frozen_balance_cents FROM users WHERE id = ?")
                .bind(user_id)
                .fetch_one(&state.pool)
                .await
                .unwrap();
        assert_eq!(balance, (950, 0));

        let owner = Owner {
            user_id,
            api_key_id: Some(key.id),
        };
        let image = item_content(&state, owner, &batch_id, "ok", 0)
            .await
            .unwrap();
        assert_eq!(image.headers()[header::CONTENT_TYPE], "image/png");
        assert_eq!(
            to_bytes(image.into_body(), MAX_IMAGE_BYTES).await.unwrap(),
            Bytes::from_static(b"smoke-png")
        );
        let zip = download_zip(&state, owner, &batch_id).await.unwrap();
        let zip = to_bytes(zip.into_body(), 2 * 1024 * 1024).await.unwrap();
        assert!(zip.starts_with(b"PK\x03\x04"));
        assert!(zip.windows(13).any(|window| window == b"manifest.json"));
        assert!(zip.windows(9).any(|window| window == b"smoke-png"));

        let second = submit_job(
            &state,
            key.clone(),
            request(vec![item("cancel", "cancel me")]),
            None,
        )
        .await
        .unwrap();
        let second_id = second["id"].as_str().unwrap();
        let cancelled = cancel_job(&state, owner, second_id).await.unwrap();
        assert_eq!(cancelled["status"], "cancelled");
        let balance: (i64, i64) =
            sqlx::query_as("SELECT balance_cents, frozen_balance_cents FROM users WHERE id = ?")
                .bind(user_id)
                .fetch_one(&state.pool)
                .await
                .unwrap();
        assert_eq!(balance, (950, 0));

        delete_outputs_internal(&state, &batch_id).await.unwrap();
        assert_eq!(
            get_job_by_batch(&state, &batch_id).await.unwrap().status,
            "output_deleted"
        );
        let missing = item_content(&state, owner, &batch_id, "ok", 0)
            .await
            .unwrap_err();
        assert_eq!(missing.code, "BATCH_IMAGE_OUTPUT_DELETED");
        server.abort();
    }

    #[tokio::test]
    async fn vertex_batch_uses_signed_token_gcs_and_safe_cleanup() {
        use std::sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        };

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let output_prefix = Arc::new(Mutex::new(String::new()));
        let token_calls = Arc::new(AtomicUsize::new(0));
        let delete_calls = Arc::new(AtomicUsize::new(0));
        let cancel_calls = Arc::new(AtomicUsize::new(0));
        let token_url = format!("{base_url}/token");
        let token_counter = token_calls.clone();
        let upload = post(
            |headers: HeaderMap, uri: axum::http::Uri, body: Bytes| async move {
                assert_eq!(headers[header::AUTHORIZATION], "Bearer vertex-access");
                assert!(uri.query().unwrap_or_default().contains("uploadType=media"));
                assert!(String::from_utf8_lossy(&body).contains("vertex prompt"));
                Json(json!({"name":"uploaded"}))
            },
        );
        let created_output = output_prefix.clone();
        let create = post(move |headers: HeaderMap, Json(payload): Json<Value>| {
            let created_output = created_output.clone();
            async move {
                assert_eq!(headers[header::AUTHORIZATION], "Bearer vertex-access");
                assert_eq!(payload["instanceConfig"]["keyField"], "key");
                assert_eq!(
                    payload["model"],
                    "publishers/google/models/gemini-3.1-flash-image"
                );
                let output = payload["outputConfig"]["gcsDestination"]["outputUriPrefix"]
                    .as_str()
                    .unwrap()
                    .to_string();
                assert!(output.starts_with("gs://test-bucket/batch-image/mini/imgbatch_"));
                *created_output.lock().unwrap() = output.clone();
                Json(json!({
                    "name":"projects/test-project/locations/global/batchPredictionJobs/job-1",
                    "state":"JOB_STATE_PENDING",
                    "outputConfig":{"gcsDestination":{"outputUriPrefix":output}}
                }))
            }
        })
        .get(|headers: HeaderMap| async move {
            assert_eq!(headers[header::AUTHORIZATION], "Bearer vertex-access");
            Json(json!({"batchPredictionJobs":[]}))
        });
        let status_output = output_prefix.clone();
        let status_route = get(move |headers: HeaderMap| {
            let status_output = status_output.clone();
            async move {
                assert_eq!(headers[header::AUTHORIZATION], "Bearer vertex-access");
                Json(json!({
                    "name":"projects/test-project/locations/global/batchPredictionJobs/job-1",
                    "state":"JOB_STATE_SUCCEEDED",
                    "outputConfig":{"gcsDestination":{"outputUriPrefix":status_output.lock().unwrap().clone()}}
                }))
            }
        });
        let list_output = output_prefix.clone();
        let list = get(move |headers: HeaderMap| {
            let list_output = list_output.clone();
            async move {
                assert_eq!(headers[header::AUTHORIZATION], "Bearer vertex-access");
                let prefix = list_output.lock().unwrap().clone();
                let object = prefix
                    .strip_prefix("gs://test-bucket/")
                    .unwrap()
                    .to_string()
                    + "predictions_1.jsonl";
                Json(json!({"items":[{"name":object}]}))
            }
        });
        let deletes = delete_calls.clone();
        let object = get(|headers: HeaderMap| async move {
            assert_eq!(headers[header::AUTHORIZATION], "Bearer vertex-access");
            Body::from("{\"key\":\"vertex-ok\",\"response\":{\"candidates\":[{\"content\":{\"parts\":[{\"inlineData\":{\"mimeType\":\"image/png\",\"data\":\"dmVydGV4LXBuZw==\"}}]}}]}}\n")
        })
        .delete(move |headers: HeaderMap| {
            let deletes = deletes.clone();
            async move {
                assert_eq!(headers[header::AUTHORIZATION], "Bearer vertex-access");
                deletes.fetch_add(1, Ordering::SeqCst);
                StatusCode::NO_CONTENT
            }
        });
        let token = post(move |body: Bytes| {
            let token_counter = token_counter.clone();
            let token_url = token_url.clone();
            async move {
                token_counter.fetch_add(1, Ordering::SeqCst);
                let fields = url::form_urlencoded::parse(&body)
                    .into_owned()
                    .collect::<HashMap<_, _>>();
                assert_eq!(
                    fields.get("grant_type").unwrap(),
                    "urn:ietf:params:oauth:grant-type:jwt-bearer"
                );
                let parts = fields
                    .get("assertion")
                    .unwrap()
                    .split('.')
                    .collect::<Vec<_>>();
                assert_eq!(parts.len(), 3);
                let claims: Value =
                    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[1]).unwrap()).unwrap();
                assert_eq!(claims["aud"], token_url);
                assert_eq!(
                    claims["iss"],
                    "batch-test@test-project.iam.gserviceaccount.com"
                );
                Json(
                    json!({"access_token":"vertex-access","expires_in":3600,"token_type":"Bearer"}),
                )
            }
        });
        let cancels = cancel_calls.clone();
        let cancel = post(move |headers: HeaderMap| {
            let cancels = cancels.clone();
            async move {
                assert_eq!(headers[header::AUTHORIZATION], "Bearer vertex-access");
                cancels.fetch_add(1, Ordering::SeqCst);
                StatusCode::NO_CONTENT
            }
        });
        let mock = Router::new()
            .route("/token", token)
            .route("/upload/storage/v1/b/test-bucket/o", upload)
            .route(
                "/v1/projects/test-project/locations/global/batchPredictionJobs",
                create,
            )
            .route(
                "/v1/projects/test-project/locations/global/batchPredictionJobs/job-1",
                status_route,
            )
            .route(
                "/v1/projects/test-project/locations/global/batchPredictionJobs/job-1:cancel",
                cancel,
            )
            .route("/storage/v1/b/test-bucket/o", list)
            .route("/storage/v1/b/test-bucket/o/{*object}", object);
        let server = tokio::spawn(async move { axum::serve(listener, mock).await });
        let (_directory, state) = test_support::state().await;
        let provider_id = configure_vertex_provider(&state, &base_url).await;
        let Json(probe) = test_provider(State(state.clone()), Path(provider_id))
            .await
            .unwrap();
        assert_eq!(probe["data"]["ok"], true);
        let encrypted: String = sqlx::query_scalar(
            "SELECT encrypted_service_account_json FROM batch_image_providers WHERE id = ?",
        )
        .bind(provider_id)
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert!(!encrypted.contains("PRIVATE KEY"));
        let (user_id, key) = user_key(&state).await;
        let submitted = submit_job(
            &state,
            key.clone(),
            SubmitRequest {
                provider: VERTEX_PROVIDER_KIND.into(),
                items: vec![item("vertex-ok", "vertex prompt")],
                ..request(Vec::new())
            },
            Some("vertex-idempotency".into()),
        )
        .await
        .unwrap();
        assert_eq!(submitted["provider"], VERTEX_PROVIDER_KIND);
        let batch_id = submitted["id"].as_str().unwrap().to_string();
        sqlx::query(
            "UPDATE batch_image_providers SET provider_type = 'gemini_api', base_url = 'http://127.0.0.1:1', \
             gcs_bucket = 'changed-bucket', gcs_prefix = 'changed/{batch_id}' WHERE id = ?",
        )
        .bind(provider_id)
        .execute(&state.pool)
        .await
        .unwrap();
        process_job(&state, &batch_id).await.unwrap();
        let completed = get_job_by_batch(&state, &batch_id).await.unwrap();
        assert_eq!(completed.status, "completed");
        assert_eq!(completed.provider_kind, VERTEX_PROVIDER_KIND);
        assert_eq!(completed.provider_gcs_bucket, "test-bucket");
        assert_eq!(completed.generated_image_count, 1);
        assert_eq!(completed.actual_cost_cents, Some(50));
        let balance: (i64, i64) =
            sqlx::query_as("SELECT balance_cents, frozen_balance_cents FROM users WHERE id = ?")
                .bind(user_id)
                .fetch_one(&state.pool)
                .await
                .unwrap();
        assert_eq!(balance, (950, 0));
        assert_eq!(token_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            ensure_safe_vertex_output(&completed, "gs://other-bucket/unmanaged/")
                .unwrap_err()
                .code,
            "VERTEX_UNSAFE_CLEANUP_PATH"
        );
        delete_outputs_internal(&state, &batch_id).await.unwrap();
        assert!(delete_calls.load(Ordering::SeqCst) >= 2);

        sqlx::query(
            "UPDATE batch_image_providers SET provider_type = 'vertex', base_url = ?, \
             gcs_bucket = 'test-bucket', gcs_prefix = ? WHERE id = ?",
        )
        .bind(&base_url)
        .bind(DEFAULT_GCS_PREFIX)
        .bind(provider_id)
        .execute(&state.pool)
        .await
        .unwrap();

        let second = submit_job(
            &state,
            key.clone(),
            SubmitRequest {
                provider: VERTEX_PROVIDER_KIND.into(),
                items: vec![item("vertex-cancel", "cancel vertex prompt")],
                ..request(Vec::new())
            },
            Some("vertex-cancel-idempotency".into()),
        )
        .await
        .unwrap();
        let cancelled = cancel_job(
            &state,
            Owner {
                user_id,
                api_key_id: Some(key.id),
            },
            second["id"].as_str().unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(cancelled["status"], "cancelled");
        assert_eq!(cancel_calls.load(Ordering::SeqCst), 1);
        let balance: (i64, i64) =
            sqlx::query_as("SELECT balance_cents, frozen_balance_cents FROM users WHERE id = ?")
                .bind(user_id)
                .fetch_one(&state.pool)
                .await
                .unwrap();
        assert_eq!(balance, (950, 0));
        assert_eq!(token_calls.load(Ordering::SeqCst), 1);
        server.abort();
    }

    #[tokio::test]
    async fn provider_submission_failure_releases_hold_and_keeps_visible_record() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let mock = Router::new().route(
            "/upload/v1beta/files",
            post(|| async {
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(json!({"error":{"message":"provider unavailable"}})),
                )
            }),
        );
        let server = tokio::spawn(async move { axum::serve(listener, mock).await });
        let (_directory, state) = test_support::state().await;
        configure_test_provider(&state, &base_url).await;
        let (user_id, key) = user_key(&state).await;

        let error = submit_job(
            &state,
            key,
            request(vec![item("failed-submit", "draw failure")]),
            Some("failed-submit-idempotency".into()),
        )
        .await
        .unwrap_err();
        assert_eq!(error.status, StatusCode::BAD_GATEWAY);
        let job: (String, Option<String>, i64, i64) = sqlx::query_as(
            "SELECT status, user_deleted_at, hold_amount_cents, fail_count FROM batch_image_jobs",
        )
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(job, ("failed".into(), None, 60, 1));
        let item_status: String = sqlx::query_scalar("SELECT status FROM batch_image_items")
            .fetch_one(&state.pool)
            .await
            .unwrap();
        assert_eq!(item_status, "failed");
        let balance: (i64, i64) =
            sqlx::query_as("SELECT balance_cents, frozen_balance_cents FROM users WHERE id = ?")
                .bind(user_id)
                .fetch_one(&state.pool)
                .await
                .unwrap();
        assert_eq!(balance, (1000, 0));
        server.abort();
    }
}
