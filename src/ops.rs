use std::{convert::Infallible, str::FromStr, time::Duration};

use axum::{
    Json, Router,
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderValue, StatusCode, header},
    response::Response,
    routing::{get, post, put},
};
use bytes::Bytes;
use chrono::{DateTime, Utc};
use cron::Schedule;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{FromRow, QueryBuilder, Sqlite};

use crate::{
    auth,
    error::{ApiError, ApiResult},
    state::AppState,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/ops/overview", get(overview))
        .route("/ops/live", get(live_metrics))
        .route("/ops/requests", get(list_requests))
        .route("/ops/requests/{id}", get(request_detail))
        .route("/ops/system-logs", get(system_logs))
        .route("/ops/reports", get(list_reports))
        .route("/ops/reports/run", post(run_manual_report))
        .route(
            "/ops/runtime-log-config",
            get(get_runtime_log_config).put(update_runtime_log_config),
        )
        .route(
            "/ops/alert-rules",
            get(list_alert_rules).post(create_alert_rule),
        )
        .route(
            "/ops/alert-rules/{id}",
            put(update_alert_rule).delete(delete_alert_rule),
        )
        .route("/ops/alert-events", get(list_alert_events))
        .route("/ops/alert-events/{id}/status", put(update_alert_status))
        .route("/ops/evaluate", post(evaluate_now))
        .route("/ops/settings", get(get_settings).put(update_settings))
}

pub fn start_scheduler(state: AppState) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(20)).await;
        if let Err(error) = rollup_usage(&state, "-7 days").await {
            tracing::warn!(%error, "initial ops rollup failed");
        }
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            if let Err(error) = rollup_usage(&state, "-10 minutes").await {
                tracing::warn!(%error, "ops rollup failed");
            }
            if let Err(error) = evaluate_alerts(&state).await {
                tracing::warn!(%error, "ops alert evaluation failed");
            }
            if let Err(error) = evaluate_reports(&state).await {
                tracing::warn!(%error, "ops report evaluation failed");
            }
            if let Err(error) = cleanup_ops_data(&state).await {
                tracing::warn!(%error, "ops retention cleanup failed");
            }
        }
    });
}

#[derive(Debug, Deserialize)]
struct OverviewQuery {
    #[serde(default = "default_range")]
    range: String,
    model: Option<String>,
    account_id: Option<i64>,
}

fn default_range() -> String {
    "24h".into()
}

#[derive(Clone, Copy)]
struct TimeRange {
    label: &'static str,
    seconds: i64,
    bucket_format: &'static str,
    bucket_seconds: f64,
}

fn parse_range(value: &str) -> ApiResult<TimeRange> {
    match value {
        "5m" => Ok(TimeRange {
            label: "5m",
            seconds: 300,
            bucket_format: "%Y-%m-%d %H:%M:00",
            bucket_seconds: 60.0,
        }),
        "30m" => Ok(TimeRange {
            label: "30m",
            seconds: 1_800,
            bucket_format: "%Y-%m-%d %H:%M:00",
            bucket_seconds: 60.0,
        }),
        "1h" => Ok(TimeRange {
            label: "1h",
            seconds: 3_600,
            bucket_format: "%Y-%m-%d %H:%M:00",
            bucket_seconds: 60.0,
        }),
        "6h" => Ok(TimeRange {
            label: "6h",
            seconds: 21_600,
            bucket_format: "%Y-%m-%d %H:%M:00",
            bucket_seconds: 60.0,
        }),
        "24h" => Ok(TimeRange {
            label: "24h",
            seconds: 86_400,
            bucket_format: "%Y-%m-%d %H:00:00",
            bucket_seconds: 3_600.0,
        }),
        "7d" => Ok(TimeRange {
            label: "7d",
            seconds: 604_800,
            bucket_format: "%Y-%m-%d %H:00:00",
            bucket_seconds: 3_600.0,
        }),
        _ => Err(ApiError::bad_request(
            "INVALID_OPS_RANGE",
            "range must be 5m, 30m, 1h, 6h, 24h, or 7d",
        )),
    }
}

fn range_modifier(range: TimeRange) -> String {
    format!("-{} seconds", range.seconds)
}

async fn rollup_usage(state: &AppState, modifier: &str) -> ApiResult<u64> {
    let result = sqlx::query(
        "INSERT INTO ops_minute_rollups \
         (bucket, requests, successes, errors, tokens, cost_microusd, duration_sum_ms, \
          duration_max_ms, ttft_sum_ms, ttft_count, ttft_max_ms, account_switches, \
          stream_requests, stream_duration_sum_ms, updated_at) \
         SELECT strftime('%Y-%m-%d %H:%M:00', created_at), COUNT(*), \
          COALESCE(SUM(CASE WHEN status_code < 400 THEN 1 ELSE 0 END), 0), \
          COALESCE(SUM(CASE WHEN status_code >= 400 THEN 1 ELSE 0 END), 0), \
          COALESCE(SUM(COALESCE(total_tokens, 0)), 0), COALESCE(SUM(cost_microusd), 0), \
          COALESCE(SUM(duration_ms), 0), COALESCE(MAX(duration_ms), 0), \
          COALESCE(SUM(COALESCE(ttft_ms, 0)), 0), COUNT(ttft_ms), \
          COALESCE(MAX(COALESCE(ttft_ms, 0)), 0), COALESCE(SUM(account_switches), 0), \
          COALESCE(SUM(CASE WHEN stream = 1 THEN 1 ELSE 0 END), 0), \
          COALESCE(SUM(CASE WHEN stream = 1 THEN duration_ms ELSE 0 END), 0), CURRENT_TIMESTAMP \
         FROM usage_logs WHERE datetime(created_at) >= datetime('now', ?) \
         GROUP BY strftime('%Y-%m-%d %H:%M:00', created_at) \
         ON CONFLICT(bucket) DO UPDATE SET requests = excluded.requests, \
          successes = excluded.successes, errors = excluded.errors, tokens = excluded.tokens, \
          cost_microusd = excluded.cost_microusd, duration_sum_ms = excluded.duration_sum_ms, \
          duration_max_ms = excluded.duration_max_ms, ttft_sum_ms = excluded.ttft_sum_ms, \
          ttft_count = excluded.ttft_count, ttft_max_ms = excluded.ttft_max_ms, \
          account_switches = excluded.account_switches, stream_requests = excluded.stream_requests, \
          stream_duration_sum_ms = excluded.stream_duration_sum_ms, updated_at = CURRENT_TIMESTAMP",
    )
    .bind(modifier)
    .execute(&state.pool)
    .await?;
    Ok(result.rows_affected())
}

async fn live_snapshot(state: &AppState) -> ApiResult<Value> {
    let row: (i64, i64, f64, i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(SUM(COALESCE(total_tokens, 0)), 0), \
         CAST(COALESCE(AVG(ttft_ms), 0) AS REAL), COALESCE(SUM(account_switches), 0), \
         COALESCE(SUM(upstream_attempts), 0) FROM usage_logs \
         WHERE datetime(created_at) >= datetime('now', '-60 seconds')",
    )
    .fetch_one(&state.pool)
    .await?;
    Ok(json!({
        "generated_at": Utc::now().to_rfc3339(),
        "qps": row.0 as f64 / 60.0,
        "tps": row.1 as f64 / 60.0,
        "average_ttft_ms": row.2.round() as i64,
        "account_switches": row.3,
        "upstream_attempts": row.4,
        "active_gateway_requests": state.active_request_count()
    }))
}

