use std::collections::HashSet;

use axum::{
    Json, Router,
    extract::{Extension, Path, State},
    http::StatusCode,
    routing::{get, put},
};
use chrono::{DateTime, FixedOffset, Timelike, Utc};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use sqlx::FromRow;

use crate::{
    auth::AuthSession,
    error::{ApiError, ApiResult},
    state::AppState,
};

const MULTIPLIER_SCALE: i64 = 1_000_000;
const MAX_MULTIPLIER_MICROS: i64 = 1_000_000_000;
const DEFAULT_UTC_OFFSET: &str = "+08:00";

pub fn admin_router() -> Router<AppState> {
    Router::new()
        .route("/groups", get(admin_list).post(create))
        .route("/groups/{id}", put(update).delete(delete_group))
        .route(
            "/groups/{id}/rate-multipliers",
            get(list_rate_multipliers)
                .put(batch_set_rate_multipliers)
                .delete(clear_rate_multipliers),
        )
}

pub fn user_router() -> Router<AppState> {
    Router::new()
        .route("/groups", get(user_list))
        .route("/groups/rates", get(user_rates))
}

#[derive(Debug, FromRow)]
struct GroupRow {
    id: i64,
    name: String,
    description: String,
    enabled: bool,
    allowed_models: String,
    sort_order: i64,
    platform: String,
    is_exclusive: bool,
    subscription_type: String,
    rate_multiplier_micros: i64,
    peak_rate_enabled: bool,
    peak_start: String,
    peak_end: String,
    peak_rate_multiplier_micros: i64,
    active_subscriptions: i64,
    allowed_users: i64,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, FromRow)]
struct UserGroupRow {
    id: i64,
    name: String,
    description: String,
    allowed_models: String,
    sort_order: i64,
    platform: String,
    is_exclusive: bool,
    subscription_type: String,
    rate_multiplier_micros: i64,
    peak_rate_enabled: bool,
    peak_start: String,
    peak_end: String,
    peak_rate_multiplier_micros: i64,
    user_rate_multiplier_micros: Option<i64>,
    active_accounts: i64,
    access_source: String,
}

async fn admin_list(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let rows = sqlx::query_as::<_, GroupRow>(
        "SELECT id, name, description, enabled, allowed_models, sort_order, platform, \
         is_exclusive, subscription_type, rate_multiplier_micros, peak_rate_enabled, \
         peak_start, peak_end, peak_rate_multiplier_micros, \
         (SELECT COUNT(*) FROM subscriptions WHERE subscriptions.group_id = groups.id \
           AND subscriptions.status = 'active' AND datetime(subscriptions.ends_at) > CURRENT_TIMESTAMP) \
           AS active_subscriptions, \
         (SELECT COUNT(*) FROM user_allowed_groups WHERE user_allowed_groups.group_id = groups.id) \
           AS allowed_users, created_at, updated_at \
         FROM groups ORDER BY sort_order ASC, id ASC",
    )
    .fetch_all(&state.pool)
    .await?;
    let mut groups = Vec::with_capacity(rows.len());
    for row in rows {
        let account_ids: Vec<i64> = sqlx::query_scalar(
            "SELECT account_id FROM account_groups WHERE group_id = ? ORDER BY account_id",
        )
        .bind(row.id)
        .fetch_all(&state.pool)
        .await?;
        groups.push(group_value(row, account_ids));
    }
    Ok(Json(json!({"data": groups})))
}

