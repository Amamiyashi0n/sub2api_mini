use std::{
    collections::{BTreeMap, HashSet},
    time::{Duration, Instant},
};

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderName, HeaderValue, StatusCode},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::FromRow;

use crate::{
    error::{ApiError, ApiResult},
    state::AppState,
};

pub fn admin_router() -> Router<AppState> {
    Router::new()
        .route("/channel-monitors", get(admin_list).post(create))
        .route(
            "/channel-monitors/{id}",
            get(admin_get).put(update).delete(delete),
        )
        .route("/channel-monitors/{id}/duplicate", post(duplicate))
        .route("/channel-monitors/{id}/run", post(run_now))
        .route("/channel-monitors/{id}/history", get(history))
        .route(
            "/channel-monitor-templates",
            get(list_templates).post(create_template),
        )
        .route(
            "/channel-monitor-templates/{id}",
            get(get_template)
                .put(update_template)
                .delete(delete_template),
        )
        .route(
            "/channel-monitor-templates/{id}/monitors",
            get(template_monitors),
        )
        .route(
            "/channel-monitor-templates/{id}/apply",
            post(apply_template),
        )
}

pub fn user_router() -> Router<AppState> {
    Router::new()
        .route("/channel-monitors", get(user_list))
        .route("/channel-monitors/{id}/status", get(user_status))
}

pub fn start_scheduler(state: AppState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(15));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if let Err(error) = run_due(&state).await {
                tracing::warn!(%error, "channel monitor scheduler failed");
            }
        }
    });
}

#[derive(Debug, Clone, FromRow)]
struct MonitorRow {
    id: i64,
    name: String,
    provider: String,
    api_mode: String,
    endpoint: String,
    encrypted_request_config: String,
    primary_model: String,
    extra_models: String,
    group_name: String,
    enabled: bool,
    interval_seconds: i64,
    jitter_seconds: i64,
    template_id: Option<i64>,
    last_checked_at: Option<String>,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct RequestConfig {
    api_key: String,
    #[serde(default)]
    extra_headers: BTreeMap<String, String>,
    #[serde(default)]
    body_override_mode: String,
    #[serde(default)]
    body_override: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct TemplateConfig {
    #[serde(default)]
    extra_headers: BTreeMap<String, String>,
    #[serde(default)]
    body_override_mode: String,
    #[serde(default)]
    body_override: Option<Value>,
}

#[derive(Debug, Clone, FromRow)]
struct TemplateRow {
    id: i64,
    name: String,
    provider: String,
    api_mode: String,
    description: String,
    encrypted_template_config: String,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
struct CheckResult {
    model: String,
    status: String,
    latency_ms: Option<i64>,
    ping_latency_ms: Option<i64>,
    message: String,
    checked_at: String,
}

const MONITOR_SELECT: &str = "SELECT id, name, provider, api_mode, endpoint, \
    encrypted_request_config, primary_model, extra_models, group_name, enabled, \
    interval_seconds, jitter_seconds, template_id, last_checked_at, created_at, updated_at \
    FROM channel_monitors";

const TEMPLATE_SELECT: &str = "SELECT id, name, provider, api_mode, description, \
    encrypted_template_config, created_at, updated_at FROM channel_monitor_templates";

#[derive(Debug, Deserialize, Default)]
struct ListQuery {
    provider: Option<String>,
    enabled: Option<bool>,
    search: Option<String>,
}

async fn admin_list(
    State(state): State<AppState>,
    Query(query): Query<ListQuery>,
) -> ApiResult<Json<Value>> {
    let rows = sqlx::query_as::<_, MonitorRow>(&format!(
        "{MONITOR_SELECT} WHERE (? IS NULL OR provider = ?) AND (? IS NULL OR enabled = ?) \
         AND (? = '' OR name LIKE '%' || ? || '%' OR primary_model LIKE '%' || ? || '%') \
         ORDER BY id DESC"
    ))
    .bind(&query.provider)
    .bind(&query.provider)
    .bind(query.enabled)
    .bind(query.enabled)
    .bind(query.search.as_deref().unwrap_or(""))
    .bind(query.search.as_deref().unwrap_or(""))
    .bind(query.search.as_deref().unwrap_or(""))
    .fetch_all(&state.pool)
    .await?;
    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        items.push(admin_view(&state, &row).await?);
    }
    Ok(Json(json!({"data": items})))
}

async fn admin_get(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    let row = get_row(&state, id).await?;
    Ok(Json(json!({"data": admin_view(&state, &row).await?})))
}

async fn admin_view(state: &AppState, row: &MonitorRow) -> ApiResult<Value> {
    let config = decrypt_config(state, row)?;
    let latest = latest_result(state, row.id, &row.primary_model).await?;
    let availability = availability(state, row.id, &row.primary_model, 7).await?;
    let extra_models = parse_models(&row.extra_models)?;
    let mut extra_status = Vec::with_capacity(extra_models.len());
    for model in &extra_models {
        let latest = latest_result(state, row.id, model).await?;
        extra_status.push(
            json!({"model": model, "status": latest.as_ref().map(|v| v.0.as_str()).unwrap_or(""),
            "latency_ms": latest.as_ref().and_then(|v| v.1),
            "ping_latency_ms": latest.as_ref().and_then(|v| v.2)}),
        );
    }
    Ok(json!({
        "id": row.id, "name": row.name, "provider": row.provider, "api_mode": row.api_mode,
        "endpoint": row.endpoint, "api_key_masked": mask_secret(&config.api_key),
        "primary_model": row.primary_model, "extra_models": extra_models,
        "group_name": row.group_name, "enabled": row.enabled,
        "interval_seconds": row.interval_seconds, "jitter_seconds": row.jitter_seconds,
        "template_id": row.template_id,
        "last_checked_at": row.last_checked_at, "created_at": row.created_at,
        "updated_at": row.updated_at,
        "primary_status": latest.as_ref().map(|v| v.0.as_str()).unwrap_or(""),
        "primary_latency_ms": latest.as_ref().and_then(|v| v.1),
        "primary_ping_latency_ms": latest.as_ref().and_then(|v| v.2),
        "availability_7d": availability,
        "extra_models_status": extra_status, "extra_headers": config.extra_headers,
        "body_override_mode": config.body_override_mode, "body_override": config.body_override
    }))
}

#[derive(Debug, Deserialize)]
struct CreateInput {
    name: String,
    provider: String,
    #[serde(default = "default_api_mode")]
    api_mode: String,
    endpoint: String,
    api_key: String,
    primary_model: String,
    #[serde(default)]
    extra_models: Vec<String>,
    #[serde(default)]
    group_name: String,
    #[serde(default = "enabled_default")]
    enabled: bool,
    #[serde(default = "default_interval")]
    interval_seconds: i64,
    #[serde(default)]
    jitter_seconds: i64,
    #[serde(default)]
    extra_headers: BTreeMap<String, String>,
    #[serde(default)]
    body_override_mode: String,
    #[serde(default)]
    body_override: Option<Value>,
    template_id: Option<i64>,
}

fn default_api_mode() -> String {
    "chat_completions".into()
}
fn enabled_default() -> bool {
    true
}
fn default_interval() -> i64 {
    300
}

async fn create(
    State(state): State<AppState>,
    Json(input): Json<CreateInput>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    validate_input(
        &input.name,
        &input.provider,
        &input.api_mode,
        &input.endpoint,
        &input.api_key,
        &input.primary_model,
        &input.extra_models,
        input.interval_seconds,
        input.jitter_seconds,
        &input.body_override_mode,
    )?;
    let mut config = RequestConfig {
        api_key: input.api_key.trim().into(),
        extra_headers: input.extra_headers,
        body_override_mode: normalize_override_mode(&input.body_override_mode),
        body_override: input.body_override,
    };
    if let Some(template_id) = input.template_id {
        let template = get_template_row(&state, template_id).await?;
        ensure_template_compatible(&template, &input.provider, &input.api_mode)?;
        apply_template_config(&mut config, decrypt_template_config(&state, &template)?);
    }
    validate_request_config(&config)?;
    let encrypted = encrypt_config(&state, &config)?;
    let result = sqlx::query(
        "INSERT INTO channel_monitors (name, provider, api_mode, endpoint, \
         encrypted_request_config, primary_model, extra_models, group_name, enabled, \
         interval_seconds, jitter_seconds, template_id) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(input.name.trim())
    .bind(input.provider)
    .bind(input.api_mode)
    .bind(input.endpoint.trim())
    .bind(encrypted)
    .bind(input.primary_model.trim())
    .bind(serialize_models(normalize_models(input.extra_models))?)
    .bind(input.group_name.trim())
    .bind(input.enabled)
    .bind(input.interval_seconds)
    .bind(input.jitter_seconds)
    .bind(input.template_id)
    .execute(&state.pool)
    .await?;
    let row = get_row(&state, result.last_insert_rowid()).await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"data": admin_view(&state, &row).await?})),
    ))
}

