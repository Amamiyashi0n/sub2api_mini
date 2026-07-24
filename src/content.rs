use axum::{
    Json, Router,
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    routing::{get, post, put},
};
use chrono::DateTime;
use pulldown_cmark::{CowStr, Event, Options, Parser, Tag, html};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::FromRow;
use std::collections::{HashMap, HashSet};

use crate::{
    auth::AuthSession,
    error::{ApiError, ApiResult},
    state::AppState,
};

pub fn public_router() -> Router<AppState> {
    Router::new()
        .route("/announcements", get(public_announcements))
        .route("/pages", get(public_pages))
        .route("/pages/{slug}", get(public_page))
}

pub fn user_router() -> Router<AppState> {
    Router::new()
        .route("/announcements", get(user_announcements))
        .route("/announcements/{id}/read", post(mark_announcement_read))
        .route("/pages", get(user_pages))
        .route("/pages/{slug}", get(user_page))
}

pub fn admin_router() -> Router<AppState> {
    Router::new()
        .route(
            "/announcements",
            get(admin_announcements).post(create_announcement),
        )
        .route(
            "/announcements/{id}",
            get(admin_announcement)
                .put(update_announcement)
                .delete(delete_announcement),
        )
        .route(
            "/announcements/{id}/read-status",
            get(announcement_read_status),
        )
        .route("/pages", get(admin_pages).post(create_page))
        .route("/pages/{id}", put(update_page).delete(delete_page))
}

#[derive(Deserialize, Default)]
struct ListQuery {
    status: Option<String>,
    search: Option<String>,
    page: Option<i64>,
    page_size: Option<i64>,
    sort_by: Option<String>,
    sort_order: Option<String>,
    unread_only: Option<bool>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
struct AnnouncementTargeting {
    #[serde(default)]
    any_of: Vec<AnnouncementConditionGroup>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
struct AnnouncementConditionGroup {
    #[serde(default)]
    all_of: Vec<AnnouncementCondition>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
struct AnnouncementCondition {
    #[serde(rename = "type")]
    kind: String,
    operator: String,
    #[serde(default)]
    group_ids: Vec<i64>,
    #[serde(default)]
    value: i64,
}

impl AnnouncementTargeting {
    fn validate(&self) -> ApiResult<()> {
        if self.any_of.len() > 50 {
            return Err(invalid_targeting());
        }
        for group in &self.any_of {
            if group.all_of.is_empty() || group.all_of.len() > 50 {
                return Err(invalid_targeting());
            }
            for condition in &group.all_of {
                match condition.kind.as_str() {
                    "subscription"
                        if condition.operator == "in"
                            && !condition.group_ids.is_empty()
                            && condition.group_ids.len() <= 100
                            && condition.group_ids.iter().all(|id| *id > 0) => {}
                    "balance"
                        if matches!(
                            condition.operator.as_str(),
                            "gt" | "gte" | "lt" | "lte" | "eq"
                        ) && (0..=100_000_000_000_i64).contains(&condition.value) => {}
                    _ => return Err(invalid_targeting()),
                }
            }
        }
        Ok(())
    }