async fn user_list(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
) -> ApiResult<Json<Value>> {
    let rows = sqlx::query_as::<_, UserGroupRow>(
        "SELECT groups.id, groups.name, groups.description, groups.allowed_models, \
         groups.sort_order, groups.platform, groups.is_exclusive, groups.subscription_type, \
         groups.rate_multiplier_micros, groups.peak_rate_enabled, groups.peak_start, \
         groups.peak_end, groups.peak_rate_multiplier_micros, rates.rate_multiplier_micros \
         AS user_rate_multiplier_micros, COUNT(accounts.id) AS active_accounts, \
         CASE WHEN viewer.role = 'admin' THEN 'administrator' \
           WHEN groups.subscription_type = 'subscription' THEN 'subscription' \
           WHEN EXISTS (SELECT 1 FROM user_allowed_groups access \
             WHERE access.user_id = viewer.id AND access.group_id = groups.id) THEN 'explicit' \
           ELSE 'public' END AS access_source FROM groups \
         JOIN users viewer ON viewer.id = ? \
         LEFT JOIN account_groups ON account_groups.group_id = groups.id \
         LEFT JOIN accounts ON accounts.id = account_groups.account_id AND accounts.enabled = 1 \
         LEFT JOIN user_group_rate_multipliers rates ON rates.group_id = groups.id \
         AND rates.user_id = viewer.id WHERE groups.enabled = 1 AND (viewer.role = 'admin' OR \
           (groups.subscription_type = 'subscription' AND EXISTS (SELECT 1 FROM subscriptions \
             WHERE subscriptions.user_id = viewer.id AND subscriptions.group_id = groups.id \
             AND subscriptions.status = 'active' \
             AND datetime(subscriptions.ends_at) > CURRENT_TIMESTAMP)) OR \
           (groups.subscription_type = 'standard' AND ( \
             (groups.is_exclusive = 0 AND viewer.allow_all_standard_groups = 1) OR \
             EXISTS (SELECT 1 FROM user_allowed_groups access \
               WHERE access.user_id = viewer.id AND access.group_id = groups.id)))) \
         GROUP BY groups.id \
         ORDER BY groups.sort_order ASC, groups.id ASC",
    )
    .bind(session.user_id)
    .fetch_all(&state.pool)
    .await?;
    let now = Utc::now();
    let (offset_minutes, offset_label) = server_utc_offset();
    Ok(Json(json!({"data": rows.into_iter().map(|row| {
        let (applied_peak, effective) = effective_rate_micros_at(
            row.rate_multiplier_micros,
            row.user_rate_multiplier_micros,
            &row.subscription_type,
            row.peak_rate_enabled,
            &row.peak_start,
            &row.peak_end,
            row.peak_rate_multiplier_micros,
            now,
            offset_minutes,
        );
        json!({
            "id": row.id, "name": row.name, "description": row.description,
            "allowed_models": parse_models(&row.allowed_models), "sort_order": row.sort_order,
            "platform": row.platform, "platform_label": platform_label(&row.platform),
            "is_exclusive": row.is_exclusive, "subscription_type": row.subscription_type,
            "rate_multiplier": micros_to_multiplier(row.rate_multiplier_micros),
            "user_rate_multiplier": row.user_rate_multiplier_micros.map(micros_to_multiplier),
            "resolved_rate_multiplier": micros_to_multiplier(
                row.user_rate_multiplier_micros.unwrap_or(row.rate_multiplier_micros)),
            "peak_rate_enabled": row.peak_rate_enabled, "peak_start": row.peak_start,
            "peak_end": row.peak_end,
            "peak_rate_multiplier": micros_to_multiplier(row.peak_rate_multiplier_micros),
            "applied_peak_multiplier": micros_to_multiplier(applied_peak),
            "effective_rate_multiplier": micros_to_multiplier(effective),
            "server_utc_offset": offset_label, "observed_at": now.to_rfc3339(),
            "active_accounts": row.active_accounts, "access_source": row.access_source
        })
    }).collect::<Vec<_>>() })))
}

#[derive(Deserialize)]
struct GroupInput {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default)]
    allowed_models: Vec<String>,
    #[serde(default)]
    sort_order: i64,
    #[serde(default)]
    account_ids: Vec<i64>,
    #[serde(default = "default_platform")]
    platform: String,
    #[serde(default)]
    is_exclusive: bool,
    #[serde(default = "default_subscription_type")]
    subscription_type: String,
    #[serde(default = "default_multiplier")]
    rate_multiplier: f64,
    #[serde(default)]
    peak_rate_enabled: bool,
    #[serde(default)]
    peak_start: String,
    #[serde(default)]
    peak_end: String,
    #[serde(default = "default_multiplier")]
    peak_rate_multiplier: f64,
}

fn default_true() -> bool {
    true
}

fn default_platform() -> String {
    "openai".into()
}

fn default_subscription_type() -> String {
    "standard".into()
}

fn default_multiplier() -> f64 {
    1.0
}