#[derive(Debug, Deserialize)]
struct UpdateInput {
    name: Option<String>,
    provider: Option<String>,
    api_mode: Option<String>,
    endpoint: Option<String>,
    api_key: Option<String>,
    primary_model: Option<String>,
    extra_models: Option<Vec<String>>,
    group_name: Option<String>,
    enabled: Option<bool>,
    interval_seconds: Option<i64>,
    jitter_seconds: Option<i64>,
    extra_headers: Option<BTreeMap<String, String>>,
    body_override_mode: Option<String>,
    #[serde(default, deserialize_with = "crate::models::deserialize_nullable")]
    body_override: Option<Option<Value>>,
    #[serde(default, deserialize_with = "crate::models::deserialize_nullable")]
    template_id: Option<Option<i64>>,
    #[serde(default)]
    clear_template: bool,
}

async fn update(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(input): Json<UpdateInput>,
) -> ApiResult<Json<Value>> {
    let row = get_row(&state, id).await?;
    let mut config = decrypt_config(&state, &row)?;
    if let Some(api_key) = input.api_key.filter(|value| !value.trim().is_empty()) {
        config.api_key = api_key.trim().into();
    }
    if let Some(headers) = input.extra_headers {
        config.extra_headers = headers;
    }
    if let Some(mode) = input.body_override_mode {
        config.body_override_mode = normalize_override_mode(&mode);
    }
    if let Some(body) = input.body_override {
        config.body_override = body;
    }
    let name = input.name.unwrap_or(row.name);
    let provider = input.provider.unwrap_or(row.provider);
    let api_mode = input.api_mode.unwrap_or(row.api_mode);
    let endpoint = input.endpoint.unwrap_or(row.endpoint);
    let primary_model = input.primary_model.unwrap_or(row.primary_model);
    let extra_models = input
        .extra_models
        .unwrap_or(parse_models(&row.extra_models)?);
    let interval_seconds = input.interval_seconds.unwrap_or(row.interval_seconds);
    let jitter_seconds = input.jitter_seconds.unwrap_or(row.jitter_seconds);
    let template_change = if input.clear_template {
        Some(None)
    } else {
        input.template_id
    };
    let template_id = template_change.unwrap_or(row.template_id);
    if let Some(template_id) = template_id {
        let template = get_template_row(&state, template_id).await?;
        ensure_template_compatible(&template, &provider, &api_mode)?;
        if template_change.is_some() {
            apply_template_config(&mut config, decrypt_template_config(&state, &template)?);
        }
    }
    validate_input(
        &name,
        &provider,
        &api_mode,
        &endpoint,
        &config.api_key,
        &primary_model,
        &extra_models,
        interval_seconds,
        jitter_seconds,
        &config.body_override_mode,
    )?;
    validate_request_config(&config)?;
    sqlx::query(
        "UPDATE channel_monitors SET name = ?, provider = ?, api_mode = ?, endpoint = ?, \
         encrypted_request_config = ?, primary_model = ?, extra_models = ?, group_name = ?, \
         enabled = ?, interval_seconds = ?, jitter_seconds = ?, template_id = ?, \
         updated_at = CURRENT_TIMESTAMP WHERE id = ?",
    )
    .bind(name.trim())
    .bind(provider)
    .bind(api_mode)
    .bind(endpoint.trim())
    .bind(encrypt_config(&state, &config)?)
    .bind(primary_model.trim())
    .bind(serialize_models(normalize_models(extra_models))?)
    .bind(input.group_name.unwrap_or(row.group_name).trim())
    .bind(input.enabled.unwrap_or(row.enabled))
    .bind(interval_seconds)
    .bind(jitter_seconds)
    .bind(template_id)
    .bind(id)
    .execute(&state.pool)
    .await?;
    let row = get_row(&state, id).await?;
    Ok(Json(json!({"data": admin_view(&state, &row).await?})))
}

