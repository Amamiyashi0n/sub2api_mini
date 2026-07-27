use std::{
    collections::HashSet,
    io,
    time::{Duration, Instant},
};

use axum::{
    Router,
    body::{Body, Bytes},
    extract::{ConnectInfo, Extension, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::Response,
    routing::{get, post},
};
use chrono::{Duration as ChronoDuration, Utc};
use futures_util::StreamExt;
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::{
    crypto::token_hash,
    error::{ApiError, ApiResult},
    groups, key_policy,
    models::{Account, AccountRow, ApiKeyContext, ApiKeyRow},
    oauth,
    state::{AppState, CachedModels, ScheduledAccount},
};

#[derive(Clone, Copy, Debug)]
enum Endpoint {
    Responses,
    ChatCompletions,
    Models,
    Messages,
    CountTokens,
}

impl Endpoint {
    fn path(self) -> &'static str {
        match self {
            Self::Responses => "responses",
            Self::ChatCompletions => "chat/completions",
            Self::Models => "models",
            Self::Messages => "messages",
            Self::CountTokens => "messages/count_tokens",
        }
    }

    fn log_name(self) -> &'static str {
        match self {
            Self::Responses => "/v1/responses",
            Self::ChatCompletions => "/v1/chat/completions",
            Self::Models => "/v1/models",
            Self::Messages => "/v1/messages",
            Self::CountTokens => "/v1/messages/count_tokens",
        }
    }

    fn platform(self) -> &'static str {
        match self {
            Self::Messages | Self::CountTokens => "anthropic",
            Self::Responses | Self::ChatCompletions | Self::Models => "openai",
        }
    }
}

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .merge(crate::batch_images::gateway_router())
        .route("/models", get(models))
        .route("/responses", post(responses))
        .route("/chat/completions", post(chat_completions))
        .route("/messages", post(messages))
        .route("/messages/count_tokens", post(count_tokens))
        .route_layer(middleware::from_fn_with_state(state, api_key_guard))
}

pub async fn available_model_catalog(state: &AppState) -> ApiResult<Value> {
    let model_cache_seconds = state.runtime_settings.read().await.model_cache_seconds;
    let rows = sqlx::query_as::<_, AccountRow>(
        "SELECT accounts.id, accounts.name, accounts.kind, accounts.platform, \
         accounts.account_type, accounts.base_url, \
         accounts.encrypted_credentials, accounts.priority, accounts.concurrency, \
         accounts.enabled, accounts.cooldown_until, accounts.last_used_at, accounts.last_error, \
         accounts.proxy_id, proxies.name AS proxy_name, CASE WHEN proxies.id IS NULL THEN NULL \
         WHEN proxies.enabled = 1 AND (proxies.expires_at IS NULL OR \
         datetime(proxies.expires_at) > CURRENT_TIMESTAMP) THEN 1 WHEN proxies.fallback_mode = 'direct' \
         THEN 1 WHEN proxies.fallback_mode = 'proxy' AND backup_proxies.enabled = 1 AND \
         (backup_proxies.expires_at IS NULL OR datetime(backup_proxies.expires_at) > CURRENT_TIMESTAMP) \
         THEN 1 ELSE 0 END AS proxy_active, CASE WHEN proxies.enabled = 1 AND \
         (proxies.expires_at IS NULL OR datetime(proxies.expires_at) > CURRENT_TIMESTAMP) \
         THEN proxies.encrypted_url WHEN proxies.fallback_mode = 'proxy' AND backup_proxies.enabled = 1 \
         AND (backup_proxies.expires_at IS NULL OR datetime(backup_proxies.expires_at) > CURRENT_TIMESTAMP) \
         THEN backup_proxies.encrypted_url ELSE NULL END AS encrypted_proxy_url, \
         accounts.parent_account_id, accounts.quota_dimension, accounts.notes, \
         accounts.crs_account_id, accounts.tls_fingerprint_profile_id, \
         accounts.created_at, accounts.updated_at FROM accounts \
         LEFT JOIN proxies ON proxies.id = accounts.proxy_id \
         LEFT JOIN proxies AS backup_proxies ON backup_proxies.id = proxies.backup_proxy_id \
         WHERE accounts.enabled = 1 AND (accounts.proxy_id IS NULL OR (proxies.enabled = 1 AND \
         (proxies.expires_at IS NULL OR datetime(proxies.expires_at) > CURRENT_TIMESTAMP)) OR \
         proxies.fallback_mode = 'direct' OR (proxies.fallback_mode = 'proxy' AND \
         backup_proxies.enabled = 1 AND (backup_proxies.expires_at IS NULL OR \
         datetime(backup_proxies.expires_at) > CURRENT_TIMESTAMP))) \
         ORDER BY accounts.priority ASC, accounts.id ASC",
    )
    .fetch_all(&state.pool)
    .await?;
    let mut model_ids = HashSet::new();
    let mut sources = Vec::with_capacity(rows.len());

    for row in rows {
        let cached = state.model_cache.lock().await.get(&row.id).cloned();
        let value = if let Some(cached) = cached
            .filter(|item| item.created_at.elapsed() < Duration::from_secs(model_cache_seconds))
        {
            Ok(cached.value)
        } else {
            let mut account = state.resolve_account(row.clone()).await?;
            if let Err(error) = oauth::refresh_if_needed(state, &mut account).await {
                Err(error)
            } else {
                match send_upstream(state, &account, Endpoint::Models, &HeaderMap::new(), None)
                    .await
                {
                    Ok(response) if response.status().is_success() => {
                        let value = normalize_models(response.json::<Value>().await?);
                        state.model_cache.lock().await.insert(
                            row.id,
                            CachedModels {
                                value: value.clone(),
                                created_at: Instant::now(),
                            },
                        );
                        Ok(value)
                    }
                    Ok(_) => Err(ApiError::new(
                        StatusCode::BAD_GATEWAY,
                        "MODEL_LIST_FAILED",
                        "upstream model list request failed",
                    )),
                    Err(error) => Err(error),
                }
            }
        };

        match value {
            Ok(value) => {
                let source_models = value
                    .get("data")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|model| model.get("id").and_then(Value::as_str))
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>();
                model_ids.extend(source_models.iter().cloned());
                sources.push(json!({
                    "id": row.id, "name": row.name, "kind": row.kind,
                    "status": "available", "models": source_models
                }));
            }
            Err(error) => sources.push(json!({
                "id": row.id, "name": row.name, "kind": row.kind,
                "status": "unavailable", "models": [], "error": error.to_string()
            })),
        }
    }

    let mut models = model_ids.into_iter().collect::<Vec<_>>();
    models.sort();
    Ok(json!({"data": {"models": models, "sources": sources}}))
}

pub(crate) async fn api_key_guard(
    State(state): State<AppState>,
    mut request: axum::extract::Request,
    next: Next,
) -> ApiResult<Response> {
    let peer_ip = request
        .extensions()
        .get::<ConnectInfo<std::net::SocketAddr>>()
        .map(|connect| connect.0.ip());
    let token = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            request
                .headers()
                .get("x-api-key")
                .and_then(|value| value.to_str().ok())
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
        .ok_or_else(|| ApiError::unauthorized("Bearer API key or x-api-key header is required"))?;
    let hash = token_hash(token);
    let row = sqlx::query_as::<_, ApiKeyRow>(
        "SELECT id, user_id, name, token_prefix, token_hash, enabled, last_used_at, created_at, \
         expires_at, quota_tokens, quota_cost_microusd, quota_reset_at, allowed_models, group_id, \
         ip_whitelist, ip_blacklist, rate_limit_5h_microusd, rate_limit_1d_microusd, \
         rate_limit_7d_microusd, rate_usage_reset_at \
         FROM api_keys WHERE token_hash = ? AND enabled = 1",
    )
    .bind(hash)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| ApiError::unauthorized("API key is invalid or disabled"))?;

    key_policy::enforce(&state, &row, peer_ip).await?;
    if let Some(user_id) = row.user_id {
        let enabled: Option<bool> = sqlx::query_scalar("SELECT enabled FROM users WHERE id = ?")
            .bind(user_id)
            .fetch_optional(&state.pool)
            .await?;
        if enabled != Some(true) {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "USER_DISABLED",
                "the API key owner is disabled",
            ));
        }
        if let Some(group_id) = row.group_id {
            groups::ensure_user_group_access(&state, user_id, group_id).await?;
        }
        enforce_subscription_quota(&state, user_id, row.group_id).await?;
    }
    let mut allowed_models = serde_json::from_str::<Vec<String>>(&row.allowed_models)
        .map_err(|_| ApiError::internal("stored API key model policy is malformed"))?;
    if let Some(group_id) = row.group_id {
        let group: Option<(bool, String)> =
            sqlx::query_as("SELECT enabled, allowed_models FROM groups WHERE id = ?")
                .bind(group_id)
                .fetch_optional(&state.pool)
                .await?;
        let (enabled, group_models) =
            group.ok_or_else(|| ApiError::forbidden("API key group no longer exists"))?;
        if !enabled {
            return Err(ApiError::forbidden("API key group is disabled"));
        }
        let group_models = serde_json::from_str::<Vec<String>>(&group_models)
            .map_err(|_| ApiError::internal("stored group model policy is malformed"))?;
        if !group_models.is_empty() {
            if allowed_models.is_empty() {
                allowed_models = group_models;
            } else {
                allowed_models.retain(|model| group_models.iter().any(|item| item == model));
            }
        }
    }

    let context = ApiKeyContext {
        id: row.id,
        user_id: row.user_id,
        allowed_models,
        group_id: row.group_id,
    };
    request.extensions_mut().insert(context);
    let pool = state.pool.clone();
    let peer_ip = peer_ip.map(|ip| ip.to_string());
    tokio::spawn(async move {
        let _ = sqlx::query(
            "UPDATE api_keys SET last_used_at = CURRENT_TIMESTAMP, last_used_ip = ?, \
             updated_at = CURRENT_TIMESTAMP WHERE id = ?",
        )
        .bind(peer_ip)
        .bind(row.id)
        .execute(&pool)
        .await;
    });
    Ok(next.run(request).await)
}

async fn enforce_subscription_quota(
    state: &AppState,
    user_id: i64,
    key_group_id: Option<i64>,
) -> ApiResult<()> {
    let subscription: Option<(i64, String, String, Option<i64>)> = sqlx::query_as(
        "SELECT token_limit, starts_at, ends_at, group_id FROM subscriptions \
         WHERE user_id = ? AND status = 'active' \
         AND datetime(ends_at) > CURRENT_TIMESTAMP \
         AND (group_id IS NULL OR group_id IS ?) \
         ORDER BY (group_id IS NOT NULL) DESC, id DESC LIMIT 1",
    )
    .bind(user_id)
    .bind(key_group_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some((token_limit, starts_at, ends_at, subscription_group_id)) = subscription else {
        return Ok(());
    };
    if token_limit == 0 {
        return Ok(());
    }
    let used_tokens: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(log.total_tokens), 0) FROM usage_logs log \
         LEFT JOIN api_keys keys ON keys.id = log.api_key_id \
         WHERE log.user_id = ? AND datetime(log.created_at) >= datetime(?) \
         AND datetime(log.created_at) < datetime(?) \
         AND (? IS NULL OR keys.group_id = ?)",
    )
    .bind(user_id)
    .bind(starts_at)
    .bind(ends_at)
    .bind(subscription_group_id)
    .bind(subscription_group_id)
    .fetch_one(&state.pool)
    .await?;
    if used_tokens >= token_limit {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "SUBSCRIPTION_QUOTA_EXHAUSTED",
            "subscription token quota has been exhausted for this group",
        ));
    }
    Ok(())
}

