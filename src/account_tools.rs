use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, post},
};
use chrono::{Duration, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::FromRow;

use crate::{
    error::{ApiError, ApiResult},
    models::Credentials,
    oauth,
    state::AppState,
};

pub fn admin_router() -> Router<AppState> {
    Router::new()
        .route("/accounts/{id}/stats", get(stats))
        .route("/accounts/{id}/duplicate", post(duplicate))
        .route("/accounts/{id}/spark-shadow", post(create_spark_shadow))
        .route("/accounts/{id}/reauth", post(reauth))
}

#[derive(Debug, FromRow)]
struct CloneAccountRow {
    name: String,
    kind: String,
    base_url: String,
    encrypted_credentials: String,
    priority: i32,
    concurrency: i32,
    proxy_id: Option<i64>,
    notes: String,
    tls_fingerprint_profile_id: Option<i64>,
    parent_account_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct StatsQuery {
    days: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct ReauthInput {
    content: String,
}

async fn stats(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Query(query): Query<StatsQuery>,
) -> ApiResult<Json<Value>> {
    ensure_account(&state, id).await?;
    let days = query.days.unwrap_or(30);
    if !(1..=90).contains(&days) {
        return Err(ApiError::bad_request(
            "INVALID_STATS_RANGE",
            "days must be between 1 and 90",
        ));
    }
    let (offset_minutes, offset_label) = crate::groups::server_utc_offset();
    let offset_modifier = format!("{offset_minutes:+} minutes");
    let range_modifier = format!("-{} days", days - 1);
    let rows: Vec<(String, i64, i64, i64, i64, i64, i64)> = sqlx::query_as(
        "SELECT date(created_at, ?) AS day, COUNT(*), \
         COALESCE(SUM(COALESCE(total_tokens, 0)), 0), \
         COALESCE(SUM(account_cost_microusd), 0), COALESCE(SUM(cost_microusd), 0), \
         COALESCE(SUM(CASE WHEN status_code < 400 THEN 1 ELSE 0 END), 0), \
         COALESCE(SUM(CASE WHEN status_code >= 400 THEN 1 ELSE 0 END), 0) \
         FROM usage_logs WHERE account_id = ? AND date(created_at, ?) >= date('now', ?, ?) \
         GROUP BY day ORDER BY day ASC",
    )
    .bind(&offset_modifier)
    .bind(id)
    .bind(&offset_modifier)
    .bind(&offset_modifier)
    .bind(&range_modifier)
    .fetch_all(&state.pool)
    .await?;
    let average_duration_ms: f64 = sqlx::query_scalar(
        "SELECT CAST(COALESCE(AVG(duration_ms), 0) AS REAL) FROM usage_logs \
         WHERE account_id = ? AND date(created_at, ?) >= date('now', ?, ?)",
    )
    .bind(id)
    .bind(&offset_modifier)
    .bind(&offset_modifier)
    .bind(&range_modifier)
    .fetch_one(&state.pool)
    .await?;
    let token_detail: (i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT COALESCE(SUM(cached_input_tokens), 0), COALESCE(SUM(cache_write_tokens), 0), \
         COALESCE(SUM(image_input_tokens), 0), COALESCE(SUM(image_output_tokens), 0), \
         COALESCE(SUM(reasoning_tokens), 0) \
         FROM usage_logs WHERE account_id = ? AND date(created_at, ?) >= date('now', ?, ?)",
    )
    .bind(id)
    .bind(&offset_modifier)
    .bind(&offset_modifier)
    .bind(&range_modifier)
    .fetch_one(&state.pool)
    .await?;

    let history = rows
        .iter()
        .map(|row| {
            json!({
                "date": row.0, "label": row.0.get(5..).unwrap_or(&row.0),
                "requests": row.1, "tokens": row.2, "cost_microusd": row.3,
                "cost": usd(row.3), "actual_cost": usd(row.3), "user_cost": usd(row.4),
                "user_cost_microusd": row.4,
                "successful_requests": row.5, "failed_requests": row.6,
            })
        })
        .collect::<Vec<_>>();
    let total_requests = rows.iter().map(|row| row.1).sum::<i64>();
    let total_tokens = rows.iter().map(|row| row.2).sum::<i64>();
    let total_cost = rows.iter().map(|row| row.3).sum::<i64>();
    let total_user_cost = rows.iter().map(|row| row.4).sum::<i64>();
    let successful_requests = rows.iter().map(|row| row.5).sum::<i64>();
    let failed_requests = rows.iter().map(|row| row.6).sum::<i64>();
    let actual_days = rows.len().max(1) as i64;
    let today = (Utc::now() + Duration::minutes(i64::from(offset_minutes)))
        .date_naive()
        .to_string();
    let today_value = rows.iter().find(|row| row.0 == today).map(|row| {
        json!({"date": row.0, "cost": usd(row.3), "cost_microusd": row.3,
            "user_cost": usd(row.4), "user_cost_microusd": row.4,
            "requests": row.1, "tokens": row.2})
    });
    let highest_cost = rows.iter().max_by_key(|row| row.3).map(|row| {
        json!({"date": row.0, "label": row.0.get(5..).unwrap_or(&row.0),
            "cost": usd(row.3), "cost_microusd": row.3, "user_cost": usd(row.4),
            "requests": row.1})
    });
    let highest_requests = rows.iter().max_by_key(|row| row.1).map(|row| {
        json!({"date": row.0, "label": row.0.get(5..).unwrap_or(&row.0),
            "requests": row.1, "cost": usd(row.3), "cost_microusd": row.3,
            "user_cost": usd(row.4)})
    });

    let models: Vec<(String, i64, i64, i64, i64, f64)> = sqlx::query_as(
        "SELECT COALESCE(NULLIF(model, ''), 'unknown'), COUNT(*), \
         COALESCE(SUM(COALESCE(total_tokens, 0)), 0), \
         COALESCE(SUM(account_cost_microusd), 0), COALESCE(SUM(cost_microusd), 0), \
         CAST(COALESCE(AVG(duration_ms), 0) AS REAL) FROM usage_logs \
         WHERE account_id = ? AND date(created_at, ?) >= date('now', ?, ?) \
         GROUP BY COALESCE(NULLIF(model, ''), 'unknown') ORDER BY COUNT(*) DESC LIMIT 20",
    )
    .bind(id)
    .bind(&offset_modifier)
    .bind(&offset_modifier)
    .bind(&range_modifier)
    .fetch_all(&state.pool)
    .await?;
    let endpoints: Vec<(String, i64, i64, i64, i64, f64)> = sqlx::query_as(
        "SELECT endpoint, COUNT(*), COALESCE(SUM(COALESCE(total_tokens, 0)), 0), \
         COALESCE(SUM(account_cost_microusd), 0), COALESCE(SUM(cost_microusd), 0), \
         CAST(COALESCE(AVG(duration_ms), 0) AS REAL) \
         FROM usage_logs WHERE account_id = ? AND date(created_at, ?) >= date('now', ?, ?) \
         GROUP BY endpoint ORDER BY COUNT(*) DESC LIMIT 20",
    )
    .bind(id)
    .bind(&offset_modifier)
    .bind(&offset_modifier)
    .bind(&range_modifier)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(json!({"data": {
        "history": history,
        "summary": {
            "days": days, "actual_days_used": actual_days,
            "total_cost": usd(total_cost), "total_user_cost": usd(total_user_cost),
            "total_standard_cost": usd(total_cost), "total_cost_microusd": total_cost,
            "total_user_cost_microusd": total_user_cost,
            "total_requests": total_requests, "successful_requests": successful_requests,
            "failed_requests": failed_requests,
            "success_rate": if total_requests == 0 { 0.0 } else { successful_requests as f64 * 100.0 / total_requests as f64 },
            "total_tokens": total_tokens, "cached_input_tokens": token_detail.0,
            "cache_write_tokens": token_detail.1, "image_input_tokens": token_detail.2,
            "image_output_tokens": token_detail.3, "reasoning_tokens": token_detail.4,
            "avg_daily_cost": usd(total_cost) / actual_days as f64,
            "avg_daily_user_cost": usd(total_user_cost) / actual_days as f64,
            "avg_daily_requests": total_requests as f64 / actual_days as f64,
            "avg_daily_tokens": total_tokens as f64 / actual_days as f64,
            "avg_duration_ms": average_duration_ms, "today": today_value,
            "highest_cost_day": highest_cost, "highest_request_day": highest_requests,
            "utc_offset": offset_label,
        },
        "models": models.into_iter().map(|row| json!({"model": row.0, "requests": row.1,
            "tokens": row.2, "cost_microusd": row.3, "cost": usd(row.3),
            "user_cost_microusd": row.4, "user_cost": usd(row.4),
            "average_duration_ms": row.5})).collect::<Vec<_>>(),
        "endpoints": endpoints.iter().map(|row| json!({"endpoint": row.0, "requests": row.1,
            "tokens": row.2, "cost_microusd": row.3, "cost": usd(row.3),
            "user_cost_microusd": row.4, "user_cost": usd(row.4),
            "average_duration_ms": row.5})).collect::<Vec<_>>(),
        "upstream_endpoints": endpoints.into_iter().map(|row| json!({"endpoint": row.0,
            "requests": row.1, "tokens": row.2, "cost_microusd": row.3,
            "cost": usd(row.3), "user_cost_microusd": row.4,
            "user_cost": usd(row.4), "average_duration_ms": row.5})).collect::<Vec<_>>(),
    }})))
}

async fn duplicate(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    let source = sqlx::query_as::<_, CloneAccountRow>(
        "SELECT name, kind, base_url, encrypted_credentials, priority, concurrency, proxy_id, \
         notes, tls_fingerprint_profile_id, parent_account_id \
         FROM accounts WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| ApiError::not_found("account not found"))?;
    if source.parent_account_id.is_some() {
        return Err(ApiError::bad_request(
            "SPARK_SHADOW_CANNOT_DUPLICATE",
            "Spark shadow accounts cannot be duplicated",
        ));
    }
    let credentials: Credentials =
        serde_json::from_slice(&state.crypto.decrypt(&source.encrypted_credentials)?)
            .map_err(|_| ApiError::internal("stored account credentials are malformed"))?;
    let encrypted = state.crypto.encrypt(
        &serde_json::to_vec(&credentials)
            .map_err(|_| ApiError::internal("credential serialization failed"))?,
    )?;
    let name = duplicate_name(&state, &source.name).await?;
    let mut transaction = state.pool.begin().await?;
    let duplicate_id = sqlx::query(
        "INSERT INTO accounts (name, kind, base_url, encrypted_credentials, priority, concurrency, \
         enabled, proxy_id, notes, tls_fingerprint_profile_id) VALUES (?, ?, ?, ?, ?, ?, 0, ?, ?, ?)",
    )
    .bind(&name)
    .bind(&source.kind)
    .bind(&source.base_url)
    .bind(encrypted)
    .bind(source.priority)
    .bind(source.concurrency)
    .bind(source.proxy_id)
    .bind(&source.notes)
    .bind(source.tls_fingerprint_profile_id)
    .execute(&mut *transaction)
    .await?
    .last_insert_rowid();
    sqlx::query(
        "INSERT INTO account_groups (account_id, group_id) \
         SELECT ?, group_id FROM account_groups WHERE account_id = ?",
    )
    .bind(duplicate_id)
    .bind(id)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(Json(
        json!({"data": {"id": duplicate_id, "name": name, "kind": source.kind,
        "base_url": source.base_url, "priority": source.priority,
        "concurrency": source.concurrency, "enabled": false, "proxy_id": source.proxy_id}}),
    ))
}

async fn create_spark_shadow(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<Value>> {
    let parent = sqlx::query_as::<_, (String, String, String, i32, i32, Option<i64>, Option<i64>)>(
        "SELECT name, kind, base_url, priority, concurrency, proxy_id, parent_account_id \
         FROM accounts WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| ApiError::not_found("account not found"))?;
    if parent.1 != "oauth" || parent.6.is_some() {
        return Err(ApiError::bad_request(
            "SPARK_PARENT_REQUIRED",
            "Spark shadows require a top-level OAuth account",
        ));
    }
    let exists: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM accounts WHERE parent_account_id = ?")
            .bind(id)
            .fetch_one(&state.pool)
            .await?;
    if exists != 0 {
        return Err(ApiError::bad_request(
            "SPARK_SHADOW_EXISTS",
            "the OAuth account already has a Spark shadow",
        ));
    }
    let name = unique_name(&state, &format!("{} Spark", parent.0)).await?;
    let placeholder = state.crypto.encrypt(
        &serde_json::to_vec(&Credentials::default())
            .map_err(|_| ApiError::internal("credential serialization failed"))?,
    )?;
    let mut transaction = state.pool.begin().await?;
    let shadow_id = sqlx::query(
        "INSERT INTO accounts (name, kind, base_url, encrypted_credentials, priority, concurrency, \
         enabled, proxy_id, parent_account_id, quota_dimension) \
         VALUES (?, 'oauth', ?, ?, ?, ?, 0, ?, ?, 'spark')",
    )
    .bind(&name)
    .bind(&parent.2)
    .bind(placeholder)
    .bind(parent.3)
    .bind(parent.4)
    .bind(parent.5)
    .bind(id)
    .execute(&mut *transaction)
    .await
    .map_err(|error| match error {
        sqlx::Error::Database(ref database) if database.is_unique_violation() => {
            ApiError::bad_request(
                "SPARK_SHADOW_EXISTS",
                "the OAuth account already has a Spark shadow",
            )
        }
        other => other.into(),
    })?
    .last_insert_rowid();
    sqlx::query(
        "INSERT INTO account_groups (account_id, group_id) \
         SELECT ?, group_id FROM account_groups WHERE account_id = ?",
    )
    .bind(shadow_id)
    .bind(id)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(Json(json!({"data": {"id": shadow_id, "name": name,
        "kind": "oauth", "parent_account_id": id, "quota_dimension": "spark",
        "enabled": false, "priority": parent.3, "concurrency": parent.4,
        "proxy_id": parent.5}})))
}

async fn reauth(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(input): Json<ReauthInput>,
) -> ApiResult<Json<Value>> {
    let row: Option<(String, String, Option<i64>)> =
        sqlx::query_as("SELECT name, kind, parent_account_id FROM accounts WHERE id = ?")
            .bind(id)
            .fetch_optional(&state.pool)
            .await?;
    let (name, kind, parent_account_id) =
        row.ok_or_else(|| ApiError::not_found("account not found"))?;
    if parent_account_id.is_some() {
        return Err(ApiError::bad_request(
            "SPARK_SHADOW_CREDENTIALS_INHERITED",
            "Spark shadow credentials are inherited from the parent account",
        ));
    }
    if kind != "oauth" {
        return Err(ApiError::bad_request(
            "NOT_OAUTH_ACCOUNT",
            "only OAuth accounts can be re-authorized",
        ));
    }
    let credentials = oauth::parse_import(&input.content)?;
    let encrypted = state.crypto.encrypt(
        &serde_json::to_vec(&credentials)
            .map_err(|_| ApiError::internal("credential serialization failed"))?,
    )?;
    sqlx::query(
        "UPDATE accounts SET encrypted_credentials = ?, cooldown_until = NULL, last_error = NULL, \
         updated_at = CURRENT_TIMESTAMP WHERE id = ?",
    )
    .bind(encrypted)
    .bind(id)
    .execute(&state.pool)
    .await?;
    state.model_cache.lock().await.remove(&id);
    Ok(Json(
        json!({"data": {"id": id, "name": name, "kind": kind, "reauthorized": true}}),
    ))
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

async fn duplicate_name(state: &AppState, source: &str) -> ApiResult<String> {
    let source = source.trim().chars().take(110).collect::<String>();
    unique_name(state, &format!("{source} Copy")).await
}

async fn unique_name(state: &AppState, base: &str) -> ApiResult<String> {
    let base = base.trim().chars().take(120).collect::<String>();
    for suffix in 0..10_000 {
        let candidate = if suffix == 0 {
            base.clone()
        } else {
            format!("{base} {}", suffix + 1)
        };
        let exists: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM accounts WHERE name = ? COLLATE NOCASE")
                .bind(&candidate)
                .fetch_one(&state.pool)
                .await?;
        if exists == 0 {
            return Ok(candidate);
        }
    }
    Err(ApiError::internal(
        "could not allocate a duplicate account name",
    ))
}

fn usd(microusd: i64) -> f64 {
    microusd as f64 / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;

    async fn insert_account(state: &AppState, name: &str, kind: &str) -> i64 {
        let credentials = if kind == "oauth" {
            Credentials {
                access_token: Some("old-access".into()),
                refresh_token: Some("old-refresh".into()),
                ..Default::default()
            }
        } else {
            Credentials {
                api_key: Some("sk-test".into()),
                ..Default::default()
            }
        };
        let encrypted = state
            .crypto
            .encrypt(&serde_json::to_vec(&credentials).unwrap())
            .unwrap();
        sqlx::query(
            "INSERT INTO accounts (name, kind, base_url, encrypted_credentials) VALUES (?, ?, ?, ?)",
        )
        .bind(name)
        .bind(kind)
        .bind(if kind == "oauth" {
            "https://chatgpt.com/backend-api/codex"
        } else {
            "https://api.openai.com"
        })
        .bind(encrypted)
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid()
    }

    #[tokio::test]
    async fn account_stats_aggregate_cost_tokens_models_and_endpoints() {
        let (_directory, state) = test_support::state().await;
        let id = insert_account(&state, "stats", "api_key").await;
        for (request, model, endpoint, tokens, cached, reasoning, cost, status, age) in [
            (
                "one",
                "gpt-5",
                "/v1/responses",
                120,
                20,
                10,
                1_500_000,
                200,
                "-1 day",
            ),
            (
                "two",
                "gpt-5",
                "/v1/chat/completions",
                80,
                0,
                5,
                500_000,
                500,
                "0 day",
            ),
        ] {
            sqlx::query(
                "INSERT INTO usage_logs (request_id, account_id, endpoint, model, status_code, \
                 total_tokens, cached_input_tokens, reasoning_tokens, cost_microusd, \
                 account_cost_microusd, duration_ms, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 100, datetime('now', ?))",
            )
            .bind(request)
            .bind(id)
            .bind(endpoint)
            .bind(model)
            .bind(status)
            .bind(tokens)
            .bind(cached)
            .bind(reasoning)
            .bind(cost)
            .bind(cost / 2)
            .bind(age)
            .execute(&state.pool)
            .await
            .unwrap();
        }
        let Json(value) = stats(State(state), Path(id), Query(StatsQuery { days: Some(30) }))
            .await
            .unwrap();
        assert_eq!(value["data"]["summary"]["total_requests"], 2);
        assert_eq!(value["data"]["summary"]["total_tokens"], 200);
        assert_eq!(value["data"]["summary"]["total_cost_microusd"], 1_000_000);
        assert_eq!(
            value["data"]["summary"]["total_user_cost_microusd"],
            2_000_000
        );
        assert_eq!(value["data"]["summary"]["cached_input_tokens"], 20);
        assert_eq!(value["data"]["models"][0]["model"], "gpt-5");
        assert_eq!(value["data"]["endpoints"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn duplicate_is_disabled_reencrypted_and_keeps_groups() {
        let (_directory, state) = test_support::state().await;
        let id = insert_account(&state, "primary", "api_key").await;
        let group_id = sqlx::query("INSERT INTO groups (name) VALUES ('group')")
            .execute(&state.pool)
            .await
            .unwrap()
            .last_insert_rowid();
        sqlx::query("INSERT INTO account_groups (account_id, group_id) VALUES (?, ?)")
            .bind(id)
            .bind(group_id)
            .execute(&state.pool)
            .await
            .unwrap();
        let source_encrypted: String =
            sqlx::query_scalar("SELECT encrypted_credentials FROM accounts WHERE id = ?")
                .bind(id)
                .fetch_one(&state.pool)
                .await
                .unwrap();
        let Json(value) = duplicate(State(state.clone()), Path(id)).await.unwrap();
        let duplicate_id = value["data"]["id"].as_i64().unwrap();
        assert_eq!(value["data"]["name"], "primary Copy");
        assert_eq!(value["data"]["enabled"], false);
        let duplicate_encrypted: String =
            sqlx::query_scalar("SELECT encrypted_credentials FROM accounts WHERE id = ?")
                .bind(duplicate_id)
                .fetch_one(&state.pool)
                .await
                .unwrap();
        assert_ne!(source_encrypted, duplicate_encrypted);
        let copied_group: i64 =
            sqlx::query_scalar("SELECT group_id FROM account_groups WHERE account_id = ?")
                .bind(duplicate_id)
                .fetch_one(&state.pool)
                .await
                .unwrap();
        assert_eq!(copied_group, group_id);
    }

    #[tokio::test]
    async fn reauth_replaces_only_oauth_credentials_and_clears_errors() {
        let (_directory, state) = test_support::state().await;
        let id = insert_account(&state, "oauth", "oauth").await;
        sqlx::query(
            "UPDATE accounts SET last_error = 'expired', \
             cooldown_until = datetime('now', '+1 hour') WHERE id = ?",
        )
        .bind(id)
        .execute(&state.pool)
        .await
        .unwrap();
        let _ = reauth(
            State(state.clone()),
            Path(id),
            Json(ReauthInput {
                content:
                    r#"{"tokens":{"access_token":"new-access","refresh_token":"new-refresh"}}"#
                        .into(),
            }),
        )
        .await
        .unwrap();
        let stored: (String, Option<String>, Option<String>) = sqlx::query_as(
            "SELECT encrypted_credentials, last_error, cooldown_until FROM accounts WHERE id = ?",
        )
        .bind(id)
        .fetch_one(&state.pool)
        .await
        .unwrap();
        let credentials: Credentials =
            serde_json::from_slice(&state.crypto.decrypt(&stored.0).unwrap()).unwrap();
        assert_eq!(credentials.access_token.as_deref(), Some("new-access"));
        assert_eq!((stored.1, stored.2), (None, None));

        let api_id = insert_account(&state, "api", "api_key").await;
        let error = reauth(
            State(state),
            Path(api_id),
            Json(ReauthInput {
                content: r#"{"access_token":"ignored"}"#.into(),
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "NOT_OAUTH_ACCOUNT");
    }

    #[tokio::test]
    async fn spark_shadow_inherits_live_credentials_groups_and_parent_lifecycle() {
        let (_directory, state) = test_support::state().await;
        let parent_id = insert_account(&state, "oauth parent", "oauth").await;
        let group_id = sqlx::query("INSERT INTO groups (name) VALUES ('spark group')")
            .execute(&state.pool)
            .await
            .unwrap()
            .last_insert_rowid();
        sqlx::query("INSERT INTO account_groups (account_id, group_id) VALUES (?, ?)")
            .bind(parent_id)
            .bind(group_id)
            .execute(&state.pool)
            .await
            .unwrap();

        let Json(created) = create_spark_shadow(State(state.clone()), Path(parent_id))
            .await
            .unwrap();
        let shadow_id = created["data"]["id"].as_i64().unwrap();
        assert_eq!(created["data"]["parent_account_id"], parent_id);
        assert_eq!(created["data"]["quota_dimension"], "spark");
        assert_eq!(created["data"]["enabled"], false);
        let shadow = crate::admin::get_account_row(&state, shadow_id)
            .await
            .unwrap();
        assert_eq!(shadow.parent_account_id, Some(parent_id));
        assert_eq!(shadow.quota_dimension, "spark");
        let resolved = state.resolve_account(shadow.clone()).await.unwrap();
        assert_eq!(
            resolved.credentials.access_token.as_deref(),
            Some("old-access")
        );
        let inherited_group: i64 =
            sqlx::query_scalar("SELECT group_id FROM account_groups WHERE account_id = ?")
                .bind(shadow_id)
                .fetch_one(&state.pool)
                .await
                .unwrap();
        assert_eq!(inherited_group, group_id);

        let updated = Credentials {
            access_token: Some("rotated-access".into()),
            refresh_token: Some("rotated-refresh".into()),
            ..Default::default()
        };
        let encrypted = state
            .crypto
            .encrypt(&serde_json::to_vec(&updated).unwrap())
            .unwrap();
        sqlx::query("UPDATE accounts SET encrypted_credentials = ? WHERE id = ?")
            .bind(encrypted)
            .bind(parent_id)
            .execute(&state.pool)
            .await
            .unwrap();
        let resolved = state.resolve_account(shadow).await.unwrap();
        assert_eq!(
            resolved.credentials.access_token.as_deref(),
            Some("rotated-access")
        );
        assert!(
            duplicate(State(state.clone()), Path(shadow_id))
                .await
                .is_err()
        );
        assert!(
            reauth(
                State(state.clone()),
                Path(shadow_id),
                Json(ReauthInput {
                    content: r#"{"access_token":"ignored"}"#.into(),
                }),
            )
            .await
            .is_err()
        );
        assert!(
            create_spark_shadow(State(state.clone()), Path(parent_id))
                .await
                .is_err()
        );
        sqlx::query("DELETE FROM accounts WHERE id = ?")
            .bind(parent_id)
            .execute(&state.pool)
            .await
            .unwrap();
        let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM accounts")
            .fetch_one(&state.pool)
            .await
            .unwrap();
        assert_eq!(remaining, 0);
    }
}
