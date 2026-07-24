use axum::{
    Json, Router,
    extract::{Extension, Query, State},
    routing::get,
};
use chrono::{Duration, NaiveDate, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::{FromRow, QueryBuilder, Sqlite, SqlitePool};

use crate::{
    auth::AuthSession,
    error::{ApiError, ApiResult},
    state::AppState,
};

pub fn admin_router() -> Router<AppState> {
    Router::new().route("/dashboard", get(admin_dashboard))
}

pub fn user_router() -> Router<AppState> {
    Router::new().route("/dashboard", get(user_dashboard))
}

#[derive(Clone, Debug, Deserialize)]
struct DashboardQuery {
    #[serde(default = "default_range")]
    range: String,
    start_date: Option<String>,
    end_date: Option<String>,
}

impl Default for DashboardQuery {
    fn default() -> Self {
        Self {
            range: default_range(),
            start_date: None,
            end_date: None,
        }
    }
}

fn default_range() -> String {
    "7d".into()
}

struct DateBounds {
    kind: String,
    start: String,
    end: String,
}

fn date_bounds(query: DashboardQuery) -> ApiResult<DateBounds> {
    let now = Utc::now();
    let (kind, start, end) = match query.range.trim() {
        "24h" => (
            "24h".to_string(),
            now - Duration::hours(24),
            now + Duration::seconds(1),
        ),
        "7d" => (
            "7d".to_string(),
            now - Duration::days(7),
            now + Duration::seconds(1),
        ),
        "30d" => (
            "30d".to_string(),
            now - Duration::days(30),
            now + Duration::seconds(1),
        ),
        "90d" => (
            "90d".to_string(),
            now - Duration::days(90),
            now + Duration::seconds(1),
        ),
        "custom" => {
            let start = parse_date(query.start_date.as_deref(), "start_date")?;
            let end = parse_date(query.end_date.as_deref(), "end_date")?;
            if start > end || (end - start).num_days() > 365 {
                return Err(ApiError::bad_request(
                    "INVALID_DASHBOARD_RANGE",
                    "custom dashboard range must be ordered and at most 365 days",
                ));
            }
            let start = start.and_hms_opt(0, 0, 0).unwrap().and_utc();
            let end = end
                .succ_opt()
                .ok_or_else(|| ApiError::bad_request("INVALID_DATE", "end_date is invalid"))?
                .and_hms_opt(0, 0, 0)
                .unwrap()
                .and_utc();
            ("custom".to_string(), start, end)
        }
        _ => {
            return Err(ApiError::bad_request(
                "INVALID_DASHBOARD_RANGE",
                "range must be 24h, 7d, 30d, 90d, or custom",
            ));
        }
    };
    Ok(DateBounds {
        kind,
        start: start.to_rfc3339(),
        end: end.to_rfc3339(),
    })
}

fn parse_date(value: Option<&str>, field: &'static str) -> ApiResult<NaiveDate> {
    let value = value.ok_or_else(|| {
        ApiError::bad_request("DASHBOARD_DATE_REQUIRED", format!("{field} is required"))
    })?;
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| ApiError::bad_request("INVALID_DATE", format!("{field} must use YYYY-MM-DD")))
}

#[derive(Default, FromRow)]
struct Summary {
    requests: i64,
    successful_requests: i64,
    failed_requests: i64,
    input_tokens: i64,
    output_tokens: i64,
    cached_input_tokens: i64,
    reasoning_tokens: i64,
    total_tokens: i64,
    cost_microusd: i64,
    average_duration_ms: f64,
    maximum_duration_ms: i64,
}

#[derive(FromRow)]
struct TrendRow {
    date: String,
    requests: i64,
    successful_requests: i64,
    failed_requests: i64,
    total_tokens: i64,
    cost_microusd: i64,
}

#[derive(FromRow)]
struct DimensionRow {
    name: String,
    requests: i64,
    total_tokens: i64,
    cost_microusd: i64,
}

fn push_usage_scope(
    query: &mut QueryBuilder<'_, Sqlite>,
    user_id: Option<i64>,
    start: Option<&str>,
    end: Option<&str>,
) {
    query.push(" WHERE 1=1");
    if let Some(user_id) = user_id {
        query.push(" AND user_id = ").push_bind(user_id);
    }
    if let Some(start) = start {
        query
            .push(" AND datetime(created_at) >= datetime(")
            .push_bind(start.to_string())
            .push(")");
    }
    if let Some(end) = end {
        query
            .push(" AND datetime(created_at) < datetime(")
            .push_bind(end.to_string())
            .push(")");
    }
}