async fn create(
    State(state): State<AppState>,
    Json(input): Json<GroupInput>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let input = validate_input(&state, input).await?;
    let mut transaction = state.pool.begin().await?;
    let result = sqlx::query(
        "INSERT INTO groups (name, description, enabled, allowed_models, sort_order, platform, \
         is_exclusive, subscription_type, rate_multiplier_micros, peak_rate_enabled, \
         peak_start, peak_end, peak_rate_multiplier_micros) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(input.name.trim())
    .bind(input.description.trim())
    .bind(input.enabled)
    .bind(serde_json::to_string(&input.allowed_models).unwrap())
    .bind(input.sort_order)
    .bind(&input.platform)
    .bind(input.is_exclusive)
    .bind(&input.subscription_type)
    .bind(multiplier_to_micros(input.rate_multiplier, false)?)
    .bind(input.peak_rate_enabled)
    .bind(&input.peak_start)
    .bind(&input.peak_end)
    .bind(multiplier_to_micros(input.peak_rate_multiplier, true)?)
    .execute(&mut *transaction)
    .await
    .map_err(unique_group_error)?;
    let id = result.last_insert_rowid();
    replace_accounts(&mut transaction, id, &input.account_ids).await?;
    transaction.commit().await?;
    Ok((StatusCode::CREATED, Json(json!({"data": {"id": id}}))))
}

async fn update(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(input): Json<GroupInput>,
) -> ApiResult<Json<Value>> {
    let input = validate_input(&state, input).await?;
    if input.subscription_type != "subscription" {
        let dependent: i64 = sqlx::query_scalar(
            "SELECT (SELECT COUNT(*) FROM plans WHERE group_id = ?) + \
             (SELECT COUNT(*) FROM subscriptions WHERE group_id = ?)",
        )
        .bind(id)
        .bind(id)
        .fetch_one(&state.pool)
        .await?;
        if dependent > 0 {
            return Err(ApiError::bad_request(
                "GROUP_HAS_SUBSCRIPTIONS",
                "a group linked to plans or subscriptions must remain a subscription group",
            ));
        }
    }
    let mut transaction = state.pool.begin().await?;
    let result = sqlx::query(
        "UPDATE groups SET name = ?, description = ?, enabled = ?, allowed_models = ?, \
         sort_order = ?, platform = ?, is_exclusive = ?, subscription_type = ?, \
         rate_multiplier_micros = ?, peak_rate_enabled = ?, peak_start = ?, peak_end = ?, \
         peak_rate_multiplier_micros = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
    )
    .bind(input.name.trim())
    .bind(input.description.trim())
    .bind(input.enabled)
    .bind(serde_json::to_string(&input.allowed_models).unwrap())
    .bind(input.sort_order)
    .bind(&input.platform)
    .bind(input.is_exclusive)
    .bind(&input.subscription_type)
    .bind(multiplier_to_micros(input.rate_multiplier, false)?)
    .bind(input.peak_rate_enabled)
    .bind(&input.peak_start)
    .bind(&input.peak_end)
    .bind(multiplier_to_micros(input.peak_rate_multiplier, true)?)
    .bind(id)
    .execute(&mut *transaction)
    .await
    .map_err(unique_group_error)?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found("group not found"));
    }
    replace_accounts(&mut transaction, id, &input.account_ids).await?;
    transaction.commit().await?;
    Ok(Json(json!({"data": {"id": id}})))
}

async fn delete_group(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    let result = sqlx::query("DELETE FROM groups WHERE id = ?")
        .bind(id)
        .execute(&state.pool)
        .await
        .map_err(|error| match error {
            sqlx::Error::Database(_) => ApiError::bad_request(
                "GROUP_IN_USE",
                "a group linked to a plan or subscription cannot be deleted",
            ),
            other => other.into(),
        })?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found("group not found"));
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn validate_input(state: &AppState, mut input: GroupInput) -> ApiResult<GroupInput> {
    input.platform = input.platform.trim().to_ascii_lowercase();
    input.subscription_type = input.subscription_type.trim().to_ascii_lowercase();
    input.peak_start = input.peak_start.trim().to_string();
    input.peak_end = input.peak_end.trim().to_string();
    if input.name.trim().is_empty()
        || input.name.chars().count() > 80
        || input.description.len() > 1000
        || !(-10_000..=10_000).contains(&input.sort_order)
        || !valid_platform(&input.platform)
        || !matches!(
            input.subscription_type.as_str(),
            "standard" | "subscription"
        )
    {
        return Err(ApiError::bad_request(
            "INVALID_GROUP",
            "group fields are invalid",
        ));
    }
    multiplier_to_micros(input.rate_multiplier, false)?;
    multiplier_to_micros(input.peak_rate_multiplier, true)?;
    if input.subscription_type != "subscription" {
        input.peak_rate_enabled = false;
        input.peak_start.clear();
        input.peak_end.clear();
        input.peak_rate_multiplier = 1.0;
    } else if input.peak_rate_enabled {
        let start = parse_minutes(&input.peak_start).ok_or_else(|| {
            ApiError::bad_request("INVALID_PEAK_WINDOW", "peak_start must use HH:MM")
        })?;
        let end = parse_minutes(&input.peak_end).ok_or_else(|| {
            ApiError::bad_request("INVALID_PEAK_WINDOW", "peak_end must use HH:MM")
        })?;
        if start >= end {
            return Err(ApiError::bad_request(
                "INVALID_PEAK_WINDOW",
                "peak_end must be later than peak_start; overnight windows are not supported",
            ));
        }
    } else if (!input.peak_start.is_empty() && parse_minutes(&input.peak_start).is_none())
        || (!input.peak_end.is_empty() && parse_minutes(&input.peak_end).is_none())
    {
        return Err(ApiError::bad_request(
            "INVALID_PEAK_WINDOW",
            "peak times must use HH:MM",
        ));
    }
    input.allowed_models = crate::user::normalize_allowed_models(input.allowed_models)?;
    input.account_ids.sort_unstable();
    input.account_ids.dedup();
    if input.account_ids.len() > 1000 {
        return Err(ApiError::bad_request("INVALID_GROUP", "too many accounts"));
    }
    for account_id in &input.account_ids {
        let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM accounts WHERE id = ?")
            .bind(account_id)
            .fetch_one(&state.pool)
            .await?;
        if exists == 0 {
            return Err(ApiError::bad_request(
                "ACCOUNT_NOT_FOUND",
                "group account was not found",
            ));
        }
    }
    Ok(input)
}