async fn duplicate(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let row = get_row(&state, id).await?;
    let result = sqlx::query(
        "INSERT INTO channel_monitors (name, provider, api_mode, endpoint, \
         encrypted_request_config, primary_model, extra_models, group_name, enabled, \
         interval_seconds, jitter_seconds, template_id) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, 0, ?, ?, ?)",
    )
    .bind(format!("{} (copy)", row.name))
    .bind(row.provider)
    .bind(row.api_mode)
    .bind(row.endpoint)
    .bind(row.encrypted_request_config)
    .bind(row.primary_model)
    .bind(row.extra_models)
    .bind(row.group_name)
    .bind(row.interval_seconds)
    .bind(row.jitter_seconds)
    .bind(row.template_id)
    .execute(&state.pool)
    .await?;
    let copy = get_row(&state, result.last_insert_rowid()).await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"data": admin_view(&state, &copy).await?})),
    ))
}

async fn delete(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    let result = sqlx::query("DELETE FROM channel_monitors WHERE id = ?")
        .bind(id)
        .execute(&state.pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found("channel monitor not found"));
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
struct TemplateInput {
    name: String,
    provider: String,
    #[serde(default = "default_api_mode")]
    api_mode: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    extra_headers: BTreeMap<String, String>,
    #[serde(default)]
    body_override_mode: String,
    #[serde(default)]
    body_override: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct TemplateUpdateInput {
    name: Option<String>,
    api_mode: Option<String>,
    description: Option<String>,
    extra_headers: Option<BTreeMap<String, String>>,
    body_override_mode: Option<String>,
    #[serde(default, deserialize_with = "crate::models::deserialize_nullable")]
    body_override: Option<Option<Value>>,
}

#[derive(Debug, Deserialize, Default)]
struct TemplateListQuery {
    provider: Option<String>,
    api_mode: Option<String>,
}

async fn list_templates(
    State(state): State<AppState>,
    Query(query): Query<TemplateListQuery>,
) -> ApiResult<Json<Value>> {
    if query
        .provider
        .as_deref()
        .is_some_and(|provider| !matches!(provider, "openai" | "anthropic" | "gemini" | "grok"))
        || query
            .api_mode
            .as_deref()
            .is_some_and(|mode| !matches!(mode, "chat_completions" | "responses"))
    {
        return Err(ApiError::bad_request(
            "INVALID_MONITOR_TEMPLATE_FILTER",
            "template provider or API mode is invalid",
        ));
    }
    let rows = sqlx::query_as::<_, TemplateRow>(&format!(
        "{TEMPLATE_SELECT} WHERE (? IS NULL OR provider = ?) \
         AND (? IS NULL OR api_mode = ?) ORDER BY provider, api_mode, name, id"
    ))
    .bind(&query.provider)
    .bind(&query.provider)
    .bind(&query.api_mode)
    .bind(&query.api_mode)
    .fetch_all(&state.pool)
    .await?;
    let mut data = Vec::with_capacity(rows.len());
    for row in rows {
        data.push(template_view(&state, &row).await?);
    }
    Ok(Json(json!({"data": data})))
}

async fn get_template(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<Value>> {
    let row = get_template_row(&state, id).await?;
    Ok(Json(json!({"data": template_view(&state, &row).await?})))
}

async fn create_template(
    State(state): State<AppState>,
    Json(input): Json<TemplateInput>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let mode = normalize_override_mode(&input.body_override_mode);
    let config = TemplateConfig {
        extra_headers: input.extra_headers,
        body_override_mode: mode,
        body_override: input.body_override,
    };
    validate_template(
        &input.name,
        &input.provider,
        &input.api_mode,
        &input.description,
        &config,
    )?;
    let encrypted = encrypt_template_config(&state, &config)?;
    let id = sqlx::query(
        "INSERT INTO channel_monitor_templates \
         (name, provider, api_mode, description, encrypted_template_config) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(input.name.trim())
    .bind(&input.provider)
    .bind(&input.api_mode)
    .bind(input.description.trim())
    .bind(encrypted)
    .execute(&state.pool)
    .await
    .map_err(template_unique_error)?
    .last_insert_rowid();
    let row = get_template_row(&state, id).await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"data": template_view(&state, &row).await?})),
    ))
}

async fn update_template(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(input): Json<TemplateUpdateInput>,
) -> ApiResult<Json<Value>> {
    let row = get_template_row(&state, id).await?;
    let mut config = decrypt_template_config(&state, &row)?;
    if let Some(headers) = input.extra_headers {
        config.extra_headers = headers;
    }
    if let Some(mode) = input.body_override_mode {
        config.body_override_mode = normalize_override_mode(&mode);
    }
    if let Some(body) = input.body_override {
        config.body_override = body;
    }
    let name = input.name.unwrap_or(row.name);
    let api_mode = input.api_mode.unwrap_or(row.api_mode);
    let description = input.description.unwrap_or(row.description);
    validate_template(&name, &row.provider, &api_mode, &description, &config)?;
    sqlx::query(
        "UPDATE channel_monitor_templates SET name = ?, api_mode = ?, description = ?, \
         encrypted_template_config = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
    )
    .bind(name.trim())
    .bind(api_mode)
    .bind(description.trim())
    .bind(encrypt_template_config(&state, &config)?)
    .bind(id)
    .execute(&state.pool)
    .await
    .map_err(template_unique_error)?;
    let row = get_template_row(&state, id).await?;
    Ok(Json(json!({"data": template_view(&state, &row).await?})))
}

