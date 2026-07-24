use std::io;

use axum::{
    Json, Router,
    body::Body,
    extract::{Extension, Path, Query, State},
    http::{HeaderValue, header},
    response::Response,
    routing::{get, post},
};
use bytes::Bytes;
use chrono::{DateTime, Duration as ChronoDuration, NaiveDate, Utc};
use futures_util::TryStreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{FromRow, QueryBuilder, Sqlite};

use crate::{
    auth::AuthSession,
    crypto::{random_token, token_hash},
    error::{ApiError, ApiResult},
    models::UsageLog,
    state::AppState,
};

const USAGE_COLUMNS: &str = "id, request_id, api_key_id, account_id, user_id, endpoint, model, \
    status_code, input_tokens, output_tokens, total_tokens, cached_input_tokens, cache_write_tokens, \
    image_input_tokens, image_output_tokens, reasoning_tokens, billing_model, mapped_model, \
    model_mapping_chain, request_type, stream, service_tier, cost_microusd, duration_ms, \
    ttft_ms, upstream_attempts, account_switches, error_summary, created_at";

pub fn admin_router() -> Router<AppState> {
    Router::new()
        .route("/usage", get(admin_list))
        .route("/usage/stats", get(admin_stats))
        .route("/usage/export", get(admin_export))
        .route("/usage/cleanup/preview", post(cleanup_preview))
        .route("/usage/cleanup/confirm", post(cleanup_confirm))
        .route("/usage/{id}", get(admin_detail))
}

pub fn user_router() -> Router<AppState> {
    Router::new()
        .route("/usage", get(user_list))
        .route("/usage/stats", get(user_stats))
        .route("/usage/export", get(user_export))
        .route("/usage/{id}", get(user_detail))
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct UsageFilter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    user_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    api_key_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    status_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    status_class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    request_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    service_tier: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    start_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    end_date: Option<String>,
}

#[derive(Deserialize)]
struct UsageQuery {
    #[serde(default = "default_page")]
    page: i64,
    #[serde(default = "default_page_size")]
    page_size: i64,
    user_id: Option<i64>,
    api_key_id: Option<i64>,
    model: Option<String>,
    endpoint: Option<String>,
    status_code: Option<i32>,
    status_class: Option<String>,
    request_type: Option<String>,
    stream: Option<bool>,
    service_tier: Option<String>,
    start_date: Option<String>,
    end_date: Option<String>,
}

impl UsageQuery {
    fn filter(&self) -> UsageFilter {
        UsageFilter {
            user_id: self.user_id,
            api_key_id: self.api_key_id,
            model: self.model.clone(),
            endpoint: self.endpoint.clone(),
            status_code: self.status_code,
            status_class: self.status_class.clone(),
            request_type: self.request_type.clone(),
            stream: self.stream,
            service_tier: self.service_tier.clone(),
            start_date: self.start_date.clone(),
            end_date: self.end_date.clone(),
        }
    }
}

fn default_page() -> i64 {
    1
}

fn default_page_size() -> i64 {
    50
}

fn normalize_filter(mut filter: UsageFilter) -> ApiResult<UsageFilter> {
    fn clean(value: &mut Option<String>, max: usize) {
        *value = value
            .take()
            .map(|value| value.trim().chars().take(max).collect::<String>())
            .filter(|value| !value.is_empty());
    }
    clean(&mut filter.model, 120);
    clean(&mut filter.endpoint, 80);
    clean(&mut filter.status_class, 24);
    clean(&mut filter.request_type, 16);
    clean(&mut filter.service_tier, 40);
    clean(&mut filter.start_date, 10);
    clean(&mut filter.end_date, 10);
    if filter.user_id.is_some_and(|value| value <= 0)
        || filter.api_key_id.is_some_and(|value| value <= 0)
    {
        return Err(ApiError::bad_request(
            "INVALID_USAGE_FILTER",
            "user_id and api_key_id must be positive",
        ));
    }
    if filter
        .status_code
        .is_some_and(|value| !(100..=599).contains(&value))
    {
        return Err(ApiError::bad_request(
            "INVALID_STATUS_CODE",
            "status_code must be between 100 and 599",
        ));
    }
    if filter
        .status_class
        .as_deref()
        .is_some_and(|value| !matches!(value, "success" | "error" | "4xx" | "5xx" | "429"))
    {
        return Err(ApiError::bad_request(
            "INVALID_STATUS_CLASS",
            "status_class must be success, error, 4xx, 5xx, or 429",
        ));
    }
    if filter
        .request_type
        .as_deref()
        .is_some_and(|value| !matches!(value, "sync" | "stream"))
    {
        return Err(ApiError::bad_request(
            "INVALID_REQUEST_TYPE",
            "request_type must be sync or stream",
        ));
    }
    let start = filter
        .start_date
        .as_deref()
        .map(|value| parse_date(value, "start_date"))
        .transpose()?;
    let end = filter
        .end_date
        .as_deref()
        .map(|value| parse_date(value, "end_date"))
        .transpose()?;
    if start.zip(end).is_some_and(|(start, end)| start > end) {
        return Err(ApiError::bad_request(
            "INVALID_DATE_RANGE",
            "start_date must not be after end_date",
        ));
    }
    Ok(filter)
}