async fn replace_accounts(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    group_id: i64,
    account_ids: &[i64],
) -> ApiResult<()> {
    sqlx::query("DELETE FROM account_groups WHERE group_id = ?")
        .bind(group_id)
        .execute(&mut **transaction)
        .await?;
    for account_id in account_ids {
        sqlx::query("INSERT INTO account_groups (account_id, group_id) VALUES (?, ?)")
            .bind(account_id)
            .bind(group_id)
            .execute(&mut **transaction)
            .await?;
    }
    Ok(())
}

fn group_value(row: GroupRow, account_ids: Vec<i64>) -> Value {
    let now = Utc::now();
    let (offset_minutes, offset_label) = server_utc_offset();
    let (applied_peak, effective) = effective_rate_micros_at(
        row.rate_multiplier_micros,
        None,
        &row.subscription_type,
        row.peak_rate_enabled,
        &row.peak_start,
        &row.peak_end,
        row.peak_rate_multiplier_micros,
        now,
        offset_minutes,
    );
    json!({
        "id": row.id, "name": row.name, "description": row.description,
        "enabled": row.enabled, "allowed_models": parse_models(&row.allowed_models),
        "sort_order": row.sort_order, "account_ids": account_ids,
        "platform": row.platform, "platform_label": platform_label(&row.platform),
        "is_exclusive": row.is_exclusive, "subscription_type": row.subscription_type,
        "rate_multiplier": micros_to_multiplier(row.rate_multiplier_micros),
        "peak_rate_enabled": row.peak_rate_enabled, "peak_start": row.peak_start,
        "peak_end": row.peak_end,
        "peak_rate_multiplier": micros_to_multiplier(row.peak_rate_multiplier_micros),
        "applied_peak_multiplier": micros_to_multiplier(applied_peak),
        "effective_rate_multiplier": micros_to_multiplier(effective),
        "active_subscriptions": row.active_subscriptions, "allowed_users": row.allowed_users,
        "server_utc_offset": offset_label, "observed_at": now.to_rfc3339(),
        "created_at": row.created_at, "updated_at": row.updated_at
    })
}