async fn summary(
    pool: &SqlitePool,
    user_id: Option<i64>,
    start: Option<&str>,
    end: Option<&str>,
) -> ApiResult<Summary> {
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT COUNT(*) AS requests, COALESCE(SUM(status_code < 400),0) AS successful_requests, \
         COALESCE(SUM(status_code >= 400),0) AS failed_requests, \
         COALESCE(SUM(input_tokens),0) AS input_tokens, \
         COALESCE(SUM(output_tokens),0) AS output_tokens, \
         COALESCE(SUM(cached_input_tokens),0) AS cached_input_tokens, \
         COALESCE(SUM(reasoning_tokens),0) AS reasoning_tokens, \
         COALESCE(SUM(total_tokens),0) AS total_tokens, \
         COALESCE(SUM(cost_microusd),0) AS cost_microusd, \
         CAST(COALESCE(AVG(duration_ms),0) AS REAL) AS average_duration_ms, \
         COALESCE(MAX(duration_ms),0) AS maximum_duration_ms FROM usage_logs",
    );
    push_usage_scope(&mut query, user_id, start, end);
    Ok(query.build_query_as().fetch_one(pool).await?)
}

async fn trend(
    pool: &SqlitePool,
    user_id: Option<i64>,
    bounds: &DateBounds,
) -> ApiResult<Vec<Value>> {
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT date(created_at) AS date, COUNT(*) AS requests, \
         COALESCE(SUM(status_code < 400),0) AS successful_requests, \
         COALESCE(SUM(status_code >= 400),0) AS failed_requests, \
         COALESCE(SUM(total_tokens),0) AS total_tokens, \
         COALESCE(SUM(cost_microusd),0) AS cost_microusd FROM usage_logs",
    );
    push_usage_scope(&mut query, user_id, Some(&bounds.start), Some(&bounds.end));
    query.push(" GROUP BY date(created_at) ORDER BY date(created_at) ASC LIMIT 366");
    let rows: Vec<TrendRow> = query.build_query_as().fetch_all(pool).await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            json!({"date":row.date,"requests":row.requests,
                "successful_requests":row.successful_requests,"failed_requests":row.failed_requests,
                "tokens":row.total_tokens,"cost_microusd":row.cost_microusd})
        })
        .collect())
}

async fn dimension(
    pool: &SqlitePool,
    user_id: Option<i64>,
    bounds: &DateBounds,
    expression: &'static str,
) -> ApiResult<Vec<Value>> {
    let mut query = QueryBuilder::<Sqlite>::new(format!(
        "SELECT {expression} AS name, COUNT(*) AS requests, \
         COALESCE(SUM(total_tokens),0) AS total_tokens, \
         COALESCE(SUM(cost_microusd),0) AS cost_microusd FROM usage_logs"
    ));
    push_usage_scope(&mut query, user_id, Some(&bounds.start), Some(&bounds.end));
    query.push(format!(
        " GROUP BY {expression} ORDER BY requests DESC LIMIT 12"
    ));
    let rows: Vec<DimensionRow> = query.build_query_as().fetch_all(pool).await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            json!({"name":row.name,"model":row.name,"requests":row.requests,
                "tokens":row.total_tokens,"cost_microusd":row.cost_microusd})
        })
        .collect())
}

fn summary_json(value: &Summary) -> Value {
    json!({
        "requests":value.requests,"successful_requests":value.successful_requests,
        "failed_requests":value.failed_requests,"input_tokens":value.input_tokens,
        "output_tokens":value.output_tokens,"cached_input_tokens":value.cached_input_tokens,
        "reasoning_tokens":value.reasoning_tokens,"total_tokens":value.total_tokens,
        "cost_microusd":value.cost_microusd,
        "average_duration_ms":value.average_duration_ms.round() as i64,
        "maximum_duration_ms":value.maximum_duration_ms,
        "success_rate":if value.requests == 0 { 100.0 } else {
            value.successful_requests as f64 * 100.0 / value.requests as f64
        }
    })
}

