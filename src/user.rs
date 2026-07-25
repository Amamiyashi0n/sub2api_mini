use axum::{
    Json, Router,
    extract::{Extension, Path, State},
    http::StatusCode,
    middleware,
    routing::{get, post, put},
};
use chrono::{Duration, Utc};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    auth::{self, AuthSession},
    crypto::{hash_password, token_hash, verify_password},
    error::{ApiError, ApiResult},
    gateway, key_policy,
    state::AppState,
};

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .merge(crate::dashboard::user_router())
        .merge(crate::batch_images::user_router())
        .route("/profile", get(profile).put(update_profile))
        .route("/profile/email/request", post(request_email_change))
        .route("/profile/email/confirm", post(confirm_email_change))
        .route("/profile/email", axum::routing::delete(remove_email))
        .route("/password", put(change_password))
        .route("/models", get(available_models))
        .route("/keys", get(list_keys).post(create_key))
        .route("/keys/batch", post(batch_key_action))
        .route("/keys/{id}", put(update_key).delete(delete_key))
        .merge(crate::usage::user_router())
        .merge(crate::groups::user_router())
        .merge(crate::subscriptions::user_router())
        .merge(crate::redeem::user_router())
        .merge(crate::orders::user_router())
        .merge(crate::content::user_router())
        .merge(crate::channel_monitor::user_router())
        .merge(crate::channels::user_router())
        .merge(crate::totp::user_router())
        .route_layer(middleware::from_fn_with_state(state, auth::user_guard))
}

async fn profile(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
) -> ApiResult<Json<Value>> {
    profile_response(&state, session.user_id).await
}

async fn profile_response(state: &AppState, user_id: i64) -> ApiResult<Json<Value>> {
    let user: Option<(
        i64,
        String,
        String,
        Option<String>,
        bool,
        i64,
        String,
        bool,
        String,
        String,
    )> = sqlx::query_as(
        "SELECT id, username, display_name, email, email_verified, balance_cents, role, enabled, created_at, updated_at \
         FROM users WHERE id = ?",
    )
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await?;
    let user = user.ok_or_else(|| ApiError::not_found("user not found"))?;
    let stats: (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM api_keys WHERE user_id = ?), \
         (SELECT COUNT(*) FROM usage_logs WHERE user_id = ?), \
         (SELECT COALESCE(SUM(total_tokens), 0) FROM usage_logs WHERE user_id = ?)",
    )
    .bind(user_id)
    .bind(user_id)
    .bind(user_id)
    .fetch_one(&state.pool)
    .await?;
    let pending_email: Option<(String, String)> = sqlx::query_as(
        "SELECT email, expires_at FROM profile_email_changes \
         WHERE user_id = ? AND consumed_at IS NULL \
         AND datetime(expires_at) > CURRENT_TIMESTAMP ORDER BY id DESC LIMIT 1",
    )
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await?;
    Ok(Json(json!({"data": {
        "id": user.0,
        "username": user.1,
        "display_name": user.2,
        "email": user.3,
        "email_verified": user.4,
        "balance_cents": user.5,
        "role": user.6,
        "enabled": user.7,
        "created_at": user.8,
        "updated_at": user.9,
        "key_count": stats.0,
        "total_requests": stats.1,
        "total_tokens": stats.2,
        "pending_email": pending_email.as_ref().map(|row| &row.0),
        "pending_email_expires_at": pending_email.as_ref().map(|row| &row.1)
    }})))
}

#[derive(Deserialize)]
struct UpdateProfileInput {
    display_name: String,
}

async fn update_profile(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
    Json(input): Json<UpdateProfileInput>,
) -> ApiResult<Json<Value>> {
    let display_name = input.display_name.trim();
    if display_name.is_empty() || display_name.chars().count() > 80 {
        return Err(ApiError::bad_request(
            "INVALID_DISPLAY_NAME",
            "display_name must be 1-80 characters",
        ));
    }
    sqlx::query("UPDATE users SET display_name = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?")
        .bind(display_name)
        .bind(session.user_id)
        .execute(&state.pool)
        .await?;
    profile_response(&state, session.user_id).await
}

#[derive(Deserialize)]
struct RequestEmailChangeInput {
    email: String,
    current_password: String,
}