    fn matches(&self, balance_cents: i64, active_plan_ids: &HashSet<i64>) -> bool {
        self.any_of.is_empty()
            || self.any_of.iter().any(|group| {
                !group.all_of.is_empty()
                    && group
                        .all_of
                        .iter()
                        .all(|condition| match condition.kind.as_str() {
                            "subscription" => {
                                condition.operator == "in"
                                    && condition
                                        .group_ids
                                        .iter()
                                        .any(|id| active_plan_ids.contains(id))
                            }
                            "balance" => match condition.operator.as_str() {
                                "gt" => balance_cents > condition.value,
                                "gte" => balance_cents >= condition.value,
                                "lt" => balance_cents < condition.value,
                                "lte" => balance_cents <= condition.value,
                                "eq" => balance_cents == condition.value,
                                _ => false,
                            },
                            _ => false,
                        })
            })
    }
}

fn invalid_targeting() -> ApiError {
    ApiError::bad_request(
        "ANNOUNCEMENT_INVALID_TARGET",
        "announcement targeting rules are invalid",
    )
}

#[derive(FromRow)]
struct AnnouncementRow {
    id: i64,
    title: String,
    content: String,
    status: String,
    notify_mode: String,
    starts_at: Option<String>,
    ends_at: Option<String>,
    targeting: String,
    created_at: String,
    updated_at: String,
    read_count: i64,
}

const ANNOUNCEMENT_SELECT: &str = "SELECT announcements.id, announcements.title, \
    announcements.content, announcements.status, announcements.notify_mode, \
    announcements.starts_at, announcements.ends_at, announcements.targeting, \
    announcements.created_at, announcements.updated_at, \
    (SELECT COUNT(*) FROM announcement_reads WHERE announcement_id = announcements.id) AS read_count \
    FROM announcements";

fn parse_targeting(value: &str) -> ApiResult<AnnouncementTargeting> {
    let targeting: AnnouncementTargeting = serde_json::from_str(value)
        .map_err(|_| ApiError::internal("stored announcement targeting is invalid"))?;
    targeting.validate()?;
    Ok(targeting)
}

fn announcement_value(row: AnnouncementRow, targeting: AnnouncementTargeting) -> Value {
    let rendered_html = render_markdown(&row.content);
    json!({
        "id": row.id, "title": row.title, "content": row.content, "status": row.status,
        "rendered_html": rendered_html, "notify_mode": row.notify_mode,
        "targeting": targeting, "starts_at": row.starts_at, "ends_at": row.ends_at,
        "created_at": row.created_at, "updated_at": row.updated_at, "read_count": row.read_count
    })
}

async fn active_plan_ids(state: &AppState, user_id: i64) -> ApiResult<HashSet<i64>> {
    let ids: Vec<i64> = sqlx::query_scalar(
        "SELECT plan_id FROM subscriptions WHERE user_id = ? AND status = 'active' \
         AND datetime(starts_at) <= CURRENT_TIMESTAMP AND datetime(ends_at) > CURRENT_TIMESTAMP",
    )
    .bind(user_id)
    .fetch_all(&state.pool)
    .await?;
    Ok(ids.into_iter().collect())
}

async fn user_target_context(state: &AppState, user_id: i64) -> ApiResult<(i64, HashSet<i64>)> {
    let balance: i64 = sqlx::query_scalar(
        "SELECT balance_cents FROM users WHERE id = ? AND enabled = 1 AND deleted_at IS NULL",
    )
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| ApiError::not_found("user not found"))?;
    Ok((balance, active_plan_ids(state, user_id).await?))
}

async fn public_announcements(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let sql = format!(
        "{ANNOUNCEMENT_SELECT} WHERE announcements.status = 'active' \
         AND (announcements.starts_at IS NULL OR datetime(announcements.starts_at) <= CURRENT_TIMESTAMP) \
         AND (announcements.ends_at IS NULL OR datetime(announcements.ends_at) > CURRENT_TIMESTAMP) \
         ORDER BY announcements.id DESC LIMIT 50"
    );
    let rows = sqlx::query_as::<_, AnnouncementRow>(&sql)
        .fetch_all(&state.pool)
        .await?;
    let mut data = Vec::new();
    for row in rows {
        let targeting = parse_targeting(&row.targeting)?;
        if targeting.any_of.is_empty() {
            data.push(announcement_value(row, targeting));
        }
        if data.len() == 20 {
            break;
        }
    }
    Ok(Json(json!({"data": data})))
}

async fn user_announcements(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
    Query(query): Query<ListQuery>,
) -> ApiResult<Json<Value>> {
    let (balance, plans) = user_target_context(&state, session.user_id).await?;
    let sql = format!(
        "{ANNOUNCEMENT_SELECT} WHERE announcements.status = 'active' \
         AND (announcements.starts_at IS NULL OR datetime(announcements.starts_at) <= CURRENT_TIMESTAMP) \
         AND (announcements.ends_at IS NULL OR datetime(announcements.ends_at) > CURRENT_TIMESTAMP) \
         ORDER BY announcements.id DESC LIMIT 200"
    );
    let rows = sqlx::query_as::<_, AnnouncementRow>(&sql)
        .fetch_all(&state.pool)
        .await?;
    let reads: HashMap<i64, String> = sqlx::query_as::<_, (i64, String)>(
        "SELECT announcement_id, read_at FROM announcement_reads WHERE user_id = ?",
    )
    .bind(session.user_id)
    .fetch_all(&state.pool)
    .await?
    .into_iter()
    .collect();
    let mut data = Vec::new();
    for row in rows {
        let targeting = parse_targeting(&row.targeting)?;
        if !targeting.matches(balance, &plans) {
            continue;
        }
        let read_at = reads.get(&row.id).cloned();
        if query.unread_only.unwrap_or(false) && read_at.is_some() {
            continue;
        }
        let mut value = announcement_value(row, targeting);
        value["is_read"] = json!(read_at.is_some());
        value["read_at"] = json!(read_at);
        data.push(value);
        if data.len() == 50 {
            break;
        }
    }
    data.sort_by(|left, right| {
        left["is_read"]
            .as_bool()
            .cmp(&right["is_read"].as_bool())
            .then_with(|| right["id"].as_i64().cmp(&left["id"].as_i64()))
    });
    Ok(Json(json!({"data": data})))
}

async fn mark_announcement_read(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    let sql = format!(
        "{ANNOUNCEMENT_SELECT} WHERE announcements.id = ? AND announcements.status = 'active' \
         AND (announcements.starts_at IS NULL OR datetime(announcements.starts_at) <= CURRENT_TIMESTAMP) \
         AND (announcements.ends_at IS NULL OR datetime(announcements.ends_at) > CURRENT_TIMESTAMP)"
    );
    let row = sqlx::query_as::<_, AnnouncementRow>(&sql)
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| ApiError::not_found("announcement not found"))?;
    let targeting = parse_targeting(&row.targeting)?;
    let (balance, plans) = user_target_context(&state, session.user_id).await?;
    if !targeting.matches(balance, &plans) {
        return Err(ApiError::not_found("announcement not found"));
    }
    sqlx::query(
        "INSERT INTO announcement_reads (announcement_id, user_id) VALUES (?, ?) \
         ON CONFLICT(announcement_id, user_id) DO UPDATE SET read_at = CURRENT_TIMESTAMP",
    )
    .bind(id)
    .bind(session.user_id)
    .execute(&state.pool)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn admin_announcements(
    State(state): State<AppState>,
    Query(query): Query<ListQuery>,
) -> ApiResult<Json<Value>> {
    let status = query.status.as_deref().unwrap_or("").trim().to_string();
    if !status.is_empty() && !matches!(status.as_str(), "draft" | "active" | "archived") {
        return Err(ApiError::bad_request(
            "INVALID_ANNOUNCEMENT_FILTER",
            "announcement status filter is invalid",
        ));
    }
    let search = query.search.as_deref().unwrap_or("").trim().to_string();
    let page = query.page.unwrap_or(1).clamp(1, 1_000_000);
    let page_size = query.page_size.unwrap_or(20).clamp(1, 100);
    let order = match query.sort_by.as_deref() {
        Some("title") => "announcements.title",
        Some("status") => "announcements.status",
        Some("notify_mode") => "announcements.notify_mode",
        Some("starts_at") => "announcements.starts_at",
        Some("ends_at") => "announcements.ends_at",
        _ => "announcements.created_at",
    };
    let direction = if query.sort_order.as_deref() == Some("asc") {
        "ASC"
    } else {
        "DESC"
    };
    let filter = " WHERE (? = '' OR announcements.status = ?) AND \
        (? = '' OR announcements.title LIKE '%' || ? || '%' OR announcements.content LIKE '%' || ? || '%')";
    let sql = format!(
        "{ANNOUNCEMENT_SELECT}{filter} ORDER BY {order} {direction}, announcements.id {direction} \
         LIMIT ? OFFSET ?"
    );
    let rows = sqlx::query_as::<_, AnnouncementRow>(&sql)
        .bind(&status)
        .bind(&status)
        .bind(&search)
        .bind(&search)
        .bind(&search)
        .bind(page_size)
        .bind((page - 1) * page_size)
        .fetch_all(&state.pool)
        .await?;
    let total: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM announcements{filter}"))
        .bind(&status)
        .bind(&status)
        .bind(&search)
        .bind(&search)
        .bind(&search)
        .fetch_one(&state.pool)
        .await?;
    let mut data = Vec::with_capacity(rows.len());
    for row in rows {
        let targeting = parse_targeting(&row.targeting)?;
        data.push(announcement_value(row, targeting));
    }
    Ok(Json(
        json!({"data": data, "meta": {"page": page, "page_size": page_size, "total": total}}),
    ))
}

async fn admin_announcement(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<Value>> {
    let row = sqlx::query_as::<_, AnnouncementRow>(&format!(
        "{ANNOUNCEMENT_SELECT} WHERE announcements.id = ?"
    ))
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| ApiError::not_found("announcement not found"))?;
    let targeting = parse_targeting(&row.targeting)?;
    Ok(Json(json!({"data": announcement_value(row, targeting)})))
}

#[derive(Deserialize)]
struct AnnouncementInput {
    title: String,
    content: String,
    #[serde(default = "draft_status")]
    status: String,
    #[serde(default = "silent_mode")]
    notify_mode: String,
    #[serde(default)]
    targeting: AnnouncementTargeting,
    starts_at: Option<String>,
    ends_at: Option<String>,
}

fn draft_status() -> String {
    "draft".into()
}
fn silent_mode() -> String {
    "silent".into()
}

async fn create_announcement(
    State(state): State<AppState>,
    Json(input): Json<AnnouncementInput>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    validate_announcement(&input)?;
    let targeting = serde_json::to_string(&input.targeting)
        .map_err(|_| ApiError::internal("cannot encode announcement targeting"))?;
    let result = sqlx::query(
        "INSERT INTO announcements (title, content, status, notify_mode, targeting, starts_at, ends_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(input.title.trim()).bind(input.content.trim()).bind(&input.status)
    .bind(&input.notify_mode).bind(targeting)
    .bind(normalize_datetime(input.starts_at.as_deref())?)
    .bind(normalize_datetime(input.ends_at.as_deref())?)
    .execute(&state.pool).await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"data": {"id": result.last_insert_rowid()}})),
    ))
}