async fn common(pool: &SqlitePool, user_id: Option<i64>, bounds: &DateBounds) -> ApiResult<Value> {
    let period = summary(pool, user_id, Some(&bounds.start), Some(&bounds.end)).await?;
    let total = summary(pool, user_id, None, None).await?;
    let last_day_start = (Utc::now() - Duration::hours(24)).to_rfc3339();
    let last_day = summary(pool, user_id, Some(&last_day_start), None).await?;
    let last_minute_start = (Utc::now() - Duration::minutes(1)).to_rfc3339();
    let last_minute = summary(pool, user_id, Some(&last_minute_start), None).await?;
    let trend = trend(pool, user_id, bounds).await?;
    let models = dimension(
        pool,
        user_id,
        bounds,
        "COALESCE(NULLIF(model,''),'unknown')",
    )
    .await?;
    let endpoints = dimension(pool, user_id, bounds, "endpoint").await?;
    Ok(json!({
        "range":{"kind":bounds.kind,"start":bounds.start,"end":bounds.end},
        "period":summary_json(&period),"total":summary_json(&total),
        "last_24h":summary_json(&last_day),
        "rpm":last_minute.requests,"tpm":last_minute.total_tokens,
        "trend":trend,"models":models,"endpoints":endpoints
    }))
}

async fn user_dashboard(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
    Query(query): Query<DashboardQuery>,
) -> ApiResult<Json<Value>> {
    let bounds = date_bounds(query)?;
    let common = common(&state.pool, Some(session.user_id), &bounds).await?;
    let user: (i64, String) =
        sqlx::query_as("SELECT balance_cents,display_name FROM users WHERE id = ?")
            .bind(session.user_id)
            .fetch_one(&state.pool)
            .await?;
    let keys: (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*),COALESCE(SUM(enabled = 1 AND \
         (expires_at IS NULL OR datetime(expires_at) > CURRENT_TIMESTAMP)),0) \
         FROM api_keys WHERE user_id = ?",
    )
    .bind(session.user_id)
    .fetch_one(&state.pool)
    .await?;
    let unread_announcements: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM announcements WHERE status = 'active' \
         AND (starts_at IS NULL OR datetime(starts_at) <= CURRENT_TIMESTAMP) \
         AND (ends_at IS NULL OR datetime(ends_at) > CURRENT_TIMESTAMP) \
         AND NOT EXISTS (SELECT 1 FROM announcement_reads \
         WHERE announcement_reads.announcement_id = announcements.id \
         AND announcement_reads.user_id = ?)",
    )
    .bind(session.user_id)
    .fetch_one(&state.pool)
    .await?;
    let subscription: Option<(i64, String, String, i64, String, String, i64, Option<i64>, Option<String>)> = sqlx::query_as(
        "SELECT subscriptions.id,plans.name,subscriptions.status,subscriptions.token_limit, \
         subscriptions.starts_at,subscriptions.ends_at, \
         COALESCE((SELECT SUM(COALESCE(log.total_tokens,0)) FROM usage_logs log \
           LEFT JOIN api_keys keys ON keys.id=log.api_key_id \
           WHERE log.user_id=subscriptions.user_id \
           AND datetime(log.created_at)>=datetime(subscriptions.starts_at) \
           AND datetime(log.created_at)<datetime(subscriptions.ends_at) \
           AND (subscriptions.group_id IS NULL OR keys.group_id=subscriptions.group_id)),0), \
         subscriptions.group_id,groups.name \
         FROM subscriptions JOIN plans ON plans.id=subscriptions.plan_id \
         LEFT JOIN groups ON groups.id=subscriptions.group_id \
         WHERE subscriptions.user_id=? AND subscriptions.status='active' \
         AND datetime(subscriptions.ends_at)>CURRENT_TIMESTAMP ORDER BY subscriptions.id DESC LIMIT 1",
    )
    .bind(session.user_id)
    .fetch_optional(&state.pool)
    .await?;
    let subscription = subscription.map(|row| {
        json!({
            "id":row.0,"plan_name":row.1,"status":row.2,"token_limit":row.3,
            "starts_at":row.4,"ends_at":row.5,"used_tokens":row.6,
            "remaining_tokens":if row.3 == 0 { 0 } else { (row.3-row.6).max(0) },
            "group_id":row.7,"group_name":row.8
        })
    });
    Ok(Json(json!({"data":{
        "display_name":user.1,"balance_cents":user.0,
        "total_api_keys":keys.0,"active_keys":keys.1,
        "unread_announcements":unread_announcements,"subscription":subscription,
        "requests_24h":common["last_24h"]["requests"],
        "errors_24h":common["last_24h"]["failed_requests"],
        "tokens_24h":common["last_24h"]["total_tokens"],
        "total_requests":common["total"]["requests"],
        "total_input_tokens":common["total"]["input_tokens"],
        "total_output_tokens":common["total"]["output_tokens"],
        "total_tokens":common["total"]["total_tokens"],
        "average_duration_ms":common["total"]["average_duration_ms"],
        "range":common["range"],"period":common["period"],"total":common["total"],
        "last_24h":common["last_24h"],"rpm":common["rpm"],"tpm":common["tpm"],
        "trend":common["trend"],"models":common["models"],"endpoints":common["endpoints"]
    }})))
}