async fn request_email_change(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
    Json(input): Json<RequestEmailChangeInput>,
) -> ApiResult<Json<Value>> {
    verify_current_password(&state, session.user_id, &input.current_password).await?;
    if !crate::mail::is_configured(&state).await? {
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "MAIL_NOT_CONFIGURED",
            "mail delivery is not configured",
        ));
    }
    let email = auth::normalize_email(&input.email)?;
    let current_email: Option<String> =
        sqlx::query_scalar("SELECT email FROM users WHERE id = ? AND enabled = 1")
            .bind(session.user_id)
            .fetch_optional(&state.pool)
            .await?
            .flatten();
    if current_email
        .as_deref()
        .is_some_and(|current| current.eq_ignore_ascii_case(&email))
    {
        return Err(ApiError::bad_request(
            "EMAIL_UNCHANGED",
            "new email must be different from the current email",
        ));
    }
    ensure_email_available(&state, session.user_id, &email).await?;
    let recent: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM profile_email_changes \
         WHERE (user_id = ? OR email = ? COLLATE NOCASE) \
         AND datetime(created_at) > datetime('now', '-60 seconds')",
    )
    .bind(session.user_id)
    .bind(&email)
    .fetch_one(&state.pool)
    .await?;
    if recent > 0 {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "VERIFICATION_RATE_LIMITED",
            "please wait before requesting another message",
        ));
    }

    let code = auth::verification_code()?;
    let code_hash = token_hash(&code);
    let expires_at = (Utc::now() + Duration::minutes(10)).to_rfc3339();
    let mut transaction = state.pool.begin().await?;
    sqlx::query("DELETE FROM profile_email_changes WHERE user_id = ? AND consumed_at IS NULL")
        .bind(session.user_id)
        .execute(&mut *transaction)
        .await?;
    let change_id = sqlx::query(
        "INSERT INTO profile_email_changes (user_id, email, code_hash, expires_at) \
         VALUES (?, ?, ?, ?)",
    )
    .bind(session.user_id)
    .bind(&email)
    .bind(&code_hash)
    .bind(&expires_at)
    .execute(&mut *transaction)
    .await?
    .last_insert_rowid();
    transaction.commit().await?;

    if let Err(error) = auth::deliver_mail(
        &state,
        "profile_email_verification",
        &email,
        Some(&code),
        None,
    )
    .await
    {
        sqlx::query("DELETE FROM profile_email_changes WHERE id = ? AND user_id = ?")
            .bind(change_id)
            .bind(session.user_id)
            .execute(&state.pool)
            .await?;
        return Err(error);
    }

    Ok(Json(json!({"data": {
        "message": "verification code sent",
        "email": email,
        "expires_at": expires_at,
        "countdown": 60
    }})))
}

#[derive(Deserialize)]
struct ConfirmEmailChangeInput {
    email: String,
    code: String,
}

async fn confirm_email_change(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
    Json(input): Json<ConfirmEmailChangeInput>,
) -> ApiResult<Json<Value>> {
    let email = auth::normalize_email(&input.email)?;
    let code = input.code.trim();
    if code.is_empty() {
        return Err(ApiError::bad_request(
            "INVALID_VERIFICATION_CODE",
            "verification code is invalid or expired",
        ));
    }
    ensure_email_available(&state, session.user_id, &email).await?;
    let mut transaction = state.pool.begin().await?;
    let consumed = sqlx::query(
        "UPDATE profile_email_changes SET consumed_at = CURRENT_TIMESTAMP \
         WHERE user_id = ? AND email = ? COLLATE NOCASE AND code_hash = ? \
         AND consumed_at IS NULL AND datetime(expires_at) > CURRENT_TIMESTAMP",
    )
    .bind(session.user_id)
    .bind(&email)
    .bind(token_hash(code))
    .execute(&mut *transaction)
    .await?;
    if consumed.rows_affected() != 1 {
        return Err(ApiError::bad_request(
            "INVALID_VERIFICATION_CODE",
            "verification code is invalid or expired",
        ));
    }
    sqlx::query(
        "UPDATE users SET email = ?, email_verified = 1, updated_at = CURRENT_TIMESTAMP \
         WHERE id = ? AND enabled = 1",
    )
    .bind(&email)
    .bind(session.user_id)
    .execute(&mut *transaction)
    .await
    .map_err(map_email_error)?;
    sqlx::query("DELETE FROM auth_sessions WHERE user_id = ? AND id != ?")
        .bind(session.user_id)
        .bind(session.id)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    profile_response(&state, session.user_id).await
}

#[derive(Deserialize)]
struct RemoveEmailInput {
    current_password: String,
}