fn parse_date(value: &str, field: &'static str) -> ApiResult<NaiveDate> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| ApiError::bad_request("INVALID_DATE", format!("{field} must use YYYY-MM-DD")))
}

fn push_filter(
    query: &mut QueryBuilder<'_, Sqlite>,
    filter: &UsageFilter,
    snapshot_max_id: Option<i64>,
) {
    query.push(" WHERE 1=1");
    if let Some(value) = filter.user_id {
        query.push(" AND user_id = ").push_bind(value);
    }
    if let Some(value) = filter.api_key_id {
        query.push(" AND api_key_id = ").push_bind(value);
    }
    if let Some(value) = &filter.model {
        query
            .push(" AND model LIKE ")
            .push_bind(format!("%{value}%"));
    }
    if let Some(value) = &filter.endpoint {
        query.push(" AND endpoint = ").push_bind(value.clone());
    }
    if let Some(value) = filter.status_code {
        query.push(" AND status_code = ").push_bind(value);
    }
    match filter.status_class.as_deref() {
        Some("success") => query.push(" AND status_code < 400"),
        Some("error") => query.push(" AND status_code >= 400"),
        Some("4xx") => query.push(" AND status_code BETWEEN 400 AND 499"),
        Some("5xx") => query.push(" AND status_code BETWEEN 500 AND 599"),
        Some("429") => query.push(" AND status_code = 429"),
        _ => query,
    };
    if let Some(value) = &filter.request_type {
        query.push(" AND request_type = ").push_bind(value.clone());
    }
    if let Some(value) = filter.stream {
        query.push(" AND stream = ").push_bind(value);
    }
    if let Some(value) = &filter.service_tier {
        query
            .push(" AND COALESCE(service_tier, '') = ")
            .push_bind(value.clone());
    }
    if let Some(value) = &filter.start_date {
        query
            .push(" AND datetime(created_at) >= datetime(")
            .push_bind(value.clone())
            .push(")");
    }
    if let Some(value) = &filter.end_date {
        query
            .push(" AND datetime(created_at) < datetime(")
            .push_bind(value.clone())
            .push(", '+1 day')");
    }
    if let Some(value) = snapshot_max_id {
        query.push(" AND id <= ").push_bind(value);
    }
}

async fn admin_list(
    State(state): State<AppState>,
    Query(query): Query<UsageQuery>,
) -> ApiResult<Json<Value>> {
    list(&state, query).await
}

async fn user_list(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
    Query(mut query): Query<UsageQuery>,
) -> ApiResult<Json<Value>> {
    query.user_id = Some(session.user_id);
    list(&state, query).await
}

async fn list(state: &AppState, query: UsageQuery) -> ApiResult<Json<Value>> {
    let filter = normalize_filter(query.filter())?;
    let page = query.page.max(1);
    let page_size = query.page_size.clamp(1, 200);
    let mut count = QueryBuilder::<Sqlite>::new("SELECT COUNT(*) FROM usage_logs");
    push_filter(&mut count, &filter, None);
    let total: i64 = count.build_query_scalar().fetch_one(&state.pool).await?;
    let mut rows = QueryBuilder::<Sqlite>::new(format!("SELECT {USAGE_COLUMNS} FROM usage_logs"));
    push_filter(&mut rows, &filter, None);
    rows.push(" ORDER BY id DESC LIMIT ")
        .push_bind(page_size)
        .push(" OFFSET ")
        .push_bind((page - 1) * page_size);
    let rows: Vec<UsageLog> = rows.build_query_as().fetch_all(&state.pool).await?;
    Ok(Json(json!({"data": rows, "meta": {
        "page": page, "page_size": page_size, "total": total
    }})))
}