async fn responses(
    State(state): State<AppState>,
    Extension(key): Extension<ApiKeyContext>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Response> {
    proxy_json(state, key, headers, body, Endpoint::Responses).await
}

async fn chat_completions(
    State(state): State<AppState>,
    Extension(key): Extension<ApiKeyContext>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Response> {
    proxy_json(state, key, headers, body, Endpoint::ChatCompletions).await
}

async fn messages(
    State(state): State<AppState>,
    Extension(key): Extension<ApiKeyContext>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Response> {
    proxy_json(state, key, headers, body, Endpoint::Messages).await
}

async fn count_tokens(
    State(state): State<AppState>,
    Extension(key): Extension<ApiKeyContext>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Response> {
    proxy_json(state, key, headers, body, Endpoint::CountTokens).await
}

async fn models(
    State(state): State<AppState>,
    Extension(key): Extension<ApiKeyContext>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let _active_request = state.track_request();
    let runtime = state.runtime_settings.read().await.clone();
    let started = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let mut excluded = HashSet::new();
    let mut last_error = None;
    let mut upstream_attempts = 0;

    for attempt in 0..runtime.retry_attempts {
        upstream_attempts += 1;
        let mut scheduled = state
            .scheduler
            .select(&state, &excluded, key.group_id, Endpoint::Models.platform())
            .await?;
        excluded.insert(scheduled.account.row.id);

        if let Some(cached) = state
            .model_cache
            .lock()
            .await
            .get(&scheduled.account.row.id)
            .cloned()
        {
            if cached.created_at.elapsed() < Duration::from_secs(runtime.model_cache_seconds) {
                log_usage(
                    &state,
                    &request_id,
                    key.id,
                    key.user_id,
                    Some(scheduled.account.row.id),
                    Endpoint::Models,
                    None,
                    None,
                    None,
                    None,
                    String::new(),
                    200,
                    Usage::default(),
                    false,
                    None,
                    started.elapsed(),
                    RequestTelemetry::with_attempts(upstream_attempts),
                    None,
                )
                .await;
                return json_response(
                    StatusCode::OK,
                    filter_models(cached.value, &key.allowed_models),
                );
            }
        }

        if let Err(error) = oauth::refresh_if_needed(&state, &mut scheduled.account).await {
            last_error = Some(error);
            continue;
        }
        match send_upstream(&state, &scheduled.account, Endpoint::Models, &headers, None).await {
            Ok(response) if response.status().is_success() => {
                let value: Value = response.json().await.map_err(ApiError::from)?;
                let value = normalize_models(value);
                state.model_cache.lock().await.insert(
                    scheduled.account.row.id,
                    CachedModels {
                        value: value.clone(),
                        created_at: Instant::now(),
                    },
                );
                clear_account_error(&state, scheduled.account.row.id).await;
                log_usage(
                    &state,
                    &request_id,
                    key.id,
                    key.user_id,
                    Some(scheduled.account.row.id),
                    Endpoint::Models,
                    None,
                    None,
                    None,
                    None,
                    String::new(),
                    200,
                    Usage::default(),
                    false,
                    None,
                    started.elapsed(),
                    RequestTelemetry::with_attempts(upstream_attempts),
                    None,
                )
                .await;
                return json_response(StatusCode::OK, filter_models(value, &key.allowed_models));
            }
            Ok(response) => {
                let mut status = response.status();
                if retryable_status(status) && attempt + 1 < runtime.retry_attempts {
                    cool_down_account(&state, scheduled.account.row.id, status, response.headers())
                        .await;
                    last_error = Some(ApiError::new(
                        StatusCode::BAD_GATEWAY,
                        "UPSTREAM_RETRYABLE",
                        format!("upstream returned {status}"),
                    ));
                    continue;
                }
                if retryable_status(status) {
                    cool_down_account(&state, scheduled.account.row.id, status, response.headers())
                        .await;
                }
                let mut body = response.bytes().await.unwrap_or_default().to_vec();
                let mut skip_monitoring = false;
                if let Some(decision) =
                    crate::error_passthrough::match_response(&state, status, &body).await?
                {
                    status = decision.status;
                    body = decision.body;
                    skip_monitoring = decision.skip_monitoring;
                }
                log_usage(
                    &state,
                    &request_id,
                    key.id,
                    key.user_id,
                    Some(scheduled.account.row.id),
                    Endpoint::Models,
                    None,
                    None,
                    None,
                    None,
                    String::new(),
                    status.as_u16() as i32,
                    Usage::default(),
                    false,
                    None,
                    started.elapsed(),
                    RequestTelemetry::with_attempts(upstream_attempts),
                    (!skip_monitoring).then(|| safe_error_summary(&body)),
                )
                .await;
                return raw_response(status, "application/json", Bytes::from(body));
            }
            Err(error) => {
                mark_transport_error(&state, scheduled.account.row.id, &error).await;
                last_error = Some(error);
            }
        }
    }

    Err(last_error.unwrap_or_else(|| {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            "UPSTREAM_UNAVAILABLE",
            "all upstream accounts failed",
        )
    }))
}

async fn proxy_json(
    state: AppState,
    key: ApiKeyContext,
    headers: HeaderMap,
    body: Bytes,
    endpoint: Endpoint,
) -> ApiResult<Response> {
    let active_request = state.track_request();
    let retry_attempts = state.runtime_settings.read().await.retry_attempts;
    let started = Instant::now();
    let request_id = Uuid::new_v4().to_string();
    let value: Value = serde_json::from_slice(&body)
        .map_err(|_| ApiError::bad_request("INVALID_JSON", "request body is not valid JSON"))?;
    let model = value
        .get("model")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let model_resolution = match model.as_deref() {
        Some(model) => Some(crate::channels::resolve_model(&state, key.group_id, model).await?),
        None => None,
    };
    if !key.allowed_models.is_empty()
        && model
            .as_deref()
            .is_none_or(|model| !key.allowed_models.iter().any(|allowed| allowed == model))
    {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "MODEL_NOT_ALLOWED",
            "the requested model is not allowed for this API key",
        ));
    }
    if let (Some(group_id), Some(model)) = (key.group_id, model.as_deref()) {
        let mapped = model_resolution
            .as_ref()
            .map(|resolution| resolution.mapped.as_str())
            .unwrap_or(model);
        if !channel_model_allowed(&state, group_id, model).await?
            && (mapped == model || !channel_model_allowed(&state, group_id, mapped).await?)
        {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "CHANNEL_MODEL_NOT_ALLOWED",
                "the requested or mapped model is not enabled for this channel",
            ));
        }
    }
    crate::risk_control::inspect(
        &state,
        &key,
        endpoint.log_name(),
        model.as_deref(),
        &value,
        &request_id,
    )
    .await?;
    crate::prompt_audit::inspect(
        &state,
        &key,
        endpoint.log_name(),
        model.as_deref(),
        &value,
        &request_id,
    )
    .await?;
    let stream_requested = value
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let service_tier = value
        .get("service_tier")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.chars().take(40).collect::<String>());
    let mut upstream_request = value.clone();
    if let Some(resolution) = &model_resolution
        && resolution.mapped != resolution.requested
        && let Some(object) = upstream_request.as_object_mut()
    {
        object.insert("model".into(), Value::String(resolution.mapped.clone()));
    }
    let mut excluded = HashSet::new();
    let mut last_error = None;
    let mut upstream_attempts = 0;

    for attempt in 0..retry_attempts {
        upstream_attempts += 1;
        let mut scheduled = state
            .scheduler
            .select(&state, &excluded, key.group_id, endpoint.platform())
            .await?;
        excluded.insert(scheduled.account.row.id);
        if let Err(error) = oauth::refresh_if_needed(&state, &mut scheduled.account).await {
            last_error = Some(error);
            continue;
        }

        let convert_chat = matches!(endpoint, Endpoint::ChatCompletions)
            && scheduled.account.row.platform == "openai"
            && scheduled.account.row.kind == "oauth";
        let upstream_endpoint = if convert_chat {
            Endpoint::Responses
        } else {
            endpoint
        };
        let upstream_value = if convert_chat {
            chat_to_responses(&upstream_request)?
        } else {
            upstream_request.clone()
        };
        let upstream_body = serde_json::to_vec(&upstream_value)
            .map_err(|_| ApiError::internal("request serialization failed"))?;

        let response = match send_upstream(
            &state,
            &scheduled.account,
            upstream_endpoint,
            &headers,
            Some(upstream_body),
        )
        .await
        {
            Ok(response)
                if response.status() == reqwest::StatusCode::UNAUTHORIZED
                    && scheduled.account.row.kind == "oauth" =>
            {
                scheduled.account.credentials.expires_at = Some(0);
                if oauth::refresh_account(&state, &mut scheduled.account)
                    .await
                    .is_ok()
                {
                    send_upstream(
                        &state,
                        &scheduled.account,
                        upstream_endpoint,
                        &headers,
                        Some(
                            serde_json::to_vec(&upstream_value)
                                .map_err(|_| ApiError::internal("request serialization failed"))?,
                        ),
                    )
                    .await
                } else {
                    Ok(response)
                }
            }
            other => other,
        };
        match response {
            Ok(response) if retryable_status(response.status()) && attempt + 1 < retry_attempts => {
                let status = response.status();
                cool_down_account(&state, scheduled.account.row.id, status, response.headers())
                    .await;
                last_error = Some(ApiError::new(
                    StatusCode::BAD_GATEWAY,
                    "UPSTREAM_RETRYABLE",
                    format!("upstream returned {status}"),
                ));
            }
            Ok(response) => {
                if response.status().is_success() {
                    clear_account_error(&state, scheduled.account.row.id).await;
                } else if retryable_status(response.status()) {
                    cool_down_account(
                        &state,
                        scheduled.account.row.id,
                        response.status(),
                        response.headers(),
                    )
                    .await;
                }
                return build_proxy_response(
                    state,
                    key,
                    scheduled,
                    endpoint,
                    model,
                    model_resolution
                        .as_ref()
                        .map(|resolution| resolution.billing.clone()),
                    model_resolution.as_ref().and_then(|resolution| {
                        (resolution.mapped != resolution.requested)
                            .then(|| resolution.mapped.clone())
                    }),
                    model_resolution
                        .as_ref()
                        .map(|resolution| resolution.mapping_chain.clone())
                        .unwrap_or_default(),
                    model_resolution
                        .as_ref()
                        .map(|resolution| resolution.billing_source.clone())
                        .unwrap_or_else(|| "requested".into()),
                    request_id,
                    started,
                    response,
                    convert_chat,
                    stream_requested,
                    service_tier,
                    active_request,
                    RequestTelemetry::with_attempts(upstream_attempts),
                )
                .await;
            }
            Err(error) => {
                mark_transport_error(&state, scheduled.account.row.id, &error).await;
                last_error = Some(error);
            }
        }
    }

    Err(last_error.unwrap_or_else(|| {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            "UPSTREAM_UNAVAILABLE",
            "all upstream accounts failed",
        )
    }))
}

fn filter_models(mut value: Value, allowed_models: &[String]) -> Value {
    if allowed_models.is_empty() {
        return value;
    }
    if let Some(models) = value.get_mut("data").and_then(Value::as_array_mut) {
        models.retain(|model| {
            model
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| allowed_models.iter().any(|allowed| allowed == id))
        });
    }
    value
}

