use axum::{
    Json, Router,
    extract::{Extension, Path, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{FromRow, Sqlite, Transaction};

use crate::{
    auth::{self, AuthSession},
    crypto::{hash_password, random_token},
    error::{ApiError, ApiResult},
    key_policy,
    models::deserialize_nullable,
    state::AppState,
};

const MAX_BALANCE_CENTS: i64 = 100_000_000_000_000;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/users", get(list_users).post(create_user))
        .route("/users/batch", post(batch_users))
        .route(
            "/users/{id}",
            get(user_detail).put(update_user).delete(delete_user),
        )
        .route(
            "/users/{id}/groups",
            get(user_groups).put(update_user_groups),
        )
        .route("/users/{id}/balance", post(adjust_balance))
}

#[derive(Debug, Serialize, FromRow)]
struct UserSummary {
    id: i64,
    username: String,
    display_name: String,
    email: Option<String>,
    email_verified: bool,
    role: String,
    enabled: bool,
    balance_cents: i64,
    notes: String,
    created_at: String,
    updated_at: String,
    key_count: i64,
    total_requests: i64,
    total_tokens: i64,
    total_cost_microusd: i64,
    active_subscriptions: i64,
    last_request_at: Option<String>,
}

const USER_SUMMARY_SELECT: &str = "SELECT users.id, users.username, users.display_name, users.email, users.email_verified, \
     users.role, users.enabled, users.balance_cents, users.notes, users.created_at, users.updated_at, \
     (SELECT COUNT(*) FROM api_keys WHERE api_keys.user_id = users.id) AS key_count, \
     (SELECT COUNT(*) FROM usage_logs WHERE usage_logs.user_id = users.id) AS total_requests, \
     COALESCE((SELECT SUM(COALESCE(usage_logs.total_tokens, 0)) FROM usage_logs \
       WHERE usage_logs.user_id = users.id), 0) AS total_tokens, \
     COALESCE((SELECT SUM(usage_logs.cost_microusd) FROM usage_logs \
       WHERE usage_logs.user_id = users.id), 0) AS total_cost_microusd, \
     (SELECT COUNT(*) FROM subscriptions WHERE subscriptions.user_id = users.id \
       AND subscriptions.status = 'active' AND datetime(subscriptions.ends_at) > CURRENT_TIMESTAMP) \
       AS active_subscriptions, \
     (SELECT MAX(usage_logs.created_at) FROM usage_logs WHERE usage_logs.user_id = users.id) \
       AS last_request_at FROM users";

async fn list_users(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let query = format!(
        "{USER_SUMMARY_SELECT} WHERE users.deleted_at IS NULL \
         ORDER BY users.role ASC, users.id ASC"
    );
    let users = sqlx::query_as::<_, UserSummary>(&query)
        .fetch_all(&state.pool)
        .await?;
    Ok(Json(json!({"data": users})))
}

async fn user_detail(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    let query = format!("{USER_SUMMARY_SELECT} WHERE users.id = ? AND users.deleted_at IS NULL");
    let user = sqlx::query_as::<_, UserSummary>(&query)
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| ApiError::not_found("user not found"))?;
    let keys = key_policy::list_keys(&state.pool, Some(id)).await?;
    let subscriptions: Vec<(
        i64,
        i64,
        String,
        String,
        i64,
        String,
        String,
        String,
        i64,
        Option<i64>,
        Option<String>,
    )> = sqlx::query_as(
        "SELECT subscriptions.id, subscriptions.plan_id, plans.name, subscriptions.status, \
         subscriptions.token_limit, subscriptions.starts_at, subscriptions.ends_at, \
         subscriptions.created_at, COALESCE((SELECT SUM(COALESCE(log.total_tokens, 0)) \
           FROM usage_logs log LEFT JOIN api_keys keys ON keys.id = log.api_key_id \
           WHERE log.user_id = subscriptions.user_id AND \
           datetime(log.created_at) >= datetime(subscriptions.starts_at) AND \
           datetime(log.created_at) < datetime(subscriptions.ends_at) AND \
           (subscriptions.group_id IS NULL OR keys.group_id = subscriptions.group_id)), 0), \
         subscriptions.group_id, groups.name \
         FROM subscriptions JOIN plans ON plans.id = subscriptions.plan_id \
         LEFT JOIN groups ON groups.id = subscriptions.group_id \
         WHERE subscriptions.user_id = ? ORDER BY subscriptions.id DESC LIMIT 50",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await?;
    let orders: Vec<(
        i64,
        i64,
        String,
        i64,
        String,
        String,
        Option<String>,
        Option<String>,
        String,
    )> = sqlx::query_as(
        "SELECT orders.id, orders.plan_id, plans.name, orders.amount_cents, orders.provider, \
             orders.status, orders.paid_at, orders.refunded_at, orders.created_at \
             FROM orders JOIN plans ON plans.id = orders.plan_id WHERE orders.user_id = ? \
             ORDER BY orders.id DESC LIMIT 50",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await?;
    let adjustments: Vec<(i64, i64, i64, String, String, Option<String>)> = sqlx::query_as(
        "SELECT adjustments.id, adjustments.delta_cents, adjustments.balance_after_cents, \
         adjustments.reason, adjustments.created_at, admins.username \
         FROM user_balance_adjustments adjustments \
         LEFT JOIN users admins ON admins.id = adjustments.admin_id \
         WHERE adjustments.user_id = ? ORDER BY adjustments.id DESC LIMIT 100",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await?;
    let trend: Vec<(String, i64, i64, i64)> = sqlx::query_as(
        "SELECT date(created_at), COUNT(*), COALESCE(SUM(total_tokens), 0), \
         COALESCE(SUM(cost_microusd), 0) FROM usage_logs WHERE user_id = ? \
         AND datetime(created_at) >= datetime('now', '-30 days') \
         GROUP BY date(created_at) ORDER BY date(created_at)",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await?;
    let external_attributes: Vec<(String, String, String, String, String)> = sqlx::query_as(
        "SELECT provider, attribute_key, attribute_name, value, updated_at \
         FROM user_external_attributes WHERE user_id = ? \
         ORDER BY provider, attribute_name, attribute_key",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(json!({"data": {
        "user": user,
        "keys": keys,
        "subscriptions": subscriptions.into_iter().map(|row| json!({
            "id": row.0, "plan_id": row.1, "plan_name": row.2, "status": row.3,
            "token_limit": row.4, "starts_at": row.5, "ends_at": row.6,
            "created_at": row.7, "used_tokens": row.8,
            "group_id": row.9, "group_name": row.10
        })).collect::<Vec<_>>(),
        "orders": orders.into_iter().map(|row| json!({
            "id": row.0, "plan_id": row.1, "plan_name": row.2, "amount_cents": row.3,
            "provider": row.4, "status": row.5, "paid_at": row.6,
            "refunded_at": row.7, "created_at": row.8
        })).collect::<Vec<_>>(),
        "balance_adjustments": adjustments.into_iter().map(|row| json!({
            "id": row.0, "delta_cents": row.1, "balance_after_cents": row.2,
            "reason": row.3, "created_at": row.4, "admin_username": row.5
        })).collect::<Vec<_>>(),
        "trend": trend.into_iter().map(|row| json!({
            "date": row.0, "requests": row.1, "tokens": row.2, "cost_microusd": row.3
        })).collect::<Vec<_>>(),
        "external_attributes": external_attributes.into_iter().map(|row| json!({
            "provider": row.0, "key": row.1, "name": row.2,
            "value": row.3, "updated_at": row.4
        })).collect::<Vec<_>>()
    }})))
}

#[derive(Deserialize)]
struct CreateUserInput {
    username: String,
    #[serde(default)]
    display_name: String,
    email: Option<String>,
    password: String,
    #[serde(default)]
    notes: String,
    #[serde(default)]
    balance_cents: i64,
}

async fn create_user(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
    Json(input): Json<CreateUserInput>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    validate_username(&input.username)?;
    validate_password(&input.password)?;
    validate_notes(&input.notes)?;
    validate_balance(input.balance_cents)?;
    let display_name = if input.display_name.trim().is_empty() {
        input.username.trim()
    } else {
        input.display_name.trim()
    };
    validate_display_name(display_name)?;
    let email = input
        .email
        .as_deref()
        .map(auth::normalize_email)
        .transpose()?;
    let mut transaction = state.pool.begin().await?;
    let result = sqlx::query(
        "INSERT INTO users \
         (username, display_name, password_hash, role, email, email_verified, balance_cents, notes) \
         VALUES (?, ?, ?, 'user', ?, ?, ?, ?)",
    )
    .bind(input.username.trim())
    .bind(display_name)
    .bind(hash_password(&input.password)?)
    .bind(&email)
    .bind(email.is_some())
    .bind(input.balance_cents)
    .bind(input.notes.trim())
    .execute(&mut *transaction)
    .await
    .map_err(map_user_unique_error)?;
    let id = result.last_insert_rowid();
    if input.balance_cents > 0 {
        sqlx::query(
            "INSERT INTO user_balance_adjustments \
             (user_id, admin_id, delta_cents, balance_after_cents, reason) \
             VALUES (?, ?, ?, ?, 'initial administrator balance')",
        )
        .bind(id)
        .bind(session.user_id)
        .bind(input.balance_cents)
        .bind(input.balance_cents)
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;
    Ok((StatusCode::CREATED, Json(json!({"data": {"id": id}}))))
}

#[derive(Deserialize)]
struct UpdateUserInput {
    username: Option<String>,
    display_name: Option<String>,
    #[serde(default, deserialize_with = "deserialize_nullable")]
    email: Option<Option<String>>,
    email_verified: Option<bool>,
    notes: Option<String>,
    password: Option<String>,
    enabled: Option<bool>,
}

async fn update_user(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(input): Json<UpdateUserInput>,
) -> ApiResult<Json<Value>> {
    ensure_regular_user(&state, id).await?;
    let username = input
        .username
        .as_deref()
        .map(|value| {
            validate_username(value)?;
            Ok::<_, ApiError>(value.trim().to_string())
        })
        .transpose()?;
    let display_name = input
        .display_name
        .as_deref()
        .map(|value| {
            validate_display_name(value)?;
            Ok::<_, ApiError>(value.trim().to_string())
        })
        .transpose()?;
    let email = input
        .email
        .map(|value| value.as_deref().map(auth::normalize_email).transpose())
        .transpose()?;
    let notes = input
        .notes
        .as_deref()
        .map(|value| {
            validate_notes(value)?;
            Ok::<_, ApiError>(value.trim().to_string())
        })
        .transpose()?;
    let password_hash = input
        .password
        .as_deref()
        .map(|password| {
            validate_password(password)?;
            hash_password(password)
        })
        .transpose()?;

    let mut transaction = state.pool.begin().await?;
    if let Some(username) = username {
        sqlx::query("UPDATE users SET username = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?")
            .bind(username)
            .bind(id)
            .execute(&mut *transaction)
            .await
            .map_err(map_user_unique_error)?;
    }
    if let Some(display_name) = display_name {
        sqlx::query(
            "UPDATE users SET display_name = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
        )
        .bind(display_name)
        .bind(id)
        .execute(&mut *transaction)
        .await?;
    }
    if let Some(email) = email {
        let verified = input.email_verified.unwrap_or(email.is_some());
        sqlx::query(
            "UPDATE users SET email = ?, email_verified = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
        )
        .bind(email)
        .bind(verified)
        .bind(id)
        .execute(&mut *transaction)
        .await
        .map_err(map_user_unique_error)?;
    } else if let Some(verified) = input.email_verified {
        sqlx::query(
            "UPDATE users SET email_verified = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
        )
        .bind(verified)
        .bind(id)
        .execute(&mut *transaction)
        .await?;
    }
    if let Some(notes) = notes {
        sqlx::query("UPDATE users SET notes = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?")
            .bind(notes)
            .bind(id)
            .execute(&mut *transaction)
            .await?;
    }
    let mut revoke_sessions = false;
    if let Some(enabled) = input.enabled {
        sqlx::query("UPDATE users SET enabled = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?")
            .bind(enabled)
            .bind(id)
            .execute(&mut *transaction)
            .await?;
        revoke_sessions = !enabled;
    }
    if let Some(password_hash) = password_hash {
        sqlx::query(
            "UPDATE users SET password_hash = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
        )
        .bind(password_hash)
        .bind(id)
        .execute(&mut *transaction)
        .await?;
        revoke_sessions = true;
    }
    if revoke_sessions {
        sqlx::query("DELETE FROM auth_sessions WHERE user_id = ?")
            .bind(id)
            .execute(&mut *transaction)
            .await?;
    }
    transaction.commit().await?;
    Ok(Json(json!({"data": {"id": id}})))
}

async fn user_groups(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    ensure_regular_user(&state, id).await?;
    let allow_all_standard_groups: bool =
        sqlx::query_scalar("SELECT allow_all_standard_groups FROM users WHERE id = ?")
            .bind(id)
            .fetch_one(&state.pool)
            .await?;
    let allowed_group_ids: Vec<i64> = sqlx::query_scalar(
        "SELECT group_id FROM user_allowed_groups WHERE user_id = ? ORDER BY group_id",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(json!({"data": {
        "user_id": id,
        "allow_all_standard_groups": allow_all_standard_groups,
        "allowed_group_ids": allowed_group_ids
    }})))
}

#[derive(Deserialize)]
struct UserGroupsInput {
    #[serde(default = "default_allow_all_groups")]
    allow_all_standard_groups: bool,
    #[serde(default)]
    allowed_group_ids: Vec<i64>,
}

fn default_allow_all_groups() -> bool {
    true
}

async fn update_user_groups(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(mut input): Json<UserGroupsInput>,
) -> ApiResult<Json<Value>> {
    ensure_regular_user(&state, id).await?;
    input.allowed_group_ids.sort_unstable();
    input.allowed_group_ids.dedup();
    if input.allowed_group_ids.len() > 1000 {
        return Err(ApiError::bad_request(
            "INVALID_GROUP_ACCESS",
            "at most 1000 groups can be assigned",
        ));
    }
    if !input.allowed_group_ids.is_empty() {
        let placeholders = std::iter::repeat_n("?", input.allowed_group_ids.len())
            .collect::<Vec<_>>()
            .join(",");
        let query = format!(
            "SELECT COUNT(*) FROM groups WHERE subscription_type = 'standard' AND id IN ({placeholders})"
        );
        let mut query = sqlx::query_scalar::<_, i64>(&query);
        for group_id in &input.allowed_group_ids {
            query = query.bind(group_id);
        }
        if query.fetch_one(&state.pool).await? != input.allowed_group_ids.len() as i64 {
            return Err(ApiError::bad_request(
                "INVALID_GROUP_ACCESS",
                "only existing standard groups can be assigned directly",
            ));
        }
    }
    let mut transaction = state.pool.begin().await?;
    sqlx::query(
        "UPDATE users SET allow_all_standard_groups = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
    )
    .bind(input.allow_all_standard_groups)
    .bind(id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query("DELETE FROM user_allowed_groups WHERE user_id = ?")
        .bind(id)
        .execute(&mut *transaction)
        .await?;
    for group_id in &input.allowed_group_ids {
        sqlx::query("INSERT INTO user_allowed_groups (user_id, group_id) VALUES (?, ?)")
            .bind(id)
            .bind(group_id)
            .execute(&mut *transaction)
            .await?;
    }
    transaction.commit().await?;
    Ok(Json(json!({"data": {
        "user_id": id,
        "allow_all_standard_groups": input.allow_all_standard_groups,
        "allowed_group_ids": input.allowed_group_ids
    }})))
}

#[derive(Deserialize)]
struct BalanceInput {
    delta_cents: i64,
    reason: String,
}

async fn adjust_balance(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
    Path(id): Path<i64>,
    Json(input): Json<BalanceInput>,
) -> ApiResult<Json<Value>> {
    if input.delta_cents == 0 || input.delta_cents.unsigned_abs() > MAX_BALANCE_CENTS as u64 {
        return Err(ApiError::bad_request(
            "INVALID_BALANCE_DELTA",
            "delta_cents must be non-zero and within the supported range",
        ));
    }
    let reason = input.reason.trim();
    if reason.is_empty() || reason.chars().count() > 200 {
        return Err(ApiError::bad_request(
            "INVALID_BALANCE_REASON",
            "reason must be 1-200 characters",
        ));
    }
    let mut transaction = state.pool.begin().await?;
    let current: Option<(String, i64)> =
        sqlx::query_as("SELECT role, balance_cents FROM users WHERE id = ? AND deleted_at IS NULL")
            .bind(id)
            .fetch_optional(&mut *transaction)
            .await?;
    let (role, current) = current.ok_or_else(|| ApiError::not_found("user not found"))?;
    if role == "admin" {
        return Err(ApiError::forbidden(
            "the administrator balance cannot be modified",
        ));
    }
    let balance = current
        .checked_add(input.delta_cents)
        .ok_or_else(|| ApiError::bad_request("INVALID_BALANCE_DELTA", "balance would overflow"))?;
    validate_balance(balance)?;
    sqlx::query("UPDATE users SET balance_cents = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?")
        .bind(balance)
        .bind(id)
        .execute(&mut *transaction)
        .await?;
    let result = sqlx::query(
        "INSERT INTO user_balance_adjustments \
         (user_id, admin_id, delta_cents, balance_after_cents, reason) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(session.user_id)
    .bind(input.delta_cents)
    .bind(balance)
    .bind(reason)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(Json(json!({"data": {
        "id": result.last_insert_rowid(), "user_id": id,
        "delta_cents": input.delta_cents, "balance_cents": balance, "reason": reason
    }})))
}

#[derive(Deserialize)]
struct BatchUsersInput {
    ids: Vec<i64>,
    action: String,
}

async fn batch_users(
    State(state): State<AppState>,
    Json(input): Json<BatchUsersInput>,
) -> ApiResult<Json<Value>> {
    let mut ids = input
        .ids
        .into_iter()
        .filter(|id| *id > 0)
        .collect::<Vec<_>>();
    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() || ids.len() > 200 {
        return Err(ApiError::bad_request(
            "INVALID_USER_SELECTION",
            "select between 1 and 200 users",
        ));
    }
    if !matches!(input.action.as_str(), "enable" | "disable" | "delete") {
        return Err(ApiError::bad_request(
            "INVALID_USER_BATCH_ACTION",
            "unsupported user batch action",
        ));
    }
    let mut transaction = state.pool.begin().await?;
    let mut affected_ids = Vec::new();
    let mut skipped = Vec::new();
    for id in ids {
        let role: Option<String> =
            sqlx::query_scalar("SELECT role FROM users WHERE id = ? AND deleted_at IS NULL")
                .bind(id)
                .fetch_optional(&mut *transaction)
                .await?;
        if role.as_deref() != Some("user") {
            skipped.push(json!({"id": id, "reason": "not_found_or_admin"}));
            continue;
        }
        match input.action.as_str() {
            "enable" => {
                sqlx::query(
                    "UPDATE users SET enabled = 1, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
                )
                .bind(id)
                .execute(&mut *transaction)
                .await?;
            }
            "disable" => {
                sqlx::query(
                    "UPDATE users SET enabled = 0, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
                )
                .bind(id)
                .execute(&mut *transaction)
                .await?;
                sqlx::query("DELETE FROM auth_sessions WHERE user_id = ?")
                    .bind(id)
                    .execute(&mut *transaction)
                    .await?;
            }
            "delete" => soft_delete_user(&mut transaction, id).await?,
            _ => unreachable!(),
        }
        affected_ids.push(id);
    }
    transaction.commit().await?;
    Ok(Json(json!({"data": {
        "affected": affected_ids.len(), "affected_ids": affected_ids, "skipped": skipped
    }})))
}

async fn delete_user(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    ensure_regular_user(&state, id).await?;
    let mut transaction = state.pool.begin().await?;
    soft_delete_user(&mut transaction, id).await?;
    transaction.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn soft_delete_user(transaction: &mut Transaction<'_, Sqlite>, id: i64) -> ApiResult<()> {
    let suffix = random_token(6)?;
    sqlx::query("DELETE FROM auth_sessions WHERE user_id = ?")
        .bind(id)
        .execute(&mut **transaction)
        .await?;
    sqlx::query("DELETE FROM api_keys WHERE user_id = ?")
        .bind(id)
        .execute(&mut **transaction)
        .await?;
    sqlx::query(
        "UPDATE subscriptions SET status = 'cancelled', auto_renew = 0, \
         renewal_status = 'disabled', next_renewal_at = NULL, last_renewal_error = '', \
         updated_at = CURRENT_TIMESTAMP \
         WHERE user_id = ? AND status = 'active'",
    )
    .bind(id)
    .execute(&mut **transaction)
    .await?;
    sqlx::query(
        "UPDATE users SET username = ?, display_name = 'Deleted user', email = NULL, \
         email_verified = 0, enabled = 0, deleted_at = CURRENT_TIMESTAMP, \
         updated_at = CURRENT_TIMESTAMP WHERE id = ? AND role = 'user'",
    )
    .bind(format!("deleted_{id}_{suffix}"))
    .bind(id)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn ensure_regular_user(state: &AppState, id: i64) -> ApiResult<()> {
    let role: Option<String> =
        sqlx::query_scalar("SELECT role FROM users WHERE id = ? AND deleted_at IS NULL")
            .bind(id)
            .fetch_optional(&state.pool)
            .await?;
    match role.as_deref() {
        Some("user") => Ok(()),
        Some("admin") => Err(ApiError::forbidden(
            "the administrator account cannot be modified here",
        )),
        _ => Err(ApiError::not_found("user not found")),
    }
}

fn validate_username(username: &str) -> ApiResult<()> {
    let username = username.trim();
    if !(3..=64).contains(&username.len())
        || !username
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._-".contains(character))
    {
        return Err(ApiError::bad_request(
            "INVALID_USERNAME",
            "username must be 3-64 ASCII letters, numbers, dots, underscores, or hyphens",
        ));
    }
    Ok(())
}

fn validate_display_name(value: &str) -> ApiResult<()> {
    if value.trim().is_empty() || value.chars().count() > 80 {
        return Err(ApiError::bad_request(
            "INVALID_DISPLAY_NAME",
            "display_name must be 1-80 characters",
        ));
    }
    Ok(())
}

fn validate_password(password: &str) -> ApiResult<()> {
    if !(8..=128).contains(&password.len()) {
        return Err(ApiError::bad_request(
            "INVALID_PASSWORD",
            "password must be 8-128 characters",
        ));
    }
    Ok(())
}

fn validate_notes(notes: &str) -> ApiResult<()> {
    if notes.chars().count() > 2_000 {
        return Err(ApiError::bad_request(
            "INVALID_USER_NOTES",
            "notes must not exceed 2000 characters",
        ));
    }
    Ok(())
}

fn validate_balance(balance: i64) -> ApiResult<()> {
    if !(0..=MAX_BALANCE_CENTS).contains(&balance) {
        return Err(ApiError::bad_request(
            "INVALID_BALANCE",
            "balance must stay within the supported range",
        ));
    }
    Ok(())
}

fn map_user_unique_error(error: sqlx::Error) -> ApiError {
    match error {
        sqlx::Error::Database(ref database) if database.is_unique_violation() => {
            ApiError::bad_request("USER_EXISTS", "username or email already exists")
        }
        other => other.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;

    #[tokio::test]
    async fn details_balance_and_bulk_actions_are_user_scoped() {
        let (_directory, state) = test_support::state().await;
        let admin_id: i64 = sqlx::query_scalar("SELECT id FROM users WHERE role = 'admin'")
            .fetch_one(&state.pool)
            .await
            .unwrap();
        let session = AuthSession {
            id: 0,
            user_id: admin_id,
            username: "admin".into(),
            display_name: "Admin".into(),
            role: "admin".into(),
        };
        let (_, created) = create_user(
            State(state.clone()),
            Extension(session.clone()),
            Json(CreateUserInput {
                username: "managed-user".into(),
                display_name: "Managed".into(),
                email: Some("managed@example.com".into()),
                password: "strong-password".into(),
                notes: "priority customer".into(),
                balance_cents: 500,
            }),
        )
        .await
        .unwrap();
        let id = created.0["data"]["id"].as_i64().unwrap();
        let adjustment = adjust_balance(
            State(state.clone()),
            Extension(session),
            Path(id),
            Json(BalanceInput {
                delta_cents: -125,
                reason: "manual correction".into(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(adjustment.0["data"]["balance_cents"], 375);

        let detail = user_detail(State(state.clone()), Path(id)).await.unwrap();
        assert_eq!(detail.0["data"]["user"]["notes"], "priority customer");
        assert_eq!(
            detail.0["data"]["balance_adjustments"]
                .as_array()
                .unwrap()
                .len(),
            2
        );

        let batch = batch_users(
            State(state.clone()),
            Json(BatchUsersInput {
                ids: vec![admin_id, id],
                action: "disable".into(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(batch.0["data"]["affected"], 1);
        assert_eq!(batch.0["data"]["skipped"].as_array().unwrap().len(), 1);
        let enabled: bool = sqlx::query_scalar("SELECT enabled FROM users WHERE id = ?")
            .bind(id)
            .fetch_one(&state.pool)
            .await
            .unwrap();
        assert!(!enabled);

        delete_user(State(state.clone()), Path(id)).await.unwrap();
        let deleted: (bool, Option<String>) =
            sqlx::query_as("SELECT enabled, deleted_at FROM users WHERE id = ?")
                .bind(id)
                .fetch_one(&state.pool)
                .await
                .unwrap();
        assert!(!deleted.0);
        assert!(deleted.1.is_some());
    }
}