async fn update_announcement(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(input): Json<AnnouncementInput>,
) -> ApiResult<Json<Value>> {
    validate_announcement(&input)?;
    let targeting = serde_json::to_string(&input.targeting)
        .map_err(|_| ApiError::internal("cannot encode announcement targeting"))?;
    let result = sqlx::query(
        "UPDATE announcements SET title = ?, content = ?, status = ?, notify_mode = ?, \
         targeting = ?, starts_at = ?, ends_at = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
    )
    .bind(input.title.trim())
    .bind(input.content.trim())
    .bind(&input.status)
    .bind(&input.notify_mode)
    .bind(targeting)
    .bind(normalize_datetime(input.starts_at.as_deref())?)
    .bind(normalize_datetime(input.ends_at.as_deref())?)
    .bind(id)
    .execute(&state.pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found("announcement not found"));
    }
    Ok(Json(json!({"data": {"id": id}})))
}

async fn delete_announcement(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    let result = sqlx::query("DELETE FROM announcements WHERE id = ?")
        .bind(id)
        .execute(&state.pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found("announcement not found"));
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn announcement_read_status(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Query(query): Query<ListQuery>,
) -> ApiResult<Json<Value>> {
    let row = sqlx::query_as::<_, AnnouncementRow>(&format!(
        "{ANNOUNCEMENT_SELECT} WHERE announcements.id = ?"
    ))
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| ApiError::not_found("announcement not found"))?;
    let targeting = parse_targeting(&row.targeting)?;
    let search = query.search.as_deref().unwrap_or("").trim().to_string();
    let page = query.page.unwrap_or(1).clamp(1, 1_000_000);
    let page_size = query.page_size.unwrap_or(20).clamp(1, 100);
    let order = match query.sort_by.as_deref() {
        Some("email") => "users.email",
        Some("balance") => "users.balance_cents",
        Some("read_at") => "announcement_reads.read_at",
        _ => "users.username",
    };
    let direction = if query.sort_order.as_deref() == Some("desc") {
        "DESC"
    } else {
        "ASC"
    };
    let user_filter = " WHERE users.deleted_at IS NULL AND (? = '' OR users.username LIKE '%' || ? || '%' \
        OR users.display_name LIKE '%' || ? || '%' OR users.email LIKE '%' || ? || '%')";
    let user_sql = format!(
        "SELECT users.id, users.email, users.username, users.balance_cents, announcement_reads.read_at \
         FROM users LEFT JOIN announcement_reads ON announcement_reads.user_id = users.id \
         AND announcement_reads.announcement_id = ?{user_filter} ORDER BY {order} {direction}, users.id ASC \
         LIMIT ? OFFSET ?"
    );
    let users: Vec<(i64, Option<String>, String, i64, Option<String>)> = sqlx::query_as(&user_sql)
        .bind(id)
        .bind(&search)
        .bind(&search)
        .bind(&search)
        .bind(&search)
        .bind(page_size)
        .bind((page - 1) * page_size)
        .fetch_all(&state.pool)
        .await?;
    let total: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM users{user_filter}"))
        .bind(&search)
        .bind(&search)
        .bind(&search)
        .bind(&search)
        .fetch_one(&state.pool)
        .await?;
    let plan_rows: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT user_id, plan_id FROM subscriptions WHERE status = 'active' \
         AND datetime(starts_at) <= CURRENT_TIMESTAMP AND datetime(ends_at) > CURRENT_TIMESTAMP",
    )
    .fetch_all(&state.pool)
    .await?;
    let mut plans_by_user: HashMap<i64, HashSet<i64>> = HashMap::new();
    for (user_id, plan_id) in plan_rows {
        plans_by_user.entry(user_id).or_default().insert(plan_id);
    }
    let data = users.into_iter().map(|user| json!({
        "user_id": user.0, "email": user.1, "username": user.2,
        "balance_cents": user.3,
        "eligible": targeting.matches(user.3, plans_by_user.get(&user.0).unwrap_or(&HashSet::new())),
        "read_at": user.4
    })).collect::<Vec<_>>();
    Ok(Json(
        json!({"data": data, "meta": {"page": page, "page_size": page_size, "total": total}}),
    ))
}

