use std::{str::FromStr, time::Duration};

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post, put},
};
use chrono::{DateTime, FixedOffset, Utc};
use cron::Schedule;
use futures_util::{StreamExt, stream};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::FromRow;

use crate::{
    admin,
    error::{ApiError, ApiResult},
    gateway,
    state::AppState,
};

const DEFAULT_CRON: &str = "*/30 * * * *";
const DEFAULT_MAX_RESULTS: i64 = 50;
const MAX_RESULTS: i64 = 500;

pub fn admin_router() -> Router<AppState> {
    Router::new()
        .route("/accounts/{id}/scheduled-test-plans", get(list_by_account))
        .route("/scheduled-test-plans", post(create))
        .route("/scheduled-test-plans/{id}", put(update).delete(delete))
        .route("/scheduled-test-plans/{id}/results", get(list_results))
        .route("/scheduled-test-plans/{id}/run", post(run_now))
}

pub fn start_scheduler(state: AppState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(15));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if let Err(error) = run_due(&state).await {
                tracing::warn!(%error, "scheduled account test scan failed");
            }
        }
    });
}

#[derive(Debug, Clone, Serialize, FromRow)]
struct PlanRow {
    id: i64,
    account_id: i64,
    model_id: String,
    cron_expression: String,
    enabled: bool,
    max_results: i64,
    auto_recover: bool,
    last_run_at: Option<String>,
    next_run_at: Option<String>,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, Serialize, FromRow)]
struct ResultRow {
    id: i64,
    plan_id: i64,
    status: String,
    response_text: String,
    error_message: String,
    latency_ms: i64,
    started_at: String,
    finished_at: String,
    created_at: String,
}

#[derive(Debug, Deserialize)]
struct CreateInput {
    account_id: i64,
    model_id: String,
    #[serde(default = "default_cron")]
    cron_expression: String,
    enabled: Option<bool>,
    max_results: Option<i64>,
    auto_recover: Option<bool>,
}

#[derive(Debug, Deserialize, Default)]
struct UpdateInput {
    model_id: Option<String>,
    cron_expression: Option<String>,
    enabled: Option<bool>,
    max_results: Option<i64>,
    auto_recover: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct ResultsQuery {
    limit: Option<i64>,
}

fn default_cron() -> String {
    DEFAULT_CRON.into()
}

async fn list_by_account(
    State(state): State<AppState>,
    Path(account_id): Path<i64>,
) -> ApiResult<Json<Value>> {
    ensure_account(&state, account_id).await?;
    let plans = sqlx::query_as::<_, PlanRow>(
        "SELECT id, account_id, model_id, cron_expression, enabled, max_results, \
         auto_recover, last_run_at, next_run_at, created_at, updated_at \
         FROM scheduled_test_plans WHERE account_id = ? ORDER BY id DESC",
    )
    .bind(account_id)
    .fetch_all(&state.pool)
    .await?;
    let (_, utc_offset) = crate::groups::server_utc_offset();
    Ok(Json(
        json!({"data": plans, "meta": {"utc_offset": utc_offset}}),
    ))
}

async fn create(
    State(state): State<AppState>,
    Json(input): Json<CreateInput>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    ensure_account(&state, input.account_id).await?;
    let model = validate_model(&input.model_id)?;
    let cron = validate_cron(&input.cron_expression)?;
    let max_results = validate_max_results(input.max_results.unwrap_or(DEFAULT_MAX_RESULTS))?;
    let next_run_at = next_run_at(&cron, Utc::now())?;
    let id = sqlx::query(
        "INSERT INTO scheduled_test_plans (account_id, model_id, cron_expression, enabled, \
         max_results, auto_recover, next_run_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(input.account_id)
    .bind(model)
    .bind(cron)
    .bind(input.enabled.unwrap_or(true))
    .bind(max_results)
    .bind(input.auto_recover.unwrap_or(false))
    .bind(next_run_at)
    .execute(&state.pool)
    .await?
    .last_insert_rowid();
    Ok((
        StatusCode::CREATED,
        Json(json!({"data": get_plan(&state, id).await?})),
    ))
}

async fn update(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(input): Json<UpdateInput>,
) -> ApiResult<Json<Value>> {
    let existing = get_plan(&state, id).await?;
    let model = match input.model_id {
        Some(value) => validate_model(&value)?,
        None => existing.model_id,
    };
    let cron = match input.cron_expression {
        Some(value) => validate_cron(&value)?,
        None => existing.cron_expression,
    };
    let max_results = validate_max_results(input.max_results.unwrap_or(existing.max_results))?;
    let next_run_at = next_run_at(&cron, Utc::now())?;
    sqlx::query(
        "UPDATE scheduled_test_plans SET model_id = ?, cron_expression = ?, enabled = ?, \
         max_results = ?, auto_recover = ?, next_run_at = ?, updated_at = CURRENT_TIMESTAMP \
         WHERE id = ?",
    )
    .bind(model)
    .bind(cron)
    .bind(input.enabled.unwrap_or(existing.enabled))
    .bind(max_results)
    .bind(input.auto_recover.unwrap_or(existing.auto_recover))
    .bind(next_run_at)
    .bind(id)
    .execute(&state.pool)
    .await?;
    Ok(Json(json!({"data": get_plan(&state, id).await?})))
}

async fn delete(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    let result = sqlx::query("DELETE FROM scheduled_test_plans WHERE id = ?")
        .bind(id)
        .execute(&state.pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found("scheduled test plan not found"));
    }
    Ok(Json(json!({"data": {"id": id, "deleted": true}})))
}

async fn list_results(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Query(query): Query<ResultsQuery>,
) -> ApiResult<Json<Value>> {
    get_plan(&state, id).await?;
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let results = sqlx::query_as::<_, ResultRow>(
        "SELECT id, plan_id, status, response_text, error_message, latency_ms, started_at, \
         finished_at, created_at FROM scheduled_test_results WHERE plan_id = ? \
         ORDER BY created_at DESC, id DESC LIMIT ?",
    )
    .bind(id)
    .bind(limit)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(json!({"data": results})))
}