async fn delete_template(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    let result = sqlx::query("DELETE FROM channel_monitor_templates WHERE id = ?")
        .bind(id)
        .execute(&state.pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found("channel monitor template not found"));
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn template_monitors(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<Value>> {
    get_template_row(&state, id).await?;
    let rows: Vec<(i64, String, String, String, bool)> = sqlx::query_as(
        "SELECT id, name, provider, api_mode, enabled FROM channel_monitors \
         WHERE template_id = ? ORDER BY name, id",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(json!({"data": rows.into_iter().map(|row| json!({
        "id": row.0, "name": row.1, "provider": row.2,
        "api_mode": row.3, "enabled": row.4
    })).collect::<Vec<_>>() })))
}

#[derive(Debug, Deserialize)]
struct ApplyTemplateInput {
    monitor_ids: Vec<i64>,
}

async fn apply_template(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(input): Json<ApplyTemplateInput>,
) -> ApiResult<Json<Value>> {
    let mut unique = HashSet::with_capacity(input.monitor_ids.len());
    if input.monitor_ids.is_empty()
        || input.monitor_ids.len() > 500
        || input
            .monitor_ids
            .iter()
            .any(|monitor_id| *monitor_id <= 0 || !unique.insert(*monitor_id))
    {
        return Err(ApiError::bad_request(
            "INVALID_MONITOR_TEMPLATE_TARGETS",
            "monitor_ids must be a non-empty unique list",
        ));
    }
    let template = get_template_row(&state, id).await?;
    let template_config = decrypt_template_config(&state, &template)?;
    let mut updates = Vec::new();
    for monitor_id in input.monitor_ids {
        let row = get_row(&state, monitor_id).await?;
        if row.template_id != Some(id)
            || row.provider != template.provider
            || row.api_mode != template.api_mode
        {
            continue;
        }
        let mut config = decrypt_config(&state, &row)?;
        apply_template_config(&mut config, template_config.clone());
        updates.push((monitor_id, encrypt_config(&state, &config)?));
    }
    let mut transaction = state.pool.begin().await?;
    let mut affected = 0_u64;
    for (monitor_id, encrypted) in updates {
        affected += sqlx::query(
            "UPDATE channel_monitors SET encrypted_request_config = ?, \
             updated_at = CURRENT_TIMESTAMP WHERE id = ? AND template_id = ? \
             AND provider = ? AND api_mode = ?",
        )
        .bind(encrypted)
        .bind(monitor_id)
        .bind(id)
        .bind(&template.provider)
        .bind(&template.api_mode)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
    }
    transaction.commit().await?;
    Ok(Json(json!({"data": {"affected": affected}})))
}

async fn run_now(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    let results = run_monitor(&state, id).await?;
    Ok(Json(json!({"data": {"results": results}})))
}

#[derive(Debug, Deserialize, Default)]
struct HistoryQuery {
    model: Option<String>,
    limit: Option<i64>,
}

async fn history(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Query(query): Query<HistoryQuery>,
) -> ApiResult<Json<Value>> {
    get_row(&state, id).await?;
    let limit = query.limit.unwrap_or(100).clamp(1, 500);
    let rows: Vec<(
        i64,
        String,
        String,
        Option<i64>,
        Option<i64>,
        String,
        String,
    )> = sqlx::query_as(
        "SELECT id, model, status, latency_ms, ping_latency_ms, message, checked_at \
             FROM channel_monitor_history WHERE monitor_id = ? AND (? IS NULL OR model = ?) \
             ORDER BY id DESC LIMIT ?",
    )
    .bind(id)
    .bind(&query.model)
    .bind(&query.model)
    .bind(limit)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(
        json!({"data": rows.into_iter().map(|row| json!({"id": row.0,
        "model": row.1, "status": row.2, "latency_ms": row.3,
        "ping_latency_ms": row.4, "message": row.5, "checked_at": row.6})).collect::<Vec<_>>() }),
    ))
}

async fn user_list(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    if !monitoring_enabled(&state).await? {
        return Ok(Json(json!({"data": []})));
    }
    let rows = sqlx::query_as::<_, MonitorRow>(&format!(
        "{MONITOR_SELECT} WHERE enabled = 1 ORDER BY group_name ASC, id ASC"
    ))
    .fetch_all(&state.pool)
    .await?;
    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        let latest = latest_result(&state, row.id, &row.primary_model).await?;
        let extra_models = parse_models(&row.extra_models)?;
        let mut extra = Vec::new();
        for model in extra_models {
            let status = latest_result(&state, row.id, &model).await?;
            extra.push(json!({"model": model, "status": status.as_ref().map(|v| v.0.as_str()).unwrap_or("error"),
                "latency_ms": status.as_ref().and_then(|v| v.1),
                "ping_latency_ms": status.as_ref().and_then(|v| v.2)}));
        }
        let timeline: Vec<(String, Option<i64>, Option<i64>, String)> = sqlx::query_as(
            "SELECT status, latency_ms, ping_latency_ms, checked_at FROM channel_monitor_history \
             WHERE monitor_id = ? AND model = ? ORDER BY id DESC LIMIT 24",
        )
        .bind(row.id)
        .bind(&row.primary_model)
        .fetch_all(&state.pool)
        .await?;
        items.push(
            json!({"id": row.id, "name": row.name, "provider": row.provider,
            "group_name": row.group_name, "primary_model": row.primary_model,
            "primary_status": latest.as_ref().map(|v| v.0.as_str()).unwrap_or("error"),
            "primary_latency_ms": latest.as_ref().and_then(|v| v.1),
            "primary_ping_latency_ms": latest.as_ref().and_then(|v| v.2),
            "availability_7d": availability(&state, row.id, &row.primary_model, 7).await?,
            "extra_models": extra, "timeline": timeline.into_iter().rev().map(|item| json!({
                "status": item.0, "latency_ms": item.1, "ping_latency_ms": item.2,
                "checked_at": item.3})).collect::<Vec<_>>() }),
        );
    }
    Ok(Json(json!({"data": items})))
}

