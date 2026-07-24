use axum::{
    Json, Router,
    extract::{Extension, Path, State},
    http::StatusCode,
    routing::{get, post, put},
};
use chrono::{DateTime, Duration, NaiveDateTime, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::FromRow;

use crate::{
    auth::AuthSession,
    error::{ApiError, ApiResult},
    state::AppState,
};

pub fn start_scheduler(state: AppState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if let Err(error) = process_due_renewals(&state).await {
                tracing::warn!(%error, "subscription renewal scheduler failed");
            }
        }
    });
}

pub fn admin_router() -> Router<AppState> {
    Router::new()
        .route("/plans", get(admin_plans).post(create_plan))
        .route("/plans/{id}", put(update_plan).delete(delete_plan))
        .route(
            "/subscriptions",
            get(admin_subscriptions).post(assign_subscription),
        )
        .route("/subscriptions/{id}", put(update_subscription))
}

pub fn user_router() -> Router<AppState> {
    Router::new()
        .route("/plans", get(user_plans))
        .route("/subscriptions", get(user_subscriptions))
        .route("/subscriptions/{id}/auto-renew", put(update_auto_renew))
        .route("/subscriptions/{id}/renew", post(retry_renewal))
}

#[derive(Deserialize)]
struct PlanInput {
    name: String,
    #[serde(default)]
    description: String,
    token_limit: i64,
    duration_days: i64,
    #[serde(default)]
    price_cents: i64,
    #[serde(default)]
    original_price_cents: i64,
    #[serde(default = "default_currency")]
    currency: String,
    #[serde(default)]
    features: Vec<String>,
    #[serde(default)]
    product_name: String,
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default)]
    sort_order: i64,
    group_id: Option<i64>,
}

fn default_true() -> bool {
    true
}

fn default_currency() -> String {
    "CNY".into()
}

#[derive(FromRow)]
struct PlanRow {
    id: i64,
    name: String,
    description: String,
    token_limit: i64,
    duration_days: i64,
    price_cents: i64,
    original_price_cents: i64,
    currency: String,
    features: String,
    product_name: String,
    enabled: bool,
    sort_order: i64,
    created_at: String,
    updated_at: String,
    group_id: Option<i64>,
    group_name: Option<String>,
}

async fn admin_plans(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    list_plans(&state, false).await
}

async fn user_plans(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    list_plans(&state, true).await
}

async fn list_plans(state: &AppState, enabled_only: bool) -> ApiResult<Json<Value>> {
    let rows: Vec<PlanRow> = sqlx::query_as(
        "SELECT plans.id, plans.name, plans.description, plans.token_limit, \
             plans.duration_days, plans.price_cents, plans.original_price_cents, plans.currency, \
             plans.features, plans.product_name, plans.enabled, plans.sort_order, \
             plans.created_at, plans.updated_at, plans.group_id, groups.name AS group_name \
             FROM plans LEFT JOIN groups ON groups.id = plans.group_id \
             WHERE (? = 0 OR (plans.enabled = 1 AND \
               (plans.group_id IS NULL OR groups.enabled = 1))) \
             ORDER BY plans.sort_order ASC, plans.id ASC",
    )
    .bind(enabled_only)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(
        json!({"data": rows.into_iter().map(plan_value).collect::<Vec<_>>()}),
    ))
}

async fn create_plan(
    State(state): State<AppState>,
    Json(input): Json<PlanInput>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    validate_plan(&input)?;
    let group_id = validate_plan_group(&state, input.group_id).await?;
    let result = sqlx::query(
        "INSERT INTO plans (name, description, token_limit, duration_days, price_cents, \
         original_price_cents, currency, features, product_name, enabled, sort_order, group_id) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(input.name.trim())
    .bind(input.description.trim())
    .bind(input.token_limit)
    .bind(input.duration_days)
    .bind(input.price_cents)
    .bind(input.original_price_cents)
    .bind(input.currency.trim().to_ascii_uppercase())
    .bind(serde_json::to_string(&normalized_features(&input.features)).unwrap())
    .bind(input.product_name.trim())
    .bind(input.enabled)
    .bind(input.sort_order)
    .bind(group_id)
    .execute(&state.pool)
    .await
    .map_err(unique_plan_error)?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"data": {"id": result.last_insert_rowid()}})),
    ))
}