async fn remove_email(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
    Json(input): Json<RemoveEmailInput>,
) -> ApiResult<Json<Value>> {
    verify_current_password(&state, session.user_id, &input.current_password).await?;
    let mut transaction = state.pool.begin().await?;
    sqlx::query(
        "UPDATE users SET email = NULL, email_verified = 0, updated_at = CURRENT_TIMESTAMP \
         WHERE id = ? AND enabled = 1",
    )
    .bind(session.user_id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query("DELETE FROM profile_email_changes WHERE user_id = ?")
        .bind(session.user_id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query("DELETE FROM auth_sessions WHERE user_id = ? AND id != ?")
        .bind(session.user_id)
        .bind(session.id)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    profile_response(&state, session.user_id).await
}

async fn verify_current_password(state: &AppState, user_id: i64, password: &str) -> ApiResult<()> {
    let password_hash: Option<String> =
        sqlx::query_scalar("SELECT password_hash FROM users WHERE id = ? AND enabled = 1")
            .bind(user_id)
            .fetch_optional(&state.pool)
            .await?;
    let password_hash = password_hash.ok_or_else(|| ApiError::not_found("user not found"))?;
    if !verify_password(password, &password_hash) {
        return Err(ApiError::bad_request(
            "CURRENT_PASSWORD_INVALID",
            "current password is invalid",
        ));
    }
    Ok(())
}

async fn ensure_email_available(state: &AppState, user_id: i64, email: &str) -> ApiResult<()> {
    let owner: Option<i64> =
        sqlx::query_scalar("SELECT id FROM users WHERE email = ? COLLATE NOCASE")
            .bind(email)
            .fetch_optional(&state.pool)
            .await?;
    if owner.is_some_and(|owner_id| owner_id != user_id) {
        return Err(ApiError::bad_request(
            "EMAIL_EXISTS",
            "email is already registered",
        ));
    }
    Ok(())
}

fn map_email_error(error: sqlx::Error) -> ApiError {
    match error {
        sqlx::Error::Database(ref database) if database.is_unique_violation() => {
            ApiError::bad_request("EMAIL_EXISTS", "email is already registered")
        }
        other => other.into(),
    }
}

#[derive(Deserialize)]
struct ChangePasswordInput {
    current_password: String,
    new_password: String,
}

async fn change_password(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
    Json(input): Json<ChangePasswordInput>,
) -> ApiResult<Json<Value>> {
    validate_password(&input.new_password)?;
    verify_current_password(&state, session.user_id, &input.current_password).await?;
    sqlx::query("UPDATE users SET password_hash = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?")
        .bind(hash_password(&input.new_password)?)
        .bind(session.user_id)
        .execute(&state.pool)
        .await?;
    sqlx::query("DELETE FROM auth_sessions WHERE user_id = ? AND id != ?")
        .bind(session.user_id)
        .bind(session.id)
        .execute(&state.pool)
        .await?;
    Ok(Json(json!({"data": {"message": "password updated"}})))
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

async fn list_keys(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
) -> ApiResult<Json<Value>> {
    Ok(Json(json!({
        "data": key_policy::list_keys(&state.pool, Some(session.user_id)).await?
    })))
}

async fn available_models(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    Ok(Json(gateway::available_model_catalog(&state).await?))
}

#[derive(Deserialize)]
struct CreateKeyInput {
    name: String,
    custom_key: Option<String>,
    expires_in_days: Option<i64>,
    quota_tokens: Option<i64>,
    quota_cost_microusd: Option<i64>,
    #[serde(default)]
    allowed_models: Vec<String>,
    group_id: Option<i64>,
    #[serde(default)]
    ip_whitelist: Vec<String>,
    #[serde(default)]
    ip_blacklist: Vec<String>,
    rate_limit_5h_microusd: Option<i64>,
    rate_limit_1d_microusd: Option<i64>,
    rate_limit_7d_microusd: Option<i64>,
}

async fn create_key(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
    Json(input): Json<CreateKeyInput>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    if input.name.trim().is_empty() {
        return Err(ApiError::bad_request(
            "KEY_NAME_REQUIRED",
            "name is required",
        ));
    }
    let expires_at = expires_from_days(input.expires_in_days)?;
    let quota_tokens = validate_quota(input.quota_tokens.unwrap_or(0))?;
    let quota_cost_microusd = key_policy::validate_microusd(
        input.quota_cost_microusd.unwrap_or(0),
        "quota_cost_microusd",
    )?;
    let allowed_models = normalize_allowed_models(input.allowed_models)?;
    let group_id = validate_group_id(&state, input.group_id).await?;
    if let Some(group_id) = group_id {
        crate::groups::ensure_user_group_access(&state, session.user_id, group_id).await?;
    }
    let ip_whitelist = key_policy::normalize_networks(input.ip_whitelist, "ip_whitelist")?;
    let ip_blacklist = key_policy::normalize_networks(input.ip_blacklist, "ip_blacklist")?;
    let rate_limit_5h_microusd = key_policy::validate_microusd(
        input.rate_limit_5h_microusd.unwrap_or(0),
        "rate_limit_5h_microusd",
    )?;
    let rate_limit_1d_microusd = key_policy::validate_microusd(
        input.rate_limit_1d_microusd.unwrap_or(0),
        "rate_limit_1d_microusd",
    )?;
    let rate_limit_7d_microusd = key_policy::validate_microusd(
        input.rate_limit_7d_microusd.unwrap_or(0),
        "rate_limit_7d_microusd",
    )?;
    let token = key_policy::issue_token(&state.pool, input.custom_key).await?;
    let prefix: String = token.chars().take(18).collect();
    let result = sqlx::query(
        "INSERT INTO api_keys \
         (name, token_prefix, token_hash, user_id, expires_at, quota_tokens, \
          quota_cost_microusd, allowed_models, group_id, ip_whitelist, ip_blacklist, \
          rate_limit_5h_microusd, rate_limit_1d_microusd, rate_limit_7d_microusd) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(input.name.trim())
    .bind(&prefix)
    .bind(token_hash(&token))
    .bind(session.user_id)
    .bind(&expires_at)
    .bind(quota_tokens)
    .bind(quota_cost_microusd)
    .bind(serde_json::to_string(&allowed_models).unwrap())
    .bind(group_id)
    .bind(serde_json::to_string(&ip_whitelist).unwrap())
    .bind(serde_json::to_string(&ip_blacklist).unwrap())
    .bind(rate_limit_5h_microusd)
    .bind(rate_limit_1d_microusd)
    .bind(rate_limit_7d_microusd)
    .execute(&state.pool)
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"data": {
            "id": result.last_insert_rowid(), "name": input.name.trim(),
            "token": token, "token_prefix": prefix, "expires_at": expires_at,
            "quota_tokens": quota_tokens, "quota_cost_microusd": quota_cost_microusd,
            "allowed_models": allowed_models, "group_id": group_id,
            "ip_whitelist": ip_whitelist, "ip_blacklist": ip_blacklist,
            "rate_limit_5h_microusd": rate_limit_5h_microusd,
            "rate_limit_1d_microusd": rate_limit_1d_microusd,
            "rate_limit_7d_microusd": rate_limit_7d_microusd
        }})),
    ))
}