fn validate_announcement(input: &AnnouncementInput) -> ApiResult<()> {
    if input.title.trim().is_empty()
        || input.title.chars().count() > 160
        || input.content.trim().is_empty()
        || input.content.len() > 200_000
        || !matches!(input.status.as_str(), "draft" | "active" | "archived")
        || !matches!(input.notify_mode.as_str(), "silent" | "popup")
    {
        return Err(ApiError::bad_request(
            "INVALID_ANNOUNCEMENT",
            "announcement fields are invalid",
        ));
    }
    input.targeting.validate()?;
    let start = normalize_datetime(input.starts_at.as_deref())?;
    let end = normalize_datetime(input.ends_at.as_deref())?;
    if let (Some(start), Some(end)) = (start, end)
        && start >= end
    {
        return Err(ApiError::bad_request(
            "INVALID_TIME_RANGE",
            "starts_at must be before ends_at",
        ));
    }
    Ok(())
}

#[derive(Deserialize)]
struct PageInput {
    slug: String,
    title: String,
    content: String,
    #[serde(default = "custom_kind")]
    kind: String,
    #[serde(default)]
    public: bool,
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default)]
    sort_order: i64,
}

fn custom_kind() -> String {
    "custom".into()
}
fn default_true() -> bool {
    true
}