async fn user_status(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    let row = get_row(&state, id).await?;
    if !row.enabled {
        return Err(ApiError::not_found("channel monitor not found"));
    }
    let mut models = vec![row.primary_model.clone()];
    models.extend(parse_models(&row.extra_models)?);
    let mut details = Vec::new();
    for model in models {
        let latest = latest_result(&state, id, &model).await?;
        let avg_latency: Option<f64> = sqlx::query_scalar(
            "SELECT AVG(latency_ms) FROM channel_monitor_history WHERE monitor_id = ? \
             AND model = ? AND checked_at >= datetime('now', '-7 days') AND latency_ms IS NOT NULL",
        )
        .bind(id)
        .bind(&model)
        .fetch_one(&state.pool)
        .await?;
        let avg_ping_latency: Option<f64> = sqlx::query_scalar(
            "SELECT AVG(ping_latency_ms) FROM channel_monitor_history WHERE monitor_id = ? \
             AND model = ? AND checked_at >= datetime('now', '-7 days') \
             AND ping_latency_ms IS NOT NULL",
        )
        .bind(id)
        .bind(&model)
        .fetch_one(&state.pool)
        .await?;
        details.push(json!({"model": model,
            "latest_status": latest.as_ref().map(|v| v.0.as_str()).unwrap_or("error"),
            "latest_latency_ms": latest.as_ref().and_then(|v| v.1),
            "latest_ping_latency_ms": latest.as_ref().and_then(|v| v.2),
            "availability_7d": availability(&state, id, &model, 7).await?,
            "availability_15d": availability(&state, id, &model, 15).await?,
            "availability_30d": availability(&state, id, &model, 30).await?,
            "avg_latency_7d_ms": avg_latency.map(|v| v.round() as i64),
            "avg_ping_latency_7d_ms": avg_ping_latency.map(|v| v.round() as i64)}));
    }
    Ok(Json(
        json!({"data": {"id": id, "name": row.name, "provider": row.provider,
        "group_name": row.group_name, "models": details}}),
    ))
}

async fn run_due(state: &AppState) -> ApiResult<()> {
    if !monitoring_enabled(state).await? {
        return Ok(());
    }
    let ids: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM channel_monitors WHERE enabled = 1 AND \
         (last_checked_at IS NULL OR datetime(last_checked_at, '+' || (interval_seconds + \
         CASE WHEN jitter_seconds > 0 THEN abs(id * 1103515245 + strftime('%s', last_checked_at)) \
         % (jitter_seconds + 1) ELSE 0 END) || ' seconds') \
         <= CURRENT_TIMESTAMP) ORDER BY id ASC LIMIT 20",
    )
    .fetch_all(&state.pool)
    .await?;
    for id in ids {
        if let Err(error) = run_monitor(state, id).await {
            tracing::warn!(monitor_id = id, %error, "channel monitor check failed");
        }
    }
    Ok(())
}

async fn monitoring_enabled(state: &AppState) -> ApiResult<bool> {
    let value: Option<String> =
        sqlx::query_scalar("SELECT value FROM app_settings WHERE key = 'channel_monitor_enabled'")
            .fetch_optional(&state.pool)
            .await?;
    Ok(value
        .and_then(|value| value.parse::<bool>().ok())
        .unwrap_or(true))
}