async fn admin_detail(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<Value>> {
    detail(&state, id, None).await
}

async fn user_detail(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
    Path(id): Path<i64>,
) -> ApiResult<Json<Value>> {
    detail(&state, id, Some(session.user_id)).await
}

async fn detail(state: &AppState, id: i64, user_id: Option<i64>) -> ApiResult<Json<Value>> {
    let mut query = QueryBuilder::<Sqlite>::new(format!(
        "SELECT {USAGE_COLUMNS} FROM usage_logs WHERE id = "
    ));
    query.push_bind(id);
    if let Some(user_id) = user_id {
        query.push(" AND user_id = ").push_bind(user_id);
    }
    let row: UsageLog = query
        .build_query_as()
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| ApiError::not_found("usage log not found"))?;
    Ok(Json(json!({"data": row})))
}

#[derive(FromRow)]
struct SummaryRow {
    requests: i64,
    successful_requests: i64,
    failed_requests: i64,
    input_tokens: i64,
    output_tokens: i64,
    total_tokens: i64,
    cached_input_tokens: i64,
    cache_write_tokens: i64,
    image_input_tokens: i64,
    image_output_tokens: i64,
    reasoning_tokens: i64,
    cost_microusd: i64,
    average_duration_ms: f64,
    maximum_duration_ms: i64,
}

#[derive(FromRow, Serialize)]
struct ModelStat {
    model: String,
    requests: i64,
    total_tokens: i64,
    cached_input_tokens: i64,
    reasoning_tokens: i64,
    cost_microusd: i64,
}

#[derive(FromRow, Serialize)]
struct TrendStat {
    date: String,
    requests: i64,
    total_tokens: i64,
    cached_input_tokens: i64,
    reasoning_tokens: i64,
    cost_microusd: i64,
}

#[derive(FromRow, Serialize)]
struct NamedCount {
    name: String,
    requests: i64,
}

async fn admin_stats(
    State(state): State<AppState>,
    Query(filter): Query<UsageFilter>,
) -> ApiResult<Json<Value>> {
    stats(&state, filter).await
}

async fn user_stats(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
    Query(mut filter): Query<UsageFilter>,
) -> ApiResult<Json<Value>> {
    filter.user_id = Some(session.user_id);
    stats(&state, filter).await
}