async fn public_pages(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    list_pages(&state, true).await
}

async fn public_page(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<Json<Value>> {
    page_by_slug(&state, &slug, true).await
}

async fn user_pages(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    list_pages(&state, false).await
}

async fn user_page(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<Json<Value>> {
    page_by_slug(&state, &slug, false).await
}

async fn admin_pages(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let rows: Vec<(i64, String, String, String, String, bool, bool, i64, String, String)> =
        sqlx::query_as(
            "SELECT id, slug, title, content, kind, public, enabled, sort_order, created_at, updated_at \
             FROM content_pages ORDER BY sort_order ASC, id ASC",
        )
        .fetch_all(&state.pool)
        .await?;
    Ok(Json(
        json!({"data": rows.into_iter().map(page_value).collect::<Vec<_>>()}),
    ))
}

async fn create_page(
    State(state): State<AppState>,
    Json(input): Json<PageInput>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    validate_page(&input)?;
    let result = sqlx::query(
        "INSERT INTO content_pages (slug, title, content, kind, public, enabled, sort_order) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(input.slug.trim())
    .bind(input.title.trim())
    .bind(input.content.trim())
    .bind(&input.kind)
    .bind(input.public)
    .bind(input.enabled)
    .bind(input.sort_order)
    .execute(&state.pool)
    .await
    .map_err(unique_page_error)?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"data": {"id": result.last_insert_rowid()}})),
    ))
}

async fn update_page(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(input): Json<PageInput>,
) -> ApiResult<Json<Value>> {
    validate_page(&input)?;
    let result = sqlx::query(
        "UPDATE content_pages SET slug = ?, title = ?, content = ?, kind = ?, public = ?, \
         enabled = ?, sort_order = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
    )
    .bind(input.slug.trim())
    .bind(input.title.trim())
    .bind(input.content.trim())
    .bind(&input.kind)
    .bind(input.public)
    .bind(input.enabled)
    .bind(input.sort_order)
    .bind(id)
    .execute(&state.pool)
    .await
    .map_err(unique_page_error)?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found("content page not found"));
    }
    Ok(Json(json!({"data": {"id": id}})))
}