async fn run_monitor(state: &AppState, id: i64) -> ApiResult<Vec<CheckResult>> {
    let row = get_row(state, id).await?;
    let config = decrypt_config(state, &row)?;
    let mut models = vec![row.primary_model.clone()];
    models.extend(parse_models(&row.extra_models)?);
    let mut results = Vec::with_capacity(models.len());
    let ping_latency_ms = ping_endpoint_origin(state, &row.endpoint).await;
    for model in models {
        let mut result = check_model(state, &row, &config, model).await;
        result.ping_latency_ms = ping_latency_ms;
        results.push(result);
    }
    let mut transaction = state.pool.begin().await?;
    for result in &results {
        sqlx::query(
            "INSERT INTO channel_monitor_history (monitor_id, model, status, latency_ms, \
             ping_latency_ms, message, checked_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(&result.model)
        .bind(&result.status)
        .bind(result.latency_ms)
        .bind(result.ping_latency_ms)
        .bind(&result.message)
        .bind(&result.checked_at)
        .execute(&mut *transaction)
        .await?;
    }
    sqlx::query(
        "UPDATE channel_monitors SET last_checked_at = CURRENT_TIMESTAMP, \
         updated_at = CURRENT_TIMESTAMP WHERE id = ?",
    )
    .bind(id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "DELETE FROM channel_monitor_history WHERE checked_at < datetime('now', '-90 days')",
    )
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(results)
}

async fn ping_endpoint_origin(state: &AppState, endpoint: &str) -> Option<i64> {
    let mut origin = url::Url::parse(endpoint).ok()?;
    origin.set_path("/");
    origin.set_query(None);
    origin.set_fragment(None);
    let started = Instant::now();
    state
        .client
        .head(origin)
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .ok()?;
    Some(started.elapsed().as_millis().min(i64::MAX as u128) as i64)
}

async fn check_model(
    state: &AppState,
    row: &MonitorRow,
    config: &RequestConfig,
    model: String,
) -> CheckResult {
    let checked_at = chrono::Utc::now().to_rfc3339();
    let mut body = if row.provider == "gemini" {
        json!({"contents": [{"parts": [{"text": "Reply with OK."}]}],
            "generationConfig": {"maxOutputTokens": 1}})
    } else if row.api_mode == "responses" {
        json!({"model": model, "input": "Reply with OK.", "max_output_tokens": 1, "stream": false})
    } else {
        json!({"model": model, "messages": [{"role": "user", "content": "Reply with OK."}],
            "max_tokens": 1, "stream": false})
    };
    if config.body_override_mode == "replace" {
        if let Some(value) = config.body_override.clone() {
            body = value;
        }
    } else if config.body_override_mode == "merge"
        && let (Some(target), Some(source)) = (
            body.as_object_mut(),
            config.body_override.as_ref().and_then(Value::as_object),
        )
    {
        target.extend(source.clone());
    }
    let endpoint = if row.endpoint.contains("{model}") {
        let encoded = url::form_urlencoded::byte_serialize(model.as_bytes()).collect::<String>();
        row.endpoint.replace("{model}", &encoded)
    } else {
        row.endpoint.clone()
    };
    let mut request = state
        .client
        .post(endpoint)
        .timeout(Duration::from_secs(30))
        .json(&body);
    request = match row.provider.as_str() {
        "anthropic" => request
            .header("x-api-key", &config.api_key)
            .header("anthropic-version", "2023-06-01"),
        "gemini" => request.header("x-goog-api-key", &config.api_key),
        _ => request.bearer_auth(&config.api_key),
    };
    for (name, value) in &config.extra_headers {
        let Ok(name) = HeaderName::from_bytes(name.as_bytes()) else {
            return failed_result(model, checked_at, "error", "invalid custom header", None);
        };
        let Ok(value) = HeaderValue::from_str(value) else {
            return failed_result(model, checked_at, "error", "invalid custom header", None);
        };
        request = request.header(name, value);
    }
    let started = Instant::now();
    match request.send().await {
        Ok(response) => {
            let latency = started.elapsed().as_millis().min(i64::MAX as u128) as i64;
            let code = response.status().as_u16();
            let (status, message) = if response.status().is_success() {
                ("operational", "request succeeded".to_string())
            } else if code == 429 {
                ("degraded", "upstream rate limited the request".to_string())
            } else if code >= 500 {
                ("failed", format!("upstream returned HTTP {code}"))
            } else {
                (
                    "error",
                    format!("upstream rejected the request with HTTP {code}"),
                )
            };
            failed_result(model, checked_at, status, &message, Some(latency))
        }
        Err(error) => failed_result(
            model,
            checked_at,
            "error",
            if error.is_timeout() {
                "request timed out"
            } else {
                "connection failed"
            },
            None,
        ),
    }
}

fn failed_result(
    model: String,
    checked_at: String,
    status: &str,
    message: &str,
    latency_ms: Option<i64>,
) -> CheckResult {
    CheckResult {
        model,
        status: status.into(),
        latency_ms,
        ping_latency_ms: None,
        message: message.into(),
        checked_at,
    }
}

async fn latest_result(
    state: &AppState,
    id: i64,
    model: &str,
) -> ApiResult<Option<(String, Option<i64>, Option<i64>)>> {
    Ok(sqlx::query_as(
        "SELECT status, latency_ms, ping_latency_ms FROM channel_monitor_history WHERE monitor_id = ? \
         AND model = ? ORDER BY id DESC LIMIT 1",
    )
    .bind(id)
    .bind(model)
    .fetch_optional(&state.pool)
    .await?)
}

async fn availability(state: &AppState, id: i64, model: &str, days: i64) -> ApiResult<f64> {
    let row: (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(SUM(CASE WHEN status = 'operational' THEN 1 ELSE 0 END), 0) \
         FROM channel_monitor_history WHERE monitor_id = ? AND model = ? \
         AND checked_at >= datetime('now', '-' || ? || ' days')",
    )
    .bind(id)
    .bind(model)
    .bind(days)
    .fetch_one(&state.pool)
    .await?;
    Ok(if row.0 == 0 {
        0.0
    } else {
        row.1 as f64 * 100.0 / row.0 as f64
    })
}

async fn get_row(state: &AppState, id: i64) -> ApiResult<MonitorRow> {
    sqlx::query_as::<_, MonitorRow>(&format!("{MONITOR_SELECT} WHERE id = ?"))
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| ApiError::not_found("channel monitor not found"))
}

async fn get_template_row(state: &AppState, id: i64) -> ApiResult<TemplateRow> {
    sqlx::query_as::<_, TemplateRow>(&format!("{TEMPLATE_SELECT} WHERE id = ?"))
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| ApiError::not_found("channel monitor template not found"))
}

async fn template_view(state: &AppState, row: &TemplateRow) -> ApiResult<Value> {
    let config = decrypt_template_config(state, row)?;
    let associated: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM channel_monitors WHERE template_id = ?")
            .bind(row.id)
            .fetch_one(&state.pool)
            .await?;
    Ok(json!({
        "id": row.id, "name": row.name, "provider": row.provider,
        "api_mode": row.api_mode, "description": row.description,
        "extra_headers": config.extra_headers,
        "body_override_mode": config.body_override_mode,
        "body_override": config.body_override,
        "associated_monitors": associated,
        "created_at": row.created_at, "updated_at": row.updated_at
    }))
}

fn encrypt_config(state: &AppState, config: &RequestConfig) -> ApiResult<String> {
    state.crypto.encrypt(
        &serde_json::to_vec(config)
            .map_err(|_| ApiError::internal("monitor request serialization failed"))?,
    )
}

fn decrypt_config(state: &AppState, row: &MonitorRow) -> ApiResult<RequestConfig> {
    serde_json::from_slice(&state.crypto.decrypt(&row.encrypted_request_config)?)
        .map_err(|_| ApiError::internal("stored monitor request is malformed"))
}

fn encrypt_template_config(state: &AppState, config: &TemplateConfig) -> ApiResult<String> {
    state.crypto.encrypt(
        &serde_json::to_vec(config)
            .map_err(|_| ApiError::internal("monitor template serialization failed"))?,
    )
}

fn decrypt_template_config(state: &AppState, row: &TemplateRow) -> ApiResult<TemplateConfig> {
    serde_json::from_slice(&state.crypto.decrypt(&row.encrypted_template_config)?)
        .map_err(|_| ApiError::internal("stored monitor template is malformed"))
}

fn apply_template_config(target: &mut RequestConfig, template: TemplateConfig) {
    target.extra_headers = template.extra_headers;
    target.body_override_mode = normalize_override_mode(&template.body_override_mode);
    target.body_override = template.body_override;
}

fn ensure_template_compatible(
    template: &TemplateRow,
    provider: &str,
    api_mode: &str,
) -> ApiResult<()> {
    if template.provider != provider || template.api_mode != api_mode {
        return Err(ApiError::bad_request(
            "MONITOR_TEMPLATE_PROTOCOL_MISMATCH",
            "monitor template provider and API mode must match the monitor",
        ));
    }
    Ok(())
}

