use axum::{
    Json, Router,
    extract::{Extension, Path, State},
    http::StatusCode,
    routing::{get, post, put},
};
use chrono::{Duration, Utc};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    auth::AuthSession,
    crypto::{random_token, token_hash},
    error::{ApiError, ApiResult},
    state::AppState,
};

pub fn admin_router() -> Router<AppState> {
    Router::new()
        .route("/redeem-codes", get(admin_list).post(create_code))
        .route("/redeem-codes/{id}", put(update_code).delete(delete_code))
}

pub fn user_router() -> Router<AppState> {
    Router::new()
        .route("/redeem", post(redeem))
        .route("/redemptions", get(user_history))
}

#[derive(Deserialize)]
struct CreateInput {
    name: String,
    plan_id: i64,
    token_limit: Option<i64>,
    duration_days: Option<i64>,
    #[serde(default = "one_use")]
    max_uses: i64,
    expires_in_days: Option<i64>,
}

fn one_use() -> i64 {
    1
}

async fn create_code(
    State(state): State<AppState>,
    Json(input): Json<CreateInput>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    if input.name.trim().is_empty()
        || input.name.chars().count() > 80
        || !(1..=100_000).contains(&input.max_uses)
        || input.token_limit.is_some_and(|value| value < 0)
        || input
            .duration_days
            .is_some_and(|value| !(1..=3650).contains(&value))
        || input
            .expires_in_days
            .is_some_and(|value| !(1..=3650).contains(&value))
    {
        return Err(ApiError::bad_request(
            "INVALID_REDEEM_CODE",
            "redeem code fields are invalid",
        ));
    }
    let plan_exists: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM plans WHERE id = ? AND enabled = 1")
            .bind(input.plan_id)
            .fetch_one(&state.pool)
            .await?;
    if plan_exists == 0 {
        return Err(ApiError::bad_request(
            "PLAN_NOT_FOUND",
            "enabled plan was not found",
        ));
    }
    let code = format!("mini-redeem_{}", random_token(18)?);
    let prefix: String = code.chars().take(20).collect();
    let expires_at = input
        .expires_in_days
        .map(|days| (Utc::now() + Duration::days(days)).to_rfc3339());
    let result = sqlx::query(
        "INSERT INTO redeem_codes \
         (name, code_prefix, code_hash, plan_id, token_limit, duration_days, max_uses, expires_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(input.name.trim())
    .bind(&prefix)
    .bind(token_hash(&code))
    .bind(input.plan_id)
    .bind(input.token_limit)
    .bind(input.duration_days)
    .bind(input.max_uses)
    .bind(&expires_at)
    .execute(&state.pool)
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"data": {
            "id": result.last_insert_rowid(), "code": code, "code_prefix": prefix
        }})),
    ))
}

async fn admin_list(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let rows: Vec<(i64, String, String, i64, String, Option<i64>, Option<i64>, i64, i64, Option<String>, bool, String)> = sqlx::query_as(
        "SELECT redeem_codes.id, redeem_codes.name, redeem_codes.code_prefix, redeem_codes.plan_id, \
         plans.name, redeem_codes.token_limit, redeem_codes.duration_days, redeem_codes.max_uses, \
         redeem_codes.used_count, redeem_codes.expires_at, redeem_codes.enabled, redeem_codes.created_at \
         FROM redeem_codes JOIN plans ON plans.id = redeem_codes.plan_id ORDER BY redeem_codes.id DESC",
    ).fetch_all(&state.pool).await?;
    Ok(Json(json!({"data": rows.into_iter().map(|row| json!({
        "id": row.0, "name": row.1, "code_prefix": row.2, "plan_id": row.3,
        "plan_name": row.4, "token_limit": row.5, "duration_days": row.6,
        "max_uses": row.7, "used_count": row.8, "expires_at": row.9,
        "enabled": row.10, "created_at": row.11
    })).collect::<Vec<_>>() })))
}

