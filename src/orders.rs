use axum::{
    Json, Router,
    extract::{Extension, Path, State},
    http::StatusCode,
    routing::{get, post},
};
use chrono::{Duration, Utc};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    auth::AuthSession,
    error::{ApiError, ApiResult},
    state::AppState,
};

pub fn user_router() -> Router<AppState> {
    Router::new()
        .route("/orders", get(user_orders))
        .route("/purchase", post(purchase))
}

pub fn admin_router() -> Router<AppState> {
    Router::new()
        .route("/orders", get(admin_orders))
        .route("/orders/dashboard", get(order_dashboard))
        .route("/orders/{id}/refund", post(refund_order))
}

#[derive(Deserialize)]
struct PurchaseInput {
    plan_id: i64,
}

async fn purchase(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
    Json(input): Json<PurchaseInput>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let plan: Option<(String, i64, i64, i64, Option<i64>)> = sqlx::query_as(
        "SELECT plans.name, plans.token_limit, plans.duration_days, plans.price_cents, \
         plans.group_id FROM plans LEFT JOIN groups ON groups.id = plans.group_id \
         WHERE plans.id = ? AND plans.enabled = 1 \
         AND (plans.group_id IS NULL OR (groups.enabled = 1 AND groups.subscription_type = 'subscription'))",
    )
    .bind(input.plan_id)
    .fetch_optional(&state.pool)
    .await?;
    let (plan_name, token_limit, duration_days, price_cents, group_id) =
        plan.ok_or_else(|| ApiError::bad_request("PLAN_NOT_FOUND", "plan is unavailable"))?;
    if price_cents <= 0 {
        return Err(ApiError::bad_request(
            "PLAN_NOT_PURCHASABLE",
            "plan is not available for balance purchase",
        ));
    }
    let starts_at = Utc::now();
    let ends_at = starts_at + Duration::days(duration_days);
    let mut transaction = state.pool.begin().await?;
    let debited = sqlx::query(
        "UPDATE users SET balance_cents = balance_cents - ?, updated_at = CURRENT_TIMESTAMP \
         WHERE id = ? AND enabled = 1 AND balance_cents >= ?",
    )
    .bind(price_cents)
    .bind(session.user_id)
    .bind(price_cents)
    .execute(&mut *transaction)
    .await?;
    if debited.rows_affected() != 1 {
        return Err(ApiError::bad_request(
            "INSUFFICIENT_BALANCE",
            "account balance is insufficient",
        ));
    }
    let order_id =
        sqlx::query("INSERT INTO orders (user_id, plan_id, amount_cents) VALUES (?, ?, ?)")
            .bind(session.user_id)
            .bind(input.plan_id)
            .bind(price_cents)
            .execute(&mut *transaction)
            .await?
            .last_insert_rowid();
    sqlx::query(
        "UPDATE subscriptions SET status = 'cancelled', auto_renew = 0, \
         renewal_status = 'disabled', next_renewal_at = NULL, last_renewal_error = '', \
         updated_at = CURRENT_TIMESTAMP \
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
    .bind(input.plan_id)
    .bind(token_limit)
    .bind(starts_at.to_rfc3339())
    .bind(ends_at.to_rfc3339())
    .bind(group_id)
    .execute(&mut *transaction)
    .await?
    .last_insert_rowid();
    sqlx::query(
        "UPDATE orders SET subscription_id = ?, status = 'paid', paid_at = CURRENT_TIMESTAMP, \
         updated_at = CURRENT_TIMESTAMP WHERE id = ?",
    )
    .bind(subscription_id)
    .bind(order_id)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"data": {
            "id": order_id, "subscription_id": subscription_id,
            "plan_name": plan_name, "amount_cents": price_cents,
            "ends_at": ends_at.to_rfc3339()
        }})),
    ))
}

async fn user_orders(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
) -> ApiResult<Json<Value>> {
    list_orders(&state, Some(session.user_id)).await
}

async fn admin_orders(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    list_orders(&state, None).await
}

async fn list_orders(state: &AppState, user_id: Option<i64>) -> ApiResult<Json<Value>> {
    let rows: Vec<(
        i64,
        i64,
        String,
        Option<String>,
        i64,
        String,
        String,
        String,
        i64,
        Option<i64>,
        Option<String>,
        Option<String>,
        String,
    )> = sqlx::query_as(
        "SELECT orders.id, orders.user_id, users.username, users.email, orders.plan_id, \
         plans.name, orders.status, orders.order_type, orders.amount_cents, orders.subscription_id, \
         orders.paid_at, orders.refunded_at, orders.created_at FROM orders \
         JOIN users ON users.id = orders.user_id JOIN plans ON plans.id = orders.plan_id \
         WHERE (? IS NULL OR orders.user_id = ?) ORDER BY orders.id DESC LIMIT 1000",
    )
    .bind(user_id)
    .bind(user_id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(json!({"data": rows.into_iter().map(|row| json!({
        "id": row.0, "user_id": row.1, "username": row.2, "email": row.3,
        "plan_id": row.4, "plan_name": row.5, "status": row.6, "order_type": row.7,
        "amount_cents": row.8, "subscription_id": row.9, "paid_at": row.10,
        "refunded_at": row.11, "created_at": row.12
    })).collect::<Vec<_>>() })))
}