#[derive(Deserialize)]
struct UpdateKeyInput {
    enabled: Option<bool>,
    name: Option<String>,
    expires_at: Option<String>,
    quota_tokens: Option<i64>,
    quota_cost_microusd: Option<i64>,
    allowed_models: Option<Vec<String>>,
    group_id: Option<i64>,
    ip_whitelist: Option<Vec<String>>,
    ip_blacklist: Option<Vec<String>>,
    rate_limit_5h_microusd: Option<i64>,
    rate_limit_1d_microusd: Option<i64>,
    rate_limit_7d_microusd: Option<i64>,
    #[serde(default)]
    reset_quota: bool,
    #[serde(default)]
    reset_rate_limit_usage: bool,
}

async fn update_key(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
    Path(id): Path<i64>,
    Json(input): Json<UpdateKeyInput>,
) -> ApiResult<Json<Value>> {
    let exists: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM api_keys WHERE id = ? AND user_id = ?")
            .bind(id)
            .bind(session.user_id)
            .fetch_one(&state.pool)
            .await?;
    if exists == 0 {
        return Err(ApiError::not_found("API key not found"));
    }
    let quota_cost_microusd = input
        .quota_cost_microusd
        .map(|value| key_policy::validate_microusd(value, "quota_cost_microusd"))
        .transpose()?;
    let rate_limit_5h_microusd = input
        .rate_limit_5h_microusd
        .map(|value| key_policy::validate_microusd(value, "rate_limit_5h_microusd"))
        .transpose()?;
    let rate_limit_1d_microusd = input
        .rate_limit_1d_microusd
        .map(|value| key_policy::validate_microusd(value, "rate_limit_1d_microusd"))
        .transpose()?;
    let rate_limit_7d_microusd = input
        .rate_limit_7d_microusd
        .map(|value| key_policy::validate_microusd(value, "rate_limit_7d_microusd"))
        .transpose()?;
    let ip_whitelist = input
        .ip_whitelist
        .map(|values| key_policy::normalize_networks(values, "ip_whitelist"))
        .transpose()?;
    let ip_blacklist = input
        .ip_blacklist
        .map(|values| key_policy::normalize_networks(values, "ip_blacklist"))
        .transpose()?;
    if let Some(enabled) = input.enabled {
        sqlx::query("UPDATE api_keys SET enabled = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?")
            .bind(enabled)
            .bind(id)
            .execute(&state.pool)
            .await?;
    }
    if let Some(name) = input.name {
        if name.trim().is_empty() || name.chars().count() > 80 {
            return Err(ApiError::bad_request(
                "INVALID_KEY_NAME",
                "name must be 1-80 characters",
            ));
        }
        sqlx::query("UPDATE api_keys SET name = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?")
            .bind(name.trim())
            .bind(id)
            .execute(&state.pool)
            .await?;
    }
    if let Some(expires_at) = input.expires_at {
        let expires_at = validate_expiry(&expires_at)?;
        sqlx::query(
            "UPDATE api_keys SET expires_at = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
        )
        .bind(expires_at)
        .bind(id)
        .execute(&state.pool)
        .await?;
    }
    if let Some(quota_tokens) = input.quota_tokens {
        sqlx::query(
            "UPDATE api_keys SET quota_tokens = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
        )
        .bind(validate_quota(quota_tokens)?)
        .bind(id)
        .execute(&state.pool)
        .await?;
    }
    if let Some(value) = quota_cost_microusd {
        sqlx::query(
            "UPDATE api_keys SET quota_cost_microusd = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
        )
        .bind(value)
        .bind(id)
        .execute(&state.pool)
        .await?;
    }
    if let Some(allowed_models) = input.allowed_models {
        let allowed_models = normalize_allowed_models(allowed_models)?;
        sqlx::query(
            "UPDATE api_keys SET allowed_models = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
        )
        .bind(serde_json::to_string(&allowed_models).unwrap())
        .bind(id)
        .execute(&state.pool)
        .await?;
    }
    if let Some(group_id) = input.group_id {
        let group_id = validate_group_id(&state, Some(group_id)).await?;
        if let Some(group_id) = group_id {
            crate::groups::ensure_user_group_access(&state, session.user_id, group_id).await?;
        }
        sqlx::query(
            "UPDATE api_keys SET group_id = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
        )
        .bind(group_id)
        .bind(id)
        .execute(&state.pool)
        .await?;
    }
    if let Some(values) = ip_whitelist {
        sqlx::query(
            "UPDATE api_keys SET ip_whitelist = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
        )
        .bind(serde_json::to_string(&values).unwrap())
        .bind(id)
        .execute(&state.pool)
        .await?;
    }
    if let Some(values) = ip_blacklist {
        sqlx::query(
            "UPDATE api_keys SET ip_blacklist = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
        )
        .bind(serde_json::to_string(&values).unwrap())
        .bind(id)
        .execute(&state.pool)
        .await?;
    }
    for (column, value) in [
        ("rate_limit_5h_microusd", rate_limit_5h_microusd),
        ("rate_limit_1d_microusd", rate_limit_1d_microusd),
        ("rate_limit_7d_microusd", rate_limit_7d_microusd),
    ] {
        if let Some(value) = value {
            let query = format!(
                "UPDATE api_keys SET {column} = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?"
            );
            sqlx::query(&query)
                .bind(value)
                .bind(id)
                .execute(&state.pool)
                .await?;
        }
    }
    if input.reset_quota {
        sqlx::query(
            "UPDATE api_keys SET quota_reset_at = CURRENT_TIMESTAMP, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
        )
        .bind(id)
        .execute(&state.pool)
        .await?;
    }
    if input.reset_rate_limit_usage {
        sqlx::query(
            "UPDATE api_keys SET rate_usage_reset_at = CURRENT_TIMESTAMP, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
        )
        .bind(id)
        .execute(&state.pool)
        .await?;
    }
    Ok(Json(json!({"data": {"id": id}})))
}