async fn run_now(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    let plan = get_plan(&state, id).await?;
    let result = execute_plan(&state, &plan).await?;
    let next = next_run_at(&plan.cron_expression, Utc::now())?;
    sqlx::query(
        "UPDATE scheduled_test_plans SET last_run_at = ?, next_run_at = ?, \
         updated_at = CURRENT_TIMESTAMP WHERE id = ?",
    )
    .bind(&result.finished_at)
    .bind(next)
    .bind(id)
    .execute(&state.pool)
    .await?;
    Ok(Json(json!({"data": result})))
}

async fn run_due(state: &AppState) -> ApiResult<()> {
    let ids: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM scheduled_test_plans WHERE enabled = 1 AND next_run_at IS NOT NULL \
         AND datetime(next_run_at) <= CURRENT_TIMESTAMP ORDER BY next_run_at ASC LIMIT 10",
    )
    .fetch_all(&state.pool)
    .await?;
    stream::iter(ids)
        .for_each_concurrent(2, |id| async move {
            if let Err(error) = claim_and_run(state, id).await {
                tracing::warn!(plan_id = id, %error, "scheduled account test failed");
            }
        })
        .await;
    Ok(())
}

async fn claim_and_run(state: &AppState, id: i64) -> ApiResult<()> {
    let plan = get_plan(state, id).await?;
    let now = Utc::now();
    let next = next_run_at(&plan.cron_expression, now)?;
    let claimed = sqlx::query(
        "UPDATE scheduled_test_plans SET last_run_at = ?, next_run_at = ?, \
         updated_at = CURRENT_TIMESTAMP WHERE id = ? AND enabled = 1 \
         AND next_run_at IS NOT NULL AND datetime(next_run_at) <= CURRENT_TIMESTAMP",
    )
    .bind(now.to_rfc3339())
    .bind(next)
    .bind(id)
    .execute(&state.pool)
    .await?;
    if claimed.rows_affected() == 0 {
        return Ok(());
    }
    execute_plan(state, &plan).await?;
    Ok(())
}