async fn delete_page(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    let result = sqlx::query("DELETE FROM content_pages WHERE id = ?")
        .bind(id)
        .execute(&state.pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found("content page not found"));
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn list_pages(state: &AppState, public_only: bool) -> ApiResult<Json<Value>> {
    let rows: Vec<(i64, String, String, String, String, bool, bool, i64, String, String)> =
        sqlx::query_as(
            "SELECT id, slug, title, content, kind, public, enabled, sort_order, created_at, updated_at \
             FROM content_pages WHERE enabled = 1 AND (? = 0 OR public = 1) \
             ORDER BY sort_order ASC, id ASC",
        )
        .bind(public_only)
        .fetch_all(&state.pool)
        .await?;
    Ok(Json(
        json!({"data": rows.into_iter().map(page_value).collect::<Vec<_>>()}),
    ))
}

async fn page_by_slug(state: &AppState, slug: &str, public_only: bool) -> ApiResult<Json<Value>> {
    let row: Option<(i64, String, String, String, String, bool, bool, i64, String, String)> =
        sqlx::query_as(
            "SELECT id, slug, title, content, kind, public, enabled, sort_order, created_at, updated_at \
             FROM content_pages WHERE slug = ? COLLATE NOCASE AND enabled = 1 \
             AND (? = 0 OR public = 1)",
        )
        .bind(slug)
        .bind(public_only)
        .fetch_optional(&state.pool)
        .await?;
    let mut value = page_value(row.ok_or_else(|| ApiError::not_found("content page not found"))?);
    let kind = value["kind"].as_str().unwrap_or_default().to_string();
    let content = value["content"].as_str().unwrap_or_default().to_string();
    if kind == "custom"
        && let Some(url) = safe_iframe_url(&content)
    {
        value["render_mode"] = json!("iframe");
        value["iframe_url"] = json!(url);
        value["rendered_html"] = json!("");
    } else {
        value["render_mode"] = json!("markdown");
        value["iframe_url"] = Value::Null;
        value["rendered_html"] = json!(render_markdown(&content));
    }
    Ok(Json(json!({"data": value})))
}

fn page_value(
    row: (
        i64,
        String,
        String,
        String,
        String,
        bool,
        bool,
        i64,
        String,
        String,
    ),
) -> Value {
    let iframe_url = if row.4 == "custom" {
        safe_iframe_url(&row.3)
    } else {
        None
    };
    let render_mode = if iframe_url.is_some() {
        "iframe"
    } else {
        "markdown"
    };
    json!({
        "id": row.0, "slug": row.1, "title": row.2, "content": row.3, "kind": row.4,
        "public": row.5, "enabled": row.6, "sort_order": row.7,
        "render_mode": render_mode, "iframe_url": iframe_url,
        "created_at": row.8, "updated_at": row.9
    })
}

fn validate_page(input: &PageInput) -> ApiResult<()> {
    let valid_slug = !input.slug.is_empty()
        && input.slug.len() <= 80
        && input
            .slug
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    if !valid_slug
        || input.title.trim().is_empty()
        || input.title.chars().count() > 160
        || input.content.len() > 500_000
        || !matches!(input.kind.as_str(), "legal" | "custom")
        || !(-10_000..=10_000).contains(&input.sort_order)
    {
        return Err(ApiError::bad_request(
            "INVALID_PAGE",
            "content page fields are invalid",
        ));
    }
    Ok(())
}

fn normalize_datetime(value: Option<&str>) -> ApiResult<Option<String>> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    DateTime::parse_from_rfc3339(value)
        .map(|value| Some(value.to_rfc3339()))
        .map_err(|_| ApiError::bad_request("INVALID_DATETIME", "date must use RFC 3339"))
}

fn unique_page_error(error: sqlx::Error) -> ApiError {
    match error {
        sqlx::Error::Database(ref database) if database.is_unique_violation() => {
            ApiError::bad_request("PAGE_SLUG_EXISTS", "page slug already exists")
        }
        other => other.into(),
    }
}

pub(crate) fn render_markdown(markdown: &str) -> String {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES;
    let parser = Parser::new_ext(markdown, options).map(sanitize_markdown_event);
    let mut output = String::with_capacity(markdown.len().saturating_add(markdown.len() / 4));
    html::push_html(&mut output, parser);
    output
}