#[derive(Deserialize)]
struct BatchKeyInput {
    ids: Vec<i64>,
    action: String,
}

async fn batch_key_action(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
    Json(input): Json<BatchKeyInput>,
) -> ApiResult<Json<Value>> {
    Ok(Json(json!({
        "data": key_policy::batch_action(
            &state.pool,
            Some(session.user_id),
            input.ids,
            &input.action,
        )
        .await?
    })))
}

pub(crate) fn expires_from_days(days: Option<i64>) -> ApiResult<Option<String>> {
    match days {
        None | Some(0) => Ok(None),
        Some(days) if (1..=3650).contains(&days) => {
            Ok(Some((Utc::now() + Duration::days(days)).to_rfc3339()))
        }
        _ => Err(ApiError::bad_request(
            "INVALID_EXPIRY",
            "expires_in_days must be 1-3650 or omitted",
        )),
    }
}

pub(crate) fn validate_expiry(value: &str) -> ApiResult<Option<String>> {
    if value.trim().is_empty() {
        return Ok(None);
    }
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|value| Some(value.to_rfc3339()))
        .map_err(|_| ApiError::bad_request("INVALID_EXPIRY", "expires_at must be RFC 3339"))
}

pub(crate) fn validate_quota(value: i64) -> ApiResult<i64> {
    if !(0..=1_000_000_000_000_000).contains(&value) {
        return Err(ApiError::bad_request(
            "INVALID_QUOTA",
            "quota_tokens must be between 0 and 1000000000000000",
        ));
    }
    Ok(value)
}

pub(crate) fn normalize_allowed_models(models: Vec<String>) -> ApiResult<Vec<String>> {
    let mut models = models
        .into_iter()
        .map(|model| model.trim().to_string())
        .filter(|model| !model.is_empty())
        .collect::<Vec<_>>();
    if models.len() > 100 || models.iter().any(|model| model.chars().count() > 128) {
        return Err(ApiError::bad_request(
            "INVALID_MODELS",
            "allowed_models supports at most 100 model IDs of 128 characters",
        ));
    }
    models.sort();
    models.dedup();
    Ok(models)
}

pub(crate) async fn validate_group_id(
    state: &AppState,
    group_id: Option<i64>,
) -> ApiResult<Option<i64>> {
    let Some(group_id) = group_id.filter(|id| *id > 0) else {
        return Ok(None);
    };
    let exists: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM groups WHERE id = ? AND enabled = 1")
            .bind(group_id)
            .fetch_one(&state.pool)
            .await?;
    if exists == 0 {
        return Err(ApiError::bad_request(
            "GROUP_NOT_FOUND",
            "enabled group was not found",
        ));
    }
    Ok(Some(group_id))
}