#[allow(clippy::too_many_arguments)]
async fn build_proxy_response(
    state: AppState,
    key: ApiKeyContext,
    scheduled: ScheduledAccount,
    endpoint: Endpoint,
    model: Option<String>,
    billing_model: Option<String>,
    mapped_model: Option<String>,
    mapping_chain: String,
    billing_source: String,
    request_id: String,
    started: Instant,
    response: reqwest::Response,
    convert_chat: bool,
    stream_requested: bool,
    service_tier: Option<String>,
    active_request: crate::state::ActiveRequestGuard,
    telemetry: RequestTelemetry,
) -> ApiResult<Response> {
    let mut status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/json")
        .to_string();
    let is_stream = stream_requested || content_type.contains("text/event-stream");

    if is_stream && status.is_success() {
        let account_id = scheduled.account.row.id;
        let state_for_stream = state.clone();
        let request_id_for_stream = request_id.clone();
        let model_for_stream = model.clone();
        let billing_model_for_stream = billing_model.clone();
        let mapped_model_for_stream = mapped_model.clone();
        let mapping_chain_for_stream = mapping_chain.clone();
        let billing_source_for_stream = billing_source.clone();
        let mut upstream = response.bytes_stream();
        let stream = async_stream::stream! {
            let _active_request = active_request;
            let _permit = scheduled._permit;
            let mut parser = SseParser::default();
            let mut usage = Usage::default();
            let mut observed_upstream_model = None;
            let mut stream_error = None;
            let mut conversion = ChatStreamState::new(model_for_stream.clone());
            let mut telemetry = telemetry;
            while let Some(item) = upstream.next().await {
                match item {
                    Ok(bytes) => {
                        if telemetry.ttft_ms.is_none() && !bytes.is_empty() {
                            telemetry.ttft_ms = Some(started.elapsed().as_millis() as i64);
                        }
                        let events = parser.push(&bytes);
                        for event in &events {
                            usage.merge(extract_usage(event));
                            if let Some(model) = extract_response_model(event) {
                                observed_upstream_model = Some(model.to_string());
                            }
                        }
                        if convert_chat {
                            for event in events {
                                for output in conversion.convert(&event) {
                                    yield Ok::<Bytes, io::Error>(Bytes::from(output));
                                }
                            }
                        } else {
                            yield Ok::<Bytes, io::Error>(bytes);
                        }
                    }
                    Err(error) => {
                        stream_error = Some(error.to_string());
                        yield Err(io::Error::other("upstream stream ended unexpectedly"));
                        break;
                    }
                }
            }
            if convert_chat && !conversion.done {
                yield Ok::<Bytes, io::Error>(Bytes::from(conversion.finish()));
            }
            let effective_billing_model = if billing_source_for_stream == "upstream" {
                observed_upstream_model.clone().or(billing_model_for_stream)
            } else {
                billing_model_for_stream
            };
            let effective_chain = extend_mapping_chain(
                &mapping_chain_for_stream,
                model_for_stream.as_deref(),
                mapped_model_for_stream.as_deref(),
                observed_upstream_model.as_deref(),
            );
            let account_stats_model = observed_upstream_model
                .or_else(|| mapped_model_for_stream.clone())
                .or_else(|| model_for_stream.clone());
            log_usage(
                &state_for_stream,
                &request_id_for_stream,
                key.id,
                key.user_id,
                Some(account_id),
                endpoint,
                model_for_stream,
                effective_billing_model,
                mapped_model_for_stream,
                account_stats_model,
                effective_chain,
                status.as_u16() as i32,
                usage,
                true,
                service_tier,
                started.elapsed(),
                telemetry,
                stream_error,
            ).await;
        };
        let mut result = Response::new(Body::from_stream(stream));
        *result.status_mut() =
            StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
        result.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream; charset=utf-8"),
        );
        result
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
        result
            .headers_mut()
            .insert("x-request-id", HeaderValue::from_str(&request_id).unwrap());
        return Ok(result);
    }

    let mut bytes = response.bytes().await.unwrap_or_default().to_vec();
    drop(active_request);
    let mut skip_monitoring = false;
    if !status.is_success()
        && let Some(decision) =
            crate::error_passthrough::match_response(&state, status, &bytes).await?
    {
        status = decision.status;
        bytes = decision.body;
        skip_monitoring = decision.skip_monitoring;
    }
    let parsed = serde_json::from_slice::<Value>(&bytes).ok();
    let usage = parsed.as_ref().map(extract_usage).unwrap_or_default();
    let upstream_model = parsed
        .as_ref()
        .and_then(extract_response_model)
        .map(ToOwned::to_owned);
    let billing_model = if billing_source == "upstream" {
        upstream_model.clone().or(billing_model)
    } else {
        billing_model
    };
    let mapping_chain = extend_mapping_chain(
        &mapping_chain,
        model.as_deref(),
        mapped_model.as_deref(),
        upstream_model.as_deref(),
    );
    let account_stats_model = upstream_model
        .or_else(|| mapped_model.clone())
        .or_else(|| model.clone());
    let error_summary =
        (!status.is_success() && !skip_monitoring).then(|| safe_error_summary(&bytes));
    let output = if convert_chat && status.is_success() {
        serde_json::to_vec(&responses_to_chat(parsed.as_ref().ok_or_else(|| {
            ApiError::new(
                StatusCode::BAD_GATEWAY,
                "UPSTREAM_INVALID_JSON",
                "upstream returned invalid JSON",
            )
        })?))
        .map(Bytes::from)
        .map_err(|_| ApiError::internal("response serialization failed"))?
    } else {
        Bytes::from(bytes)
    };
    log_usage(
        &state,
        &request_id,
        key.id,
        key.user_id,
        Some(scheduled.account.row.id),
        endpoint,
        model,
        billing_model,
        mapped_model,
        account_stats_model,
        mapping_chain,
        status.as_u16() as i32,
        usage,
        is_stream,
        service_tier,
        started.elapsed(),
        telemetry,
        error_summary,
    )
    .await;
    let response_type = if convert_chat {
        "application/json"
    } else {
        &content_type
    };
    let mut result = raw_response(status, response_type, output)?;
    result
        .headers_mut()
        .insert("x-request-id", HeaderValue::from_str(&request_id).unwrap());
    Ok(result)
}

async fn send_upstream(
    state: &AppState,
    account: &Account,
    endpoint: Endpoint,
    incoming_headers: &HeaderMap,
    body: Option<Vec<u8>>,
) -> ApiResult<reqwest::Response> {
    let url = upstream_url(account, endpoint)?;
    let method = if matches!(endpoint, Endpoint::Models) {
        Method::GET
    } else {
        Method::POST
    };
    let client = state.client_for_account(account).await?;
    let mut request = client.request(method, url);
    for name in [
        "accept",
        "content-type",
        "user-agent",
        "anthropic-version",
        "anthropic-beta",
        "openai-beta",
        "originator",
        "session_id",
        "x-client-request-id",
    ] {
        if let Some(value) = incoming_headers.get(name) {
            request = request.header(name, value);
        }
    }
    request = request.header(header::CONTENT_TYPE, "application/json");
    if account.row.platform == "anthropic" {
        request = request.header(
            "anthropic-version",
            incoming_headers
                .get("anthropic-version")
                .and_then(|value| value.to_str().ok())
                .unwrap_or("2023-06-01"),
        );
        if account.row.kind == "oauth" {
            let token = account.credentials.access_token.as_deref().ok_or_else(|| {
                ApiError::new(
                    StatusCode::BAD_GATEWAY,
                    "OAUTH_ACCESS_TOKEN_MISSING",
                    "Claude OAuth access token is missing",
                )
            })?;
            request = request
                .bearer_auth(token)
                .header("anthropic-beta", claude_oauth_beta(incoming_headers))
                .header("user-agent", "claude-cli/2.1.161 (external, cli)")
                .header("x-app", "cli")
                .header("x-stainless-lang", "js")
                .header("x-stainless-package-version", "0.94.0")
                .header("anthropic-dangerous-direct-browser-access", "true");
        } else {
            let token = account.credentials.api_key.as_deref().ok_or_else(|| {
                ApiError::new(
                    StatusCode::BAD_GATEWAY,
                    "UPSTREAM_API_KEY_MISSING",
                    "Anthropic API key is missing",
                )
            })?;
            request = request.header("x-api-key", token);
        }
    } else if account.row.kind == "oauth" {
        let token = account.credentials.access_token.as_deref().ok_or_else(|| {
            ApiError::new(
                StatusCode::BAD_GATEWAY,
                "OAUTH_ACCESS_TOKEN_MISSING",
                "OAuth access token is missing",
            )
        })?;
        request = request.bearer_auth(token);
        if let Some(account_id) = account.credentials.chatgpt_account_id.as_deref() {
            request = request.header("chatgpt-account-id", account_id);
        }
        request = request.header("originator", "codex_cli_rs");
    } else {
        let token = account.credentials.api_key.as_deref().ok_or_else(|| {
            ApiError::new(
                StatusCode::BAD_GATEWAY,
                "UPSTREAM_API_KEY_MISSING",
                "upstream API key is missing",
            )
        })?;
        request = request.bearer_auth(token);
    }
    if let Some(body) = body {
        request = request.body(body);
    }
    request.send().await.map_err(ApiError::from)
}

fn upstream_url(account: &Account, endpoint: Endpoint) -> ApiResult<String> {
    let base = account.row.base_url.trim_end_matches('/');
    let suffix = if account.row.platform == "anthropic" {
        if base.ends_with("/v1") {
            endpoint.path().to_string()
        } else {
            format!("v1/{}", endpoint.path())
        }
    } else if account.row.kind == "oauth" {
        endpoint.path().to_string()
    } else if base.ends_with("/v1") {
        endpoint.path().to_string()
    } else {
        format!("v1/{}", endpoint.path())
    };
    let mut url = format!("{base}/{suffix}");
    if account.row.kind == "oauth" && matches!(endpoint, Endpoint::Models) {
        url.push_str("?client_version=0.1.0");
    }
    url::Url::parse(&url)
        .map(|url| url.to_string())
        .map_err(|_| ApiError::internal("stored upstream URL is invalid"))
}

fn claude_oauth_beta(headers: &HeaderMap) -> String {
    const REQUIRED: [&str; 3] = [
        "claude-code-20250219",
        "oauth-2025-04-20",
        "interleaved-thinking-2025-05-14",
    ];
    let incoming = headers
        .get("anthropic-beta")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let mut values = Vec::new();
    for value in REQUIRED
        .into_iter()
        .chain(incoming.split(',').map(str::trim))
    {
        if !value.is_empty() && !values.contains(&value) {
            values.push(value);
        }
    }
    values.join(",")
}

pub async fn probe_account(state: &AppState, account: &mut Account) -> ApiResult<Value> {
    oauth::refresh_if_needed(state, account).await?;
    let response = send_upstream(state, account, Endpoint::Models, &HeaderMap::new(), None).await?;
    let status = response.status();
    if !status.is_success() {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "ACCOUNT_TEST_FAILED",
            format!("upstream returned {status}"),
        ));
    }
    let value: Value = response.json().await?;
    Ok(json!({
        "ok": true,
        "models": normalize_models(value).get("data").and_then(Value::as_array).map(Vec::len).unwrap_or(0)
    }))
}

pub async fn probe_account_model(
    state: &AppState,
    account: &mut Account,
    model: &str,
) -> ApiResult<Value> {
    oauth::refresh_if_needed(state, account).await?;
    let (endpoint, payload) = if account.row.platform == "anthropic" {
        (
            Endpoint::Messages,
            json!({
                "model": model,
                "messages": [{"role": "user", "content": "Reply exactly with OK."}],
                "max_tokens": 16,
                "stream": false
            }),
        )
    } else {
        (
            Endpoint::Responses,
            json!({
                "model": model,
                "input": "Reply exactly with OK.",
                "max_output_tokens": 16,
                "store": false,
                "stream": false
            }),
        )
    };
    let body = serde_json::to_vec(&payload)
        .map_err(|_| ApiError::internal("account test serialization failed"))?;
    let response = send_upstream(state, account, endpoint, &HeaderMap::new(), Some(body)).await?;
    let status = response.status();
    if !status.is_success() {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "ACCOUNT_TEST_FAILED",
            format!("upstream returned {status}"),
        ));
    }
    response.json().await.map_err(ApiError::from)
}

fn retryable_status(status: reqwest::StatusCode) -> bool {
    matches!(status.as_u16(), 429 | 502 | 503 | 504)
}