fn sanitize_markdown_event(event: Event<'_>) -> Event<'_> {
    match event {
        Event::Html(value) | Event::InlineHtml(value) => Event::Text(value),
        Event::Start(Tag::Link {
            link_type,
            dest_url,
            title,
            id,
        }) => Event::Start(Tag::Link {
            link_type,
            dest_url: safe_markdown_url(&dest_url, false),
            title,
            id,
        }),
        Event::Start(Tag::Image {
            link_type,
            dest_url,
            title,
            id,
        }) => Event::Start(Tag::Image {
            link_type,
            dest_url: safe_markdown_url(&dest_url, true),
            title,
            id,
        }),
        other => other,
    }
}

fn safe_markdown_url<'a>(value: &CowStr<'a>, image: bool) -> CowStr<'a> {
    let raw = value.trim();
    let relative = raw.starts_with('#')
        || raw.starts_with('/')
        || raw.starts_with("./")
        || raw.starts_with("../")
        || (!raw.is_empty() && !raw.contains(':'));
    let absolute = url::Url::parse(raw).ok().is_some_and(|url| {
        matches!(url.scheme(), "http" | "https") || (!image && url.scheme() == "mailto")
    });
    if relative || absolute {
        value.clone()
    } else {
        CowStr::Borrowed("")
    }
}