fn template_unique_error(error: sqlx::Error) -> ApiError {
    match error {
        sqlx::Error::Database(ref database) if database.is_unique_violation() => {
            ApiError::bad_request(
                "MONITOR_TEMPLATE_EXISTS",
                "a template with this provider, API mode, and name already exists",
            )
        }
        other => other.into(),
    }
}

fn parse_models(value: &str) -> ApiResult<Vec<String>> {
    serde_json::from_str(value)
        .map_err(|_| ApiError::internal("stored monitor models are malformed"))
}

fn serialize_models(values: Vec<String>) -> ApiResult<String> {
    serde_json::to_string(&values)
        .map_err(|_| ApiError::internal("monitor model serialization failed"))
}

fn normalize_models(values: Vec<String>) -> Vec<String> {
    let mut result = Vec::new();
    for value in values {
        let value = value.trim().to_string();
        if !value.is_empty() && !result.contains(&value) {
            result.push(value);
        }
    }
    result
}

fn normalize_override_mode(value: &str) -> String {
    match value {
        "merge" | "replace" => value.into(),
        _ => "off".into(),
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_input(
    name: &str,
    provider: &str,
    api_mode: &str,
    endpoint: &str,
    api_key: &str,
    primary_model: &str,
    extra_models: &[String],
    interval_seconds: i64,
    jitter_seconds: i64,
    body_override_mode: &str,
) -> ApiResult<()> {
    let url = url::Url::parse(endpoint)
        .map_err(|_| ApiError::bad_request("INVALID_MONITOR_ENDPOINT", "endpoint is invalid"))?;
    if name.trim().is_empty()
        || name.chars().count() > 100
        || !matches!(provider, "openai" | "anthropic" | "gemini" | "grok")
        || !matches!(api_mode, "chat_completions" | "responses")
        || (api_mode == "responses" && provider != "openai")
        || !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || api_key.trim().is_empty()
        || primary_model.trim().is_empty()
        || extra_models.len() > 20
        || !(30..=86400).contains(&interval_seconds)
        || !(0..=3600).contains(&jitter_seconds)
        || !matches!(body_override_mode, "" | "off" | "merge" | "replace")
    {
        return Err(ApiError::bad_request(
            "INVALID_CHANNEL_MONITOR",
            "channel monitor settings are invalid",
        ));
    }
    Ok(())
}

fn validate_template(
    name: &str,
    provider: &str,
    api_mode: &str,
    description: &str,
    config: &TemplateConfig,
) -> ApiResult<()> {
    if name.trim().is_empty()
        || name.chars().count() > 100
        || description.chars().count() > 500
        || !matches!(provider, "openai" | "anthropic" | "gemini" | "grok")
        || !matches!(api_mode, "chat_completions" | "responses")
        || (api_mode == "responses" && provider != "openai")
    {
        return Err(ApiError::bad_request(
            "INVALID_MONITOR_TEMPLATE",
            "monitor template name or protocol is invalid",
        ));
    }
    validate_snapshot(
        &config.extra_headers,
        &config.body_override_mode,
        config.body_override.as_ref(),
    )
}

fn validate_request_config(config: &RequestConfig) -> ApiResult<()> {
    validate_snapshot(
        &config.extra_headers,
        &config.body_override_mode,
        config.body_override.as_ref(),
    )
}

fn validate_snapshot(
    headers: &BTreeMap<String, String>,
    mode: &str,
    body: Option<&Value>,
) -> ApiResult<()> {
    if headers.len() > 32 {
        return Err(ApiError::bad_request(
            "INVALID_MONITOR_HEADERS",
            "at most 32 custom headers are supported",
        ));
    }
    for (name, value) in headers {
        let normalized = name.trim().to_ascii_lowercase();
        if name.len() > 128
            || value.len() > 8192
            || HeaderName::from_bytes(name.as_bytes()).is_err()
            || HeaderValue::from_str(value).is_err()
            || matches!(
                normalized.as_str(),
                "host" | "content-length" | "content-encoding" | "transfer-encoding" | "connection"
            )
        {
            return Err(ApiError::bad_request(
                "INVALID_MONITOR_HEADERS",
                "custom headers contain an invalid or managed header",
            ));
        }
    }
    if !matches!(mode, "off" | "merge" | "replace") {
        return Err(ApiError::bad_request(
            "INVALID_MONITOR_BODY_OVERRIDE",
            "body override mode is invalid",
        ));
    }
    if matches!(mode, "merge" | "replace")
        && !body.is_some_and(|value| value.as_object().is_some_and(|object| !object.is_empty()))
    {
        return Err(ApiError::bad_request(
            "MONITOR_BODY_OVERRIDE_REQUIRED",
            "merge and replace modes require a non-empty JSON object",
        ));
    }
    if body
        .and_then(|value| serde_json::to_vec(value).ok())
        .is_some_and(|bytes| bytes.len() > 65_536)
    {
        return Err(ApiError::bad_request(
            "MONITOR_BODY_OVERRIDE_TOO_LARGE",
            "body override exceeds 64 KiB",
        ));
    }
    Ok(())
}

fn mask_secret(value: &str) -> String {
    if value.len() <= 8 {
        "********".into()
    } else {
        format!("{}...{}", &value[..4], &value[value.len() - 4..])
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};

    use super::*;
    use crate::test_support;

    #[tokio::test]
    async fn monitor_config_is_encrypted_and_history_drives_availability() {
        let (_directory, state) = test_support::state().await;
        let (_, Json(created)) = create(
            State(state.clone()),
            Json(CreateInput {
                name: "OpenAI".into(),
                provider: "openai".into(),
                api_mode: "chat_completions".into(),
                endpoint: "http://127.0.0.1:9/v1/chat/completions".into(),
                api_key: "secret-monitor-key".into(),
                primary_model: "gpt-test".into(),
                extra_models: vec![],
                group_name: "default".into(),
                enabled: true,
                interval_seconds: 300,
                jitter_seconds: 0,
                extra_headers: Default::default(),
                body_override_mode: "off".into(),
                body_override: None,
                template_id: None,
            }),
        )
        .await
        .unwrap();
        let id = created["data"]["id"].as_i64().unwrap();
        let encrypted: String = sqlx::query_scalar(
            "SELECT encrypted_request_config FROM channel_monitors WHERE id = ?",
        )
        .bind(id)
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert!(!encrypted.contains("secret-monitor-key"));
        for status in ["operational", "operational", "failed"] {
            sqlx::query(
                "INSERT INTO channel_monitor_history (monitor_id, model, status) VALUES (?, 'gpt-test', ?)",
            )
            .bind(id)
            .bind(status)
            .execute(&state.pool)
            .await
            .unwrap();
        }
        let rate = availability(&state, id, "gpt-test", 7).await.unwrap();
        assert!((rate - 66.666).abs() < 0.01);
    }

    #[tokio::test]
    async fn templates_are_encrypted_snapshot_applied_and_delete_safe() {
        let (_directory, state) = test_support::state().await;
        let (_, Json(template)) = create_template(
            State(state.clone()),
            Json(TemplateInput {
                name: "Codex headers".into(),
                provider: "openai".into(),
                api_mode: "responses".into(),
                description: "shared request snapshot".into(),
                extra_headers: BTreeMap::from([("X-Template-Secret".into(), "first".into())]),
                body_override_mode: "off".into(),
                body_override: None,
            }),
        )
        .await
        .unwrap();
        let template_id = template["data"]["id"].as_i64().unwrap();
        let encrypted: String = sqlx::query_scalar(
            "SELECT encrypted_template_config FROM channel_monitor_templates WHERE id = ?",
        )
        .bind(template_id)
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert!(!encrypted.contains("X-Template-Secret"));
        assert!(!encrypted.contains("first"));

        let (_, Json(monitor)) = create(
            State(state.clone()),
            Json(CreateInput {
                name: "Templated".into(),
                provider: "openai".into(),
                api_mode: "responses".into(),
                endpoint: "http://127.0.0.1:9/v1/responses".into(),
                api_key: "template-monitor-key".into(),
                primary_model: "gpt-test".into(),
                extra_models: vec![],
                group_name: String::new(),
                enabled: false,
                interval_seconds: 300,
                jitter_seconds: 0,
                extra_headers: BTreeMap::from([("X-Manual".into(), "ignored".into())]),
                body_override_mode: "off".into(),
                body_override: None,
                template_id: Some(template_id),
            }),
        )
        .await
        .unwrap();
        let monitor_id = monitor["data"]["id"].as_i64().unwrap();
        assert_eq!(monitor["data"]["template_id"], template_id);
        assert_eq!(
            monitor["data"]["extra_headers"]["X-Template-Secret"],
            "first"
        );
        assert!(monitor["data"]["extra_headers"].get("X-Manual").is_none());

        let _ = update_template(
            State(state.clone()),
            Path(template_id),
            Json(TemplateUpdateInput {
                name: None,
                api_mode: None,
                description: None,
                extra_headers: Some(BTreeMap::from([(
                    "X-Template-Secret".into(),
                    "second".into(),
                )])),
                body_override_mode: None,
                body_override: None,
            }),
        )
        .await
        .unwrap();
        let Json(before_apply) = admin_get(State(state.clone()), Path(monitor_id))
            .await
            .unwrap();
        assert_eq!(
            before_apply["data"]["extra_headers"]["X-Template-Secret"],
            "first"
        );
        let Json(applied) = apply_template(
            State(state.clone()),
            Path(template_id),
            Json(ApplyTemplateInput {
                monitor_ids: vec![monitor_id],
            }),
        )
        .await
        .unwrap();
        assert_eq!(applied["data"]["affected"], 1);
        let Json(after_apply) = admin_get(State(state.clone()), Path(monitor_id))
            .await
            .unwrap();
        assert_eq!(
            after_apply["data"]["extra_headers"]["X-Template-Secret"],
            "second"
        );
        let Json(associated) = template_monitors(State(state.clone()), Path(template_id))
            .await
            .unwrap();
        assert_eq!(associated["data"][0]["id"], monitor_id);

        delete_template(State(state.clone()), Path(template_id))
            .await
            .unwrap();
        let Json(detached) = admin_get(State(state), Path(monitor_id)).await.unwrap();
        assert!(detached["data"]["template_id"].is_null());
        assert_eq!(
            detached["data"]["extra_headers"]["X-Template-Secret"],
            "second"
        );
    }

    #[tokio::test]
    async fn run_monitor_sends_authenticated_probe_and_records_result() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = vec![0; 8192];
                let read = stream.read(&mut request).unwrap();
                request.truncate(read);
                let request = String::from_utf8_lossy(&request).to_string();
                if request.starts_with("POST ") {
                    sender.send(request).unwrap();
                    stream
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
                        )
                        .unwrap();
                } else {
                    stream
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        )
                        .unwrap();
                }
            }
        });
        let (_directory, state) = test_support::state().await;
        let (_, Json(created)) = create(
            State(state.clone()),
            Json(CreateInput {
                name: "Probe".into(),
                provider: "openai".into(),
                api_mode: "chat_completions".into(),
                endpoint: format!("http://{address}/v1/chat/completions"),
                api_key: "probe-secret".into(),
                primary_model: "gpt-probe".into(),
                extra_models: vec![],
                group_name: String::new(),
                enabled: false,
                interval_seconds: 300,
                jitter_seconds: 0,
                extra_headers: Default::default(),
                body_override_mode: "off".into(),
                body_override: None,
                template_id: None,
            }),
        )
        .await
        .unwrap();
        let id = created["data"]["id"].as_i64().unwrap();
        let results = run_monitor(&state, id).await.unwrap();
        server.join().unwrap();
        let request = receiver.recv().unwrap();
        assert!(
            request
                .to_ascii_lowercase()
                .contains("authorization: bearer probe-secret")
        );
        assert!(request.contains("gpt-probe"));
        assert_eq!(results[0].status, "operational");
        assert!(results[0].ping_latency_ms.is_some());
        let stored: (String, Option<i64>) = sqlx::query_as(
            "SELECT status, ping_latency_ms FROM channel_monitor_history WHERE monitor_id = ?",
        )
        .bind(id)
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(stored.0, "operational");
        assert!(stored.1.is_some());
    }
}