async fn stats(state: &AppState, filter: UsageFilter) -> ApiResult<Json<Value>> {
    let filter = normalize_filter(filter)?;
    let mut summary = QueryBuilder::<Sqlite>::new(
        "SELECT COUNT(*) AS requests, \
         COALESCE(SUM(status_code < 400),0) AS successful_requests, \
         COALESCE(SUM(status_code >= 400),0) AS failed_requests, \
         COALESCE(SUM(input_tokens),0) AS input_tokens, \
         COALESCE(SUM(output_tokens),0) AS output_tokens, \
         COALESCE(SUM(total_tokens),0) AS total_tokens, \
         COALESCE(SUM(cached_input_tokens),0) AS cached_input_tokens, \
         COALESCE(SUM(cache_write_tokens),0) AS cache_write_tokens, \
         COALESCE(SUM(image_input_tokens),0) AS image_input_tokens, \
         COALESCE(SUM(image_output_tokens),0) AS image_output_tokens, \
         COALESCE(SUM(reasoning_tokens),0) AS reasoning_tokens, \
         COALESCE(SUM(cost_microusd),0) AS cost_microusd, \
         CAST(COALESCE(AVG(duration_ms),0) AS REAL) AS average_duration_ms, \
         COALESCE(MAX(duration_ms),0) AS maximum_duration_ms FROM usage_logs",
    );
    push_filter(&mut summary, &filter, None);
    let summary: SummaryRow = summary.build_query_as().fetch_one(&state.pool).await?;

    let mut models = QueryBuilder::<Sqlite>::new(
        "SELECT COALESCE(NULLIF(model,''),'unknown') AS model, COUNT(*) AS requests, \
         COALESCE(SUM(total_tokens),0) AS total_tokens, \
         COALESCE(SUM(cached_input_tokens),0) AS cached_input_tokens, \
         COALESCE(SUM(reasoning_tokens),0) AS reasoning_tokens, \
         COALESCE(SUM(cost_microusd),0) AS cost_microusd FROM usage_logs",
    );
    push_filter(&mut models, &filter, None);
    models.push(" GROUP BY COALESCE(NULLIF(model,''),'unknown') ORDER BY requests DESC LIMIT 20");
    let models: Vec<ModelStat> = models.build_query_as().fetch_all(&state.pool).await?;

    let mut trend = QueryBuilder::<Sqlite>::new(
        "SELECT date(created_at) AS date, COUNT(*) AS requests, \
         COALESCE(SUM(total_tokens),0) AS total_tokens, \
         COALESCE(SUM(cached_input_tokens),0) AS cached_input_tokens, \
         COALESCE(SUM(reasoning_tokens),0) AS reasoning_tokens, \
         COALESCE(SUM(cost_microusd),0) AS cost_microusd FROM usage_logs",
    );
    push_filter(&mut trend, &filter, None);
    trend.push(" GROUP BY date(created_at) ORDER BY date(created_at) DESC LIMIT 366");
    let trend: Vec<TrendStat> = trend.build_query_as().fetch_all(&state.pool).await?;

    let mut errors = QueryBuilder::<Sqlite>::new(
        "SELECT CASE WHEN status_code < 400 THEN 'success' WHEN status_code = 429 THEN 'rate_limited' \
         WHEN status_code BETWEEN 400 AND 499 THEN 'client_error' \
         WHEN status_code BETWEEN 500 AND 599 THEN 'upstream_error' ELSE 'transport_error' END AS name, \
         COUNT(*) AS requests FROM usage_logs",
    );
    push_filter(&mut errors, &filter, None);
    errors.push(" GROUP BY name ORDER BY requests DESC");
    let errors: Vec<NamedCount> = errors.build_query_as().fetch_all(&state.pool).await?;

    let mut request_types = QueryBuilder::<Sqlite>::new(
        "SELECT request_type AS name, COUNT(*) AS requests FROM usage_logs",
    );
    push_filter(&mut request_types, &filter, None);
    request_types.push(" GROUP BY request_type ORDER BY requests DESC");
    let request_types: Vec<NamedCount> = request_types
        .build_query_as()
        .fetch_all(&state.pool)
        .await?;

    let mut service_tiers = QueryBuilder::<Sqlite>::new(
        "SELECT COALESCE(NULLIF(service_tier,''),'default') AS name, COUNT(*) AS requests FROM usage_logs",
    );
    push_filter(&mut service_tiers, &filter, None);
    service_tiers
        .push(" GROUP BY COALESCE(NULLIF(service_tier,''),'default') ORDER BY requests DESC");
    let service_tiers: Vec<NamedCount> = service_tiers
        .build_query_as()
        .fetch_all(&state.pool)
        .await?;

    Ok(Json(json!({"data": {
        "summary": {
            "requests": summary.requests,
            "successful_requests": summary.successful_requests,
            "failed_requests": summary.failed_requests,
            "input_tokens": summary.input_tokens,
            "output_tokens": summary.output_tokens,
            "total_tokens": summary.total_tokens,
            "cached_input_tokens": summary.cached_input_tokens,
            "cache_write_tokens": summary.cache_write_tokens,
            "image_input_tokens": summary.image_input_tokens,
            "image_output_tokens": summary.image_output_tokens,
            "reasoning_tokens": summary.reasoning_tokens,
            "cost_microusd": summary.cost_microusd,
            "average_duration_ms": summary.average_duration_ms.round() as i64,
            "maximum_duration_ms": summary.maximum_duration_ms
        },
        "models": models,
        "trend": trend,
        "errors": errors,
        "request_types": request_types,
        "service_tiers": service_tiers
    }})))
}