async fn live_metrics(State(state): State<AppState>) -> ApiResult<Response> {
    let stream = async_stream::stream! {
        let mut interval = tokio::time::interval(Duration::from_secs(2));
        loop {
            interval.tick().await;
            let payload = match live_snapshot(&state).await {
                Ok(value) => json!({"data": value}),
                Err(error) => json!({"error": {"code": error.code, "message": error.message}}),
            };
            let event = format!("event: metrics\ndata: {payload}\n\n");
            yield Ok::<Bytes, Infallible>(Bytes::from(event));
        }
    };
    let mut response = Response::new(Body::from_stream(stream));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream; charset=utf-8"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
        .headers_mut()
        .insert(header::CONNECTION, HeaderValue::from_static("keep-alive"));
    Ok(response)
}

async fn overview(
    State(state): State<AppState>,
    Query(query): Query<OverviewQuery>,
) -> ApiResult<Json<Value>> {
    let range = parse_range(&query.range)?;
    let modifier = range_modifier(range);
    let model = query
        .model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let totals: (i64, i64, i64, i64, i64, i64, f64, i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), \
         COALESCE(SUM(CASE WHEN status_code < 400 THEN 1 ELSE 0 END), 0), \
         COALESCE(SUM(CASE WHEN status_code >= 400 THEN 1 ELSE 0 END), 0), \
         COALESCE(SUM(CASE WHEN status_code = 429 THEN 1 ELSE 0 END), 0), \
         COALESCE(SUM(CASE WHEN status_code >= 500 THEN 1 ELSE 0 END), 0), \
         COALESCE(SUM(COALESCE(total_tokens, 0)), 0), \
         CAST(COALESCE(AVG(duration_ms), 0) AS REAL), COALESCE(MAX(duration_ms), 0), \
         COALESCE(SUM(cost_microusd), 0) FROM usage_logs \
         WHERE datetime(created_at) >= datetime('now', ?) \
         AND (? IS NULL OR model LIKE '%' || ? || '%') \
         AND (? IS NULL OR account_id = ?)",
    )
    .bind(&modifier)
    .bind(&model)
    .bind(&model)
    .bind(query.account_id)
    .bind(query.account_id)
    .fetch_one(&state.pool)
    .await?;
    let durations: Vec<i64> = sqlx::query_scalar(
        "SELECT duration_ms FROM usage_logs WHERE datetime(created_at) >= datetime('now', ?) \
         AND (? IS NULL OR model LIKE '%' || ? || '%') AND (? IS NULL OR account_id = ?) \
         ORDER BY duration_ms LIMIT 100000",
    )
    .bind(&modifier)
    .bind(&model)
    .bind(&model)
    .bind(query.account_id)
    .bind(query.account_id)
    .fetch_all(&state.pool)
    .await?;
    let use_rollup = range.seconds >= 86_400 && model.is_none() && query.account_id.is_none();
    let trend: Vec<(String, i64, i64, i64, i64)> = if use_rollup {
        rollup_usage(&state, &modifier).await?;
        sqlx::query_as(
            "SELECT strftime(?, bucket), COALESCE(SUM(requests), 0), \
             COALESCE(SUM(tokens), 0), COALESCE(SUM(errors), 0), \
             COALESCE(SUM(cost_microusd), 0) FROM ops_minute_rollups \
             WHERE datetime(bucket) >= datetime('now', ?) GROUP BY strftime(?, bucket) ORDER BY bucket",
        )
        .bind(range.bucket_format)
        .bind(&modifier)
        .bind(range.bucket_format)
        .fetch_all(&state.pool)
        .await?
    } else {
        sqlx::query_as(
            "SELECT strftime(?, created_at) AS bucket, COUNT(*), \
             COALESCE(SUM(COALESCE(total_tokens, 0)), 0), \
             COALESCE(SUM(CASE WHEN status_code >= 400 THEN 1 ELSE 0 END), 0), \
             COALESCE(SUM(cost_microusd), 0) FROM usage_logs \
             WHERE datetime(created_at) >= datetime('now', ?) \
             AND (? IS NULL OR model LIKE '%' || ? || '%') AND (? IS NULL OR account_id = ?) \
             GROUP BY bucket ORDER BY bucket",
        )
        .bind(range.bucket_format)
        .bind(&modifier)
        .bind(&model)
        .bind(&model)
        .bind(query.account_id)
        .bind(query.account_id)
        .fetch_all(&state.pool)
        .await?
    };
    let telemetry: (f64, i64, i64, i64, f64) = sqlx::query_as(
        "SELECT CAST(COALESCE(AVG(ttft_ms), 0) AS REAL), \
         COALESCE(SUM(upstream_attempts), 0), COALESCE(SUM(account_switches), 0), \
         COALESCE(SUM(CASE WHEN stream = 1 THEN 1 ELSE 0 END), 0), \
         CAST(COALESCE(AVG(CASE WHEN stream = 1 THEN duration_ms END), 0) AS REAL) \
         FROM usage_logs WHERE datetime(created_at) >= datetime('now', ?) \
         AND (? IS NULL OR model LIKE '%' || ? || '%') AND (? IS NULL OR account_id = ?)",
    )
    .bind(&modifier)
    .bind(&model)
    .bind(&model)
    .bind(query.account_id)
    .bind(query.account_id)
    .fetch_one(&state.pool)
    .await?;
    let recent: (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(SUM(COALESCE(total_tokens, 0)), 0) FROM usage_logs \
         WHERE datetime(created_at) >= datetime('now', '-60 seconds')",
    )
    .fetch_one(&state.pool)
    .await?;
    let models: Vec<(String, i64, i64, i64, f64)> = sqlx::query_as(
        "SELECT COALESCE(NULLIF(model, ''), 'unknown'), COUNT(*), \
         COALESCE(SUM(COALESCE(total_tokens, 0)), 0), COALESCE(SUM(cost_microusd), 0), \
         CAST(COALESCE(AVG(duration_ms), 0) AS REAL) FROM usage_logs \
         WHERE datetime(created_at) >= datetime('now', ?) AND (? IS NULL OR account_id = ?) \
         GROUP BY COALESCE(NULLIF(model, ''), 'unknown') ORDER BY COUNT(*) DESC LIMIT 20",
    )
    .bind(&modifier)
    .bind(query.account_id)
    .bind(query.account_id)
    .fetch_all(&state.pool)
    .await?;
    let errors: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT status_code, COUNT(*) FROM usage_logs \
         WHERE datetime(created_at) >= datetime('now', ?) AND status_code >= 400 \
         AND (? IS NULL OR model LIKE '%' || ? || '%') AND (? IS NULL OR account_id = ?) \
         GROUP BY status_code ORDER BY COUNT(*) DESC",
    )
    .bind(&modifier)
    .bind(&model)
    .bind(&model)
    .bind(query.account_id)
    .bind(query.account_id)
    .fetch_all(&state.pool)
    .await?;
    let accounts: Vec<(
        i64,
        String,
        String,
        bool,
        i64,
        Option<String>,
        Option<String>,
        Option<String>,
        i64,
        i64,
    )> = sqlx::query_as(
        "SELECT accounts.id, accounts.name, accounts.kind, accounts.enabled, accounts.concurrency, \
         accounts.cooldown_until, accounts.last_used_at, accounts.last_error, \
         (SELECT COUNT(*) FROM usage_logs WHERE usage_logs.account_id = accounts.id AND \
           datetime(usage_logs.created_at) >= datetime('now', ?)), \
         (SELECT COUNT(*) FROM usage_logs WHERE usage_logs.account_id = accounts.id AND \
           datetime(usage_logs.created_at) >= datetime('now', ?) AND usage_logs.status_code >= 400) \
         FROM accounts ORDER BY accounts.priority, accounts.id",
    )
    .bind(&modifier)
    .bind(&modifier)
    .fetch_all(&state.pool)
    .await?;
    let latency = latency_summary(&durations, totals.6, totals.7);
    let process = process_metrics(&state);
    let request_count = totals.0.max(0) as f64;
    let success_rate = if request_count > 0.0 {
        totals.1 as f64 * 100.0 / request_count
    } else {
        100.0
    };
    let error_rate = if request_count > 0.0 {
        totals.2 as f64 * 100.0 / request_count
    } else {
        0.0
    };
    let peak_qps = trend
        .iter()
        .map(|row| row.1 as f64 / range.bucket_seconds)
        .fold(0.0_f64, f64::max);
    let peak_tps = trend
        .iter()
        .map(|row| row.2 as f64 / range.bucket_seconds)
        .fold(0.0_f64, f64::max);
    let available_accounts = accounts
        .iter()
        .filter(|row| row.3 && !cooldown_active(row.5.as_deref()))
        .count();
    let health_score = ((success_rate * 0.8)
        + if accounts.is_empty() || available_accounts > 0 {
            20.0
        } else {
            0.0
        })
    .round()
    .clamp(0.0, 100.0);

    Ok(Json(json!({"data": {
        "generated_at": Utc::now().to_rfc3339(), "range": range.label,
        "start_time": (Utc::now() - chrono::Duration::seconds(range.seconds)).to_rfc3339(),
        "end_time": Utc::now().to_rfc3339(), "health_score": health_score,
        "summary": {
            "request_count": totals.0, "success_count": totals.1, "error_count": totals.2,
            "business_limited_count": totals.3, "upstream_error_count": totals.4,
            "token_count": totals.5, "cost_microusd": totals.8,
            "success_rate": success_rate, "error_rate": error_rate,
            "qps": {"current": recent.0 as f64 / 60.0, "peak": peak_qps,
                "average": totals.0 as f64 / range.seconds as f64},
            "tps": {"current": recent.1 as f64 / 60.0, "peak": peak_tps,
                "average": totals.5 as f64 / range.seconds as f64},
            "telemetry": {
                "average_ttft_ms": telemetry.0.round() as i64,
                "upstream_attempts": telemetry.1,
                "account_switches": telemetry.2,
                "switch_rate": if totals.0 > 0 { telemetry.2 as f64 * 100.0 / totals.0 as f64 } else { 0.0 },
                "stream_requests": telemetry.3,
                "average_stream_duration_ms": telemetry.4.round() as i64
            },
            "latency": latency
        },
        "preaggregated_trend": use_rollup,
        "latency_histogram": latency_histogram(&durations),
        "trend": trend.into_iter().map(|row| json!({
            "bucket": row.0, "requests": row.1, "tokens": row.2,
            "errors": row.3, "cost_microusd": row.4,
            "qps": row.1 as f64 / range.bucket_seconds,
            "tps": row.2 as f64 / range.bucket_seconds
        })).collect::<Vec<_>>(),
        "models": models.into_iter().map(|row| json!({
            "model": row.0, "requests": row.1, "tokens": row.2,
            "cost_microusd": row.3, "average_duration_ms": row.4.round() as i64
        })).collect::<Vec<_>>(),
        "errors": errors.into_iter().map(|row| json!({"status_code": row.0, "count": row.1})).collect::<Vec<_>>(),
        "accounts": accounts.into_iter().map(|row| json!({
            "id": row.0, "name": row.1, "kind": row.2, "enabled": row.3,
            "concurrency": row.4, "cooldown_until": row.5, "last_used_at": row.6,
            "last_error": row.7, "requests": row.8, "errors": row.9,
            "available": row.3 && !cooldown_active(row.5.as_deref())
        })).collect::<Vec<_>>(),
        "system": process
    }})))
}