async fn update_plan(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(input): Json<PlanInput>,
) -> ApiResult<Json<Value>> {
    validate_plan(&input)?;
    let group_id = validate_plan_group(&state, input.group_id).await?;
    let result = sqlx::query(
        "UPDATE plans SET name = ?, description = ?, token_limit = ?, duration_days = ?, \
         price_cents = ?, original_price_cents = ?, currency = ?, features = ?, \
         product_name = ?, enabled = ?, sort_order = ?, group_id = ?, \
         updated_at = CURRENT_TIMESTAMP WHERE id = ?",
    )
    .bind(input.name.trim())
    .bind(input.description.trim())
    .bind(input.token_limit)
    .bind(input.duration_days)
    .bind(input.price_cents)
    .bind(input.original_price_cents)
    .bind(input.currency.trim().to_ascii_uppercase())
    .bind(serde_json::to_string(&normalized_features(&input.features)).unwrap())
    .bind(input.product_name.trim())
    .bind(input.enabled)
    .bind(input.sort_order)
    .bind(group_id)
    .bind(id)
    .execute(&state.pool)
    .await
    .map_err(unique_plan_error)?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found("plan not found"));
    }
    Ok(Json(json!({"data": {"id": id}})))
}

async fn delete_plan(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    let result = sqlx::query("DELETE FROM plans WHERE id = ?")
        .bind(id)
        .execute(&state.pool)
        .await
        .map_err(|error| match error {
            sqlx::Error::Database(_) => ApiError::bad_request(
                "PLAN_IN_USE",
                "plan with subscription history cannot be deleted",
            ),
            other => other.into(),
        })?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found("plan not found"));
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct AssignInput {
    user_id: i64,
    plan_id: i64,
    token_limit: Option<i64>,
    duration_days: Option<i64>,
}

async fn assign_subscription(
    State(state): State<AppState>,
    Json(input): Json<AssignInput>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let plan: Option<(i64, i64, Option<i64>)> = sqlx::query_as(
        "SELECT token_limit, duration_days, group_id FROM plans WHERE id = ? AND enabled = 1",
    )
    .bind(input.plan_id)
    .fetch_optional(&state.pool)
    .await?;
    let (plan_tokens, plan_days, group_id) =
        plan.ok_or_else(|| ApiError::bad_request("PLAN_NOT_FOUND", "enabled plan was not found"))?;
    if let Some(group_id) = group_id {
        validate_plan_group(&state, Some(group_id)).await?;
    }
    let user_exists: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE id = ? AND enabled = 1")
            .bind(input.user_id)
            .fetch_one(&state.pool)
            .await?;
    if user_exists == 0 {
        return Err(ApiError::bad_request(
            "USER_NOT_FOUND",
            "enabled user was not found",
        ));
    }
    let token_limit = input.token_limit.unwrap_or(plan_tokens);
    let duration_days = input.duration_days.unwrap_or(plan_days);
    if token_limit < 0 || !(1..=3650).contains(&duration_days) {
        return Err(ApiError::bad_request(
            "INVALID_SUBSCRIPTION",
            "subscription limits are invalid",
        ));
    }
    let starts_at = Utc::now();
    let ends_at = starts_at + Duration::days(duration_days);
    let mut transaction = state.pool.begin().await?;
    sqlx::query(
        "UPDATE subscriptions SET status = 'cancelled', auto_renew = 0, \
                 renewal_status = 'disabled', next_renewal_at = NULL, \
                 last_renewal_error = '', updated_at = CURRENT_TIMESTAMP \
                 WHERE user_id = ? AND status = 'active' AND group_id IS ?",
    )
    .bind(input.user_id)
    .bind(group_id)
    .execute(&mut *transaction)
    .await?;
    let result = sqlx::query(
        "INSERT INTO subscriptions (user_id, plan_id, token_limit, starts_at, ends_at, group_id) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(input.user_id)
    .bind(input.plan_id)
    .bind(token_limit)
    .bind(starts_at.to_rfc3339())
    .bind(ends_at.to_rfc3339())
    .bind(group_id)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"data": {"id": result.last_insert_rowid()}})),
    ))
}

#[derive(Deserialize)]
struct SubscriptionUpdate {
    status: String,
}