#[derive(Deserialize)]
struct UpdateInput {
    enabled: bool,
}

async fn update_code(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(input): Json<UpdateInput>,
) -> ApiResult<Json<Value>> {
    let result = sqlx::query("UPDATE redeem_codes SET enabled = ? WHERE id = ?")
        .bind(input.enabled)
        .bind(id)
        .execute(&state.pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found("redeem code not found"));
    }
    Ok(Json(json!({"data": {"id": id, "enabled": input.enabled}})))
}

async fn delete_code(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    let result = sqlx::query("DELETE FROM redeem_codes WHERE id = ? AND used_count = 0")
        .bind(id)
        .execute(&state.pool)
        .await
        .map_err(|_| ApiError::bad_request("CODE_IN_USE", "used redeem code cannot be deleted"))?;
    if result.rows_affected() == 0 {
        return Err(ApiError::bad_request(
            "CODE_IN_USE",
            "redeem code was not found or has been used",
        ));
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct RedeemInput {
    code: String,
}

async fn redeem(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
    Json(input): Json<RedeemInput>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let code: Option<(i64, i64, Option<i64>, Option<i64>, i64, i64)> = sqlx::query_as(
        "SELECT id, plan_id, token_limit, duration_days, max_uses, used_count FROM redeem_codes \
         WHERE code_hash = ? AND enabled = 1 \
         AND (expires_at IS NULL OR datetime(expires_at) > CURRENT_TIMESTAMP)",
    )
    .bind(token_hash(input.code.trim()))
    .fetch_optional(&state.pool)
    .await?;
    let (code_id, plan_id, override_tokens, override_days, max_uses, used_count) = code
        .ok_or_else(|| {
            ApiError::bad_request("INVALID_REDEEM_CODE", "redeem code is invalid or expired")
        })?;
    if used_count >= max_uses {
        return Err(ApiError::bad_request(
            "REDEEM_CODE_EXHAUSTED",
            "redeem code has no remaining uses",
        ));
    }
    let plan: Option<(i64, i64, String, Option<i64>)> = sqlx::query_as(
        "SELECT plans.token_limit, plans.duration_days, plans.name, plans.group_id \
         FROM plans LEFT JOIN groups ON groups.id = plans.group_id \
         WHERE plans.id = ? AND plans.enabled = 1 \
         AND (plans.group_id IS NULL OR (groups.enabled = 1 AND groups.subscription_type = 'subscription'))",
    )
    .bind(plan_id)
    .fetch_optional(&state.pool)
    .await?;
    let (plan_tokens, plan_days, plan_name, group_id) = plan.ok_or_else(|| {
        ApiError::bad_request("PLAN_NOT_FOUND", "redeem code plan is unavailable")
    })?;
    let token_limit = override_tokens.unwrap_or(plan_tokens);
    let duration_days = override_days.unwrap_or(plan_days);
    let starts_at = Utc::now();
    let ends_at = starts_at + Duration::days(duration_days);
    let mut transaction = state.pool.begin().await?;
    let duplicate: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM redemptions WHERE redeem_code_id = ? AND user_id = ?",
    )
    .bind(code_id)
    .bind(session.user_id)
    .fetch_one(&mut *transaction)
    .await?;
    if duplicate > 0 {
        return Err(ApiError::bad_request(
            "ALREADY_REDEEMED",
            "this code was already redeemed by the user",
        ));
    }
    let claimed = sqlx::query(
        "UPDATE redeem_codes SET used_count = used_count + 1 \
         WHERE id = ? AND enabled = 1 AND used_count < max_uses",
    )
    .bind(code_id)
    .execute(&mut *transaction)
    .await?;
    if claimed.rows_affected() == 0 {
        return Err(ApiError::bad_request(
            "REDEEM_CODE_EXHAUSTED",
            "redeem code has no remaining uses",
        ));
    }
    sqlx::query(
        "UPDATE subscriptions SET status = 'cancelled', auto_renew = 0, \
                 renewal_status = 'disabled', next_renewal_at = NULL, \
                 last_renewal_error = '', updated_at = CURRENT_TIMESTAMP \
                 WHERE user_id = ? AND status = 'active' AND group_id IS ?",
    )
    .bind(session.user_id)
    .bind(group_id)
    .execute(&mut *transaction)
    .await?;
    let subscription_id = sqlx::query(
        "INSERT INTO subscriptions (user_id, plan_id, token_limit, starts_at, ends_at, group_id) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(session.user_id)
    .bind(plan_id)
    .bind(token_limit)
    .bind(starts_at.to_rfc3339())
    .bind(ends_at.to_rfc3339())
    .bind(group_id)
    .execute(&mut *transaction)
    .await?
    .last_insert_rowid();
    sqlx::query(
        "INSERT INTO redemptions (redeem_code_id, user_id, subscription_id) VALUES (?, ?, ?)",
    )
    .bind(code_id)
    .bind(session.user_id)
    .bind(subscription_id)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"data": {
            "subscription_id": subscription_id, "plan_name": plan_name,
            "token_limit": token_limit, "ends_at": ends_at.to_rfc3339()
        }})),
    ))
}