fn percentile(values: &[i64], percentile: f64) -> i64 {
    if values.is_empty() {
        return 0;
    }
    let index = ((values.len() - 1) as f64 * percentile).round() as usize;
    values[index]
}

fn latency_summary(values: &[i64], average: f64, maximum: i64) -> Value {
    json!({
        "p50_ms": percentile(values, 0.50), "p90_ms": percentile(values, 0.90),
        "p95_ms": percentile(values, 0.95), "p99_ms": percentile(values, 0.99),
        "average_ms": average.round() as i64, "maximum_ms": maximum,
        "sample_size": values.len(), "sample_capped": values.len() == 100000
    })
}

fn latency_histogram(values: &[i64]) -> Vec<Value> {
    let ranges = [
        ("<250ms", i64::MIN, 250),
        ("250-500ms", 250, 500),
        ("500ms-1s", 500, 1_000),
        ("1-3s", 1_000, 3_000),
        ("3-10s", 3_000, 10_000),
        (">=10s", 10_000, i64::MAX),
    ];
    ranges
        .into_iter()
        .map(|(label, start, end)| {
            json!({"range": label, "count": values.iter().filter(|value| **value >= start && **value < end).count()})
        })
        .collect()
}

fn cooldown_active(value: Option<&str>) -> bool {
    value
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .is_some_and(|value| value > Utc::now())
}

fn process_metrics(state: &AppState) -> Value {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let rss_kb = proc_value(&status, "VmRSS:");
    let threads = proc_value(&status, "Threads:");
    let cgroup_bytes = std::fs::read_to_string("/sys/fs/cgroup/memory.current")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok());
    json!({
        "uptime_seconds": state.started_at.elapsed().as_secs(),
        "rss_kb": rss_kb, "threads": threads,
        "cgroup_memory_bytes": cgroup_bytes,
        "db_pool_size": state.pool.size(), "db_idle_connections": state.pool.num_idle(),
        "active_gateway_requests": state.active_request_count()
    })
}

fn proc_value(status: &str, key: &str) -> u64 {
    status
        .lines()
        .find(|line| line.starts_with(key))
        .and_then(|line| line.split_ascii_whitespace().nth(1))
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}

#[derive(Debug, Deserialize)]
struct RequestQuery {
    #[serde(default = "default_page")]
    page: i64,
    #[serde(default = "default_page_size")]
    page_size: i64,
    #[serde(default = "default_range")]
    range: String,
    #[serde(default = "default_kind")]
    kind: String,
    model: Option<String>,
    request_id: Option<String>,
    user_id: Option<i64>,
    api_key_id: Option<i64>,
    account_id: Option<i64>,
    min_duration_ms: Option<i64>,
}

fn default_page() -> i64 {
    1
}
fn default_page_size() -> i64 {
    50
}
fn default_kind() -> String {
    "all".into()
}

#[derive(Debug, Serialize, FromRow)]
struct RequestRow {
    id: i64,
    request_id: String,
    api_key_id: Option<i64>,
    api_key_name: Option<String>,
    account_id: Option<i64>,
    account_name: Option<String>,
    user_id: Option<i64>,
    username: Option<String>,
    endpoint: String,
    model: Option<String>,
    status_code: i32,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    total_tokens: Option<i64>,
    cost_microusd: i64,
    duration_ms: i64,
    ttft_ms: Option<i64>,
    upstream_attempts: i64,
    account_switches: i64,
    error_summary: Option<String>,
    created_at: String,
}