async fn order_dashboard(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let totals: (i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), \
         COALESCE(SUM(CASE WHEN status = 'paid' THEN amount_cents ELSE 0 END), 0), \
         COALESCE(SUM(CASE WHEN status = 'refunded' THEN amount_cents ELSE 0 END), 0), \
         COALESCE(SUM(CASE WHEN status = 'paid' THEN 1 ELSE 0 END), 0), \
         COALESCE(SUM(CASE WHEN order_type = 'renewal' AND status = 'paid' THEN 1 ELSE 0 END), 0) FROM orders",
    )
    .fetch_one(&state.pool)
    .await?;
    Ok(Json(json!({"data": {
        "orders": totals.0, "revenue_cents": totals.1,
        "refunded_cents": totals.2, "paid_orders": totals.3,
        "renewal_orders": totals.4
    }})))
}

async fn refund_order(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<Value>> {
    let mut transaction = state.pool.begin().await?;
    let order: Option<(i64, i64, Option<i64>, String)> = sqlx::query_as(
        "SELECT user_id, amount_cents, subscription_id, status FROM orders WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&mut *transaction)
    .await?;
    let (user_id, amount_cents, subscription_id, status) =
        order.ok_or_else(|| ApiError::not_found("order not found"))?;
    if status != "paid" {
        return Err(ApiError::bad_request(
            "ORDER_NOT_REFUNDABLE",
            "only paid orders can be refunded",
        ));
    }
    let changed = sqlx::query(
        "UPDATE orders SET status = 'refunded', refunded_at = CURRENT_TIMESTAMP, \
         updated_at = CURRENT_TIMESTAMP WHERE id = ? AND status = 'paid'",
    )
    .bind(id)
    .execute(&mut *transaction)
    .await?;
    if changed.rows_affected() != 1 {
        return Err(ApiError::bad_request(
            "ORDER_NOT_REFUNDABLE",
            "order was already changed",
        ));
    }
    sqlx::query("UPDATE users SET balance_cents = balance_cents + ? WHERE id = ?")
        .bind(amount_cents)
        .bind(user_id)
        .execute(&mut *transaction)
        .await?;
    if let Some(subscription_id) = subscription_id {
        sqlx::query(
            "UPDATE subscriptions SET status = 'cancelled', auto_renew = 0, \
             renewal_status = 'disabled', next_renewal_at = NULL, last_renewal_error = '', \
             updated_at = CURRENT_TIMESTAMP \
             WHERE id = ? AND status = 'active'",
        )
        .bind(subscription_id)
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;
    Ok(Json(json!({"data": {
        "id": id, "refunded_cents": amount_cents
    }})))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{crypto::hash_password, test_support};

    #[tokio::test]
    async fn balance_purchase_and_refund_are_atomic() {
        let (_directory, state) = test_support::state().await;
        let user_id = sqlx::query(
            "INSERT INTO users (username, display_name, password_hash, balance_cents) \
             VALUES ('buyer', 'buyer', ?, 2000)",
        )
        .bind(hash_password("buyer-password").unwrap())
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        let plan_id = sqlx::query(
            "INSERT INTO plans (name, token_limit, duration_days, price_cents) \
             VALUES ('paid', 5000, 30, 1200)",
        )
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        let session = AuthSession {
            id: 1,
            user_id,
            username: "buyer".into(),
            display_name: "buyer".into(),
            role: "user".into(),
        };
        let (_, Json(result)) = purchase(
            State(state.clone()),
            Extension(session),
            Json(PurchaseInput { plan_id }),
        )
        .await
        .unwrap();
        let order_id = result["data"]["id"].as_i64().unwrap();
        let purchased: (i64, i64, String) = sqlx::query_as(
            "SELECT users.balance_cents, COUNT(subscriptions.id), orders.status FROM users \
             JOIN orders ON orders.user_id = users.id \
             JOIN subscriptions ON subscriptions.id = orders.subscription_id \
             WHERE users.id = ?",
        )
        .bind(user_id)
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(purchased, (800, 1, "paid".into()));
        let _ = refund_order(State(state.clone()), Path(order_id))
            .await
            .unwrap();
        let refunded: (i64, String, String) = sqlx::query_as(
            "SELECT users.balance_cents, orders.status, subscriptions.status FROM users \
             JOIN orders ON orders.user_id = users.id \
             JOIN subscriptions ON subscriptions.id = orders.subscription_id \
             WHERE orders.id = ?",
        )
        .bind(order_id)
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(refunded, (2000, "refunded".into(), "cancelled".into()));
        assert!(refund_order(State(state), Path(order_id)).await.is_err());
    }
}