async fn admin_export(
    State(state): State<AppState>,
    Query(filter): Query<UsageFilter>,
) -> ApiResult<Response> {
    export(state, filter).await
}

async fn user_export(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
    Query(mut filter): Query<UsageFilter>,
) -> ApiResult<Response> {
    filter.user_id = Some(session.user_id);
    export(state, filter).await
}

async fn export(state: AppState, filter: UsageFilter) -> ApiResult<Response> {
    let filter = normalize_filter(filter)?;
    let pool = state.pool.clone();
    let stream = async_stream::stream! {
        yield Ok::<Bytes, io::Error>(Bytes::from_static(b"id,created_at,request_id,user_id,api_key_id,account_id,endpoint,model,billing_model,mapped_model,model_mapping_chain,status_code,request_type,stream,service_tier,input_tokens,output_tokens,cached_input_tokens,cache_write_tokens,image_input_tokens,image_output_tokens,reasoning_tokens,total_tokens,cost_microusd,duration_ms,ttft_ms,upstream_attempts,account_switches,error_summary\r\n"));
        let mut query = QueryBuilder::<Sqlite>::new(format!("SELECT {USAGE_COLUMNS} FROM usage_logs"));
        push_filter(&mut query, &filter, None);
        query.push(" ORDER BY id DESC");
        let mut rows = query.build_query_as::<UsageLog>().fetch(&pool);
        loop {
            match rows.try_next().await {
                Ok(Some(row)) => yield Ok(Bytes::from(csv_row(&row))),
                Ok(None) => break,
                Err(error) => {
                    yield Err(io::Error::other(error));
                    break;
                }
            }
        }
    };
    let mut response = Response::new(Body::from_stream(stream));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/csv; charset=utf-8"),
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!(
            "attachment; filename=usage-{}.csv",
            Utc::now().format("%Y%m%d-%H%M%S")
        ))
        .expect("CSV filename is a valid header"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

fn csv_row(row: &UsageLog) -> String {
    let values = [
        row.id.to_string(),
        row.created_at.clone(),
        row.request_id.clone(),
        row.user_id
            .map(|value| value.to_string())
            .unwrap_or_default(),
        row.api_key_id
            .map(|value| value.to_string())
            .unwrap_or_default(),
        row.account_id
            .map(|value| value.to_string())
            .unwrap_or_default(),
        row.endpoint.clone(),
        row.model.clone().unwrap_or_default(),
        row.billing_model.clone().unwrap_or_default(),
        row.mapped_model.clone().unwrap_or_default(),
        row.model_mapping_chain.clone(),
        row.status_code.to_string(),
        row.request_type.clone(),
        row.stream.to_string(),
        row.service_tier.clone().unwrap_or_default(),
        row.input_tokens
            .map(|value| value.to_string())
            .unwrap_or_default(),
        row.output_tokens
            .map(|value| value.to_string())
            .unwrap_or_default(),
        row.cached_input_tokens.to_string(),
        row.cache_write_tokens.to_string(),
        row.image_input_tokens.to_string(),
        row.image_output_tokens.to_string(),
        row.reasoning_tokens.to_string(),
        row.total_tokens
            .map(|value| value.to_string())
            .unwrap_or_default(),
        row.cost_microusd.to_string(),
        row.duration_ms.to_string(),
        row.ttft_ms
            .map(|value| value.to_string())
            .unwrap_or_default(),
        row.upstream_attempts.to_string(),
        row.account_switches.to_string(),
        row.error_summary.clone().unwrap_or_default(),
    ];
    let mut output = values
        .iter()
        .map(|value| csv_cell(value))
        .collect::<Vec<_>>()
        .join(",");
    output.push_str("\r\n");
    output
}

fn csv_cell(value: &str) -> String {
    let value = if value.starts_with(['=', '+', '-', '@']) {
        format!("'{value}")
    } else {
        value.to_string()
    };
    if value.contains([',', '"', '\r', '\n']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value
    }
}

async fn cleanup_preview(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
    Json(filter): Json<UsageFilter>,
) -> ApiResult<Json<Value>> {
    let filter = normalize_filter(filter)?;
    if filter.end_date.is_none() {
        return Err(ApiError::bad_request(
            "USAGE_CLEANUP_END_DATE_REQUIRED",
            "end_date is required for usage cleanup",
        ));
    }
    let mut query =
        QueryBuilder::<Sqlite>::new("SELECT COUNT(*), COALESCE(MAX(id),0) FROM usage_logs");
    push_filter(&mut query, &filter, None);
    let (matched_count, snapshot_max_id): (i64, i64) =
        query.build_query_as().fetch_one(&state.pool).await?;
    let filter_json = serde_json::to_string(&filter)
        .map_err(|_| ApiError::internal("usage cleanup filter serialization failed"))?;
    let filter_hash = token_hash(&format!("{filter_json}:{snapshot_max_id}"));
    let confirmation_token = random_token(32)?;
    let expires_at = Utc::now() + ChronoDuration::minutes(5);
    sqlx::query(
        "DELETE FROM usage_delete_previews WHERE datetime(expires_at) <= CURRENT_TIMESTAMP",
    )
    .execute(&state.pool)
    .await?;
    sqlx::query(
        "INSERT INTO usage_delete_previews \
         (token_hash,admin_id,filter_hash,filter_json,snapshot_max_id,expires_at) VALUES (?,?,?,?,?,?)",
    )
    .bind(token_hash(&confirmation_token))
    .bind(session.user_id)
    .bind(&filter_hash)
    .bind(&filter_json)
    .bind(snapshot_max_id)
    .bind(expires_at.to_rfc3339())
    .execute(&state.pool)
    .await?;
    Ok(Json(json!({"data": {
        "matched_count": matched_count,
        "snapshot_max_id": snapshot_max_id,
        "filter": filter,
        "filter_hash": filter_hash,
        "confirmation_token": confirmation_token,
        "expires_at": expires_at.to_rfc3339()
    }})))
}

#[derive(Deserialize)]
struct CleanupConfirmInput {
    filter: UsageFilter,
    snapshot_max_id: i64,
    filter_hash: String,
    confirmation_token: String,
    confirm: bool,
}

async fn cleanup_confirm(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
    Json(input): Json<CleanupConfirmInput>,
) -> ApiResult<Json<Value>> {
    if !input.confirm {
        return Err(cleanup_confirmation_error());
    }
    let filter = normalize_filter(input.filter)?;
    if filter.end_date.is_none() {
        return Err(cleanup_confirmation_error());
    }
    let filter_json = serde_json::to_string(&filter)
        .map_err(|_| ApiError::internal("usage cleanup filter serialization failed"))?;
    let preview: Option<(i64, String, String, i64, String)> = sqlx::query_as(
        "SELECT admin_id,filter_hash,filter_json,snapshot_max_id,expires_at \
         FROM usage_delete_previews WHERE token_hash = ?",
    )
    .bind(token_hash(&input.confirmation_token))
    .fetch_optional(&state.pool)
    .await?;
    let preview = preview.ok_or_else(cleanup_confirmation_error)?;
    let expected_hash = token_hash(&format!("{filter_json}:{}", input.snapshot_max_id));
    if preview.0 != session.user_id
        || preview.1 != input.filter_hash
        || preview.1 != expected_hash
        || preview.2 != filter_json
        || preview.3 != input.snapshot_max_id
        || DateTime::parse_from_rfc3339(&preview.4)
            .ok()
            .is_none_or(|expires| expires <= Utc::now())
    {
        return Err(cleanup_confirmation_error());
    }
    let mut transaction = state.pool.begin().await?;
    let mut delete = QueryBuilder::<Sqlite>::new("DELETE FROM usage_logs");
    push_filter(&mut delete, &filter, Some(input.snapshot_max_id));
    let deleted_rows = delete
        .build()
        .execute(&mut *transaction)
        .await?
        .rows_affected();
    sqlx::query("DELETE FROM usage_delete_previews WHERE token_hash = ?")
        .bind(token_hash(&input.confirmation_token))
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(Json(json!({"data": {
        "deleted_rows": deleted_rows,
        "snapshot_max_id": input.snapshot_max_id
    }})))
}

fn cleanup_confirmation_error() -> ApiError {
    ApiError::bad_request(
        "USAGE_CLEANUP_CONFIRMATION_INVALID",
        "usage cleanup confirmation is invalid or expired",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;

    #[test]
    fn csv_escapes_formulas_quotes_and_newlines() {
        assert_eq!(csv_cell("=SUM(A1:A2)"), "'=SUM(A1:A2)");
        assert_eq!(csv_cell("a,\"b\"\n"), "\"a,\"\"b\"\"\n\"");
    }

    #[test]
    fn validates_request_and_date_filters() {
        assert!(
            normalize_filter(UsageFilter {
                request_type: Some("stream".into()),
                start_date: Some("2026-07-01".into()),
                end_date: Some("2026-07-02".into()),
                ..Default::default()
            })
            .is_ok()
        );
        assert!(
            normalize_filter(UsageFilter {
                request_type: Some("socket".into()),
                ..Default::default()
            })
            .is_err()
        );
    }

    #[tokio::test]
    async fn cleanup_confirmation_honors_snapshot_high_watermark() {
        let (_directory, state) = test_support::state().await;
        let admin_id: i64 = sqlx::query_scalar("SELECT id FROM users WHERE role = 'admin'")
            .fetch_one(&state.pool)
            .await
            .unwrap();
        let session = AuthSession {
            id: 1,
            user_id: admin_id,
            username: "admin".into(),
            display_name: "Administrator".into(),
            role: "admin".into(),
        };
        sqlx::query(
            "INSERT INTO usage_logs (request_id,endpoint,status_code,duration_ms) \
             VALUES ('before-preview','/v1/responses',200,10)",
        )
        .execute(&state.pool)
        .await
        .unwrap();
        let filter = UsageFilter {
            end_date: Some(Utc::now().format("%Y-%m-%d").to_string()),
            ..Default::default()
        };
        let Json(preview) = cleanup_preview(
            State(state.clone()),
            Extension(session.clone()),
            Json(filter.clone()),
        )
        .await
        .unwrap();
        let preview = &preview["data"];
        assert_eq!(preview["matched_count"], 1);

        sqlx::query(
            "INSERT INTO usage_logs (request_id,endpoint,status_code,duration_ms) \
             VALUES ('after-preview','/v1/responses',200,10)",
        )
        .execute(&state.pool)
        .await
        .unwrap();
        let input = CleanupConfirmInput {
            filter,
            snapshot_max_id: preview["snapshot_max_id"].as_i64().unwrap(),
            filter_hash: preview["filter_hash"].as_str().unwrap().to_string(),
            confirmation_token: preview["confirmation_token"].as_str().unwrap().to_string(),
            confirm: true,
        };
        let Json(result) = cleanup_confirm(State(state.clone()), Extension(session), Json(input))
            .await
            .unwrap();
        assert_eq!(result["data"]["deleted_rows"], 1);
        let remaining: Vec<String> =
            sqlx::query_scalar("SELECT request_id FROM usage_logs ORDER BY id")
                .fetch_all(&state.pool)
                .await
                .unwrap();
        assert_eq!(remaining, vec!["after-preview"]);
    }

    #[tokio::test]
    async fn stats_filter_keeps_user_usage_isolated() {
        let (_directory, state) = test_support::state().await;
        let user_one = sqlx::query(
            "INSERT INTO users (username,display_name,password_hash) VALUES ('usage-one','One','x')",
        )
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        let user_two = sqlx::query(
            "INSERT INTO users (username,display_name,password_hash) VALUES ('usage-two','Two','x')",
        )
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        for (request_id, user_id, tokens) in [("one", user_one, 11), ("two", user_two, 99)] {
            sqlx::query(
                "INSERT INTO usage_logs \
                 (request_id,user_id,endpoint,status_code,total_tokens,cached_input_tokens,reasoning_tokens,duration_ms) \
                 VALUES (?,?,'/v1/responses',200,?,3,2,10)",
            )
            .bind(request_id)
            .bind(user_id)
            .bind(tokens)
            .execute(&state.pool)
            .await
            .unwrap();
        }
        let Json(result) = stats(
            &state,
            UsageFilter {
                user_id: Some(user_one),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(result["data"]["summary"]["requests"], 1);
        assert_eq!(result["data"]["summary"]["total_tokens"], 11);
        assert_eq!(result["data"]["summary"]["cached_input_tokens"], 3);
        assert_eq!(result["data"]["summary"]["reasoning_tokens"], 2);
    }
}