async fn update_subscription(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(input): Json<SubscriptionUpdate>,
) -> ApiResult<Json<Value>> {
    if !matches!(input.status.as_str(), "cancelled" | "expired") {
        return Err(ApiError::bad_request(
            "INVALID_STATUS",
            "subscription can only be cancelled or expired",
        ));
    }
    let result = sqlx::query(
        "UPDATE subscriptions SET status = ?, auto_renew = 0, renewal_status = 'disabled', \
         next_renewal_at = NULL, last_renewal_error = '', updated_at = CURRENT_TIMESTAMP WHERE id = ?",
    )
    .bind(&input.status)
    .bind(id)
    .execute(&state.pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found("subscription not found"));
    }
    Ok(Json(json!({"data": {"id": id, "status": input.status}})))
}

async fn admin_subscriptions(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    subscriptions_for_user(&state, None).await
}

async fn user_subscriptions(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
) -> ApiResult<Json<Value>> {
    subscriptions_for_user(&state, Some(session.user_id)).await
}

#[derive(Deserialize)]
struct AutoRenewInput {
    enabled: bool,
}

#[derive(FromRow)]
struct SubscriptionRow {
    id: i64,
    user_id: i64,
    username: String,
    plan_id: i64,
    plan_name: String,
    status: String,
    starts_at: String,
    ends_at: String,
    created_at: String,
    token_limit: i64,
    group_id: Option<i64>,
    group_name: Option<String>,
    auto_renew: bool,
    renewal_status: String,
    next_renewal_at: Option<String>,
    last_renewal_at: Option<String>,
    last_renewal_error: String,
    renewal_price_cents: i64,
}

async fn update_auto_renew(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
    Path(id): Path<i64>,
    Json(input): Json<AutoRenewInput>,
) -> ApiResult<Json<Value>> {
    let subscription: Option<(String, i64, bool)> = sqlx::query_as(
        "SELECT subscriptions.status, plans.price_cents, plans.enabled FROM subscriptions \
         JOIN plans ON plans.id = subscriptions.plan_id WHERE subscriptions.id = ? AND subscriptions.user_id = ?",
    )
    .bind(id)
    .bind(session.user_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some((status, price_cents, plan_enabled)) = subscription else {
        return Err(ApiError::not_found("subscription not found"));
    };
    if input.enabled
        && (!matches!(status.as_str(), "active" | "expired") || price_cents <= 0 || !plan_enabled)
    {
        return Err(ApiError::bad_request(
            "AUTO_RENEW_UNAVAILABLE",
            "subscription cannot be renewed automatically",
        ));
    }
    let result = sqlx::query(
        "UPDATE subscriptions SET auto_renew=?, renewal_status=?, \
         next_renewal_at=CASE WHEN ? THEN ends_at ELSE NULL END, \
         last_renewal_error='', updated_at=CURRENT_TIMESTAMP WHERE id=? AND user_id=?",
    )
    .bind(input.enabled)
    .bind(if input.enabled {
        "scheduled"
    } else {
        "disabled"
    })
    .bind(input.enabled)
    .bind(id)
    .bind(session.user_id)
    .execute(&state.pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found("subscription not found"));
    }
    Ok(Json(
        json!({"data": {"id": id, "auto_renew": input.enabled}}),
    ))
}

async fn retry_renewal(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
    Path(id): Path<i64>,
) -> ApiResult<Json<Value>> {
    let owner: Option<i64> = sqlx::query_scalar("SELECT user_id FROM subscriptions WHERE id=?")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?;
    if owner != Some(session.user_id) {
        return Err(ApiError::not_found("subscription not found"));
    }
    let renewed = renew_subscription(&state, id, true).await?;
    Ok(Json(json!({"data": {"id": id, "renewed": renewed}})))
}