pub(crate) async fn ensure_user_group_access(
    state: &AppState,
    user_id: i64,
    group_id: i64,
) -> ApiResult<()> {
    let row: Option<(String, bool, String, bool)> = sqlx::query_as(
        "SELECT users.role, users.allow_all_standard_groups, groups.subscription_type, \
         groups.is_exclusive FROM users CROSS JOIN groups \
         WHERE users.id = ? AND users.enabled = 1 AND users.deleted_at IS NULL \
         AND groups.id = ? AND groups.enabled = 1",
    )
    .bind(user_id)
    .bind(group_id)
    .fetch_optional(&state.pool)
    .await?;
    let (role, allow_all, subscription_type, exclusive) = row.ok_or_else(|| {
        ApiError::bad_request("GROUP_NOT_FOUND", "enabled group or user was not found")
    })?;
    if role == "admin" {
        return Ok(());
    }
    if subscription_type == "subscription" {
        let active: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM subscriptions WHERE user_id = ? AND group_id = ? \
             AND status = 'active' AND datetime(ends_at) > CURRENT_TIMESTAMP",
        )
        .bind(user_id)
        .bind(group_id)
        .fetch_one(&state.pool)
        .await?;
        if active == 0 {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "GROUP_SUBSCRIPTION_REQUIRED",
                "an active subscription is required for this group",
            ));
        }
        return Ok(());
    }
    let explicitly_allowed: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM user_allowed_groups WHERE user_id = ? AND group_id = ?",
    )
    .bind(user_id)
    .bind(group_id)
    .fetch_one(&state.pool)
    .await?;
    if explicitly_allowed == 0 && (exclusive || !allow_all) {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "GROUP_ACCESS_DENIED",
            "the API key owner is not allowed to use this group",
        ));
    }
    Ok(())
}

#[derive(Deserialize)]
struct RateMultiplierEntryInput {
    user_id: i64,
    rate_multiplier: f64,
}

#[derive(Deserialize)]
struct BatchRateMultiplierInput {
    #[serde(default)]
    entries: Vec<RateMultiplierEntryInput>,
}

async fn list_rate_multipliers(
    State(state): State<AppState>,
    Path(group_id): Path<i64>,
) -> ApiResult<Json<Value>> {
    ensure_group(&state, group_id).await?;
    let rows: Vec<(i64, String, String, Option<String>, String, bool, i64)> = sqlx::query_as(
        "SELECT users.id, users.username, users.display_name, users.email, users.notes, \
         users.enabled, rates.rate_multiplier_micros FROM user_group_rate_multipliers rates \
         JOIN users ON users.id = rates.user_id WHERE rates.group_id = ? \
         AND users.deleted_at IS NULL ORDER BY users.username COLLATE NOCASE, users.id",
    )
    .bind(group_id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(json!({"data": rows.into_iter().map(|row| json!({
        "user_id": row.0, "user_name": row.1, "display_name": row.2,
        "user_email": row.3, "user_notes": row.4,
        "user_status": if row.5 { "active" } else { "disabled" },
        "rate_multiplier": micros_to_multiplier(row.6)
    })).collect::<Vec<_>>() })))
}