async fn delete_key(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    let result = sqlx::query("DELETE FROM api_keys WHERE id = ? AND user_id = ?")
        .bind(id)
        .bind(session.user_id)
        .execute(&state.pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found("API key not found"));
    }
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use chrono::{Duration, Utc};
    use std::sync::Arc;
    use tokio::sync::Mutex;
    use tower::ServiceExt;

    use crate::{crypto::Crypto, crypto::token_hash, state::AppState, test_support};

    async fn authenticated_user(state: &AppState, email: Option<&str>) -> (i64, i64) {
        let user_id = sqlx::query(
            "INSERT INTO users (username, display_name, password_hash, email, email_verified) \
             VALUES ('email-user', 'Email User', ?, ?, ?)",
        )
        .bind(hash_password("current-password").unwrap())
        .bind(email)
        .bind(email.is_some())
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        let expires_at = (Utc::now() + Duration::hours(1)).to_rfc3339();
        let session_id = sqlx::query(
            "INSERT INTO auth_sessions (user_id, token_hash, csrf_hash, expires_at) VALUES (?, ?, ?, ?)",
        )
        .bind(user_id)
        .bind(token_hash("email-session"))
        .bind(token_hash("email-csrf"))
        .bind(&expires_at)
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        sqlx::query(
            "INSERT INTO auth_sessions (user_id, token_hash, csrf_hash, expires_at) VALUES (?, ?, ?, ?)",
        )
        .bind(user_id)
        .bind(token_hash("email-other-session"))
        .bind(token_hash("email-other-csrf"))
        .bind(expires_at)
        .execute(&state.pool)
        .await
        .unwrap();
        (user_id, session_id)
    }

    fn email_request(method: &str, uri: &str, body: &str) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .header("cookie", "mini_session=email-session")
            .header("x-csrf-token", "email-csrf")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    #[tokio::test]
    async fn updates_profile_and_password_through_authenticated_routes() {
        let (_directory, state) = test_support::state().await;
        let user_id = sqlx::query(
            "INSERT INTO users (username, display_name, password_hash) VALUES ('member', 'Member', ?)",
        )
        .bind(hash_password("old-password").unwrap())
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        let expires_at = (Utc::now() + Duration::hours(1)).to_rfc3339();
        let current_id = sqlx::query(
            "INSERT INTO auth_sessions (user_id, token_hash, csrf_hash, expires_at) VALUES (?, ?, ?, ?)",
        )
        .bind(user_id)
        .bind(token_hash("current-session"))
        .bind(token_hash("csrf-token"))
        .bind(&expires_at)
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        sqlx::query(
            "INSERT INTO auth_sessions (user_id, token_hash, csrf_hash, expires_at) VALUES (?, ?, ?, ?)",
        )
        .bind(user_id)
        .bind(token_hash("other-session"))
        .bind(token_hash("other-csrf"))
        .bind(&expires_at)
        .execute(&state.pool)
        .await
        .unwrap();

        let app = Router::new()
            .nest("/api/user", router(state.clone()))
            .with_state(state.clone());
        let profile_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/user/profile")
                    .header("content-type", "application/json")
                    .header("cookie", "mini_session=current-session")
                    .header("x-csrf-token", "csrf-token")
                    .body(Body::from(r#"{"display_name":"New Name"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(profile_response.status(), StatusCode::OK);
        let body = to_bytes(profile_response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let profile: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(profile["data"]["display_name"], "New Name");

        let password_response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/user/password")
                    .header("content-type", "application/json")
                    .header("cookie", "mini_session=current-session")
                    .header("x-csrf-token", "csrf-token")
                    .body(Body::from(
                        r#"{"current_password":"old-password","new_password":"new-password"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(password_response.status(), StatusCode::OK);

        let password_hash: String =
            sqlx::query_scalar("SELECT password_hash FROM users WHERE id = ?")
                .bind(user_id)
                .fetch_one(&state.pool)
                .await
                .unwrap();
        assert!(verify_password("new-password", &password_hash));
        assert!(!verify_password("old-password", &password_hash));
        let sessions: Vec<i64> =
            sqlx::query_scalar("SELECT id FROM auth_sessions WHERE user_id = ?")
                .bind(user_id)
                .fetch_all(&state.pool)
                .await
                .unwrap();
        assert_eq!(sessions, vec![current_id]);
    }

    #[tokio::test]
    async fn email_change_is_password_gated_one_time_and_revokes_other_sessions() {
        let (_directory, base_state) = test_support::state().await;
        let (user_id, current_session_id) =
            authenticated_user(&base_state, Some("old@example.com")).await;
        let captured = Arc::new(Mutex::new(None::<Value>));
        let webhook =
            Router::new()
                .route(
                    "/mail",
                    post(
                        |State(captured): State<Arc<Mutex<Option<Value>>>>,
                         Json(body): Json<Value>| async move {
                            *captured.lock().await = Some(body);
                            StatusCode::NO_CONTENT
                        },
                    ),
                )
                .with_state(captured.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let webhook_task = tokio::spawn(async move { axum::serve(listener, webhook).await });
        let mut config = base_state.config.clone();
        config.mail_webhook_url = Some(format!("http://{address}/mail"));
        let state = AppState::new(base_state.pool.clone(), Crypto::new(&[9; 32]), config).unwrap();
        let app = Router::new()
            .nest("/api/user", router(state.clone()))
            .with_state(state.clone());

        let wrong_password = app
            .clone()
            .oneshot(email_request(
                "POST",
                "/api/user/profile/email/request",
                r#"{"email":"new@example.com","current_password":"wrong-password"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(wrong_password.status(), StatusCode::BAD_REQUEST);

        let requested = app
            .clone()
            .oneshot(email_request(
                "POST",
                "/api/user/profile/email/request",
                r#"{"email":"New@Example.com","current_password":"current-password"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(requested.status(), StatusCode::OK);
        let mail = captured.lock().await.clone().unwrap();
        assert_eq!(mail["kind"], "profile_email_verification");
        assert_eq!(mail["to"], "new@example.com");
        let code = mail["code"].as_str().unwrap().to_string();
        let stored_hash: String =
            sqlx::query_scalar("SELECT code_hash FROM profile_email_changes WHERE user_id = ?")
                .bind(user_id)
                .fetch_one(&state.pool)
                .await
                .unwrap();
        assert_eq!(stored_hash, token_hash(&code));
        assert_ne!(stored_hash, code);

        let invalid = app
            .clone()
            .oneshot(email_request(
                "POST",
                "/api/user/profile/email/confirm",
                r#"{"email":"new@example.com","code":"invalid"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);

        let confirmed = app
            .clone()
            .oneshot(email_request(
                "POST",
                "/api/user/profile/email/confirm",
                &json!({"email": "new@example.com", "code": code}).to_string(),
            ))
            .await
            .unwrap();
        assert_eq!(confirmed.status(), StatusCode::OK);
        let row: (Option<String>, bool) =
            sqlx::query_as("SELECT email, email_verified FROM users WHERE id = ?")
                .bind(user_id)
                .fetch_one(&state.pool)
                .await
                .unwrap();
        assert_eq!(row, (Some("new@example.com".into()), true));
        let sessions: Vec<i64> =
            sqlx::query_scalar("SELECT id FROM auth_sessions WHERE user_id = ?")
                .bind(user_id)
                .fetch_all(&state.pool)
                .await
                .unwrap();
        assert_eq!(sessions, vec![current_session_id]);

        let reused = app
            .oneshot(email_request(
                "POST",
                "/api/user/profile/email/confirm",
                &json!({"email": "new@example.com", "code": code}).to_string(),
            ))
            .await
            .unwrap();
        assert_eq!(reused.status(), StatusCode::BAD_REQUEST);
        webhook_task.abort();
    }

    #[tokio::test]
    async fn removing_email_requires_password_and_keeps_only_current_session() {
        let (_directory, state) = test_support::state().await;
        let (user_id, current_session_id) =
            authenticated_user(&state, Some("remove@example.com")).await;
        sqlx::query(
            "INSERT INTO profile_email_changes (user_id, email, code_hash, expires_at) \
             VALUES (?, 'pending@example.com', 'pending-hash', datetime('now', '+10 minutes'))",
        )
        .bind(user_id)
        .execute(&state.pool)
        .await
        .unwrap();
        let app = Router::new()
            .nest("/api/user", router(state.clone()))
            .with_state(state.clone());

        let rejected = app
            .clone()
            .oneshot(email_request(
                "DELETE",
                "/api/user/profile/email",
                r#"{"current_password":"wrong-password"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);
        let email: Option<String> = sqlx::query_scalar("SELECT email FROM users WHERE id = ?")
            .bind(user_id)
            .fetch_one(&state.pool)
            .await
            .unwrap();
        assert_eq!(email.as_deref(), Some("remove@example.com"));

        let removed = app
            .oneshot(email_request(
                "DELETE",
                "/api/user/profile/email",
                r#"{"current_password":"current-password"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(removed.status(), StatusCode::OK);
        let row: (Option<String>, bool) =
            sqlx::query_as("SELECT email, email_verified FROM users WHERE id = ?")
                .bind(user_id)
                .fetch_one(&state.pool)
                .await
                .unwrap();
        assert_eq!(row, (None, false));
        let sessions: Vec<i64> =
            sqlx::query_scalar("SELECT id FROM auth_sessions WHERE user_id = ?")
                .bind(user_id)
                .fetch_all(&state.pool)
                .await
                .unwrap();
        assert_eq!(sessions, vec![current_session_id]);
        let pending: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM profile_email_changes WHERE user_id = ?")
                .bind(user_id)
                .fetch_one(&state.pool)
                .await
                .unwrap();
        assert_eq!(pending, 0);
    }

    #[tokio::test]
    async fn key_policies_and_usage_filters_are_user_scoped() {
        let (_directory, state) = test_support::state().await;
        let user_id: i64 = sqlx::query_scalar("SELECT id FROM users WHERE role = 'admin'")
            .fetch_one(&state.pool)
            .await
            .unwrap();
        let other_user_id = sqlx::query(
            "INSERT INTO users (username, display_name, password_hash) VALUES ('other', 'Other', ?)",
        )
        .bind(hash_password("other-password").unwrap())
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        let own_key_id = sqlx::query(
            "INSERT INTO api_keys (user_id, name, token_prefix, token_hash) VALUES (?, 'own', 'own', 'own-hash')",
        )
        .bind(user_id)
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        let other_key_id = sqlx::query(
            "INSERT INTO api_keys (user_id, name, token_prefix, token_hash) VALUES (?, 'other', 'other', 'other-hash')",
        )
        .bind(other_user_id)
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        sqlx::query(
            "INSERT INTO usage_logs \
             (request_id, api_key_id, user_id, endpoint, model, status_code, total_tokens, duration_ms) \
             VALUES ('own-request', ?, ?, '/v1/responses', 'gpt-keep', 200, 12, 5), \
                    ('other-request', ?, ?, '/v1/responses', 'gpt-keep', 200, 99, 5)",
        )
        .bind(own_key_id)
        .bind(user_id)
        .bind(other_key_id)
        .bind(other_user_id)
        .execute(&state.pool)
        .await
        .unwrap();
        let expires_at = (Utc::now() + Duration::hours(1)).to_rfc3339();
        sqlx::query(
            "INSERT INTO auth_sessions (user_id, token_hash, csrf_hash, expires_at) VALUES (?, ?, ?, ?)",
        )
        .bind(user_id)
        .bind(token_hash("policy-session"))
        .bind(token_hash("policy-csrf"))
        .bind(expires_at)
        .execute(&state.pool)
        .await
        .unwrap();
        let app = Router::new()
            .nest("/api/user", router(state.clone()))
            .with_state(state.clone());

        let create_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/user/keys")
                    .header("content-type", "application/json")
                    .header("cookie", "mini_session=policy-session")
                    .header("x-csrf-token", "policy-csrf")
                    .body(Body::from(
                        r#"{"name":"restricted","expires_in_days":30,"quota_tokens":500,"quota_cost_microusd":2500000,"allowed_models":["gpt-b","gpt-a","gpt-a"],"ip_whitelist":["10.2.3.4/8"],"ip_blacklist":["10.1.0.0/16"],"rate_limit_5h_microusd":100000,"rate_limit_1d_microusd":200000,"rate_limit_7d_microusd":300000}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(create_response.status(), StatusCode::CREATED);
        let body = to_bytes(create_response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let created: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(created["data"]["quota_tokens"], 500);
        assert_eq!(created["data"]["quota_cost_microusd"], 2_500_000);
        assert_eq!(created["data"]["allowed_models"], json!(["gpt-a", "gpt-b"]));
        assert_eq!(created["data"]["ip_whitelist"], json!(["10.0.0.0/8"]));

        let list_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/user/keys")
                    .header("cookie", "mini_session=policy-session")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(list_response.status(), StatusCode::OK);
        let body = to_bytes(list_response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let listed: Value = serde_json::from_slice(&body).unwrap();
        let restricted = listed["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|key| key["name"] == "restricted")
            .unwrap();
        assert_eq!(restricted["rate_limit_7d_microusd"], 300_000);
        assert_eq!(restricted["ip_blacklist"], json!(["10.1.0.0/16"]));

        let batch_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/user/keys/batch")
                    .header("content-type", "application/json")
                    .header("cookie", "mini_session=policy-session")
                    .header("x-csrf-token", "policy-csrf")
                    .body(Body::from(format!(
                        r#"{{"ids":[{own_key_id},{other_key_id}],"action":"disable"}}"#
                    )))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(batch_response.status(), StatusCode::OK);
        let body = to_bytes(batch_response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let batch: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(batch["data"]["affected"], 1);
        let key_states: Vec<(i64, bool)> =
            sqlx::query_as("SELECT id, enabled FROM api_keys WHERE id IN (?, ?) ORDER BY id")
                .bind(own_key_id)
                .bind(other_key_id)
                .fetch_all(&state.pool)
                .await
                .unwrap();
        assert_eq!(key_states, vec![(own_key_id, false), (other_key_id, true)]);

        let usage_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/user/usage?api_key_id={own_key_id}&model=gpt-keep&status_code=200"
                    ))
                    .header("cookie", "mini_session=policy-session")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(usage_response.status(), StatusCode::OK);
        let body = to_bytes(usage_response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let usage: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(usage["meta"]["total"], 1);
        assert_eq!(usage["data"][0]["request_id"], "own-request");

        let other_detail = app
            .oneshot(
                Request::builder()
                    .uri("/api/user/usage/2")
                    .header("cookie", "mini_session=policy-session")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(other_detail.status(), StatusCode::NOT_FOUND);
    }
}