async fn subscriptions_for_user(state: &AppState, user_id: Option<i64>) -> ApiResult<Json<Value>> {
    sqlx::query("UPDATE subscriptions SET status = 'expired', updated_at = CURRENT_TIMESTAMP WHERE status = 'active' AND datetime(ends_at) <= CURRENT_TIMESTAMP")
        .execute(&state.pool).await?;
    let rows: Vec<SubscriptionRow> = sqlx::query_as(
        "SELECT subscriptions.id, subscriptions.user_id, users.username, subscriptions.plan_id, \
         plans.name, subscriptions.status, subscriptions.starts_at, subscriptions.ends_at, \
         subscriptions.created_at, subscriptions.token_limit, subscriptions.group_id, groups.name, \
         subscriptions.auto_renew, subscriptions.renewal_status, subscriptions.next_renewal_at, \
         subscriptions.last_renewal_at, subscriptions.last_renewal_error, \
         plans.price_cents AS renewal_price_cents \
         FROM subscriptions \
         JOIN users ON users.id = subscriptions.user_id JOIN plans ON plans.id = subscriptions.plan_id \
         LEFT JOIN groups ON groups.id = subscriptions.group_id \
         WHERE (? IS NULL OR subscriptions.user_id = ?) ORDER BY subscriptions.id DESC",
    ).bind(user_id).bind(user_id).fetch_all(&state.pool).await?;
    let mut result = Vec::with_capacity(rows.len());
    for row in rows {
        let used_tokens: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(log.total_tokens), 0) FROM usage_logs log \
             LEFT JOIN api_keys keys ON keys.id = log.api_key_id WHERE log.user_id = ? \
             AND datetime(log.created_at) >= datetime(?) AND datetime(log.created_at) < datetime(?) \
             AND (? IS NULL OR keys.group_id = ?)",
        )
        .bind(row.user_id)
        .bind(&row.starts_at)
        .bind(&row.ends_at)
        .bind(row.group_id)
        .bind(row.group_id)
        .fetch_one(&state.pool)
        .await?;
        result.push(json!({
            "id": row.id, "user_id": row.user_id, "username": row.username,
            "plan_id": row.plan_id, "plan_name": row.plan_name, "status": row.status,
            "starts_at": row.starts_at, "ends_at": row.ends_at,
            "created_at": row.created_at, "token_limit": row.token_limit,
            "used_tokens": used_tokens, "group_id": row.group_id,
            "group_name": row.group_name, "auto_renew": row.auto_renew,
            "renewal_status": row.renewal_status, "next_renewal_at": row.next_renewal_at,
            "last_renewal_at": row.last_renewal_at,
            "last_renewal_error": row.last_renewal_error,
            "renewal_price_cents": row.renewal_price_cents
        }));
    }
    Ok(Json(json!({"data": result})))
}

async fn process_due_renewals(state: &AppState) -> ApiResult<i64> {
    let ids: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM subscriptions WHERE auto_renew=1 AND status IN ('active','expired') \
         AND next_renewal_at IS NOT NULL AND datetime(next_renewal_at)<=CURRENT_TIMESTAMP \
         ORDER BY datetime(next_renewal_at),id LIMIT 100",
    )
    .fetch_all(&state.pool)
    .await?;
    let mut renewed = 0;
    for id in ids {
        match renew_subscription(state, id, false).await {
            Ok(true) => renewed += 1,
            Ok(false) => {}
            Err(error) => tracing::warn!(subscription_id=id, %error, "subscription renewal failed"),
        }
    }
    Ok(renewed)
}