async fn batch_set_rate_multipliers(
    State(state): State<AppState>,
    Path(group_id): Path<i64>,
    Json(input): Json<BatchRateMultiplierInput>,
) -> ApiResult<Json<Value>> {
    if input.entries.len() > 10_000 {
        return Err(ApiError::bad_request(
            "INVALID_RATE_MULTIPLIERS",
            "too many rate multiplier entries",
        ));
    }
    let mut seen = HashSet::with_capacity(input.entries.len());
    let mut entries = Vec::with_capacity(input.entries.len());
    for entry in input.entries {
        if entry.user_id <= 0 || !seen.insert(entry.user_id) {
            return Err(ApiError::bad_request(
                "INVALID_RATE_MULTIPLIERS",
                "user IDs must be positive and unique",
            ));
        }
        entries.push((
            entry.user_id,
            multiplier_to_micros(entry.rate_multiplier, false)?,
        ));
    }
    let mut transaction = state.pool.begin().await?;
    let group_exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM groups WHERE id = ?")
        .bind(group_id)
        .fetch_one(&mut *transaction)
        .await?;
    if group_exists == 0 {
        return Err(ApiError::not_found("group not found"));
    }
    for (user_id, _) in &entries {
        let exists: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE id = ? AND deleted_at IS NULL")
                .bind(user_id)
                .fetch_one(&mut *transaction)
                .await?;
        if exists == 0 {
            return Err(ApiError::bad_request(
                "USER_NOT_FOUND",
                format!("user {user_id} was not found"),
            ));
        }
    }
    sqlx::query("DELETE FROM user_group_rate_multipliers WHERE group_id = ?")
        .bind(group_id)
        .execute(&mut *transaction)
        .await?;
    for (user_id, rate) in &entries {
        sqlx::query(
            "INSERT INTO user_group_rate_multipliers \
             (user_id, group_id, rate_multiplier_micros) VALUES (?, ?, ?)",
        )
        .bind(user_id)
        .bind(group_id)
        .bind(rate)
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;
    Ok(Json(json!({"data": {"updated": entries.len()}})))
}

async fn clear_rate_multipliers(
    State(state): State<AppState>,
    Path(group_id): Path<i64>,
) -> ApiResult<StatusCode> {
    ensure_group(&state, group_id).await?;
    sqlx::query("DELETE FROM user_group_rate_multipliers WHERE group_id = ?")
        .bind(group_id)
        .execute(&state.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn user_rates(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
) -> ApiResult<Json<Value>> {
    let rows: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT group_id, rate_multiplier_micros FROM user_group_rate_multipliers \
         WHERE user_id = ? ORDER BY group_id",
    )
    .bind(session.user_id)
    .fetch_all(&state.pool)
    .await?;
    let mut rates = Map::with_capacity(rows.len());
    for (group_id, rate) in rows {
        rates.insert(group_id.to_string(), json!(micros_to_multiplier(rate)));
    }
    Ok(Json(json!({"data": rates})))
}

async fn ensure_group(state: &AppState, group_id: i64) -> ApiResult<()> {
    let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM groups WHERE id = ?")
        .bind(group_id)
        .fetch_one(&state.pool)
        .await?;
    if exists == 0 {
        return Err(ApiError::not_found("group not found"));
    }
    Ok(())
}

fn parse_models(value: &str) -> Vec<String> {
    serde_json::from_str(value).unwrap_or_default()
}

fn valid_platform(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

pub(crate) fn platform_label(platform: &str) -> String {
    match platform {
        "openai" => "OpenAI".into(),
        "codex" | "openai-codex" => "OpenAI Codex".into(),
        "anthropic" => "Anthropic".into(),
        "gemini" | "google" => "Google Gemini".into(),
        "grok" => "xAI Grok".into(),
        value => value.to_ascii_uppercase(),
    }
}

pub(crate) fn platform_category(platform: &str) -> &'static str {
    match platform {
        "openai" | "codex" | "openai-codex" | "grok" => "openai-compatible",
        "anthropic" => "anthropic",
        "gemini" | "google" => "google",
        _ => "custom",
    }
}

fn multiplier_to_micros(value: f64, allow_zero: bool) -> ApiResult<i64> {
    let minimum_valid = if allow_zero {
        value >= 0.0
    } else {
        value > 0.0
    };
    if !value.is_finite() || !minimum_valid || value > 1000.0 {
        return Err(ApiError::bad_request(
            "INVALID_RATE_MULTIPLIER",
            if allow_zero {
                "rate multiplier must be between 0 and 1000"
            } else {
                "rate multiplier must be greater than 0 and at most 1000"
            },
        ));
    }
    let micros = (value * MULTIPLIER_SCALE as f64).round() as i64;
    if (!allow_zero && micros == 0) || !(0..=MAX_MULTIPLIER_MICROS).contains(&micros) {
        return Err(ApiError::bad_request(
            "INVALID_RATE_MULTIPLIER",
            "rate multiplier has too much precision or is outside the supported range",
        ));
    }
    Ok(micros)
}

pub(crate) fn micros_to_multiplier(value: i64) -> f64 {
    value as f64 / MULTIPLIER_SCALE as f64
}

fn parse_minutes(value: &str) -> Option<i32> {
    let bytes = value.as_bytes();
    if bytes.len() != 5
        || bytes[2] != b':'
        || !bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| index == 2 || byte.is_ascii_digit())
    {
        return None;
    }
    let hour = ((bytes[0] - b'0') * 10 + bytes[1] - b'0') as i32;
    let minute = ((bytes[3] - b'0') * 10 + bytes[4] - b'0') as i32;
    (hour < 24 && minute < 60).then_some(hour * 60 + minute)
}

fn parse_utc_offset(value: &str) -> Option<i32> {
    let bytes = value.as_bytes();
    if bytes.len() != 6
        || !matches!(bytes[0], b'+' | b'-')
        || bytes[3] != b':'
        || ![bytes[1], bytes[2], bytes[4], bytes[5]]
            .into_iter()
            .all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let hour = ((bytes[1].checked_sub(b'0')?) * 10 + bytes[2].checked_sub(b'0')?) as i32;
    let minute = ((bytes[4].checked_sub(b'0')?) * 10 + bytes[5].checked_sub(b'0')?) as i32;
    if hour > 14 || minute > 59 || (hour == 14 && minute != 0) {
        return None;
    }
    let total = hour * 60 + minute;
    Some(if bytes[0] == b'-' { -total } else { total })
}

pub(crate) fn server_utc_offset() -> (i32, String) {
    let configured =
        std::env::var("SUB2API_MINI_UTC_OFFSET").unwrap_or_else(|_| DEFAULT_UTC_OFFSET.into());
    parse_utc_offset(&configured)
        .map(|minutes| (minutes, configured))
        .unwrap_or_else(|| (480, DEFAULT_UTC_OFFSET.into()))
}

pub(crate) fn effective_rate_micros_at(
    group_rate: i64,
    user_rate: Option<i64>,
    subscription_type: &str,
    peak_enabled: bool,
    peak_start: &str,
    peak_end: &str,
    peak_rate: i64,
    now: DateTime<Utc>,
    utc_offset_minutes: i32,
) -> (i64, i64) {
    let applied_peak = peak_multiplier_micros_at(
        subscription_type,
        peak_enabled,
        peak_start,
        peak_end,
        peak_rate,
        now,
        utc_offset_minutes,
    );
    let resolved = user_rate
        .unwrap_or(group_rate)
        .clamp(1, MAX_MULTIPLIER_MICROS);
    let effective = ((resolved as i128 * applied_peak as i128 + 500_000) / 1_000_000)
        .clamp(0, i64::MAX as i128) as i64;
    (applied_peak, effective)
}

fn peak_multiplier_micros_at(
    subscription_type: &str,
    enabled: bool,
    start: &str,
    end: &str,
    multiplier: i64,
    now: DateTime<Utc>,
    utc_offset_minutes: i32,
) -> i64 {
    if subscription_type != "subscription" || !enabled {
        return MULTIPLIER_SCALE;
    }
    let (Some(start), Some(end)) = (parse_minutes(start), parse_minutes(end)) else {
        return MULTIPLIER_SCALE;
    };
    if start >= end || !(0..=MAX_MULTIPLIER_MICROS).contains(&multiplier) {
        return MULTIPLIER_SCALE;
    }
    let offset = FixedOffset::east_opt(utc_offset_minutes.clamp(-840, 840) * 60)
        .unwrap_or_else(|| FixedOffset::east_opt(0).unwrap());
    let local = now.with_timezone(&offset);
    let current = local.hour() as i32 * 60 + local.minute() as i32;
    if current >= start && current < end {
        multiplier
    } else {
        MULTIPLIER_SCALE
    }
}

fn unique_group_error(error: sqlx::Error) -> ApiError {
    match error {
        sqlx::Error::Database(ref database) if database.is_unique_violation() => {
            ApiError::bad_request("GROUP_NAME_EXISTS", "group name already exists")
        }
        other => other.into(),
    }
}

#[cfg(test)]
mod tests {
    use axum::extract::{Extension, Path, State};
    use chrono::TimeZone;

    use super::*;
    use crate::test_support;

    #[test]
    fn peak_window_uses_server_offset_and_left_closed_interval() {
        let at = |hour, minute| Utc.with_ymd_and_hms(2026, 7, 23, hour, minute, 0).unwrap();
        assert_eq!(
            peak_multiplier_micros_at(
                "subscription",
                true,
                "14:00",
                "18:00",
                2_000_000,
                at(6, 0),
                480,
            ),
            2_000_000
        );
        assert_eq!(
            peak_multiplier_micros_at(
                "subscription",
                true,
                "14:00",
                "18:00",
                2_000_000,
                at(10, 0),
                480,
            ),
            1_000_000
        );
        assert_eq!(
            peak_multiplier_micros_at("standard", true, "14:00", "18:00", 2_000_000, at(7, 0), 480,),
            1_000_000
        );
    }

    #[test]
    fn user_rate_overrides_group_before_peak_is_applied() {
        let now = Utc.with_ymd_and_hms(2026, 7, 23, 7, 30, 0).unwrap();
        let (peak, effective) = effective_rate_micros_at(
            800_000,
            Some(600_000),
            "subscription",
            true,
            "14:00",
            "18:00",
            1_500_000,
            now,
            480,
        );
        assert_eq!(peak, 1_500_000);
        assert_eq!(effective, 900_000);
    }

    #[test]
    fn validates_time_and_utc_offset_formats() {
        assert_eq!(parse_minutes("09:05"), Some(545));
        assert_eq!(parse_minutes("9:05"), None);
        assert_eq!(parse_minutes("24:00"), None);
        assert_eq!(parse_utc_offset("+08:00"), Some(480));
        assert_eq!(parse_utc_offset("-05:30"), Some(-330));
        assert_eq!(parse_utc_offset("+14:30"), None);
    }

    #[tokio::test]
    async fn resolves_public_exclusive_and_subscription_group_access() {
        let (_directory, state) = test_support::state().await;
        let user_id = sqlx::query(
            "INSERT INTO users (username, display_name, password_hash) \
             VALUES ('group-user', 'Group User', 'unused')",
        )
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        let public_id = sqlx::query("INSERT INTO groups (name) VALUES ('public-group')")
            .execute(&state.pool)
            .await
            .unwrap()
            .last_insert_rowid();
        let exclusive_id =
            sqlx::query("INSERT INTO groups (name, is_exclusive) VALUES ('exclusive-group', 1)")
                .execute(&state.pool)
                .await
                .unwrap()
                .last_insert_rowid();
        let subscription_id = sqlx::query(
            "INSERT INTO groups (name, subscription_type) VALUES ('subscription-group', 'subscription')",
        )
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();

        ensure_user_group_access(&state, user_id, public_id)
            .await
            .unwrap();
        assert!(
            ensure_user_group_access(&state, user_id, exclusive_id)
                .await
                .is_err()
        );
        assert!(
            ensure_user_group_access(&state, user_id, subscription_id)
                .await
                .is_err()
        );

        sqlx::query("INSERT INTO user_allowed_groups (user_id, group_id) VALUES (?, ?)")
            .bind(user_id)
            .bind(exclusive_id)
            .execute(&state.pool)
            .await
            .unwrap();
        ensure_user_group_access(&state, user_id, exclusive_id)
            .await
            .unwrap();

        let plan_id = sqlx::query(
            "INSERT INTO plans (name, token_limit, duration_days, group_id) \
             VALUES ('group-plan', 100, 30, ?)",
        )
        .bind(subscription_id)
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        sqlx::query(
            "INSERT INTO subscriptions \
             (user_id, plan_id, group_id, token_limit, starts_at, ends_at) \
             VALUES (?, ?, ?, 100, datetime('now', '-1 hour'), datetime('now', '+1 day'))",
        )
        .bind(user_id)
        .bind(plan_id)
        .bind(subscription_id)
        .execute(&state.pool)
        .await
        .unwrap();
        ensure_user_group_access(&state, user_id, subscription_id)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn persists_group_policy_and_replaces_user_rate_overrides() {
        let (_directory, state) = test_support::state().await;
        let user_id = sqlx::query(
            "INSERT INTO users (username, display_name, password_hash) \
             VALUES ('rate-user', 'Rate User', 'unused')",
        )
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        let (_, Json(created)) = create(
            State(state.clone()),
            Json(GroupInput {
                name: "Subscriber".into(),
                description: "Peak pricing".into(),
                enabled: true,
                allowed_models: vec!["gpt-5".into()],
                sort_order: 3,
                account_ids: Vec::new(),
                platform: "OpenAI".into(),
                is_exclusive: false,
                subscription_type: "subscription".into(),
                rate_multiplier: 0.8,
                peak_rate_enabled: true,
                peak_start: "14:00".into(),
                peak_end: "18:00".into(),
                peak_rate_multiplier: 1.5,
            }),
        )
        .await
        .unwrap();
        let group_id = created["data"]["id"].as_i64().unwrap();

        let _ = batch_set_rate_multipliers(
            State(state.clone()),
            Path(group_id),
            Json(BatchRateMultiplierInput {
                entries: vec![RateMultiplierEntryInput {
                    user_id,
                    rate_multiplier: 0.6,
                }],
            }),
        )
        .await
        .unwrap();
        let Json(rates) = user_rates(
            State(state.clone()),
            Extension(AuthSession {
                id: 1,
                user_id,
                username: "rate-user".into(),
                display_name: "Rate User".into(),
                role: "user".into(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(rates["data"][group_id.to_string()], 0.6);

        let stored: (String, String, i64, bool, i64) = sqlx::query_as(
            "SELECT platform, subscription_type, rate_multiplier_micros, \
             peak_rate_enabled, peak_rate_multiplier_micros FROM groups WHERE id = ?",
        )
        .bind(group_id)
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(
            stored,
            (
                "openai".into(),
                "subscription".into(),
                800_000,
                true,
                1_500_000
            )
        );

        let _ = batch_set_rate_multipliers(
            State(state.clone()),
            Path(group_id),
            Json(BatchRateMultiplierInput { entries: vec![] }),
        )
        .await
        .unwrap();
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM user_group_rate_multipliers WHERE group_id = ?",
        )
        .bind(group_id)
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(count, 0);
    }
}