async fn cool_down_account(
    state: &AppState,
    id: i64,
    status: reqwest::StatusCode,
    headers: &reqwest::header::HeaderMap,
) {
    let seconds = if status.as_u16() == 429 {
        let default_seconds = state.runtime_settings.read().await.cooldown_429_seconds;
        headers
            .get(header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(default_seconds)
            .clamp(1, 3600)
    } else {
        state.runtime_settings.read().await.cooldown_5xx_seconds
    };
    let until = Utc::now() + ChronoDuration::seconds(seconds);
    let _ = sqlx::query(
        "UPDATE accounts SET cooldown_until = ?, last_error = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
    )
    .bind(until.to_rfc3339())
    .bind(format!("upstream returned {status}"))
    .bind(id)
    .execute(&state.pool)
    .await;
}

async fn mark_transport_error(state: &AppState, id: i64, error: &ApiError) {
    let seconds = state.runtime_settings.read().await.cooldown_5xx_seconds;
    let until = Utc::now() + ChronoDuration::seconds(seconds);
    let _ = sqlx::query(
        "UPDATE accounts SET cooldown_until = ?, last_error = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
    )
    .bind(until.to_rfc3339())
    .bind(error.message.chars().take(240).collect::<String>())
    .bind(id)
    .execute(&state.pool)
    .await;
}

async fn clear_account_error(state: &AppState, id: i64) {
    let _ = sqlx::query(
        "UPDATE accounts SET last_used_at = CURRENT_TIMESTAMP, last_error = NULL, cooldown_until = NULL, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
    )
    .bind(id)
    .execute(&state.pool)
    .await;
}

#[derive(Debug, Clone, Copy, Default)]
struct Usage {
    input: Option<i64>,
    output: Option<i64>,
    total: Option<i64>,
    cached: Option<i64>,
    cache_write: Option<i64>,
    image_input: Option<i64>,
    image_output: Option<i64>,
    reasoning: Option<i64>,
}

impl Usage {
    fn merge(&mut self, other: Usage) {
        if other.input.is_some() {
            self.input = other.input;
        }
        if other.output.is_some() {
            self.output = other.output;
        }
        if other.total.is_some() {
            self.total = other.total;
        }
        if other.cached.is_some() {
            self.cached = other.cached;
        }
        if other.cache_write.is_some() {
            self.cache_write = other.cache_write;
        }
        if other.image_input.is_some() {
            self.image_input = other.image_input;
        }
        if other.image_output.is_some() {
            self.image_output = other.image_output;
        }
        if other.reasoning.is_some() {
            self.reasoning = other.reasoning;
        }
    }
}

fn extract_usage(value: &Value) -> Usage {
    let value = value
        .get("usage")
        .or_else(|| value.get("response").and_then(|value| value.get("usage")))
        .unwrap_or(value);
    let input = value
        .get("input_tokens")
        .or_else(|| value.get("prompt_tokens"))
        .and_then(Value::as_i64);
    let output = value
        .get("output_tokens")
        .or_else(|| value.get("completion_tokens"))
        .and_then(Value::as_i64);
    let total = value
        .get("total_tokens")
        .and_then(Value::as_i64)
        .or_else(|| match (input, output) {
            (Some(input), Some(output)) => Some(input + output),
            _ => None,
        });
    let cached = value
        .get("input_tokens_details")
        .or_else(|| value.get("prompt_tokens_details"))
        .and_then(|details| details.get("cached_tokens"))
        .or_else(|| value.get("cached_tokens"))
        .or_else(|| value.get("cache_read_input_tokens"))
        .and_then(Value::as_i64);
    let cache_write = value
        .get("input_tokens_details")
        .or_else(|| value.get("prompt_tokens_details"))
        .and_then(|details| {
            details
                .get("cache_write_tokens")
                .or_else(|| details.get("cache_creation_tokens"))
        })
        .or_else(|| value.get("cache_write_tokens"))
        .or_else(|| value.get("cache_creation_input_tokens"))
        .and_then(Value::as_i64);
    let image_input = value
        .get("input_tokens_details")
        .or_else(|| value.get("prompt_tokens_details"))
        .and_then(|details| details.get("image_tokens"))
        .or_else(|| value.get("image_input_tokens"))
        .and_then(Value::as_i64);
    let image_output = value
        .get("output_tokens_details")
        .or_else(|| value.get("completion_tokens_details"))
        .and_then(|details| details.get("image_tokens"))
        .or_else(|| value.get("image_output_tokens"))
        .and_then(Value::as_i64);
    let reasoning = value
        .get("output_tokens_details")
        .or_else(|| value.get("completion_tokens_details"))
        .and_then(|details| details.get("reasoning_tokens"))
        .or_else(|| value.get("reasoning_tokens"))
        .and_then(Value::as_i64);
    Usage {
        input,
        output,
        total,
        cached,
        cache_write,
        image_input,
        image_output,
        reasoning,
    }
}

fn extract_response_model(value: &Value) -> Option<&str> {
    value
        .get("model")
        .or_else(|| {
            value
                .get("response")
                .and_then(|response| response.get("model"))
        })
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|model| !model.is_empty())
}

fn extend_mapping_chain(
    chain: &str,
    requested: Option<&str>,
    mapped: Option<&str>,
    upstream: Option<&str>,
) -> String {
    let Some(upstream) = upstream else {
        return chain.to_string();
    };
    let previous = mapped.or(requested);
    if previous == Some(upstream) {
        return chain.to_string();
    }
    if chain.is_empty() {
        requested
            .filter(|requested| *requested != upstream)
            .map(|requested| format!("{requested}->{upstream}"))
            .unwrap_or_default()
    } else {
        format!("{chain}->{upstream}")
    }
}

#[derive(Clone, Copy, Debug)]
struct RequestTelemetry {
    ttft_ms: Option<i64>,
    upstream_attempts: i64,
}

impl RequestTelemetry {
    fn with_attempts(upstream_attempts: i64) -> Self {
        Self {
            ttft_ms: None,
            upstream_attempts,
        }
    }

    fn account_switches(self) -> i64 {
        self.upstream_attempts.saturating_sub(1)
    }
}