async fn user_history(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
) -> ApiResult<Json<Value>> {
    let rows: Vec<(i64, String, String, String, i64)> = sqlx::query_as(
        "SELECT redemptions.id, redeem_codes.name, redeem_codes.code_prefix, \
         redemptions.redeemed_at, redemptions.subscription_id FROM redemptions \
         JOIN redeem_codes ON redeem_codes.id = redemptions.redeem_code_id \
         WHERE redemptions.user_id = ? ORDER BY redemptions.id DESC",
    )
    .bind(session.user_id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(json!({"data": rows.into_iter().map(|row| json!({
        "id": row.0, "name": row.1, "code_prefix": row.2,
        "redeemed_at": row.3, "subscription_id": row.4
    })).collect::<Vec<_>>() })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;

    #[tokio::test]
    async fn redeems_once_and_creates_a_subscription_atomically() {
        let (_directory, state) = test_support::state().await;
        let plan_id = sqlx::query(
            "INSERT INTO plans (name, token_limit, duration_days) VALUES ('gift', 1000, 30)",
        )
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        let (_, Json(created)) = create_code(
            State(state.clone()),
            Json(CreateInput {
                name: "gift code".into(),
                plan_id,
                token_limit: Some(2000),
                duration_days: Some(60),
                max_uses: 2,
                expires_in_days: Some(10),
            }),
        )
        .await
        .unwrap();
        let code = created["data"]["code"].as_str().unwrap().to_string();
        let user_id: i64 = sqlx::query_scalar("SELECT id FROM users WHERE role = 'admin'")
            .fetch_one(&state.pool)
            .await
            .unwrap();
        let session = AuthSession {
            id: 1,
            user_id,
            username: "admin".into(),
            display_name: "admin".into(),
            role: "admin".into(),
        };
        let result = redeem(
            State(state.clone()),
            Extension(session.clone()),
            Json(RedeemInput { code: code.clone() }),
        )
        .await
        .unwrap();
        assert_eq!(result.0, StatusCode::CREATED);
        assert_eq!(result.1.0["data"]["token_limit"], 2000);
        let active: (i64, i64) = sqlx::query_as(
            "SELECT COUNT(*), COALESCE(MAX(token_limit), 0) FROM subscriptions WHERE user_id = ? AND status = 'active'",
        )
        .bind(user_id)
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(active, (1, 2000));

        let duplicate = redeem(
            State(state.clone()),
            Extension(session),
            Json(RedeemInput { code }),
        )
        .await;
        assert!(duplicate.is_err());
        let used_count: i64 = sqlx::query_scalar("SELECT used_count FROM redeem_codes")
            .fetch_one(&state.pool)
            .await
            .unwrap();
        assert_eq!(used_count, 1);
    }
}