async fn admin_dashboard(
    State(state): State<AppState>,
    Query(query): Query<DashboardQuery>,
) -> ApiResult<Json<Value>> {
    let bounds = date_bounds(query)?;
    let common = common(&state.pool, None, &bounds).await?;
    let entities: (i64, i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT \
         (SELECT COUNT(*) FROM users WHERE role='user' AND deleted_at IS NULL), \
         (SELECT COUNT(*) FROM users WHERE role='user' AND enabled=1 AND deleted_at IS NULL), \
         (SELECT COUNT(*) FROM accounts), \
         (SELECT COUNT(*) FROM accounts WHERE enabled=1), \
         (SELECT COUNT(*) FROM api_keys), \
         (SELECT COUNT(*) FROM api_keys WHERE enabled=1)",
    )
    .fetch_one(&state.pool)
    .await?;
    let new_users: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM users WHERE role='user' AND deleted_at IS NULL \
         AND datetime(created_at)>=datetime(?) AND datetime(created_at)<datetime(?)",
    )
    .bind(&bounds.start)
    .bind(&bounds.end)
    .fetch_one(&state.pool)
    .await?;
    let orders: (i64, i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT \
         COALESCE(SUM(status='paid'),0), \
         COALESCE(SUM(CASE WHEN status='paid' THEN amount_cents ELSE 0 END),0), \
         COALESCE(SUM(status='paid' AND datetime(COALESCE(paid_at,created_at))>=datetime(?) \
           AND datetime(COALESCE(paid_at,created_at))<datetime(?)),0), \
         COALESCE(SUM(CASE WHEN status='paid' AND datetime(COALESCE(paid_at,created_at))>=datetime(?) \
           AND datetime(COALESCE(paid_at,created_at))<datetime(?) THEN amount_cents ELSE 0 END),0), \
         COALESCE(SUM(status='refunded'),0), \
         COALESCE(SUM(CASE WHEN status='refunded' THEN amount_cents ELSE 0 END),0) FROM orders",
    )
    .bind(&bounds.start)
    .bind(&bounds.end)
    .bind(&bounds.start)
    .bind(&bounds.end)
    .fetch_one(&state.pool)
    .await?;
    let active_subscriptions: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM subscriptions WHERE status='active' AND datetime(ends_at)>CURRENT_TIMESTAMP",
    )
    .fetch_one(&state.pool)
    .await?;
    let mut top_users = QueryBuilder::<Sqlite>::new(
        "SELECT COALESCE(users.username,'system') AS name,COUNT(*) AS requests, \
         COALESCE(SUM(usage_logs.total_tokens),0) AS total_tokens, \
         COALESCE(SUM(usage_logs.cost_microusd),0) AS cost_microusd \
         FROM usage_logs LEFT JOIN users ON users.id=usage_logs.user_id",
    );
    top_users
        .push(" WHERE datetime(usage_logs.created_at)>=datetime(")
        .push_bind(bounds.start.clone())
        .push(") AND datetime(usage_logs.created_at)<datetime(")
        .push_bind(bounds.end.clone())
        .push(")");
    top_users.push(" GROUP BY usage_logs.user_id,users.username ORDER BY cost_microusd DESC,requests DESC LIMIT 10");
    let top_users: Vec<DimensionRow> = top_users.build_query_as().fetch_all(&state.pool).await?;
    let top_users = top_users
        .into_iter()
        .map(|row| {
            json!({
                "username":row.name,"requests":row.requests,"tokens":row.total_tokens,
                "cost_microusd":row.cost_microusd
            })
        })
        .collect::<Vec<_>>();
    let groups: Vec<(String, i64, i64, i64)> = sqlx::query_as(
        "SELECT COALESCE(groups.name,'unassigned'),COUNT(*),COALESCE(SUM(usage_logs.total_tokens),0), \
         COALESCE(SUM(usage_logs.cost_microusd),0) FROM usage_logs \
         LEFT JOIN api_keys ON api_keys.id=usage_logs.api_key_id \
         LEFT JOIN groups ON groups.id=api_keys.group_id \
         WHERE datetime(usage_logs.created_at)>=datetime(?) AND datetime(usage_logs.created_at)<datetime(?) \
         GROUP BY api_keys.group_id,groups.name ORDER BY COUNT(*) DESC LIMIT 10",
    )
    .bind(&bounds.start)
    .bind(&bounds.end)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(json!({"data":{
        "users":entities.0,"active_users":entities.1,"new_users":new_users,
        "accounts":entities.2,"active_accounts":entities.3,
        "keys":entities.4,"active_keys":entities.5,"active_subscriptions":active_subscriptions,
        "paid_orders":orders.0,"revenue_cents":orders.1,"period_paid_orders":orders.2,
        "period_revenue_cents":orders.3,"refunded_orders":orders.4,"refunded_cents":orders.5,
        "requests_24h":common["last_24h"]["requests"],
        "errors_24h":common["last_24h"]["failed_requests"],
        "tokens_24h":common["last_24h"]["total_tokens"],
        "total_requests":common["total"]["requests"],
        "total_input_tokens":common["total"]["input_tokens"],
        "total_output_tokens":common["total"]["output_tokens"],
        "total_tokens":common["total"]["total_tokens"],
        "average_duration_ms":common["total"]["average_duration_ms"],
        "range":common["range"],"period":common["period"],"total":common["total"],
        "last_24h":common["last_24h"],"rpm":common["rpm"],"tpm":common["tpm"],
        "trend":common["trend"],"models":common["models"],"endpoints":common["endpoints"],
        "top_users":top_users,
        "groups":groups.into_iter().map(|row|json!({"name":row.0,"requests":row.1,
          "tokens":row.2,"cost_microusd":row.3})).collect::<Vec<_>>()
    }})))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;

    #[tokio::test]
    async fn admin_dashboard_handles_empty_tables() {
        let (_directory, state) = test_support::state().await;
        let Json(value) = admin_dashboard(State(state), Query(DashboardQuery::default()))
            .await
            .unwrap();
        assert_eq!(value["data"]["total_requests"], 0);
        assert_eq!(value["data"]["period"]["success_rate"], 100.0);
        assert_eq!(value["data"]["trend"], json!([]));
    }

    #[tokio::test]
    async fn user_dashboard_includes_balance_subscription_and_scoped_usage() {
        let (_directory, state) = test_support::state().await;
        let user_id = sqlx::query(
            "INSERT INTO users (username,display_name,password_hash,balance_cents) \
             VALUES ('dashboard-user','Dashboard User','x',12345)",
        )
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        let other_id = sqlx::query(
            "INSERT INTO users (username,display_name,password_hash) VALUES ('dashboard-other','Other','x')",
        )
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        let plan_id = sqlx::query(
            "INSERT INTO plans (name,token_limit,duration_days) VALUES ('Dashboard Plan',1000,30)",
        )
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        let starts = (Utc::now() - Duration::days(1)).to_rfc3339();
        let ends = (Utc::now() + Duration::days(29)).to_rfc3339();
        sqlx::query(
            "INSERT INTO subscriptions (user_id,plan_id,token_limit,starts_at,ends_at) VALUES (?,?,?,?,?)",
        )
        .bind(user_id)
        .bind(plan_id)
        .bind(1000_i64)
        .bind(&starts)
        .bind(&ends)
        .execute(&state.pool)
        .await
        .unwrap();
        for (id, owner, tokens) in [("mine", user_id, 120_i64), ("other", other_id, 900_i64)] {
            sqlx::query(
                "INSERT INTO usage_logs (request_id,user_id,endpoint,status_code,total_tokens,cost_microusd,duration_ms) \
                 VALUES (?,?,'/v1/responses',200,?,1000,20)",
            )
            .bind(id)
            .bind(owner)
            .bind(tokens)
            .execute(&state.pool)
            .await
            .unwrap();
        }
        let session = AuthSession {
            id: 1,
            user_id,
            username: "dashboard-user".into(),
            display_name: "Dashboard User".into(),
            role: "user".into(),
        };
        let Json(value) = user_dashboard(
            State(state),
            Extension(session),
            Query(DashboardQuery::default()),
        )
        .await
        .unwrap();
        assert_eq!(value["data"]["balance_cents"], 12345);
        assert_eq!(value["data"]["period"]["total_tokens"], 120);
        assert_eq!(value["data"]["subscription"]["used_tokens"], 120);
        assert_eq!(value["data"]["subscription"]["remaining_tokens"], 880);
    }

    #[test]
    fn custom_range_is_bounded() {
        assert!(
            date_bounds(DashboardQuery {
                range: "custom".into(),
                start_date: Some("2026-01-01".into()),
                end_date: Some("2027-02-01".into()),
            })
            .is_err()
        );
    }
}