#[allow(clippy::too_many_arguments)]
async fn log_usage(
    state: &AppState,
    request_id: &str,
    api_key_id: i64,
    api_key_user_id: Option<i64>,
    account_id: Option<i64>,
    endpoint: Endpoint,
    model: Option<String>,
    billing_model: Option<String>,
    mapped_model: Option<String>,
    account_stats_model: Option<String>,
    mapping_chain: String,
    status_code: i32,
    usage: Usage,
    stream: bool,
    service_tier: Option<String>,
    duration: Duration,
    telemetry: RequestTelemetry,
    error_summary: Option<String>,
) {
    let costs = if status_code < 400 {
        calculate_cost_breakdown(
            state,
            api_key_id,
            billing_model.as_deref().or(model.as_deref()),
            usage,
        )
        .await
        .unwrap_or_default()
    } else {
        CostBreakdown::default()
    };
    let account_cost_microusd = if status_code < 400 {
        match account_id {
            Some(account_id) => calculate_account_stats_cost(
                state,
                api_key_id,
                account_id,
                account_stats_model.as_deref(),
                usage,
                costs,
            )
            .await
            .unwrap_or(costs.billed),
            None => 0,
        }
    } else {
        0
    };
    if let Err(error) = sqlx::query(
        "INSERT INTO usage_logs (request_id, api_key_id, account_id, user_id, endpoint, model, status_code, \
         input_tokens, output_tokens, total_tokens, cached_input_tokens, cache_write_tokens, \
         image_input_tokens, image_output_tokens, reasoning_tokens, billing_model, mapped_model, \
         model_mapping_chain, request_type, stream, service_tier, cost_microusd, \
         account_cost_microusd, duration_ms, ttft_ms, upstream_attempts, account_switches, error_summary) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(request_id)
    .bind(api_key_id)
    .bind(account_id)
    .bind(api_key_user_id)
    .bind(endpoint.log_name())
    .bind(model)
    .bind(status_code)
    .bind(usage.input)
    .bind(usage.output)
    .bind(usage.total)
    .bind(usage.cached.unwrap_or(0).max(0))
    .bind(usage.cache_write.unwrap_or(0).max(0))
    .bind(usage.image_input.unwrap_or(0).max(0))
    .bind(usage.image_output.unwrap_or(0).max(0))
    .bind(usage.reasoning.unwrap_or(0).max(0))
    .bind(billing_model)
    .bind(mapped_model)
    .bind(mapping_chain)
    .bind(if stream { "stream" } else { "sync" })
    .bind(stream)
    .bind(service_tier)
    .bind(costs.billed)
    .bind(account_cost_microusd)
    .bind(duration.as_millis() as i64)
    .bind(telemetry.ttft_ms)
    .bind(telemetry.upstream_attempts)
    .bind(telemetry.account_switches())
    .bind(error_summary.map(|value| value.chars().take(500).collect::<String>()))
    .execute(&state.pool)
    .await
    {
        tracing::warn!(%error, "failed to write usage log");
    }
}

async fn channel_model_allowed(state: &AppState, group_id: i64, model: &str) -> ApiResult<bool> {
    let channel: Option<(i64, bool)> = sqlx::query_as(
        "SELECT channels.id, channels.restrict_models FROM channels \
         JOIN channel_groups ON channel_groups.channel_id = channels.id \
         WHERE channel_groups.group_id = ? AND channels.status = 'active'",
    )
    .bind(group_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some((channel_id, restricted)) = channel else {
        return Ok(true);
    };
    if !restricted {
        return Ok(true);
    }
    let exists: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM channel_model_pricing WHERE channel_id = ? \
         AND EXISTS (SELECT 1 FROM json_each(channel_model_pricing.models) WHERE value = ? \
           OR (substr(value, -1) = '*' AND lower(?) LIKE \
             lower(substr(value, 1, length(value) - 1)) || '%'))",
    )
    .bind(channel_id)
    .bind(model)
    .bind(model)
    .fetch_one(&state.pool)
    .await?;
    Ok(exists > 0)
}

#[cfg(test)]
async fn calculate_cost(
    state: &AppState,
    api_key_id: i64,
    model: Option<&str>,
    usage: Usage,
) -> ApiResult<i64> {
    calculate_cost_at(state, api_key_id, model, usage, Utc::now()).await
}

#[cfg(test)]
async fn calculate_cost_at(
    state: &AppState,
    api_key_id: i64,
    model: Option<&str>,
    usage: Usage,
    now: chrono::DateTime<Utc>,
) -> ApiResult<i64> {
    Ok(
        calculate_cost_breakdown_at(state, api_key_id, model, usage, now)
            .await?
            .billed,
    )
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct CostBreakdown {
    base: i64,
    billed: i64,
}

async fn calculate_cost_breakdown(
    state: &AppState,
    api_key_id: i64,
    model: Option<&str>,
    usage: Usage,
) -> ApiResult<CostBreakdown> {
    calculate_cost_breakdown_at(state, api_key_id, model, usage, Utc::now()).await
}

async fn calculate_cost_breakdown_at(
    state: &AppState,
    api_key_id: i64,
    model: Option<&str>,
    usage: Usage,
    now: chrono::DateTime<Utc>,
) -> ApiResult<CostBreakdown> {
    let Some(model) = model else {
        return Ok(CostBreakdown::default());
    };
    let rows: Vec<(
        i64,
        String,
        i64,
        i64,
        i64,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<String>,
        Option<bool>,
        Option<String>,
        Option<String>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
    )> = sqlx::query_as(
        "SELECT pricing.id, pricing.billing_mode, pricing.input_microusd_per_million, \
         pricing.output_microusd_per_million, pricing.per_request_microusd, \
         pricing.cache_read_microusd_per_million, pricing.cache_write_microusd_per_million, \
         groups.rate_multiplier_micros, groups.subscription_type, groups.peak_rate_enabled, \
         groups.peak_start, groups.peak_end, groups.peak_rate_multiplier_micros, \
         rates.rate_multiplier_micros, pricing.image_input_microusd_per_million, \
         pricing.image_output_microusd_per_million \
         FROM channel_model_pricing AS pricing \
         JOIN channels ON channels.id = pricing.channel_id AND channels.status = 'active' \
         LEFT JOIN channel_groups ON channel_groups.channel_id = channels.id \
         JOIN api_keys ON api_keys.id = ? \
         LEFT JOIN groups ON groups.id = api_keys.group_id \
         LEFT JOIN user_group_rate_multipliers rates ON rates.group_id = groups.id \
         AND rates.user_id = api_keys.user_id \
         WHERE EXISTS (SELECT 1 FROM json_each(pricing.models) WHERE value = ? \
           OR (substr(value, -1) = '*' AND lower(?) LIKE \
             lower(substr(value, 1, length(value) - 1)) || '%')) \
         AND ((api_keys.group_id IS NOT NULL AND channel_groups.group_id = api_keys.group_id) \
         OR api_keys.group_id IS NULL) ORDER BY pricing.id LIMIT 2",
    )
    .bind(api_key_id)
    .bind(model)
    .bind(model)
    .fetch_all(&state.pool)
    .await?;
    if rows.len() != 1 {
        return Ok(CostBreakdown::default());
    }
    let row = &rows[0];
    let resolved_rate = row
        .13
        .or(row.7)
        .unwrap_or(1_000_000)
        .clamp(1, 1_000_000_000);
    let (offset_minutes, _) = groups::server_utc_offset();
    let (_, effective_rate) = groups::effective_rate_micros_at(
        row.7.unwrap_or(1_000_000),
        row.13,
        row.8.as_deref().unwrap_or("standard"),
        row.9.unwrap_or(false),
        row.10.as_deref().unwrap_or(""),
        row.11.as_deref().unwrap_or(""),
        row.12.unwrap_or(1_000_000),
        now,
        offset_minutes,
    );
    if row.1 == "request" {
        let billed = row.4 as i128 * resolved_rate as i128 / 1_000_000;
        return Ok(CostBreakdown {
            base: row.4,
            billed: billed.clamp(0, i64::MAX as i128) as i64,
        });
    }
    let input = usage.input.unwrap_or(0).max(0) as i128;
    let output = usage.output.unwrap_or(0).max(0) as i128;
    let total = usage
        .total
        .unwrap_or_else(|| (input + output).clamp(0, i64::MAX as i128) as i64)
        .max(0);
    let interval: Option<(Option<i64>, Option<i64>, Option<i64>, Option<i64>)> = sqlx::query_as(
        "SELECT input_microusd_per_million, output_microusd_per_million, \
         cache_read_microusd_per_million, cache_write_microusd_per_million \
         FROM channel_pricing_intervals \
         WHERE pricing_id = ? AND ? > min_tokens \
         AND (max_tokens IS NULL OR ? <= max_tokens) ORDER BY min_tokens LIMIT 1",
    )
    .bind(row.0)
    .bind(total)
    .bind(total)
    .fetch_optional(&state.pool)
    .await?;
    let input_price = interval.as_ref().and_then(|value| value.0).unwrap_or(row.2);
    let output_price = interval.as_ref().and_then(|value| value.1).unwrap_or(row.3);
    let cache_read_price = interval
        .as_ref()
        .and_then(|value| value.2)
        .or(row.5)
        .unwrap_or(input_price);
    let cache_write_price = interval
        .as_ref()
        .and_then(|value| value.3)
        .or(row.6)
        .unwrap_or(input_price);
    let image_input_price = row.14.unwrap_or(input_price);
    let image_output_price = row.15.unwrap_or(output_price);
    let cached = (usage.cached.unwrap_or(0).max(0) as i128).min(input);
    let remaining_input = input - cached;
    let cache_write = (usage.cache_write.unwrap_or(0).max(0) as i128).min(remaining_input);
    let remaining_input = remaining_input - cache_write;
    let image_input = (usage.image_input.unwrap_or(0).max(0) as i128).min(remaining_input);
    let ordinary_input = remaining_input - image_input;
    let image_output = (usage.image_output.unwrap_or(0).max(0) as i128).min(output);
    let ordinary_output = output - image_output;
    let numerator = ordinary_input * input_price as i128
        + cached * cache_read_price as i128
        + cache_write * cache_write_price as i128
        + image_input * image_input_price as i128
        + ordinary_output * output_price as i128
        + image_output * image_output_price as i128;
    let base = numerator / 1_000_000_i128;
    let billed = numerator * effective_rate as i128 / 1_000_000_000_000_i128;
    Ok(CostBreakdown {
        base: base.clamp(0, i64::MAX as i128) as i64,
        billed: billed.clamp(0, i64::MAX as i128) as i64,
    })
}

async fn calculate_account_stats_cost(
    state: &AppState,
    api_key_id: i64,
    account_id: i64,
    model: Option<&str>,
    usage: Usage,
    customer_cost: CostBreakdown,
) -> ApiResult<i64> {
    let channel: Option<(i64, bool, String, i64)> = sqlx::query_as(
        "SELECT channels.id, channels.apply_pricing_to_account_stats, groups.platform, groups.id \
         FROM api_keys JOIN groups ON groups.id = api_keys.group_id \
         JOIN channel_groups ON channel_groups.group_id = groups.id \
         JOIN channels ON channels.id = channel_groups.channel_id AND channels.status = 'active' \
         WHERE api_keys.id = ?",
    )
    .bind(api_key_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some((channel_id, apply_channel_pricing, platform, group_id)) = channel else {
        return Ok(customer_cost.billed);
    };
    let fallback = if apply_channel_pricing && customer_cost.base > 0 {
        customer_cost.base
    } else {
        customer_cost.billed
    };
    let Some(model) = model else {
        return Ok(fallback);
    };
    let row: Option<(
        i64,
        String,
        i64,
        i64,
        i64,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
    )> = sqlx::query_as(
        "SELECT pricing.id, pricing.billing_mode, pricing.input_microusd_per_million, \
         pricing.output_microusd_per_million, pricing.per_request_microusd, \
         pricing.cache_read_microusd_per_million, pricing.cache_write_microusd_per_million, \
         pricing.image_input_microusd_per_million, pricing.image_output_microusd_per_million \
         FROM channel_account_stats_rules AS rules \
         JOIN channel_account_stats_pricing AS pricing ON pricing.rule_id = rules.id \
         WHERE rules.channel_id = ? AND ( \
           EXISTS (SELECT 1 FROM channel_account_stats_rule_accounts scoped_accounts \
             WHERE scoped_accounts.rule_id = rules.id AND scoped_accounts.account_id = ?) OR \
           EXISTS (SELECT 1 FROM channel_account_stats_rule_groups scoped_groups \
             WHERE scoped_groups.rule_id = rules.id AND scoped_groups.group_id = ?)) \
         AND lower(pricing.platform) = lower(?) \
         AND EXISTS (SELECT 1 FROM json_each(pricing.models) WHERE lower(value) = lower(?) \
           OR (substr(value, -1) = '*' AND lower(?) LIKE \
             lower(substr(value, 1, length(value) - 1)) || '%')) \
         ORDER BY rules.sort_order, rules.id, \
           CASE WHEN EXISTS (SELECT 1 FROM json_each(pricing.models) \
             WHERE lower(value) = lower(?)) THEN 0 ELSE 1 END, pricing.id LIMIT 1",
    )
    .bind(channel_id)
    .bind(account_id)
    .bind(group_id)
    .bind(&platform)
    .bind(model)
    .bind(model)
    .bind(model)
    .fetch_optional(&state.pool)
    .await?;
    let Some(row) = row else {
        return Ok(fallback);
    };
    if row.1 == "request" {
        return Ok(if row.4 > 0 { row.4 } else { fallback });
    }
    let input = usage.input.unwrap_or(0).max(0) as i128;
    let output = usage.output.unwrap_or(0).max(0) as i128;
    let total = usage
        .total
        .unwrap_or_else(|| (input + output).clamp(0, i64::MAX as i128) as i64)
        .max(0);
    let interval: Option<(Option<i64>, Option<i64>, Option<i64>, Option<i64>)> = sqlx::query_as(
        "SELECT input_microusd_per_million, output_microusd_per_million, \
             cache_read_microusd_per_million, cache_write_microusd_per_million \
             FROM channel_account_stats_intervals WHERE pricing_id = ? AND ? > min_tokens \
             AND (max_tokens IS NULL OR ? <= max_tokens) ORDER BY min_tokens, sort_order LIMIT 1",
    )
    .bind(row.0)
    .bind(total)
    .bind(total)
    .fetch_optional(&state.pool)
    .await?;
    let input_price = interval.as_ref().and_then(|value| value.0).unwrap_or(row.2);
    let output_price = interval.as_ref().and_then(|value| value.1).unwrap_or(row.3);
    let cache_read_price = interval
        .as_ref()
        .and_then(|value| value.2)
        .or(row.5)
        .unwrap_or(input_price);
    let cache_write_price = interval
        .as_ref()
        .and_then(|value| value.3)
        .or(row.6)
        .unwrap_or(input_price);
    let image_input_price = row.7.unwrap_or(input_price);
    let image_output_price = row.8.unwrap_or(output_price);
    let cached = (usage.cached.unwrap_or(0).max(0) as i128).min(input);
    let remaining_input = input - cached;
    let cache_write = (usage.cache_write.unwrap_or(0).max(0) as i128).min(remaining_input);
    let remaining_input = remaining_input - cache_write;
    let image_input = (usage.image_input.unwrap_or(0).max(0) as i128).min(remaining_input);
    let ordinary_input = remaining_input - image_input;
    let image_output = (usage.image_output.unwrap_or(0).max(0) as i128).min(output);
    let ordinary_output = output - image_output;
    let cost = (ordinary_input * input_price as i128
        + cached * cache_read_price as i128
        + cache_write * cache_write_price as i128
        + image_input * image_input_price as i128
        + ordinary_output * output_price as i128
        + image_output * image_output_price as i128)
        / 1_000_000_i128;
    let cost = cost.clamp(0, i64::MAX as i128) as i64;
    Ok(if cost > 0 { cost } else { fallback })
}

fn chat_to_responses(chat: &Value) -> ApiResult<Value> {
    if chat
        .get("n")
        .and_then(Value::as_i64)
        .is_some_and(|value| value != 1)
        || chat.get("logprobs").is_some()
        || chat.get("functions").is_some()
    {
        return Err(ApiError::bad_request(
            "UNSUPPORTED_CHAT_PARAMETER",
            "OAuth Chat adapter does not support n != 1, logprobs, or legacy functions",
        ));
    }
    let messages = chat
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| ApiError::bad_request("MESSAGES_REQUIRED", "messages is required"))?;
    let mut instructions = Vec::new();
    let mut input = Vec::new();
    for message in messages {
        let role = message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("user");
        if matches!(role, "system" | "developer") {
            if let Some(text) = content_text(message.get("content")) {
                instructions.push(text);
            }
            continue;
        }
        if role == "tool" {
            input.push(json!({
                "type": "function_call_output",
                "call_id": message.get("tool_call_id").and_then(Value::as_str).unwrap_or("call"),
                "output": content_text(message.get("content")).unwrap_or_default()
            }));
            continue;
        }
        input.push(json!({"role": role, "content": message.get("content").cloned().unwrap_or(Value::String(String::new()))}));
        if let Some(tool_calls) = message.get("tool_calls").and_then(Value::as_array) {
            for call in tool_calls {
                if let Some(function) = call.get("function") {
                    input.push(json!({
                        "type": "function_call",
                        "call_id": call.get("id").and_then(Value::as_str).unwrap_or("call"),
                        "name": function.get("name").and_then(Value::as_str).unwrap_or("tool"),
                        "arguments": function.get("arguments").and_then(Value::as_str).unwrap_or("{}")
                    }));
                }
            }
        }
    }

    let mut output = Map::new();
    output.insert(
        "model".into(),
        chat.get("model")
            .cloned()
            .unwrap_or(Value::String("gpt-5".into())),
    );
    output.insert("input".into(), Value::Array(input));
    output.insert(
        "stream".into(),
        chat.get("stream").cloned().unwrap_or(Value::Bool(false)),
    );
    output.insert("store".into(), Value::Bool(false));
    if !instructions.is_empty() {
        output.insert(
            "instructions".into(),
            Value::String(instructions.join("\n\n")),
        );
    }
    if let Some(max_tokens) = chat
        .get("max_completion_tokens")
        .or_else(|| chat.get("max_tokens"))
    {
        output.insert("max_output_tokens".into(), max_tokens.clone());
    }
    if let Some(effort) = chat.get("reasoning_effort") {
        output.insert("reasoning".into(), json!({"effort": effort}));
    }
    if let Some(tools) = chat.get("tools").and_then(Value::as_array) {
        let converted = tools.iter().filter_map(|tool| {
            let function = tool.get("function")?;
            Some(json!({
                "type": "function",
                "name": function.get("name")?,
                "description": function.get("description").cloned().unwrap_or(Value::String(String::new())),
                "parameters": function.get("parameters").cloned().unwrap_or_else(|| json!({"type":"object","properties":{}}))
            }))
        }).collect();
        output.insert("tools".into(), Value::Array(converted));
    }
    if let Some(choice) = chat.get("tool_choice") {
        let converted = if let Some(function) = choice.get("function") {
            json!({"type":"function", "name": function.get("name").cloned().unwrap_or(Value::String(String::new()))})
        } else {
            choice.clone()
        };
        output.insert("tool_choice".into(), converted);
    }
    Ok(Value::Object(output))
}

fn responses_to_chat(response: &Value) -> Value {
    let mut content = String::new();
    let mut tool_calls = Vec::new();
    if let Some(output) = response.get("output").and_then(Value::as_array) {
        for item in output {
            match item.get("type").and_then(Value::as_str) {
                Some("message") => {
                    if let Some(parts) = item.get("content").and_then(Value::as_array) {
                        for part in parts {
                            if matches!(part.get("type").and_then(Value::as_str), Some("output_text") | Some("text")) {
                                if let Some(text) = part.get("text").and_then(Value::as_str) { content.push_str(text); }
                            }
                        }
                    }
                }
                Some("function_call") => tool_calls.push(json!({
                    "id": item.get("call_id").or_else(|| item.get("id")).cloned().unwrap_or(Value::String("call".into())),
                    "type": "function",
                    "function": {
                        "name": item.get("name").cloned().unwrap_or(Value::String("tool".into())),
                        "arguments": item.get("arguments").cloned().unwrap_or(Value::String("{}".into()))
                    }
                })),
                _ => {}
            }
        }
    }
    let mut message = json!({"role":"assistant", "content": content});
    if !tool_calls.is_empty() {
        message["tool_calls"] = Value::Array(tool_calls);
    }
    let finish_reason = if message.get("tool_calls").is_some() {
        "tool_calls"
    } else {
        "stop"
    };
    let usage = extract_usage(response);
    json!({
        "id": response.get("id").cloned().unwrap_or(Value::String(format!("chatcmpl-{}", Uuid::new_v4()))),
        "object": "chat.completion",
        "created": response.get("created_at").and_then(Value::as_i64).unwrap_or_else(|| Utc::now().timestamp()),
        "model": response.get("model").cloned().unwrap_or(Value::String("unknown".into())),
        "choices": [{"index":0, "message":message, "finish_reason":finish_reason}],
        "usage": {"prompt_tokens":usage.input, "completion_tokens":usage.output, "total_tokens":usage.total}
    })
}

fn content_text(content: Option<&Value>) -> Option<String> {
    match content? {
        Value::String(value) => Some(value.clone()),
        Value::Array(parts) => Some(
            parts
                .iter()
                .filter_map(|part| {
                    part.get("text")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned)
                })
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        _ => None,
    }
}

#[derive(Default)]
struct SseParser {
    buffer: Vec<u8>,
}

impl SseParser {
    fn push(&mut self, bytes: &[u8]) -> Vec<Value> {
        self.buffer.extend_from_slice(bytes);
        let mut values = Vec::new();
        while let Some(position) = self.buffer.windows(2).position(|window| window == b"\n\n") {
            let event: Vec<u8> = self.buffer.drain(..position + 2).collect();
            let text = String::from_utf8_lossy(&event);
            for line in text.lines() {
                if let Some(data) = line.strip_prefix("data: ") {
                    if data != "[DONE]" {
                        if let Ok(value) = serde_json::from_str(data) {
                            values.push(value);
                        }
                    }
                }
            }
        }
        values
    }
}

struct ChatStreamState {
    id: String,
    model: String,
    created: i64,
    role_sent: bool,
    done: bool,
}

impl ChatStreamState {
    fn new(model: Option<String>) -> Self {
        Self {
            id: format!("chatcmpl-{}", Uuid::new_v4()),
            model: model.unwrap_or_else(|| "unknown".into()),
            created: Utc::now().timestamp(),
            role_sent: false,
            done: false,
        }
    }

    fn chunk(&self, delta: Value, finish_reason: Value) -> String {
        format!(
            "data: {}\n\n",
            json!({
                "id": self.id, "object":"chat.completion.chunk", "created":self.created,
                "model":self.model, "choices":[{"index":0,"delta":delta,"finish_reason":finish_reason}]
            })
        )
    }

    fn convert(&mut self, event: &Value) -> Vec<String> {
        let mut output = Vec::new();
        if let Some(response) = event.get("response") {
            if let Some(id) = response.get("id").and_then(Value::as_str) {
                self.id = id.into();
            }
            if let Some(model) = response.get("model").and_then(Value::as_str) {
                self.model = model.into();
            }
            if let Some(created) = response.get("created_at").and_then(Value::as_i64) {
                self.created = created;
            }
        }
        if !self.role_sent {
            output.push(self.chunk(json!({"role":"assistant"}), Value::Null));
            self.role_sent = true;
        }
        match event.get("type").and_then(Value::as_str) {
            Some("response.output_text.delta") => {
                if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                    output.push(self.chunk(json!({"content":delta}), Value::Null));
                }
            }
            Some("response.output_item.added")
                if event
                    .get("item")
                    .and_then(|v| v.get("type"))
                    .and_then(Value::as_str)
                    == Some("function_call") =>
            {
                let item = &event["item"];
                output.push(self.chunk(json!({"tool_calls":[{
                    "index":event.get("output_index").and_then(Value::as_i64).unwrap_or(0),
                    "id":item.get("call_id").or_else(|| item.get("id")).cloned().unwrap_or(Value::String("call".into())),
                    "type":"function", "function":{"name":item.get("name").cloned().unwrap_or(Value::String("tool".into())),"arguments":""}
                }]}), Value::Null));
            }
            Some("response.function_call_arguments.delta") => {
                output.push(self.chunk(json!({"tool_calls":[{
                    "index":event.get("output_index").and_then(Value::as_i64).unwrap_or(0),
                    "function":{"arguments":event.get("delta").cloned().unwrap_or(Value::String(String::new()))}
                }]}), Value::Null));
            }
            Some("response.completed") => {
                output.push(self.chunk(
                    Map::<String, Value>::new().into(),
                    Value::String("stop".into()),
                ));
                output.push("data: [DONE]\n\n".into());
                self.done = true;
            }
            _ => {}
        }
        output
    }

    fn finish(&mut self) -> String {
        self.done = true;
        format!(
            "{}data: [DONE]\n\n",
            self.chunk(
                Map::<String, Value>::new().into(),
                Value::String("stop".into())
            )
        )
    }
}

fn normalize_models(value: Value) -> Value {
    if value.get("data").and_then(Value::as_array).is_some() {
        return value;
    }
    let models = value
        .get("models")
        .and_then(Value::as_array)
        .cloned()
        .or_else(|| value.as_array().cloned())
        .unwrap_or_default();
    let data = models
        .into_iter()
        .filter_map(|model| {
            let id = model
                .get("id")
                .or_else(|| model.get("slug"))
                .or_else(|| model.get("model"))
                .or_else(|| model.get("name"))
                .and_then(Value::as_str)?;
            Some(json!({"id":id, "object":"model", "owned_by":"upstream"}))
        })
        .collect::<Vec<_>>();
    json!({"object":"list", "data":data})
}

fn safe_error_summary(bytes: &[u8]) -> String {
    if let Ok(value) = serde_json::from_slice::<Value>(bytes) {
        if let Some(message) = value
            .pointer("/error/message")
            .or_else(|| value.get("message"))
            .and_then(Value::as_str)
        {
            return message.chars().take(300).collect();
        }
    }
    "upstream request failed".into()
}

fn json_response(status: StatusCode, value: Value) -> ApiResult<Response> {
    let bytes = serde_json::to_vec(&value)
        .map_err(|_| ApiError::internal("response serialization failed"))?;
    raw_response(
        reqwest::StatusCode::from_u16(status.as_u16()).unwrap(),
        "application/json",
        Bytes::from(bytes),
    )
}

fn raw_response(
    status: reqwest::StatusCode,
    content_type: &str,
    body: impl Into<Body>,
) -> ApiResult<Response> {
    let mut response = Response::new(body.into());
    *response.status_mut() =
        StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(content_type)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, body::to_bytes, http::Request, response::IntoResponse};
    use chrono::TimeZone;
    use tempfile::TempDir;
    use tower::ServiceExt;

    use crate::{config::Config, crypto::Crypto, db, models::Credentials};

    #[test]
    fn converts_chat_request_to_responses() {
        let input = json!({
            "model":"gpt-5", "stream":false,
            "messages":[{"role":"system","content":"Be concise"},{"role":"user","content":"hello"}],
            "tools":[{"type":"function","function":{"name":"lookup","parameters":{"type":"object"}}}]
        });
        let output = chat_to_responses(&input).unwrap();
        assert_eq!(output["instructions"], "Be concise");
        assert_eq!(output["input"][0]["role"], "user");
        assert_eq!(output["tools"][0]["name"], "lookup");
    }

    #[test]
    fn converts_responses_result_to_chat() {
        let input = json!({
            "id":"resp_1", "model":"gpt-5",
            "output":[{"type":"message","content":[{"type":"output_text","text":"hello"}]}],
            "usage":{"input_tokens":2,"output_tokens":1,"total_tokens":3}
        });
        let output = responses_to_chat(&input);
        assert_eq!(output["choices"][0]["message"]["content"], "hello");
        assert_eq!(output["usage"]["total_tokens"], 3);
    }

    #[test]
    fn extracts_cached_and_reasoning_tokens_from_openai_usage_shapes() {
        let responses = json!({
            "usage": {
                "input_tokens": 20,
                "output_tokens": 8,
                "total_tokens": 28,
                "input_tokens_details": {"cached_tokens": 7, "cache_write_tokens": 4, "image_tokens": 2},
                "output_tokens_details": {"reasoning_tokens": 3, "image_tokens": 1}
            }
        });
        let usage = extract_usage(&responses);
        assert_eq!(usage.cached, Some(7));
        assert_eq!(usage.cache_write, Some(4));
        assert_eq!(usage.image_input, Some(2));
        assert_eq!(usage.image_output, Some(1));
        assert_eq!(usage.reasoning, Some(3));

        let chat = json!({
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 4,
                "prompt_tokens_details": {"cached_tokens": 6},
                "completion_tokens_details": {"reasoning_tokens": 2}
            }
        });
        let usage = extract_usage(&chat);
        assert_eq!(usage.total, Some(14));
        assert_eq!(usage.cached, Some(6));
        assert_eq!(usage.reasoning, Some(2));

        let anthropic = json!({
            "usage": {"input_tokens":12,"output_tokens":5,
                "cache_read_input_tokens":3,"cache_creation_input_tokens":4}
        });
        let usage = extract_usage(&anthropic);
        assert_eq!(usage.cached, Some(3));
        assert_eq!(usage.cache_write, Some(4));
    }

    #[test]
    fn parses_split_sse_events() {
        let mut parser = SseParser::default();
        assert!(
            parser
                .push(b"data: {\"type\":\"response.output_")
                .is_empty()
        );
        let events = parser.push(b"text.delta\",\"delta\":\"hi\"}\n\n");
        assert_eq!(events[0]["delta"], "hi");
    }

    #[test]
    fn claude_oauth_beta_keeps_required_values_and_deduplicates_client_values() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "anthropic-beta",
            HeaderValue::from_static("custom-beta,oauth-2025-04-20"),
        );
        assert_eq!(
            claude_oauth_beta(&headers),
            "claude-code-20250219,oauth-2025-04-20,interleaved-thinking-2025-05-14,custom-beta"
        );
    }

    #[test]
    fn rejects_unsupported_chat_parameters() {
        assert!(chat_to_responses(&json!({"messages":[],"n":2})).is_err());
    }

    #[tokio::test]
    async fn retries_429_on_a_second_account_and_logs_usage() {
        let first = start_mock(StatusCode::TOO_MANY_REQUESTS, "limited").await;
        let second = start_mock(StatusCode::OK, "success").await;
        let (_directory, state) = test_state().await;
        let first_id = insert_test_account(&state, "first", &first).await;
        let second_id = insert_test_account(&state, "second", &second).await;
        let downstream = "sk-mini-integration-test";
        sqlx::query(
            "INSERT INTO api_keys (name, token_prefix, token_hash) VALUES ('test', 'sk-mini-int', ?)",
        )
        .bind(token_hash(downstream))
        .execute(&state.pool)
        .await
        .unwrap();

        let app = Router::new()
            .nest("/v1", router(state.clone()))
            .with_state(state.clone());
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/responses")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {downstream}"))
                    .body(Body::from(r#"{"model":"gpt-test","input":"hello"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap()["id"],
            "success"
        );

        let cooldown: Option<String> =
            sqlx::query_scalar("SELECT cooldown_until FROM accounts WHERE id = ?")
                .bind(first_id)
                .fetch_one(&state.pool)
                .await
                .unwrap();
        assert!(cooldown.is_some());
        let logged_account: i64 =
            sqlx::query_scalar("SELECT account_id FROM usage_logs ORDER BY id DESC LIMIT 1")
                .fetch_one(&state.pool)
                .await
                .unwrap();
        assert_eq!(logged_account, second_id);
    }

    #[tokio::test]
    async fn messages_routes_only_to_anthropic_and_accepts_downstream_x_api_key() {
        async fn anthropic_handler(
            headers: HeaderMap,
            Json(body): Json<Value>,
        ) -> impl IntoResponse {
            assert_eq!(
                headers
                    .get("x-api-key")
                    .and_then(|value| value.to_str().ok()),
                Some("anthropic-secret")
            );
            assert_eq!(
                headers
                    .get("anthropic-version")
                    .and_then(|value| value.to_str().ok()),
                Some("2023-06-01")
            );
            assert!(headers.get(header::AUTHORIZATION).is_none());
            assert_eq!(body["model"], "claude-test");
            Json(json!({
                "id": "msg_1",
                "type": "message",
                "role": "assistant",
                "model": "claude-test",
                "content": [{"type": "text", "text": "hello"}],
                "usage": {"input_tokens": 3, "output_tokens": 1}
            }))
        }

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route("/v1/messages", post(anthropic_handler)),
            )
            .await
            .unwrap();
        });
        let (_directory, state) = test_state().await;
        let credentials = state
            .crypto
            .encrypt(br#"{"api_key":"anthropic-secret"}"#)
            .unwrap();
        sqlx::query(
            "INSERT INTO accounts (name, kind, platform, account_type, base_url, \
             encrypted_credentials) VALUES ('claude', 'api_key', 'anthropic', 'api_key', ?, ?)",
        )
        .bind(format!("http://{address}"))
        .bind(credentials)
        .execute(&state.pool)
        .await
        .unwrap();
        let downstream = "sk-mini-anthropic-test";
        sqlx::query(
            "INSERT INTO api_keys (name, token_prefix, token_hash) VALUES ('claude', 'sk-mini-ant', ?)",
        )
        .bind(token_hash(downstream))
        .execute(&state.pool)
        .await
        .unwrap();
        let app = Router::new()
            .nest("/v1", router(state.clone()))
            .with_state(state);
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/messages")
                    .header("content-type", "application/json")
                    .header("x-api-key", downstream)
                    .body(Body::from(
                        json!({"model":"claude-test","max_tokens":16,"messages":[]}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap()["id"],
            "msg_1"
        );
    }

    #[tokio::test]
    async fn enforces_api_key_model_quota_and_expiry_policies() {
        let (_directory, state) = test_state().await;
        let downstream = "sk-mini-policy-integration-test";
        let key_id = sqlx::query(
            "INSERT INTO api_keys \
             (name, token_prefix, token_hash, allowed_models) \
             VALUES ('policy', 'sk-mini-policy', ?, '[\"gpt-allowed\"]')",
        )
        .bind(token_hash(downstream))
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        let app = Router::new()
            .nest("/v1", router(state.clone()))
            .with_state(state.clone());

        let denied_model = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/responses")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {downstream}"))
                    .body(Body::from(r#"{"model":"gpt-denied","input":"hello"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(denied_model.status(), StatusCode::FORBIDDEN);
        let body = to_bytes(denied_model.into_body(), 1024 * 1024)
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap()["error"]["code"],
            "MODEL_NOT_ALLOWED"
        );

        sqlx::query("UPDATE api_keys SET allowed_models = '[]', quota_tokens = 10 WHERE id = ?")
            .bind(key_id)
            .execute(&state.pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO usage_logs \
             (request_id, api_key_id, endpoint, status_code, total_tokens, duration_ms) \
             VALUES ('quota', ?, '/v1/responses', 200, 10, 1)",
        )
        .bind(key_id)
        .execute(&state.pool)
        .await
        .unwrap();
        let exhausted = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/responses")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {downstream}"))
                    .body(Body::from(r#"{"model":"gpt-any","input":"hello"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(exhausted.status(), StatusCode::TOO_MANY_REQUESTS);

        sqlx::query("UPDATE api_keys SET quota_tokens = 0, quota_cost_microusd = 25 WHERE id = ?")
            .bind(key_id)
            .execute(&state.pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO usage_logs \
             (request_id, api_key_id, endpoint, status_code, cost_microusd, duration_ms) \
             VALUES ('cost-quota', ?, '/v1/responses', 200, 25, 1)",
        )
        .bind(key_id)
        .execute(&state.pool)
        .await
        .unwrap();
        let cost_exhausted = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/models")
                    .header("authorization", format!("Bearer {downstream}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(cost_exhausted.into_body(), 1024 * 1024)
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap()["error"]["code"],
            "API_KEY_COST_QUOTA_EXHAUSTED"
        );

        sqlx::query(
            "UPDATE api_keys SET quota_cost_microusd = 0, rate_limit_5h_microusd = 25 WHERE id = ?",
        )
        .bind(key_id)
        .execute(&state.pool)
        .await
        .unwrap();
        let rate_exhausted = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/models")
                    .header("authorization", format!("Bearer {downstream}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(rate_exhausted.into_body(), 1024 * 1024)
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap()["error"]["code"],
            "API_KEY_RATE_5H_EXCEEDED"
        );

        sqlx::query(
            "UPDATE api_keys SET rate_limit_5h_microusd = 0, \
             ip_whitelist = '[\"10.0.0.0/8\"]' WHERE id = ?",
        )
        .bind(key_id)
        .execute(&state.pool)
        .await
        .unwrap();
        let ip_denied = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/models")
                    .header("authorization", format!("Bearer {downstream}"))
                    .extension(ConnectInfo(
                        "192.168.1.20:43110"
                            .parse::<std::net::SocketAddr>()
                            .unwrap(),
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(ip_denied.into_body(), 1024 * 1024).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap()["error"]["code"],
            "API_KEY_IP_FORBIDDEN"
        );

        sqlx::query(
            "UPDATE api_keys SET ip_whitelist = '[]', expires_at = '2020-01-01T00:00:00Z' WHERE id = ?",
        )
        .bind(key_id)
        .execute(&state.pool)
        .await
        .unwrap();
        let expired = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/models")
                    .header("authorization", format!("Bearer {downstream}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(expired.status(), StatusCode::FORBIDDEN);
        let body = to_bytes(expired.into_body(), 1024 * 1024).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap()["error"]["code"],
            "API_KEY_EXPIRED"
        );
    }

    #[tokio::test]
    async fn rejects_keys_owned_by_disabled_users() {
        let (_directory, state) = test_state().await;
        let user_id: i64 = sqlx::query_scalar("SELECT id FROM users WHERE role = 'admin'")
            .fetch_one(&state.pool)
            .await
            .unwrap();
        let downstream = "sk-mini-disabled-owner";
        sqlx::query(
            "INSERT INTO api_keys (name, token_prefix, token_hash, user_id) VALUES ('disabled', 'sk-mini-dis', ?, ?)",
        )
        .bind(token_hash(downstream))
        .bind(user_id)
        .execute(&state.pool)
        .await
        .unwrap();
        sqlx::query("UPDATE users SET enabled = 0 WHERE id = ?")
            .bind(user_id)
            .execute(&state.pool)
            .await
            .unwrap();
        let app = Router::new()
            .nest("/v1", router(state.clone()))
            .with_state(state);
        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/models")
                    .header("authorization", format!("Bearer {downstream}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap()["error"]["code"],
            "USER_DISABLED"
        );
    }

    #[tokio::test]
    async fn routes_group_keys_only_to_bound_accounts() {
        let first = start_mock(StatusCode::OK, "unbound").await;
        let second = start_mock(StatusCode::OK, "bound").await;
        let (_directory, state) = test_state().await;
        let _first_id = insert_test_account(&state, "first", &first).await;
        let second_id = insert_test_account(&state, "second", &second).await;
        let group_id = sqlx::query("INSERT INTO groups (name) VALUES ('restricted')")
            .execute(&state.pool)
            .await
            .unwrap()
            .last_insert_rowid();
        sqlx::query("INSERT INTO account_groups (account_id, group_id) VALUES (?, ?)")
            .bind(second_id)
            .bind(group_id)
            .execute(&state.pool)
            .await
            .unwrap();
        let downstream = "sk-mini-group-integration-test";
        sqlx::query(
            "INSERT INTO api_keys (name, token_prefix, token_hash, group_id) VALUES ('group', 'sk-mini-group', ?, ?)",
        )
        .bind(token_hash(downstream))
        .bind(group_id)
        .execute(&state.pool)
        .await
        .unwrap();
        let app = Router::new()
            .nest("/v1", router(state.clone()))
            .with_state(state.clone());
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/responses")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {downstream}"))
                    .body(Body::from(r#"{"model":"gpt-test","input":"hello"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap()["id"],
            "bound"
        );

        sqlx::query("UPDATE groups SET allowed_models = '[\"gpt-allowed\"]' WHERE id = ?")
            .bind(group_id)
            .execute(&state.pool)
            .await
            .unwrap();
        let denied = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/responses")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {downstream}"))
                    .body(Body::from(r#"{"model":"gpt-denied","input":"hello"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn maps_models_before_forwarding_and_preserves_usage_lineage() {
        async fn mapped_handler(Json(body): Json<Value>) -> Json<Value> {
            assert_eq!(body["model"], "gpt-upstream");
            Json(json!({
                "id":"mapped-response", "model":"gpt-real", "output":[],
                "usage":{"input_tokens":10,"output_tokens":2,"total_tokens":12}
            }))
        }

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route("/v1/responses", post(mapped_handler)),
            )
            .await
            .unwrap();
        });

        let (_directory, state) = test_state().await;
        let account_id = insert_test_account(&state, "mapped", &format!("http://{address}")).await;
        let group_id = sqlx::query(
            "INSERT INTO groups (name, rate_multiplier_micros) \
             VALUES ('mapped-group', 500000)",
        )
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        sqlx::query("INSERT INTO account_groups (account_id, group_id) VALUES (?, ?)")
            .bind(account_id)
            .bind(group_id)
            .execute(&state.pool)
            .await
            .unwrap();
        let channel_id = sqlx::query(
            "INSERT INTO channels \
             (name, restrict_models, model_mapping, billing_model_source) \
             VALUES ('mapped-channel', 1, \
             '{\"openai\":{\"gpt-client\":\"gpt-upstream\"}}', 'upstream')",
        )
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        sqlx::query("INSERT INTO channel_groups (channel_id, group_id) VALUES (?, ?)")
            .bind(channel_id)
            .bind(group_id)
            .execute(&state.pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO channel_model_pricing \
             (channel_id, models, input_microusd_per_million, output_microusd_per_million) \
             VALUES (?, '[\"gpt-upstream\",\"gpt-real\"]', 1000000, 2000000)",
        )
        .bind(channel_id)
        .execute(&state.pool)
        .await
        .unwrap();
        let stats_rule_id = sqlx::query(
            "INSERT INTO channel_account_stats_rules (channel_id, name) \
             VALUES (?, 'actual upstream pricing')",
        )
        .bind(channel_id)
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        sqlx::query(
            "INSERT INTO channel_account_stats_rule_accounts (rule_id, account_id) VALUES (?, ?)",
        )
        .bind(stats_rule_id)
        .bind(account_id)
        .execute(&state.pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO channel_account_stats_pricing \
             (rule_id, platform, models, input_microusd_per_million, \
              output_microusd_per_million) VALUES (?, 'openai', '[\"gpt-real\"]', 3000000, 4000000)",
        )
        .bind(stats_rule_id)
        .execute(&state.pool)
        .await
        .unwrap();
        let user_id: i64 = sqlx::query_scalar("SELECT id FROM users WHERE role = 'admin'")
            .fetch_one(&state.pool)
            .await
            .unwrap();
        let downstream = "sk-mini-mapped-test";
        sqlx::query(
            "INSERT INTO api_keys (user_id, name, token_prefix, token_hash, group_id) \
             VALUES (?, 'mapped', 'sk-mini-mapped', ?, ?)",
        )
        .bind(user_id)
        .bind(token_hash(downstream))
        .bind(group_id)
        .execute(&state.pool)
        .await
        .unwrap();

        let response = Router::new()
            .nest("/v1", router(state.clone()))
            .with_state(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/responses")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {downstream}"))
                    .body(Body::from(r#"{"model":"gpt-client","input":"hello"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let lineage: (String, String, String, String, i64, i64) = sqlx::query_as(
            "SELECT model, billing_model, mapped_model, model_mapping_chain, cost_microusd, \
             account_cost_microusd \
             FROM usage_logs ORDER BY id DESC LIMIT 1",
        )
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(
            lineage,
            (
                "gpt-client".into(),
                "gpt-real".into(),
                "gpt-upstream".into(),
                "gpt-client->gpt-upstream->gpt-real".into(),
                7,
                38,
            )
        );

        sqlx::query("DELETE FROM channel_account_stats_rules WHERE id = ?")
            .bind(stats_rule_id)
            .execute(&state.pool)
            .await
            .unwrap();
        sqlx::query("UPDATE channels SET apply_pricing_to_account_stats = 1 WHERE id = ?")
            .bind(channel_id)
            .execute(&state.pool)
            .await
            .unwrap();
        let response = Router::new()
            .nest("/v1", router(state.clone()))
            .with_state(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/responses")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {downstream}"))
                    .body(Body::from(r#"{"model":"gpt-client","input":"hello"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let fallback_costs: (i64, i64) = sqlx::query_as(
            "SELECT cost_microusd, account_cost_microusd \
             FROM usage_logs ORDER BY id DESC LIMIT 1",
        )
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(fallback_costs, (7, 14));
    }

    #[tokio::test]
    async fn enforces_active_subscription_token_quota() {
        let (_directory, state) = test_state().await;
        let user_id: i64 = sqlx::query_scalar("SELECT id FROM users WHERE role = 'admin'")
            .fetch_one(&state.pool)
            .await
            .unwrap();
        let plan_id = sqlx::query(
            "INSERT INTO plans (name, token_limit, duration_days) VALUES ('limited', 10, 30)",
        )
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        sqlx::query(
            "INSERT INTO subscriptions (user_id, plan_id, token_limit, starts_at, ends_at) \
             VALUES (?, ?, 10, datetime('now', '-1 hour'), datetime('now', '+1 day'))",
        )
        .bind(user_id)
        .bind(plan_id)
        .execute(&state.pool)
        .await
        .unwrap();
        let downstream = "sk-mini-subscription-test";
        let key_id = sqlx::query(
            "INSERT INTO api_keys (user_id, name, token_prefix, token_hash) VALUES (?, 'sub', 'sk-mini-sub', ?)",
        )
        .bind(user_id)
        .bind(token_hash(downstream))
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        sqlx::query(
            "INSERT INTO usage_logs \
             (request_id, api_key_id, user_id, endpoint, status_code, total_tokens, duration_ms) \
             VALUES ('sub-used', ?, ?, '/v1/responses', 200, 10, 1)",
        )
        .bind(key_id)
        .bind(user_id)
        .execute(&state.pool)
        .await
        .unwrap();
        let app = Router::new()
            .nest("/v1", router(state.clone()))
            .with_state(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/models")
                    .header("authorization", format!("Bearer {downstream}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap()["error"]["code"],
            "SUBSCRIPTION_QUOTA_EXHAUSTED"
        );
    }

    #[tokio::test]
    async fn isolates_subscription_quota_by_group() {
        let (_directory, state) = test_state().await;
        let user_id = sqlx::query(
            "INSERT INTO users (username, display_name, password_hash) \
             VALUES ('quota-user', 'Quota User', 'unused')",
        )
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        let mut group_ids = Vec::new();
        for name in ["quota-a", "quota-b"] {
            group_ids.push(
                sqlx::query(
                    "INSERT INTO groups (name, subscription_type) VALUES (?, 'subscription')",
                )
                .bind(name)
                .execute(&state.pool)
                .await
                .unwrap()
                .last_insert_rowid(),
            );
        }
        for (index, group_id) in group_ids.iter().enumerate() {
            let plan_id = sqlx::query(
                "INSERT INTO plans (name, token_limit, duration_days, group_id) VALUES (?, 10, 30, ?)",
            )
            .bind(format!("quota-plan-{index}"))
            .bind(group_id)
            .execute(&state.pool)
            .await
            .unwrap()
            .last_insert_rowid();
            sqlx::query(
                "INSERT INTO subscriptions \
                 (user_id, plan_id, group_id, token_limit, starts_at, ends_at) \
                 VALUES (?, ?, ?, 10, datetime('now', '-1 hour'), datetime('now', '+1 day'))",
            )
            .bind(user_id)
            .bind(plan_id)
            .bind(group_id)
            .execute(&state.pool)
            .await
            .unwrap();
        }
        let key_a = sqlx::query(
            "INSERT INTO api_keys (user_id, name, token_prefix, token_hash, group_id) \
             VALUES (?, 'quota-a', 'sk-a', 'hash-a', ?)",
        )
        .bind(user_id)
        .bind(group_ids[0])
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        sqlx::query(
            "INSERT INTO usage_logs \
             (request_id, api_key_id, user_id, endpoint, status_code, total_tokens, duration_ms) \
             VALUES ('quota-a-used', ?, ?, '/v1/responses', 200, 10, 1)",
        )
        .bind(key_a)
        .bind(user_id)
        .execute(&state.pool)
        .await
        .unwrap();

        assert!(
            enforce_subscription_quota(&state, user_id, Some(group_ids[0]))
                .await
                .is_err()
        );
        enforce_subscription_quota(&state, user_id, Some(group_ids[1]))
            .await
            .unwrap();
    }

    async fn start_mock(status: StatusCode, id: &'static str) -> String {
        async fn handler(
            State((status, id)): State<(StatusCode, &'static str)>,
            headers: HeaderMap,
            Json(_body): Json<Value>,
        ) -> impl IntoResponse {
            assert_eq!(
                headers
                    .get(header::AUTHORIZATION)
                    .and_then(|value| value.to_str().ok()),
                Some("Bearer upstream-secret")
            );
            (
                status,
                Json(json!({
                    "id": id,
                    "object": "response",
                    "output": [],
                    "usage": {"input_tokens": 4, "output_tokens": 2, "total_tokens": 6}
                })),
            )
        }

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = Router::new()
            .route("/v1/responses", post(handler))
            .with_state((status, id));
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{address}")
    }

    async fn test_state() -> (TempDir, AppState) {
        let directory = tempfile::tempdir().unwrap();
        let config = Config {
            bind: "127.0.0.1:0".parse().unwrap(),
            callback_bind: "127.0.0.1:0".parse().unwrap(),
            database_path: directory.path().join("test.sqlite3"),
            admin_username: "admin".into(),
            admin_password: "test-password".into(),
            master_key: [9; 32],
            public_ui_url: "http://localhost:8080".into(),
            session_hours: 12,
            mail_webhook_url: None,
            mail_webhook_token: None,
            turnstile_verify_url: "http://127.0.0.1:0/turnstile".into(),
        };
        let pool = db::connect(
            &config.database_path,
            &config.admin_username,
            &config.admin_password,
        )
        .await
        .unwrap();
        let state = AppState::new(pool, Crypto::new(&config.master_key), config).unwrap();
        (directory, state)
    }

    #[tokio::test]
    async fn channel_restricts_models_and_calculates_integer_cost() {
        let (_directory, state) = test_state().await;
        let group_id = sqlx::query(
            "INSERT INTO groups (name, allowed_models, rate_multiplier_micros) \
             VALUES ('priced', '[]', 1250000)",
        )
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        let channel_id =
            sqlx::query("INSERT INTO channels (name, restrict_models) VALUES ('OpenAI', 1)")
                .execute(&state.pool)
                .await
                .unwrap()
                .last_insert_rowid();
        sqlx::query("INSERT INTO channel_groups (channel_id, group_id) VALUES (?, ?)")
            .bind(channel_id)
            .bind(group_id)
            .execute(&state.pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO channel_model_pricing (channel_id, models, input_microusd_per_million, \
             output_microusd_per_million) VALUES (?, '[\"gpt-priced\"]', 2000000, 4000000)",
        )
        .bind(channel_id)
        .execute(&state.pool)
        .await
        .unwrap();
        let user_id: i64 =
            sqlx::query_scalar("SELECT id FROM users WHERE role = 'admin' ORDER BY id LIMIT 1")
                .fetch_one(&state.pool)
                .await
                .unwrap();
        sqlx::query(
            "INSERT INTO user_group_rate_multipliers \
             (user_id, group_id, rate_multiplier_micros) VALUES (?, ?, 500000)",
        )
        .bind(user_id)
        .bind(group_id)
        .execute(&state.pool)
        .await
        .unwrap();
        let key_id = sqlx::query(
            "INSERT INTO api_keys (name, token_prefix, token_hash, group_id, user_id) \
             VALUES ('priced', 'sk-mini_test', 'priced-hash', ?, ?)",
        )
        .bind(group_id)
        .bind(user_id)
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        assert!(
            channel_model_allowed(&state, group_id, "gpt-priced")
                .await
                .unwrap()
        );
        assert!(
            !channel_model_allowed(&state, group_id, "gpt-other")
                .await
                .unwrap()
        );
        let cost = calculate_cost(
            &state,
            key_id,
            Some("gpt-priced"),
            Usage {
                input: Some(500_000),
                output: Some(250_000),
                total: Some(750_000),
                cached: None,
                cache_write: None,
                image_input: None,
                image_output: None,
                reasoning: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(cost, 1_000_000);

        let pricing_id: i64 = sqlx::query_scalar(
            "SELECT id FROM channel_model_pricing WHERE channel_id = ? AND billing_mode = 'tokens'",
        )
        .bind(channel_id)
        .fetch_one(&state.pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO channel_pricing_intervals (pricing_id, min_tokens, max_tokens, \
             input_microusd_per_million, output_microusd_per_million, \
             cache_read_microusd_per_million) VALUES (?, 0, 1000000, 4000000, 8000000, 1000000)",
        )
        .bind(pricing_id)
        .execute(&state.pool)
        .await
        .unwrap();
        let tiered_cost = calculate_cost(
            &state,
            key_id,
            Some("gpt-priced"),
            Usage {
                input: Some(500_000),
                output: Some(250_000),
                total: Some(750_000),
                cached: Some(200_000),
                cache_write: None,
                image_input: None,
                image_output: None,
                reasoning: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(tiered_cost, 1_700_000);

        sqlx::query(
            "UPDATE channel_model_pricing SET image_input_microusd_per_million = 3000000, \
             image_output_microusd_per_million = 10000000 WHERE id = ?",
        )
        .bind(pricing_id)
        .execute(&state.pool)
        .await
        .unwrap();
        sqlx::query(
            "UPDATE channel_pricing_intervals SET cache_write_microusd_per_million = 2000000 \
             WHERE pricing_id = ?",
        )
        .bind(pricing_id)
        .execute(&state.pool)
        .await
        .unwrap();
        let advanced_cost = calculate_cost(
            &state,
            key_id,
            Some("gpt-priced"),
            Usage {
                input: Some(500_000),
                output: Some(200_000),
                total: Some(700_000),
                cached: Some(100_000),
                cache_write: Some(100_000),
                image_input: Some(100_000),
                image_output: Some(50_000),
                reasoning: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(advanced_cost, 1_550_000);

        sqlx::query(
            "UPDATE groups SET subscription_type = 'subscription', peak_rate_enabled = 1, \
             peak_start = '14:00', peak_end = '18:00', peak_rate_multiplier_micros = 3000000 \
             WHERE id = ?",
        )
        .bind(group_id)
        .execute(&state.pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO channel_model_pricing (channel_id, models, billing_mode, \
             per_request_microusd) VALUES (?, '[\"image-request\"]', 'request', 2000000)",
        )
        .bind(channel_id)
        .execute(&state.pool)
        .await
        .unwrap();
        let peak_time = Utc.with_ymd_and_hms(2026, 7, 23, 7, 0, 0).unwrap();
        let request_cost = calculate_cost_at(
            &state,
            key_id,
            Some("image-request"),
            Usage {
                input: None,
                output: None,
                total: None,
                cached: None,
                cache_write: None,
                image_input: None,
                image_output: None,
                reasoning: None,
            },
            peak_time,
        )
        .await
        .unwrap();
        assert_eq!(request_cost, 1_000_000);
    }

    async fn insert_test_account(state: &AppState, name: &str, base_url: &str) -> i64 {
        let credentials = Credentials {
            api_key: Some("upstream-secret".into()),
            ..Default::default()
        };
        let encrypted = state
            .crypto
            .encrypt(&serde_json::to_vec(&credentials).unwrap())
            .unwrap();
        sqlx::query(
            "INSERT INTO accounts (name, kind, base_url, encrypted_credentials, priority, concurrency) \
             VALUES (?, 'api_key', ?, ?, 50, 3)",
        )
        .bind(name)
        .bind(base_url)
        .bind(encrypted)
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid()
    }
}