async fn list_requests(
    State(state): State<AppState>,
    Query(query): Query<RequestQuery>,
) -> ApiResult<Json<Value>> {
    let range = parse_range(&query.range)?;
    if !matches!(query.kind.as_str(), "all" | "success" | "error") {
        return Err(ApiError::bad_request(
            "INVALID_REQUEST_KIND",
            "kind must be all, success, or error",
        ));
    }
    let page = query.page.max(1);
    let page_size = query.page_size.clamp(1, 200);
    let modifier = range_modifier(range);
    let mut count = QueryBuilder::<Sqlite>::new(
        "SELECT COUNT(*) FROM usage_logs logs WHERE datetime(logs.created_at) >= datetime('now', ",
    );
    count.push_bind(&modifier).push(")");
    push_request_filters(&mut count, &query);
    let total: i64 = count.build_query_scalar().fetch_one(&state.pool).await?;

    let mut rows = QueryBuilder::<Sqlite>::new(
        "SELECT logs.id, logs.request_id, logs.api_key_id, keys.name AS api_key_name, \
         logs.account_id, accounts.name AS account_name, logs.user_id, users.username, \
         logs.endpoint, logs.model, logs.status_code, logs.input_tokens, logs.output_tokens, \
         logs.total_tokens, logs.cost_microusd, logs.duration_ms, logs.ttft_ms, \
         logs.upstream_attempts, logs.account_switches, logs.error_summary, logs.created_at \
         FROM usage_logs logs LEFT JOIN api_keys keys ON keys.id = logs.api_key_id \
         LEFT JOIN accounts ON accounts.id = logs.account_id LEFT JOIN users ON users.id = logs.user_id \
         WHERE datetime(logs.created_at) >= datetime('now', ",
    );
    rows.push_bind(&modifier).push(")");
    push_request_filters(&mut rows, &query);
    rows.push(" ORDER BY logs.id DESC LIMIT ")
        .push_bind(page_size)
        .push(" OFFSET ")
        .push_bind((page - 1) * page_size);
    let rows = rows
        .build_query_as::<RequestRow>()
        .fetch_all(&state.pool)
        .await?;
    Ok(Json(json!({"data": rows, "meta": {
        "page": page, "page_size": page_size, "total": total
    }})))
}

fn push_request_filters(builder: &mut QueryBuilder<'_, Sqlite>, query: &RequestQuery) {
    match query.kind.as_str() {
        "success" => builder.push(" AND logs.status_code < 400"),
        "error" => builder.push(" AND logs.status_code >= 400"),
        _ => builder,
    };
    if let Some(model) = query
        .model
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        builder
            .push(" AND logs.model LIKE ")
            .push_bind(format!("%{model}%"));
    }
    if let Some(request_id) = query
        .request_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        builder
            .push(" AND logs.request_id LIKE ")
            .push_bind(format!("%{request_id}%"));
    }
    for (column, value) in [
        ("logs.user_id", query.user_id),
        ("logs.api_key_id", query.api_key_id),
        ("logs.account_id", query.account_id),
    ] {
        if let Some(value) = value {
            builder
                .push(" AND ")
                .push(column)
                .push(" = ")
                .push_bind(value);
        }
    }
    if let Some(value) = query.min_duration_ms.filter(|value| *value >= 0) {
        builder.push(" AND logs.duration_ms >= ").push_bind(value);
    }
}

async fn request_detail(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<Value>> {
    let row = sqlx::query_as::<_, RequestRow>(
        "SELECT logs.id, logs.request_id, logs.api_key_id, keys.name AS api_key_name, \
         logs.account_id, accounts.name AS account_name, logs.user_id, users.username, \
         logs.endpoint, logs.model, logs.status_code, logs.input_tokens, logs.output_tokens, \
         logs.total_tokens, logs.cost_microusd, logs.duration_ms, logs.ttft_ms, \
         logs.upstream_attempts, logs.account_switches, logs.error_summary, logs.created_at \
         FROM usage_logs logs LEFT JOIN api_keys keys ON keys.id = logs.api_key_id \
         LEFT JOIN accounts ON accounts.id = logs.account_id LEFT JOIN users ON users.id = logs.user_id \
         WHERE logs.id = ?",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| ApiError::not_found("request log not found"))?;
    Ok(Json(json!({"data": row})))
}

#[derive(Deserialize)]
struct SystemLogQuery {
    #[serde(default = "default_page_size")]
    limit: i64,
    level: Option<String>,
}

async fn system_logs(
    State(state): State<AppState>,
    Query(query): Query<SystemLogQuery>,
) -> ApiResult<Json<Value>> {
    let limit = query.limit.clamp(1, 200) as usize;
    let level = query.level.as_deref().unwrap_or("all");
    if !matches!(level, "all" | "trace" | "debug" | "info" | "warn" | "error") {
        return Err(ApiError::bad_request(
            "INVALID_LOG_LEVEL",
            "level must be all, trace, debug, info, warn, or error",
        ));
    }
    let audit: Vec<(i64, String, String, String, String, Option<i64>, String)> = sqlx::query_as(
        "SELECT id, created_at, action, method, path, user_id, request_id \
         FROM audit_logs ORDER BY id DESC LIMIT ?",
    )
    .bind(limit as i64)
    .fetch_all(&state.pool)
    .await?;
    let errors: Vec<(i64, String, i32, String, Option<String>, String)> = sqlx::query_as(
        "SELECT id, created_at, status_code, endpoint, error_summary, request_id \
         FROM usage_logs WHERE status_code >= 400 ORDER BY id DESC LIMIT ?",
    )
    .bind(limit as i64)
    .fetch_all(&state.pool)
    .await?;
    let runtime: Vec<(i64, String, String, String, Option<String>, String, String)> =
        sqlx::query_as(
            "SELECT id, level, target, message, request_id, fields_json, created_at \
             FROM runtime_logs WHERE (? = 'all' OR level = ?) ORDER BY id DESC LIMIT ?",
        )
        .bind(level)
        .bind(level)
        .bind(limit as i64)
        .fetch_all(&state.pool)
        .await?;
    let mut rows = audit
        .into_iter()
        .filter(|_| matches!(level, "all" | "info"))
        .map(|row| {
            json!({"id": format!("audit-{}", row.0), "created_at": row.1, "level": "info",
                "source": "audit", "message": format!("{} {} {}", row.3, row.4, row.2),
                "actor_user_id": row.5, "request_id": row.6})
        })
        .chain(errors.into_iter().filter(|_| matches!(level, "all" | "error")).map(|row| {
            json!({"id": format!("gateway-{}", row.0), "created_at": row.1, "level": "error",
                "source": "gateway", "message": row.4.unwrap_or_else(|| format!("HTTP {}", row.2)),
                "status_code": row.2, "endpoint": row.3, "request_id": row.5})
        }))
        .chain(runtime.into_iter().map(|row| {
            let fields = serde_json::from_str::<Value>(&row.5).unwrap_or_else(|_| json!({}));
            json!({"id": format!("runtime-{}", row.0), "created_at": row.6, "level": row.1,
                "source": "runtime", "target": row.2, "message": row.3,
                "request_id": row.4, "fields": fields})
        }))
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        right["created_at"]
            .as_str()
            .cmp(&left["created_at"].as_str())
    });
    rows.truncate(limit);
    Ok(Json(json!({"data": rows})))
}

#[derive(Debug, Serialize, FromRow)]
struct AlertRule {
    id: i64,
    name: String,
    description: String,
    enabled: bool,
    metric_type: String,
    operator: String,
    threshold: f64,
    window_minutes: i64,
    severity: String,
    cooldown_minutes: i64,
    notify_email: bool,
    last_triggered_at: Option<String>,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, Deserialize)]
struct AlertRuleInput {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default = "default_true")]
    enabled: bool,
    metric_type: String,
    operator: String,
    threshold: f64,
    #[serde(default = "default_window")]
    window_minutes: i64,
    #[serde(default = "default_severity")]
    severity: String,
    #[serde(default = "default_cooldown")]
    cooldown_minutes: i64,
    #[serde(default)]
    notify_email: bool,
}

fn default_true() -> bool {
    true
}
fn default_window() -> i64 {
    5
}
fn default_severity() -> String {
    "warning".into()
}
fn default_cooldown() -> i64 {
    15
}

fn validate_alert_rule(input: &AlertRuleInput) -> ApiResult<()> {
    if input.name.trim().is_empty()
        || input.name.chars().count() > 120
        || input.description.chars().count() > 1_000
        || !matches!(
            input.metric_type.as_str(),
            "success_rate"
                | "error_rate"
                | "upstream_error_rate"
                | "request_count"
                | "token_count"
                | "latency_p95_ms"
                | "active_requests"
                | "available_accounts"
        )
        || !matches!(
            input.operator.as_str(),
            ">" | ">=" | "<" | "<=" | "==" | "!="
        )
        || !input.threshold.is_finite()
        || !(1..=1_440).contains(&input.window_minutes)
        || !matches!(input.severity.as_str(), "info" | "warning" | "critical")
        || !(1..=10_080).contains(&input.cooldown_minutes)
    {
        return Err(ApiError::bad_request(
            "INVALID_ALERT_RULE",
            "alert rule fields are invalid",
        ));
    }
    Ok(())
}