async fn renew_subscription(state: &AppState, id: i64, _manual: bool) -> ApiResult<bool> {
    let row: Option<(i64, i64, String, bool, String, i64, i64, i64, bool, bool)> =
        sqlx::query_as(
            "SELECT subscriptions.user_id,subscriptions.plan_id,subscriptions.ends_at, \
             subscriptions.auto_renew,subscriptions.status,plans.token_limit,plans.duration_days, \
             plans.price_cents,plans.enabled,users.enabled FROM subscriptions \
             JOIN plans ON plans.id=subscriptions.plan_id JOIN users ON users.id=subscriptions.user_id \
             WHERE subscriptions.id=?",
        )
        .bind(id)
        .fetch_optional(&state.pool)
        .await?;
    let Some((
        user_id,
        plan_id,
        period_end,
        auto_renew,
        _,
        token_limit,
        duration_days,
        price_cents,
        plan_enabled,
        user_enabled,
    )) = row
    else {
        return Err(ApiError::not_found("subscription not found"));
    };
    if !auto_renew {
        return Ok(false);
    }
    let parsed_end = parse_database_time(&period_end)
        .ok_or_else(|| ApiError::internal("subscription end time is malformed"))?;
    if parsed_end > Utc::now() {
        return Ok(false);
    }
    let renewal_key = format!("subscription:{id}:{period_end}");
    let mut transaction = state.pool.begin().await?;
    sqlx::query(
        "INSERT INTO subscription_renewal_attempts (subscription_id,period_end,status,amount_cents) \
         VALUES (?,?,'processing',?) ON CONFLICT(subscription_id,period_end) DO UPDATE SET \
         status=CASE WHEN status='succeeded' THEN status ELSE 'processing' END, \
         attempt_count=attempt_count+CASE WHEN status='succeeded' THEN 0 ELSE 1 END, \
         attempted_at=CURRENT_TIMESTAMP,updated_at=CURRENT_TIMESTAMP",
    )
    .bind(id)
    .bind(&period_end)
    .bind(price_cents.max(0))
    .execute(&mut *transaction)
    .await?;
    let attempt_status: String = sqlx::query_scalar(
        "SELECT status FROM subscription_renewal_attempts WHERE subscription_id=? AND period_end=?",
    )
    .bind(id)
    .bind(&period_end)
    .fetch_one(&mut *transaction)
    .await?;
    if attempt_status == "succeeded" {
        transaction.rollback().await?;
        return Ok(false);
    }
    if !plan_enabled || !user_enabled || price_cents <= 0 {
        sqlx::query(
            "UPDATE subscription_renewal_attempts SET status='failed',error_code='PLAN_UNAVAILABLE', \
             updated_at=CURRENT_TIMESTAMP WHERE subscription_id=? AND period_end=?",
        )
        .bind(id).bind(&period_end).execute(&mut *transaction).await?;
        sqlx::query(
            "UPDATE subscriptions SET status='expired',renewal_status='plan_unavailable', \
             last_renewal_error='PLAN_UNAVAILABLE',next_renewal_at=datetime('now','+1 day'), \
             updated_at=CURRENT_TIMESTAMP WHERE id=?",
        )
        .bind(id)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        return Ok(false);
    }
    let debited = sqlx::query(
        "UPDATE users SET balance_cents=balance_cents-?,updated_at=CURRENT_TIMESTAMP \
         WHERE id=? AND enabled=1 AND balance_cents>=?",
    )
    .bind(price_cents)
    .bind(user_id)
    .bind(price_cents)
    .execute(&mut *transaction)
    .await?;
    if debited.rows_affected() != 1 {
        sqlx::query(
            "UPDATE subscription_renewal_attempts SET status='failed',error_code='INSUFFICIENT_BALANCE', \
             updated_at=CURRENT_TIMESTAMP WHERE subscription_id=? AND period_end=?",
        )
        .bind(id).bind(&period_end).execute(&mut *transaction).await?;
        sqlx::query(
            "UPDATE subscriptions SET status='expired',renewal_status='insufficient_balance', \
             last_renewal_error='INSUFFICIENT_BALANCE',next_renewal_at=datetime('now','+6 hours'), \
             updated_at=CURRENT_TIMESTAMP WHERE id=?",
        )
        .bind(id)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        return Ok(false);
    }
    let starts_at = Utc::now();
    let ends_at = starts_at + Duration::days(duration_days);
    let order_id = sqlx::query(
        "INSERT INTO orders (user_id,plan_id,subscription_id,amount_cents,provider,status,paid_at, \
         order_type,renewal_key) VALUES (?,?,?,?,'balance','paid',CURRENT_TIMESTAMP,'renewal',?)",
    )
    .bind(user_id)
    .bind(plan_id)
    .bind(id)
    .bind(price_cents)
    .bind(&renewal_key)
    .execute(&mut *transaction)
    .await?
    .last_insert_rowid();
    let updated = sqlx::query(
        "UPDATE subscriptions SET status='active',token_limit=?,starts_at=?,ends_at=?, \
         renewal_status='succeeded',next_renewal_at=?,last_renewal_at=CURRENT_TIMESTAMP, \
         last_renewal_error='',updated_at=CURRENT_TIMESTAMP WHERE id=? AND ends_at=?",
    )
    .bind(token_limit)
    .bind(starts_at.to_rfc3339())
    .bind(ends_at.to_rfc3339())
    .bind(ends_at.to_rfc3339())
    .bind(id)
    .bind(&period_end)
    .execute(&mut *transaction)
    .await?;
    if updated.rows_affected() != 1 {
        transaction.rollback().await?;
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "SUBSCRIPTION_RENEWAL_CONFLICT",
            "subscription period changed during renewal",
        ));
    }
    sqlx::query(
        "UPDATE subscription_renewal_attempts SET status='succeeded',order_id=?,error_code='', \
         updated_at=CURRENT_TIMESTAMP WHERE subscription_id=? AND period_end=?",
    )
    .bind(order_id)
    .bind(id)
    .bind(&period_end)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(true)
}