async fn execute_plan(state: &AppState, plan: &PlanRow) -> ApiResult<ResultRow> {
    let started = Utc::now();
    let timer = std::time::Instant::now();
    let outcome = match admin::get_account_row(state, plan.account_id).await {
        Ok(row) => match state.resolve_account(row).await {
            Ok(mut account) => {
                gateway::probe_account_model(state, &mut account, &plan.model_id).await
            }
            Err(error) => Err(error),
        },
        Err(error) => Err(error),
    };
    let finished = Utc::now();
    let latency_ms = timer.elapsed().as_millis().min(i64::MAX as u128) as i64;
    let (status, response_text, error_message) = match outcome {
        Ok(value) => (
            "success",
            truncate(&serde_json::to_string(&value).unwrap_or_default(), 2_000),
            String::new(),
        ),
        Err(error) => (
            "failed",
            String::new(),
            truncate(&format!("{}: {}", error.code, error.message), 1_000),
        ),
    };
    let mut transaction = state.pool.begin().await?;
    let result_id = sqlx::query(
        "INSERT INTO scheduled_test_results (plan_id, status, response_text, error_message, \
         latency_ms, started_at, finished_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(plan.id)
    .bind(status)
    .bind(response_text)
    .bind(error_message)
    .bind(latency_ms)
    .bind(started.to_rfc3339())
    .bind(finished.to_rfc3339())
    .execute(&mut *transaction)
    .await?
    .last_insert_rowid();
    sqlx::query(
        "DELETE FROM scheduled_test_results WHERE plan_id = ? AND id NOT IN \
         (SELECT id FROM scheduled_test_results WHERE plan_id = ? \
          ORDER BY created_at DESC, id DESC LIMIT ?)",
    )
    .bind(plan.id)
    .bind(plan.id)
    .bind(plan.max_results)
    .execute(&mut *transaction)
    .await?;
    if status == "success" && plan.auto_recover {
        sqlx::query(
            "UPDATE accounts SET cooldown_until = NULL, last_error = NULL, \
             updated_at = CURRENT_TIMESTAMP WHERE id = ?",
        )
        .bind(plan.account_id)
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;
    get_result(state, result_id).await
}

async fn ensure_account(state: &AppState, id: i64) -> ApiResult<()> {
    let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM accounts WHERE id = ?")
        .bind(id)
        .fetch_one(&state.pool)
        .await?;
    if exists == 0 {
        return Err(ApiError::not_found("account not found"));
    }
    Ok(())
}

async fn get_plan(state: &AppState, id: i64) -> ApiResult<PlanRow> {
    sqlx::query_as::<_, PlanRow>(
        "SELECT id, account_id, model_id, cron_expression, enabled, max_results, \
         auto_recover, last_run_at, next_run_at, created_at, updated_at \
         FROM scheduled_test_plans WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| ApiError::not_found("scheduled test plan not found"))
}

async fn get_result(state: &AppState, id: i64) -> ApiResult<ResultRow> {
    sqlx::query_as::<_, ResultRow>(
        "SELECT id, plan_id, status, response_text, error_message, latency_ms, started_at, \
         finished_at, created_at FROM scheduled_test_results WHERE id = ?",
    )
    .bind(id)
    .fetch_one(&state.pool)
    .await
    .map_err(Into::into)
}

fn validate_model(value: &str) -> ApiResult<String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 100 {
        return Err(ApiError::bad_request(
            "INVALID_SCHEDULED_TEST_MODEL",
            "model must contain 1 to 100 characters",
        ));
    }
    Ok(value.into())
}

fn validate_max_results(value: i64) -> ApiResult<i64> {
    if !(1..=MAX_RESULTS).contains(&value) {
        return Err(ApiError::bad_request(
            "INVALID_SCHEDULED_TEST_RETENTION",
            "max_results must be between 1 and 500",
        ));
    }
    Ok(value)
}

fn validate_cron(value: &str) -> ApiResult<String> {
    let value = value.trim();
    if value.len() > 100 || value.split_whitespace().count() != 5 {
        return Err(invalid_cron());
    }
    Schedule::from_str(&format!("0 {value}")).map_err(|_| invalid_cron())?;
    Ok(value.into())
}

fn invalid_cron() -> ApiError {
    ApiError::bad_request(
        "INVALID_SCHEDULED_TEST_CRON",
        "cron_expression must be a valid five-field cron expression",
    )
}

fn next_run_at(value: &str, from: DateTime<Utc>) -> ApiResult<String> {
    let cron = validate_cron(value)?;
    let schedule = Schedule::from_str(&format!("0 {cron}")).map_err(|_| invalid_cron())?;
    let (offset_minutes, _) = crate::groups::server_utc_offset();
    let offset = FixedOffset::east_opt(offset_minutes.clamp(-840, 840) * 60)
        .ok_or_else(|| ApiError::internal("configured UTC offset is invalid"))?;
    let local = from.with_timezone(&offset);
    schedule
        .after(&local)
        .next()
        .map(|next| next.with_timezone(&Utc).to_rfc3339())
        .ok_or_else(|| invalid_cron())
}

fn truncate(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{models::Credentials, test_support};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn insert_account(state: &AppState, base_url: &str) -> i64 {
        let credentials = Credentials {
            api_key: Some("sk-test".into()),
            ..Default::default()
        };
        let encrypted = state
            .crypto
            .encrypt(&serde_json::to_vec(&credentials).unwrap())
            .unwrap();
        sqlx::query(
            "INSERT INTO accounts (name, kind, base_url, encrypted_credentials) \
             VALUES ('scheduled', 'api_key', ?, ?)",
        )
        .bind(base_url)
        .bind(encrypted)
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid()
    }

    #[test]
    fn five_field_cron_uses_server_utc_offset() {
        let from = DateTime::parse_from_rfc3339("2026-07-23T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(
            next_run_at("0 9 * * *", from).unwrap(),
            "2026-07-23T01:00:00+00:00"
        );
        assert!(validate_cron("0 0 9 * * *").is_err());
        assert!(validate_cron("not a cron").is_err());
    }

    #[tokio::test]
    async fn plan_crud_run_history_and_auto_recovery_work() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let upstream = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let mut chunk = [0_u8; 2048];
                let read = socket.read(&mut chunk).await.unwrap();
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..read]);
                let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n")
                else {
                    continue;
                };
                let headers = String::from_utf8_lossy(&request[..header_end]);
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|value| value.trim().parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                if request.len() >= header_end + 4 + content_length {
                    break;
                }
            }
            let request = String::from_utf8_lossy(&request);
            assert!(request.starts_with("POST /v1/responses HTTP/1.1"));
            assert!(request.contains(r#""model":"gpt-5""#));
            let body = r#"{"id":"resp-test","output_text":"OK"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        });

        let (_directory, state) = test_support::state().await;
        let account_id = insert_account(&state, &format!("http://{address}")).await;
        sqlx::query(
            "UPDATE accounts SET cooldown_until = datetime('now', '+1 hour'), \
             last_error = 'old failure' WHERE id = ?",
        )
        .bind(account_id)
        .execute(&state.pool)
        .await
        .unwrap();
        let (status, Json(created)) = create(
            State(state.clone()),
            Json(CreateInput {
                account_id,
                model_id: "gpt-5".into(),
                cron_expression: "*/30 * * * *".into(),
                enabled: Some(true),
                max_results: Some(2),
                auto_recover: Some(true),
            }),
        )
        .await
        .unwrap();
        assert_eq!(status, StatusCode::CREATED);
        let plan_id = created["data"]["id"].as_i64().unwrap();
        let Json(result) = run_now(State(state.clone()), Path(plan_id)).await.unwrap();
        upstream.await.unwrap();
        assert_eq!(result["data"]["status"], "success");
        assert!(
            result["data"]["response_text"]
                .as_str()
                .unwrap()
                .contains("resp-test")
        );

        let (cooldown, error): (Option<String>, Option<String>) =
            sqlx::query_as("SELECT cooldown_until, last_error FROM accounts WHERE id = ?")
                .bind(account_id)
                .fetch_one(&state.pool)
                .await
                .unwrap();
        assert!(cooldown.is_none());
        assert!(error.is_none());

        let Json(history) = list_results(
            State(state.clone()),
            Path(plan_id),
            Query(ResultsQuery { limit: Some(20) }),
        )
        .await
        .unwrap();
        assert_eq!(history["data"].as_array().unwrap().len(), 1);

        let Json(updated) = update(
            State(state.clone()),
            Path(plan_id),
            Json(UpdateInput {
                enabled: Some(false),
                ..Default::default()
            }),
        )
        .await
        .unwrap();
        assert_eq!(updated["data"]["enabled"], false);
        let _ = delete(State(state), Path(plan_id)).await.unwrap();
    }
}