async fn list_alert_rules(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let rows = sqlx::query_as::<_, AlertRule>(
        "SELECT id, name, description, enabled, metric_type, operator, threshold, \
         window_minutes, severity, cooldown_minutes, notify_email, last_triggered_at, \
         created_at, updated_at FROM ops_alert_rules ORDER BY enabled DESC, id DESC",
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(json!({"data": rows})))
}

async fn create_alert_rule(
    State(state): State<AppState>,
    Json(input): Json<AlertRuleInput>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    validate_alert_rule(&input)?;
    let result = sqlx::query(
        "INSERT INTO ops_alert_rules (name, description, enabled, metric_type, operator, \
         threshold, window_minutes, severity, cooldown_minutes, notify_email) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(input.name.trim())
    .bind(input.description.trim())
    .bind(input.enabled)
    .bind(input.metric_type)
    .bind(input.operator)
    .bind(input.threshold)
    .bind(input.window_minutes)
    .bind(input.severity)
    .bind(input.cooldown_minutes)
    .bind(input.notify_email)
    .execute(&state.pool)
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"data": {"id": result.last_insert_rowid()}})),
    ))
}

async fn update_alert_rule(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(input): Json<AlertRuleInput>,
) -> ApiResult<Json<Value>> {
    validate_alert_rule(&input)?;
    let result = sqlx::query(
        "UPDATE ops_alert_rules SET name = ?, description = ?, enabled = ?, metric_type = ?, \
         operator = ?, threshold = ?, window_minutes = ?, severity = ?, cooldown_minutes = ?, \
         notify_email = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
    )
    .bind(input.name.trim())
    .bind(input.description.trim())
    .bind(input.enabled)
    .bind(input.metric_type)
    .bind(input.operator)
    .bind(input.threshold)
    .bind(input.window_minutes)
    .bind(input.severity)
    .bind(input.cooldown_minutes)
    .bind(input.notify_email)
    .bind(id)
    .execute(&state.pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found("alert rule not found"));
    }
    Ok(Json(json!({"data": {"id": id}})))
}

async fn delete_alert_rule(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    let result = sqlx::query("DELETE FROM ops_alert_rules WHERE id = ?")
        .bind(id)
        .execute(&state.pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found("alert rule not found"));
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Serialize, FromRow)]
struct AlertEvent {
    id: i64,
    rule_id: i64,
    rule_name: String,
    severity: String,
    status: String,
    title: String,
    description: String,
    metric_value: f64,
    threshold_value: f64,
    email_sent: bool,
    fired_at: String,
    resolved_at: Option<String>,
    created_at: String,
}

#[derive(Deserialize)]
struct AlertEventQuery {
    status: Option<String>,
    #[serde(default = "default_page_size")]
    limit: i64,
}

async fn list_alert_events(
    State(state): State<AppState>,
    Query(query): Query<AlertEventQuery>,
) -> ApiResult<Json<Value>> {
    let rows = sqlx::query_as::<_, AlertEvent>(
        "SELECT events.id, events.rule_id, rules.name AS rule_name, events.severity, events.status, \
         events.title, events.description, events.metric_value, events.threshold_value, \
         events.email_sent, events.fired_at, events.resolved_at, events.created_at \
         FROM ops_alert_events events JOIN ops_alert_rules rules ON rules.id = events.rule_id \
         WHERE (? IS NULL OR events.status = ?) ORDER BY events.id DESC LIMIT ?",
    )
    .bind(&query.status)
    .bind(&query.status)
    .bind(query.limit.clamp(1, 200))
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(json!({"data": rows})))
}

#[derive(Deserialize)]
struct AlertStatusInput {
    status: String,
}

async fn update_alert_status(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(input): Json<AlertStatusInput>,
) -> ApiResult<Json<Value>> {
    if !matches!(input.status.as_str(), "resolved" | "manual_resolved") {
        return Err(ApiError::bad_request(
            "INVALID_ALERT_STATUS",
            "status must be resolved or manual_resolved",
        ));
    }
    let result = sqlx::query(
        "UPDATE ops_alert_events SET status = ?, resolved_at = CURRENT_TIMESTAMP \
         WHERE id = ? AND status = 'firing'",
    )
    .bind(input.status)
    .bind(id)
    .execute(&state.pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found("firing alert event not found"));
    }
    Ok(Json(json!({"data": {"id": id}})))
}

async fn evaluate_now(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    Ok(Json(json!({"data": evaluate_alerts(&state).await?})))
}

async fn evaluate_alerts(state: &AppState) -> ApiResult<Value> {
    let rules = sqlx::query_as::<_, AlertRule>(
        "SELECT id, name, description, enabled, metric_type, operator, threshold, \
         window_minutes, severity, cooldown_minutes, notify_email, last_triggered_at, \
         created_at, updated_at FROM ops_alert_rules WHERE enabled = 1 ORDER BY id",
    )
    .fetch_all(&state.pool)
    .await?;
    let mut fired = 0;
    let mut resolved = 0;
    for rule in rules {
        let value = alert_metric(state, &rule.metric_type, rule.window_minutes).await?;
        let matches = compare(value, &rule.operator, rule.threshold);
        let firing: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM ops_alert_events WHERE rule_id = ? AND status = 'firing' \
             ORDER BY id DESC LIMIT 1",
        )
        .bind(rule.id)
        .fetch_optional(&state.pool)
        .await?;
        if matches && firing.is_none() && cooldown_elapsed(&rule) {
            let title = format!(
                "{}: {} {} {}",
                rule.name, rule.metric_type, rule.operator, rule.threshold
            );
            let description = format!(
                "{} is {:.3} for the last {} minutes",
                rule.metric_type, value, rule.window_minutes
            );
            let result = sqlx::query(
                "INSERT INTO ops_alert_events (rule_id, severity, title, description, \
                 metric_value, threshold_value) VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(rule.id)
            .bind(&rule.severity)
            .bind(&title)
            .bind(&description)
            .bind(value)
            .bind(rule.threshold)
            .execute(&state.pool)
            .await?;
            sqlx::query(
                "UPDATE ops_alert_rules SET last_triggered_at = CURRENT_TIMESTAMP, \
                 updated_at = CURRENT_TIMESTAMP WHERE id = ?",
            )
            .bind(rule.id)
            .execute(&state.pool)
            .await?;
            if rule.notify_email
                && deliver_alert_email(state, &title, &description, &rule.severity)
                    .await
                    .unwrap_or(false)
            {
                sqlx::query("UPDATE ops_alert_events SET email_sent = 1 WHERE id = ?")
                    .bind(result.last_insert_rowid())
                    .execute(&state.pool)
                    .await?;
            }
            fired += 1;
        } else if !matches {
            if let Some(event_id) = firing {
                sqlx::query(
                    "UPDATE ops_alert_events SET status = 'resolved', resolved_at = CURRENT_TIMESTAMP \
                     WHERE id = ?",
                )
                .bind(event_id)
                .execute(&state.pool)
                .await?;
                resolved += 1;
            }
        }
    }
    Ok(json!({"fired": fired, "resolved": resolved, "evaluated_at": Utc::now().to_rfc3339()}))
}

fn cooldown_elapsed(rule: &AlertRule) -> bool {
    rule.last_triggered_at
        .as_deref()
        .and_then(parse_timestamp)
        .is_none_or(|value| value + chrono::Duration::minutes(rule.cooldown_minutes) <= Utc::now())
}