pub(crate) fn parse_database_time(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .ok()
        .or_else(|| {
            NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S")
                .map(|value| value.and_utc())
                .ok()
        })
}

fn validate_plan(input: &PlanInput) -> ApiResult<()> {
    let currency = input.currency.trim().to_ascii_uppercase();
    let features = normalized_features(&input.features);
    if input.name.trim().is_empty()
        || input.name.chars().count() > 80
        || input.description.len() > 1000
        || input.token_limit < 0
        || !(1..=3650).contains(&input.duration_days)
        || input.price_cents < 0
        || input.original_price_cents < 0
        || currency.len() != 3
        || !currency.bytes().all(|byte| byte.is_ascii_uppercase())
        || input.product_name.chars().count() > 100
        || features.len() > 20
        || features.iter().any(|value| value.chars().count() > 120)
        || !(-10_000..=10_000).contains(&input.sort_order)
    {
        return Err(ApiError::bad_request(
            "INVALID_PLAN",
            "plan fields are invalid",
        ));
    }
    Ok(())
}

fn normalized_features(values: &[String]) -> Vec<String> {
    let mut result = Vec::new();
    for value in values {
        let value = value.trim();
        if !value.is_empty() && !result.iter().any(|current| current == value) {
            result.push(value.to_string());
        }
    }
    result
}

async fn validate_plan_group(state: &AppState, group_id: Option<i64>) -> ApiResult<Option<i64>> {
    let Some(group_id) = group_id.filter(|id| *id > 0) else {
        return Ok(None);
    };
    let valid: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM groups WHERE id = ? AND enabled = 1 \
         AND subscription_type = 'subscription'",
    )
    .bind(group_id)
    .fetch_one(&state.pool)
    .await?;
    if valid == 0 {
        return Err(ApiError::bad_request(
            "INVALID_PLAN_GROUP",
            "plan group must be an enabled subscription group",
        ));
    }
    Ok(Some(group_id))
}

fn plan_value(row: PlanRow) -> Value {
    let features = serde_json::from_str::<Vec<String>>(&row.features).unwrap_or_default();
    json!({
        "id": row.id, "name": row.name, "description": row.description,
        "token_limit": row.token_limit, "duration_days": row.duration_days,
        "price_cents": row.price_cents, "original_price_cents": row.original_price_cents,
        "currency": row.currency, "features": features, "product_name": row.product_name,
        "enabled": row.enabled, "sort_order": row.sort_order,
        "created_at": row.created_at, "updated_at": row.updated_at,
        "group_id": row.group_id, "group_name": row.group_name
    })
}