pub(crate) fn safe_iframe_url(value: &str) -> Option<String> {
    let value = value.trim();
    if value.len() > 2048 {
        return None;
    }
    let parsed = url::Url::parse(value).ok()?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return None;
    }
    Some(parsed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use tower::ServiceExt;

    use crate::test_support;

    #[test]
    fn markdown_renderer_escapes_html_and_rejects_active_urls() {
        let rendered = render_markdown(
            "# Terms\n\n<script>alert(1)</script>\n\n[bad](javascript:alert(1))\n\n| A | B |\n| - | - |\n| 1 | 2 |",
        );
        assert!(rendered.contains("<h1>Terms</h1>"));
        assert!(rendered.contains("&lt;script&gt;"));
        assert!(!rendered.contains("<script>"));
        assert!(!rendered.contains("javascript:"));
        assert!(rendered.contains("<table>"));
    }

    #[test]
    fn iframe_urls_are_bounded_http_urls_without_credentials() {
        assert_eq!(
            safe_iframe_url("https://docs.example.com/guide?q=1").as_deref(),
            Some("https://docs.example.com/guide?q=1")
        );
        assert!(safe_iframe_url("javascript:alert(1)").is_none());
        assert!(safe_iframe_url("https://user:secret@example.com").is_none());
        assert!(safe_iframe_url("/relative").is_none());
    }

    #[test]
    fn announcement_targeting_uses_or_groups_and_and_conditions() {
        let targeting = AnnouncementTargeting {
            any_of: vec![
                AnnouncementConditionGroup {
                    all_of: vec![
                        AnnouncementCondition {
                            kind: "balance".into(),
                            operator: "gte".into(),
                            group_ids: vec![],
                            value: 10_000,
                        },
                        AnnouncementCondition {
                            kind: "subscription".into(),
                            operator: "in".into(),
                            group_ids: vec![7],
                            value: 0,
                        },
                    ],
                },
                AnnouncementConditionGroup {
                    all_of: vec![AnnouncementCondition {
                        kind: "balance".into(),
                        operator: "lt".into(),
                        group_ids: vec![],
                        value: 500,
                    }],
                },
            ],
        };
        targeting.validate().unwrap();
        assert!(targeting.matches(100, &HashSet::new()));
        assert!(!targeting.matches(10_000, &HashSet::new()));
        assert!(targeting.matches(10_000, &HashSet::from([7])));
    }

    #[tokio::test]
    async fn targeted_announcements_enforce_visibility_and_report_unread_users() {
        let (_directory, state) = test_support::state().await;
        let admin_id: i64 = sqlx::query_scalar("SELECT id FROM users WHERE role = 'admin'")
            .fetch_one(&state.pool)
            .await
            .unwrap();
        sqlx::query("UPDATE users SET balance_cents = 700 WHERE id = ?")
            .bind(admin_id)
            .execute(&state.pool)
            .await
            .unwrap();
        let other_id = sqlx::query(
            "INSERT INTO users (username, display_name, password_hash, balance_cents) \
             VALUES ('announcement-other', 'other', 'hash', 100)",
        )
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        let (_, created) = create_announcement(
            State(state.clone()),
            Json(AnnouncementInput {
                title: "Targeted".into(),
                content: "Important update".into(),
                status: "active".into(),
                notify_mode: "popup".into(),
                targeting: AnnouncementTargeting {
                    any_of: vec![AnnouncementConditionGroup {
                        all_of: vec![AnnouncementCondition {
                            kind: "balance".into(),
                            operator: "gte".into(),
                            group_ids: vec![],
                            value: 500,
                        }],
                    }],
                },
                starts_at: None,
                ends_at: None,
            }),
        )
        .await
        .unwrap();
        let announcement_id = created.0["data"]["id"].as_i64().unwrap();
        let session = |user_id| AuthSession {
            id: 1,
            user_id,
            username: "test".into(),
            display_name: "test".into(),
            role: "user".into(),
        };
        let visible = user_announcements(
            State(state.clone()),
            Extension(session(admin_id)),
            Query(ListQuery::default()),
        )
        .await
        .unwrap();
        assert_eq!(visible.0["data"].as_array().unwrap().len(), 1);
        let hidden = user_announcements(
            State(state.clone()),
            Extension(session(other_id)),
            Query(ListQuery::default()),
        )
        .await
        .unwrap();
        assert!(hidden.0["data"].as_array().unwrap().is_empty());
        let denied = mark_announcement_read(
            State(state.clone()),
            Extension(session(other_id)),
            Path(announcement_id),
        )
        .await
        .unwrap_err();
        assert_eq!(denied.status, StatusCode::NOT_FOUND);
        mark_announcement_read(
            State(state.clone()),
            Extension(session(admin_id)),
            Path(announcement_id),
        )
        .await
        .unwrap();
        let unread = user_announcements(
            State(state.clone()),
            Extension(session(admin_id)),
            Query(ListQuery {
                unread_only: Some(true),
                ..Default::default()
            }),
        )
        .await
        .unwrap();
        assert!(unread.0["data"].as_array().unwrap().is_empty());
        let read_status = announcement_read_status(
            State(state.clone()),
            Path(announcement_id),
            Query(ListQuery::default()),
        )
        .await
        .unwrap();
        let rows = read_status.0["data"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows.iter().filter(|row| row["eligible"] == true).count(), 1);
        assert_eq!(
            rows.iter().filter(|row| !row["read_at"].is_null()).count(),
            1
        );
    }

    #[tokio::test]
    async fn publishes_announcements_and_controls_public_page_visibility() {
        let (_directory, state) = test_support::state().await;
        let app = Router::new()
            .nest("/api/admin", admin_router())
            .nest("/api/public", public_router())
            .with_state(state.clone());

        let announcement = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/admin/announcements")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"title":"Service notice","content":"Available now","status":"active","notify_mode":"silent"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(announcement.status(), StatusCode::CREATED);
        let public_list = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/public/announcements")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(public_list.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["data"][0]["title"], "Service notice");

        let page = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/admin/pages")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"slug":"terms","title":"Terms","content":"Terms body","kind":"legal","public":true,"enabled":true}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(page.status(), StatusCode::CREATED);
        let public_page = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/public/pages/terms")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(public_page.status(), StatusCode::OK);
        let body = to_bytes(public_page.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["data"]["kind"], "legal");
        assert_eq!(value["data"]["render_mode"], "markdown");
        assert_eq!(value["data"]["rendered_html"], "<p>Terms body</p>\n");

        let iframe = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/admin/pages")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"slug":"status-board","title":"Status","content":"https://status.example.com/board","kind":"custom","public":true,"enabled":true}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(iframe.status(), StatusCode::CREATED);
        let iframe_page = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/public/pages/status-board")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(iframe_page.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["data"]["render_mode"], "iframe");
        assert_eq!(
            value["data"]["iframe_url"],
            "https://status.example.com/board"
        );

        let hidden = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/admin/pages/1")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"slug":"terms","title":"Terms","content":"Terms body","kind":"legal","public":false,"enabled":true}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(hidden.status(), StatusCode::OK);
        let no_longer_public = app
            .oneshot(
                Request::builder()
                    .uri("/api/public/pages/terms")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(no_longer_public.status(), StatusCode::NOT_FOUND);
    }
}