fn parse_timestamp(value: &str) -> Option<chrono::DateTime<Utc>> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .ok()
        .or_else(|| {
            chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S")
                .map(|value| value.and_utc())
                .ok()
        })
}

fn compare(value: f64, operator: &str, threshold: f64) -> bool {
    match operator {
        ">" => value > threshold,
        ">=" => value >= threshold,
        "<" => value < threshold,
        "<=" => value <= threshold,
        "==" => (value - threshold).abs() < f64::EPSILON,
        "!=" => (value - threshold).abs() >= f64::EPSILON,
        _ => false,
    }
}

async fn alert_metric(state: &AppState, metric: &str, window_minutes: i64) -> ApiResult<f64> {
    if metric == "active_requests" {
        return Ok(state.active_request_count() as f64);
    }
    if metric == "available_accounts" {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM accounts WHERE enabled = 1 AND \
             (cooldown_until IS NULL OR datetime(cooldown_until) <= CURRENT_TIMESTAMP)",
        )
        .fetch_one(&state.pool)
        .await?;
        return Ok(count as f64);
    }
    let modifier = format!("-{window_minutes} minutes");
    let row: (i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(SUM(CASE WHEN status_code < 400 THEN 1 ELSE 0 END), 0), \
         COALESCE(SUM(CASE WHEN status_code >= 400 THEN 1 ELSE 0 END), 0), \
         COALESCE(SUM(CASE WHEN status_code >= 500 THEN 1 ELSE 0 END), 0), \
         COALESCE(SUM(COALESCE(total_tokens, 0)), 0) FROM usage_logs \
         WHERE datetime(created_at) >= datetime('now', ?)",
    )
    .bind(&modifier)
    .fetch_one(&state.pool)
    .await?;
    match metric {
        "request_count" => Ok(row.0 as f64),
        "token_count" => Ok(row.4 as f64),
        "success_rate" => Ok(if row.0 > 0 {
            row.1 as f64 * 100.0 / row.0 as f64
        } else {
            100.0
        }),
        "error_rate" => Ok(if row.0 > 0 {
            row.2 as f64 * 100.0 / row.0 as f64
        } else {
            0.0
        }),
        "upstream_error_rate" => Ok(if row.0 > 0 {
            row.3 as f64 * 100.0 / row.0 as f64
        } else {
            0.0
        }),
        "latency_p95_ms" => {
            let values: Vec<i64> = sqlx::query_scalar(
                "SELECT duration_ms FROM usage_logs WHERE datetime(created_at) >= datetime('now', ?) \
                 ORDER BY duration_ms LIMIT 100000",
            )
            .bind(&modifier)
            .fetch_all(&state.pool)
            .await?;
            Ok(percentile(&values, 0.95) as f64)
        }
        _ => Err(ApiError::internal("stored alert metric is invalid")),
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
struct OpsSettings {
    auto_refresh_seconds: u64,
    alert_recipients: Vec<String>,
    email_enabled: bool,
    request_retention_days: i64,
    report_recipients: Vec<String>,
    daily_report_enabled: bool,
    daily_report_cron: String,
    weekly_report_enabled: bool,
    weekly_report_cron: String,
}

impl Default for OpsSettings {
    fn default() -> Self {
        Self {
            auto_refresh_seconds: 10,
            alert_recipients: Vec::new(),
            email_enabled: false,
            request_retention_days: 90,
            report_recipients: Vec::new(),
            daily_report_enabled: false,
            daily_report_cron: "0 9 * * *".into(),
            weekly_report_enabled: false,
            weekly_report_cron: "0 9 * * 1".into(),
        }
    }
}

async fn load_settings(state: &AppState) -> ApiResult<OpsSettings> {
    let value: Option<String> =
        sqlx::query_scalar("SELECT value FROM app_settings WHERE key = 'ops_settings'")
            .fetch_optional(&state.pool)
            .await?;
    value
        .map(|value| {
            serde_json::from_str(&value)
                .map_err(|_| ApiError::internal("stored ops settings are malformed"))
        })
        .transpose()
        .map(|value| value.unwrap_or_default())
}

async fn get_settings(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let settings = load_settings(&state).await?;
    let mail_configured = crate::mail::is_configured(&state).await?;
    let runtime_log = runtime_log_config(&state).await?;
    Ok(Json(json!({"data": {
        "auto_refresh_seconds": settings.auto_refresh_seconds,
        "alert_recipients": settings.alert_recipients,
        "email_enabled": settings.email_enabled,
        "request_retention_days": settings.request_retention_days,
        "report_recipients": settings.report_recipients,
        "daily_report_enabled": settings.daily_report_enabled,
        "daily_report_cron": settings.daily_report_cron,
        "weekly_report_enabled": settings.weekly_report_enabled,
        "weekly_report_cron": settings.weekly_report_cron,
        "runtime_log": runtime_log,
        "mail_configured": mail_configured
    }})))
}

async fn update_settings(
    State(state): State<AppState>,
    Json(mut input): Json<OpsSettings>,
) -> ApiResult<Json<Value>> {
    if !(5..=300).contains(&input.auto_refresh_seconds)
        || !(1..=3_650).contains(&input.request_retention_days)
        || input.alert_recipients.len() > 20
        || input.report_recipients.len() > 20
    {
        return Err(ApiError::bad_request(
            "INVALID_OPS_SETTINGS",
            "ops settings are outside the supported range",
        ));
    }
    input.alert_recipients = input
        .alert_recipients
        .into_iter()
        .map(|email| auth::normalize_email(&email))
        .collect::<ApiResult<Vec<_>>>()?;
    input.alert_recipients.sort();
    input.alert_recipients.dedup();
    input.report_recipients = input
        .report_recipients
        .into_iter()
        .map(|email| auth::normalize_email(&email))
        .collect::<ApiResult<Vec<_>>>()?;
    input.report_recipients.sort();
    input.report_recipients.dedup();
    input.daily_report_cron = validate_report_cron(&input.daily_report_cron)?;
    input.weekly_report_cron = validate_report_cron(&input.weekly_report_cron)?;
    let reports_enabled = input.daily_report_enabled || input.weekly_report_enabled;
    let mail_configured = crate::mail::is_configured(&state).await?;
    if (input.email_enabled && input.alert_recipients.is_empty())
        || (reports_enabled && input.report_recipients.is_empty())
        || ((input.email_enabled || reports_enabled) && !mail_configured)
    {
        return Err(ApiError::bad_request(
            "MAIL_NOT_CONFIGURED",
            "configure mail delivery and recipients before enabling email notifications",
        ));
    }
    sqlx::query(
        "INSERT INTO app_settings (key, value) VALUES ('ops_settings', ?) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = CURRENT_TIMESTAMP",
    )
    .bind(serde_json::to_string(&input).unwrap())
    .execute(&state.pool)
    .await?;
    get_settings(State(state)).await
}

fn validate_report_cron(value: &str) -> ApiResult<String> {
    let value = value.trim();
    if value.split_ascii_whitespace().count() != 5
        || Schedule::from_str(&format!("0 {value}")).is_err()
    {
        return Err(ApiError::bad_request(
            "INVALID_REPORT_CRON",
            "report schedule must be a valid five-field cron expression",
        ));
    }
    Ok(value.to_string())
}

async fn runtime_log_config(state: &AppState) -> ApiResult<Value> {
    let level: Option<String> =
        sqlx::query_scalar("SELECT value FROM app_settings WHERE key = 'runtime_log_level'")
            .fetch_optional(&state.pool)
            .await?;
    Ok(crate::runtime_log::safe_config_json(
        level.as_deref().unwrap_or("info"),
    ))
}

async fn get_runtime_log_config(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    Ok(Json(json!({"data": runtime_log_config(&state).await?})))
}

#[derive(Deserialize)]
struct RuntimeLogConfigInput {
    level: String,
    db_enabled: bool,
}

async fn update_runtime_log_config(
    State(state): State<AppState>,
    Json(input): Json<RuntimeLogConfigInput>,
) -> ApiResult<Json<Value>> {
    let level = input.level.trim().to_ascii_lowercase();
    crate::runtime_log::set_level(&level)?;
    let mut transaction = state.pool.begin().await?;
    for (key, value) in [
        ("runtime_log_level", level.as_str()),
        (
            "runtime_log_db_enabled",
            if input.db_enabled { "true" } else { "false" },
        ),
    ] {
        sqlx::query(
            "INSERT INTO app_settings(key,value) VALUES(?,?) \
             ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=CURRENT_TIMESTAMP",
        )
        .bind(key)
        .bind(value)
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;
    crate::runtime_log::set_db_enabled(input.db_enabled);
    get_runtime_log_config(State(state)).await
}

#[derive(Deserialize)]
struct ManualReportInput {
    #[serde(default = "default_report_range")]
    range: String,
}

fn default_report_range() -> String {
    "24h".into()
}

async fn list_reports(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let rows: Vec<(
        i64,
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        Option<String>,
    )> = sqlx::query_as(
        "SELECT id,report_type,period_start,period_end,recipients,status,metrics_json, \
             error_summary,created_at,completed_at FROM ops_report_runs ORDER BY id DESC LIMIT 50",
    )
    .fetch_all(&state.pool)
    .await?;
    let rows = rows
        .into_iter()
        .map(|row| {
            json!({
                "id": row.0, "report_type": row.1, "period_start": row.2, "period_end": row.3,
                "recipients": serde_json::from_str::<Value>(&row.4).unwrap_or_else(|_| json!([])),
                "status": row.5,
                "metrics": serde_json::from_str::<Value>(&row.6).unwrap_or_else(|_| json!({})),
                "error_summary": row.7, "created_at": row.8, "completed_at": row.9
            })
        })
        .collect::<Vec<_>>();
    Ok(Json(json!({"data": rows})))
}

async fn run_manual_report(
    State(state): State<AppState>,
    Json(input): Json<ManualReportInput>,
) -> ApiResult<Json<Value>> {
    let seconds = match input.range.as_str() {
        "24h" => 86_400,
        "7d" => 604_800,
        _ => {
            return Err(ApiError::bad_request(
                "INVALID_REPORT_RANGE",
                "report range must be 24h or 7d",
            ));
        }
    };
    let settings = load_settings(&state).await?;
    if settings.report_recipients.is_empty() || !crate::mail::is_configured(&state).await? {
        return Err(ApiError::bad_request(
            "MAIL_NOT_CONFIGURED",
            "configure mail delivery and report recipients first",
        ));
    }
    let end = Utc::now();
    let metrics = run_report(
        &state,
        "manual",
        end - chrono::Duration::seconds(seconds),
        end,
        &settings.report_recipients,
    )
    .await?;
    Ok(Json(json!({"data": metrics})))
}

async fn evaluate_reports(state: &AppState) -> ApiResult<()> {
    let settings = load_settings(state).await?;
    let now = Utc::now();
    for (kind, enabled, expression, seconds) in [
        (
            "daily",
            settings.daily_report_enabled,
            settings.daily_report_cron.as_str(),
            86_400,
        ),
        (
            "weekly",
            settings.weekly_report_enabled,
            settings.weekly_report_cron.as_str(),
            604_800,
        ),
    ] {
        if !enabled {
            continue;
        }
        let expression = validate_report_cron(expression)?;
        let schedule = Schedule::from_str(&format!("0 {expression}"))
            .map_err(|_| ApiError::internal("stored report schedule is invalid"))?;
        let window_start = now - chrono::Duration::seconds(70);
        let Some(scheduled_at) = schedule
            .after(&window_start)
            .next()
            .filter(|value| *value <= now)
        else {
            continue;
        };
        run_report(
            state,
            kind,
            scheduled_at - chrono::Duration::seconds(seconds),
            scheduled_at,
            &settings.report_recipients,
        )
        .await?;
    }
    Ok(())
}

async fn run_report(
    state: &AppState,
    report_type: &str,
    period_start: DateTime<Utc>,
    period_end: DateTime<Utc>,
    recipients: &[String],
) -> ApiResult<Value> {
    let period_start = period_start.to_rfc3339();
    let period_end = period_end.to_rfc3339();
    let recipients_json = serde_json::to_string(recipients).unwrap_or_else(|_| "[]".into());
    let inserted = sqlx::query(
        "INSERT OR IGNORE INTO ops_report_runs \
         (report_type,period_start,period_end,recipients,status) VALUES(?,?,?,?, 'processing')",
    )
    .bind(report_type)
    .bind(&period_start)
    .bind(&period_end)
    .bind(&recipients_json)
    .execute(&state.pool)
    .await?;
    if inserted.rows_affected() == 0 {
        return Ok(json!({"duplicate": true, "report_type": report_type}));
    }
    let run_id = inserted.last_insert_rowid();
    let summary: (i64, i64, i64, i64, i64, f64, f64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(SUM(CASE WHEN status_code < 400 THEN 1 ELSE 0 END),0), \
         COALESCE(SUM(CASE WHEN status_code >= 400 THEN 1 ELSE 0 END),0), \
         COALESCE(SUM(COALESCE(total_tokens,0)),0), COALESCE(SUM(cost_microusd),0), \
         CAST(COALESCE(AVG(duration_ms),0) AS REAL), CAST(COALESCE(AVG(ttft_ms),0) AS REAL), \
         COALESCE(SUM(account_switches),0) FROM usage_logs \
         WHERE datetime(created_at) >= datetime(?) AND datetime(created_at) < datetime(?)",
    )
    .bind(&period_start)
    .bind(&period_end)
    .fetch_one(&state.pool)
    .await?;
    let durations: Vec<i64> = sqlx::query_scalar(
        "SELECT duration_ms FROM usage_logs WHERE datetime(created_at) >= datetime(?) \
         AND datetime(created_at) < datetime(?) ORDER BY duration_ms LIMIT 100000",
    )
    .bind(&period_start)
    .bind(&period_end)
    .fetch_all(&state.pool)
    .await?;
    let metrics = json!({
        "request_count": summary.0, "success_count": summary.1, "error_count": summary.2,
        "token_count": summary.3, "cost_microusd": summary.4,
        "average_duration_ms": summary.5.round() as i64, "p95_duration_ms": percentile(&durations, 0.95),
        "average_ttft_ms": summary.6.round() as i64, "account_switches": summary.7
    });
    let metrics_json = serde_json::to_string(&metrics).unwrap_or_else(|_| "{}".into());
    if recipients.is_empty() {
        sqlx::query(
            "UPDATE ops_report_runs SET status='skipped',metrics_json=?, \
             error_summary='no recipients configured',completed_at=CURRENT_TIMESTAMP WHERE id=?",
        )
        .bind(&metrics_json)
        .bind(run_id)
        .execute(&state.pool)
        .await?;
        return Ok(metrics);
    }
    let subject = format!("Sub2API Mini {report_type} operations report");
    let html = format!(
        "<h2>{}</h2><p>{} - {}</p><ul><li>Requests: {}</li><li>Errors: {}</li><li>Tokens: {}</li><li>P95 latency: {} ms</li><li>Average TTFT: {} ms</li><li>Account switches: {}</li></ul>",
        crate::mail::escape_html(&subject),
        crate::mail::escape_html(&period_start),
        crate::mail::escape_html(&period_end),
        summary.0,
        summary.2,
        summary.3,
        metrics["p95_duration_ms"],
        metrics["average_ttft_ms"],
        summary.7
    );
    for recipient in recipients {
        let body = json!({
            "kind": "ops_report", "to": recipient, "site_name": "Sub2API Mini",
            "report_type": report_type, "period_start": period_start,
            "period_end": period_end, "metrics": metrics
        });
        if let Err(error) = crate::mail::deliver(state, body, recipient, &subject, &html).await {
            sqlx::query(
                "UPDATE ops_report_runs SET status='failed',metrics_json=?,error_summary=?, \
                 completed_at=CURRENT_TIMESTAMP WHERE id=?",
            )
            .bind(&metrics_json)
            .bind(error.message.chars().take(500).collect::<String>())
            .bind(run_id)
            .execute(&state.pool)
            .await?;
            return Err(error);
        }
    }
    sqlx::query(
        "UPDATE ops_report_runs SET status='sent',metrics_json=?,completed_at=CURRENT_TIMESTAMP WHERE id=?",
    )
    .bind(&metrics_json)
    .bind(run_id)
    .execute(&state.pool)
    .await?;
    Ok(metrics)
}

async fn cleanup_ops_data(state: &AppState) -> ApiResult<()> {
    let settings = load_settings(state).await?;
    let modifier = format!("-{} days", settings.request_retention_days);
    sqlx::query("DELETE FROM usage_logs WHERE datetime(created_at) < datetime('now', ?)")
        .bind(&modifier)
        .execute(&state.pool)
        .await?;
    sqlx::query("DELETE FROM ops_minute_rollups WHERE datetime(bucket) < datetime('now', ?)")
        .bind(&modifier)
        .execute(&state.pool)
        .await?;
    sqlx::query(
        "DELETE FROM runtime_logs WHERE datetime(created_at) < datetime('now', '-30 days')",
    )
    .execute(&state.pool)
    .await?;
    Ok(())
}

async fn deliver_alert_email(
    state: &AppState,
    title: &str,
    message: &str,
    severity: &str,
) -> ApiResult<bool> {
    let settings = load_settings(state).await?;
    if !settings.email_enabled || settings.alert_recipients.is_empty() {
        return Ok(false);
    }
    for recipient in settings.alert_recipients {
        let body = json!({
            "kind": "ops_alert", "to": recipient, "site_name": "Sub2API Mini",
            "title": title, "message": message, "severity": severity
        });
        let html = format!(
            "<h2>{}</h2><p>{}</p><p>Severity: <strong>{}</strong></p>",
            crate::mail::escape_html(title),
            crate::mail::escape_html(message),
            crate::mail::escape_html(severity)
        );
        crate::mail::deliver(state, body, &recipient, title, &html).await?;
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;

    #[test]
    fn percentile_and_histogram_are_deterministic() {
        let values = vec![10, 100, 250, 500, 1_000, 3_000, 10_000];
        assert_eq!(percentile(&values, 0.5), 500);
        let histogram = latency_histogram(&values);
        assert_eq!(histogram[0]["count"], 2);
        assert_eq!(histogram[5]["count"], 1);
    }

    #[test]
    fn report_cron_requires_five_fields() {
        assert_eq!(validate_report_cron("0 9 * * *").unwrap(), "0 9 * * *");
        assert_eq!(
            validate_report_cron("0 0 9 * * *").unwrap_err().code,
            "INVALID_REPORT_CRON"
        );
    }

    #[tokio::test]
    async fn rollup_and_live_snapshot_include_gateway_telemetry() {
        let (_directory, state) = test_support::state().await;
        sqlx::query(
            "INSERT INTO usage_logs \
             (request_id,endpoint,status_code,total_tokens,duration_ms,ttft_ms,upstream_attempts,account_switches,stream) \
             VALUES ('telemetry','/v1/responses',200,21,900,120,2,1,1)",
        )
        .execute(&state.pool)
        .await
        .unwrap();
        assert!(rollup_usage(&state, "-1 hour").await.unwrap() > 0);
        let rollup: (i64, i64, i64, i64) = sqlx::query_as(
            "SELECT requests,tokens,ttft_sum_ms,account_switches FROM ops_minute_rollups LIMIT 1",
        )
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(rollup, (1, 21, 120, 1));
        let live = live_snapshot(&state).await.unwrap();
        assert_eq!(live["account_switches"], 1);
        assert_eq!(live["upstream_attempts"], 2);
        assert_eq!(live["average_ttft_ms"], 120);
    }

    #[tokio::test]
    async fn report_run_records_metrics_without_request_content() {
        let (_directory, state) = test_support::state().await;
        sqlx::query(
            "INSERT INTO usage_logs \
             (request_id,endpoint,status_code,total_tokens,duration_ms,ttft_ms,account_switches) \
             VALUES ('report','/v1/responses',200,34,200,40,1)",
        )
        .execute(&state.pool)
        .await
        .unwrap();
        let end = Utc::now() + chrono::Duration::seconds(1);
        let metrics = run_report(&state, "manual", end - chrono::Duration::hours(1), end, &[])
            .await
            .unwrap();
        assert_eq!(metrics["request_count"], 1);
        assert_eq!(metrics["average_ttft_ms"], 40);
        let row: (String, String) =
            sqlx::query_as("SELECT status,metrics_json FROM ops_report_runs LIMIT 1")
                .fetch_one(&state.pool)
                .await
                .unwrap();
        assert_eq!(row.0, "skipped");
        assert!(!row.1.contains("request_body"));
    }

    #[tokio::test]
    async fn alert_evaluation_fires_and_resolves_once() {
        let (_directory, state) = test_support::state().await;
        sqlx::query(
            "INSERT INTO ops_alert_rules \
             (name, metric_type, operator, threshold, window_minutes, severity, cooldown_minutes) \
             VALUES ('errors', 'error_rate', '>', 10, 5, 'critical', 1)",
        )
        .execute(&state.pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO usage_logs (request_id, endpoint, status_code, duration_ms) \
             VALUES ('failed', '/v1/responses', 500, 10)",
        )
        .execute(&state.pool)
        .await
        .unwrap();
        let result = evaluate_alerts(&state).await.unwrap();
        assert_eq!(result["fired"], 1);
        let duplicate = evaluate_alerts(&state).await.unwrap();
        assert_eq!(duplicate["fired"], 0);
        sqlx::query("UPDATE usage_logs SET status_code = 200")
            .execute(&state.pool)
            .await
            .unwrap();
        let resolved = evaluate_alerts(&state).await.unwrap();
        assert_eq!(resolved["resolved"], 1);
        let status: String =
            sqlx::query_scalar("SELECT status FROM ops_alert_events ORDER BY id DESC LIMIT 1")
                .fetch_one(&state.pool)
                .await
                .unwrap();
        assert_eq!(status, "resolved");
    }

    #[tokio::test]
    async fn overview_reports_usage_without_request_bodies() {
        let (_directory, state) = test_support::state().await;
        sqlx::query(
            "INSERT INTO usage_logs \
             (request_id, endpoint, model, status_code, total_tokens, cost_microusd, duration_ms, \
              ttft_ms, upstream_attempts, account_switches) \
             VALUES ('ops-ok', '/v1/responses', 'gpt-test', 200, 12, 42, 100, 20, 1, 0), \
                    ('ops-fail', '/v1/responses', 'gpt-test', 503, 0, 0, 300, 50, 2, 1)",
        )
        .execute(&state.pool)
        .await
        .unwrap();
        let response = overview(
            State(state),
            Query(OverviewQuery {
                range: "1h".into(),
                model: None,
                account_id: None,
            }),
        )
        .await
        .unwrap();
        assert_eq!(response.0["data"]["summary"]["request_count"], 2);
        assert_eq!(response.0["data"]["summary"]["error_count"], 1);
        assert_eq!(response.0["data"]["summary"]["latency"]["p50_ms"], 300);
        assert_eq!(
            response.0["data"]["summary"]["telemetry"]["account_switches"],
            1
        );
        assert!(response.0["data"].get("request_body").is_none());
    }
}