fn unique_plan_error(error: sqlx::Error) -> ApiError {
    match error {
        sqlx::Error::Database(ref database) if database.is_unique_violation() => {
            ApiError::bad_request("PLAN_NAME_EXISTS", "plan name already exists")
        }
        other => other.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;

    #[tokio::test]
    async fn plan_marketing_and_currency_fields_round_trip() {
        let (_directory, state) = test_support::state().await;
        let group_id = sqlx::query(
            "INSERT INTO groups(name,subscription_type) VALUES('Marketing group','subscription')",
        )
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        let (_, Json(created)) = create_plan(
            State(state.clone()),
            Json(PlanInput {
                name: "Professional".into(),
                description: "For individual production workloads".into(),
                token_limit: 50_000,
                duration_days: 30,
                price_cents: 1_500,
                original_price_cents: 2_000,
                currency: "usd".into(),
                features: vec![
                    "Priority routing".into(),
                    "Priority routing".into(),
                    "SSE".into(),
                ],
                product_name: "Pro gateway".into(),
                enabled: true,
                sort_order: 2,
                group_id: Some(group_id),
            }),
        )
        .await
        .unwrap();
        assert!(created["data"]["id"].as_i64().unwrap() > 0);
        let Json(listed) = admin_plans(State(state.clone())).await.unwrap();
        let plan = &listed["data"][0];
        assert_eq!(plan["currency"], "USD");
        assert_eq!(plan["original_price_cents"], 2_000);
        assert_eq!(plan["product_name"], "Pro gateway");
        assert_eq!(plan["features"], json!(["Priority routing", "SSE"]));
        let stored: String =
            sqlx::query_scalar("SELECT features FROM plans WHERE name='Professional'")
                .fetch_one(&state.pool)
                .await
                .unwrap();
        assert_eq!(
            serde_json::from_str::<Vec<String>>(&stored).unwrap(),
            vec!["Priority routing", "SSE"]
        );
    }

    #[tokio::test]
    async fn assigns_one_active_subscription_per_user_and_group() {
        let (_directory, state) = test_support::state().await;
        let user_id = sqlx::query(
            "INSERT INTO users (username, display_name, password_hash) \
             VALUES ('multi-sub-user', 'Multi Sub', 'unused')",
        )
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        let mut plan_ids = Vec::new();
        for index in 0..2 {
            let group_id = sqlx::query(
                "INSERT INTO groups (name, subscription_type) VALUES (?, 'subscription')",
            )
            .bind(format!("multi-group-{index}"))
            .execute(&state.pool)
            .await
            .unwrap()
            .last_insert_rowid();
            plan_ids.push(
                sqlx::query(
                    "INSERT INTO plans (name, token_limit, duration_days, group_id) \
                     VALUES (?, 100, 30, ?)",
                )
                .bind(format!("multi-plan-{index}"))
                .bind(group_id)
                .execute(&state.pool)
                .await
                .unwrap()
                .last_insert_rowid(),
            );
        }
        for plan_id in &plan_ids {
            let _ = assign_subscription(
                State(state.clone()),
                Json(AssignInput {
                    user_id,
                    plan_id: *plan_id,
                    token_limit: None,
                    duration_days: None,
                }),
            )
            .await
            .unwrap();
        }
        let active: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM subscriptions WHERE user_id = ? AND status = 'active'",
        )
        .bind(user_id)
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(active, 2);

        let _ = assign_subscription(
            State(state.clone()),
            Json(AssignInput {
                user_id,
                plan_id: plan_ids[0],
                token_limit: Some(200),
                duration_days: None,
            }),
        )
        .await
        .unwrap();
        let counts: (i64, i64) = sqlx::query_as(
            "SELECT SUM(status = 'active'), SUM(status = 'cancelled') FROM subscriptions \
             WHERE user_id = ?",
        )
        .bind(user_id)
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(counts, (2, 1));
    }

    #[tokio::test]
    async fn balance_renewal_is_idempotent_and_retries_after_funding() {
        let (_directory, state) = test_support::state().await;
        let user_id = sqlx::query(
            "INSERT INTO users (username,display_name,password_hash,balance_cents) \
             VALUES ('renew-user','Renew User','unused',500)",
        )
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        let plan_id = sqlx::query(
            "INSERT INTO plans (name,token_limit,duration_days,price_cents) \
             VALUES ('renew-plan',9000,30,1200)",
        )
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        let subscription_id = sqlx::query(
            "INSERT INTO subscriptions (user_id,plan_id,token_limit,starts_at,ends_at,status, \
             auto_renew,renewal_status,next_renewal_at) VALUES (?,?,9000,datetime('now','-31 days'), \
             datetime('now','-1 day'),'expired',1,'scheduled',datetime('now','-1 day'))",
        )
        .bind(user_id)
        .bind(plan_id)
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        assert_eq!(process_due_renewals(&state).await.unwrap(), 0);
        let failed: (String, String, i64, i64) = sqlx::query_as(
            "SELECT subscriptions.status,subscriptions.renewal_status,users.balance_cents, \
             subscription_renewal_attempts.attempt_count FROM subscriptions \
             JOIN users ON users.id=subscriptions.user_id JOIN subscription_renewal_attempts \
             ON subscription_renewal_attempts.subscription_id=subscriptions.id WHERE subscriptions.id=?",
        )
        .bind(subscription_id)
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(
            failed,
            ("expired".into(), "insufficient_balance".into(), 500, 1)
        );
        sqlx::query("UPDATE users SET balance_cents=2000 WHERE id=?")
            .bind(user_id)
            .execute(&state.pool)
            .await
            .unwrap();
        assert!(
            renew_subscription(&state, subscription_id, true)
                .await
                .unwrap()
        );
        let renewed: (String, String, i64, i64, String, i64) = sqlx::query_as(
            "SELECT subscriptions.status,subscriptions.renewal_status,users.balance_cents, \
             COUNT(orders.id),MAX(orders.order_type),MAX(subscription_renewal_attempts.attempt_count) \
             FROM subscriptions JOIN users ON users.id=subscriptions.user_id \
             JOIN orders ON orders.subscription_id=subscriptions.id JOIN subscription_renewal_attempts \
             ON subscription_renewal_attempts.subscription_id=subscriptions.id WHERE subscriptions.id=?",
        )
        .bind(subscription_id)
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(
            renewed,
            (
                "active".into(),
                "succeeded".into(),
                800,
                1,
                "renewal".into(),
                2
            )
        );
        assert!(
            !renew_subscription(&state, subscription_id, true)
                .await
                .unwrap()
        );
        let balance: i64 = sqlx::query_scalar("SELECT balance_cents FROM users WHERE id=?")
            .bind(user_id)
            .fetch_one(&state.pool)
            .await
            .unwrap();
        assert_eq!(balance, 800);
    }

    #[tokio::test]
    async fn auto_renew_requires_a_paid_plan_and_is_cleared_on_cancel() {
        let (_directory, state) = test_support::state().await;
        let user_id = sqlx::query(
            "INSERT INTO users (username,display_name,password_hash) \
             VALUES ('renew-toggle-user','Renew Toggle','unused')",
        )
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        let plan_id = sqlx::query(
            "INSERT INTO plans (name,token_limit,duration_days,price_cents) \
             VALUES ('renew-toggle-plan',1000,30,0)",
        )
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        let starts_at = Utc::now();
        let ends_at = starts_at + Duration::days(30);
        let subscription_id = sqlx::query(
            "INSERT INTO subscriptions (user_id,plan_id,token_limit,starts_at,ends_at) \
             VALUES (?,?,?,?,?)",
        )
        .bind(user_id)
        .bind(plan_id)
        .bind(1000_i64)
        .bind(starts_at.to_rfc3339())
        .bind(ends_at.to_rfc3339())
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        let session = AuthSession {
            id: 1,
            user_id,
            username: "renew-toggle-user".into(),
            display_name: "Renew Toggle".into(),
            role: "user".into(),
        };

        let error = update_auto_renew(
            State(state.clone()),
            Extension(session.clone()),
            Path(subscription_id),
            Json(AutoRenewInput { enabled: true }),
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "AUTO_RENEW_UNAVAILABLE");

        sqlx::query("UPDATE plans SET price_cents=500 WHERE id=?")
            .bind(plan_id)
            .execute(&state.pool)
            .await
            .unwrap();
        let _ = update_auto_renew(
            State(state.clone()),
            Extension(session),
            Path(subscription_id),
            Json(AutoRenewInput { enabled: true }),
        )
        .await
        .unwrap();
        let enabled: (bool, String, String) = sqlx::query_as(
            "SELECT auto_renew,renewal_status,next_renewal_at FROM subscriptions WHERE id=?",
        )
        .bind(subscription_id)
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(enabled, (true, "scheduled".into(), ends_at.to_rfc3339()));

        let _ = update_subscription(
            State(state.clone()),
            Path(subscription_id),
            Json(SubscriptionUpdate {
                status: "cancelled".into(),
            }),
        )
        .await
        .unwrap();
        let cancelled: (String, bool, String, Option<String>) = sqlx::query_as(
            "SELECT status,auto_renew,renewal_status,next_renewal_at FROM subscriptions WHERE id=?",
        )
        .bind(subscription_id)
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(
            cancelled,
            ("cancelled".into(), false, "disabled".into(), None)
        );
    }
}
