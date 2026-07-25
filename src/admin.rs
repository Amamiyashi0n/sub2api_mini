use axum::{
    Json, Router,
    extract::{Extension, Path, State},
    http::StatusCode,
    middleware,
    routing::{get, post, put},
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    auth::{self, AuthSession},
    crypto::token_hash,
    error::{ApiError, ApiResult},
    gateway, key_policy,
    models::{AccountRow, Credentials, normalize_base_url},
    oauth,
    state::{AppState, RuntimeSettings},
};

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .merge(crate::dashboard::admin_router())
        .merge(crate::batch_images::admin_router())
        .merge(crate::account_data::admin_router())
        .merge(crate::account_tools::admin_router())
        .merge(crate::scheduled_tests::admin_router())
        .route("/accounts", get(list_accounts).post(create_account))
        .route("/accounts/bulk-update", post(bulk_update_accounts))
        .route("/accounts/batch-clear-error", post(batch_clear_accounts))
        .route("/accounts/batch-refresh", post(batch_refresh_accounts))
        .route("/accounts/batch-delete", post(batch_delete_accounts))
        .route("/accounts/{id}", put(update_account).delete(delete_account))
        .route("/accounts/{id}/test", post(test_account))
        .route("/accounts/{id}/refresh", post(refresh_account))
        .route("/accounts/{id}/recover", post(recover_account))
        .route("/oauth/import", post(import_oauth))
        .route("/oauth/start", post(start_oauth))
        .route("/keys", get(list_keys).post(create_key))
        .route("/keys/batch", post(batch_key_action))
        .route("/keys/{id}", put(update_key).delete(delete_key))
        .merge(crate::usage::admin_router())
        .merge(crate::mail::admin_router())
        .route("/settings", get(settings).put(update_settings))
        .merge(crate::groups::admin_router())
        .merge(crate::admin_users::router())
        .merge(crate::subscriptions::admin_router())
        .merge(crate::redeem::admin_router())
        .merge(crate::risk_control::admin_router())
        .merge(crate::channel_monitor::admin_router())
        .merge(crate::channels::admin_router())
        .merge(crate::orders::admin_router())
        .merge(crate::ops::router())
        .merge(crate::proxies::admin_router())
        .merge(crate::prompt_audit::admin_router())
        .merge(crate::content::admin_router())
        .merge(crate::audit::router())
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            crate::audit::capture,
        ))
        .route_layer(middleware::from_fn_with_state(state, auth::admin_guard))
}

async fn settings(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let mail_configured = crate::mail::is_configured(&state).await?;
    let values: Vec<(String, String)> = sqlx::query_as(
        "SELECT key, value FROM app_settings WHERE key IN \
         ('site_name', 'site_subtitle', 'site_logo', 'contact_info', 'doc_url', \
          'home_content', 'audit_retention_days', 'registration_enabled', \
          'email_verification_enabled', 'password_reset_enabled', \
          'channel_monitor_enabled', 'channel_monitor_default_interval_seconds', \
          'turnstile_enabled', 'turnstile_site_key', 'turnstile_secret_key_encrypted', \
          'default_theme')",
    )
    .fetch_all(&state.pool)
    .await?;
    let site_name = values
        .iter()
        .find(|row| row.0 == "site_name")
        .map(|row| row.1.clone())
        .unwrap_or_else(|| "Sub2API Mini".into());
    let audit_retention_days = values
        .iter()
        .find(|row| row.0 == "audit_retention_days")
        .and_then(|row| row.1.parse::<i64>().ok())
        .unwrap_or(90);
    let setting_bool = |key: &str, default: bool| {
        values
            .iter()
            .find(|row| row.0 == key)
            .and_then(|row| row.1.parse::<bool>().ok())
            .unwrap_or(default)
    };
    let setting_string = |key: &str, default: &str| {
        values
            .iter()
            .find(|row| row.0 == key)
            .map(|row| row.1.as_str())
            .unwrap_or(default)
            .to_string()
    };
    let runtime = state.runtime_settings.read().await.clone();
    Ok(Json(json!({"data": {
        "site_name": site_name,
        "site_subtitle": setting_string("site_subtitle", "个人 AI API 网关"),
        "site_logo": setting_string("site_logo", ""),
        "contact_info": setting_string("contact_info", ""),
        "doc_url": setting_string("doc_url", ""),
        "home_content": setting_string("home_content", ""),
        "default_theme": crate::public::normalize_theme(values.iter().find(|row| row.0 == "default_theme").map(|row| row.1.as_str())),
        "audit_retention_days": audit_retention_days,
        "retry_attempts": runtime.retry_attempts,
        "model_cache_seconds": runtime.model_cache_seconds,
        "cooldown_5xx_seconds": runtime.cooldown_5xx_seconds,
        "cooldown_429_seconds": runtime.cooldown_429_seconds,
        "bind": state.config.bind.to_string(),
        "callback_bind": state.config.callback_bind.to_string(),
        "database_path": state.config.database_path.display().to_string(),
        "session_hours": state.config.session_hours
        ,"registration_enabled": setting_bool("registration_enabled", false)
        ,"email_verification_enabled": setting_bool("email_verification_enabled", false)
        ,"password_reset_enabled": setting_bool("password_reset_enabled", true)
        ,"mail_configured": mail_configured
        ,"channel_monitor_enabled": setting_bool("channel_monitor_enabled", true)
        ,"channel_monitor_default_interval_seconds": values.iter().find(|row| row.0 == "channel_monitor_default_interval_seconds").and_then(|row| row.1.parse::<i64>().ok()).unwrap_or(300)
        ,"turnstile_enabled": setting_bool("turnstile_enabled", false)
        ,"turnstile_site_key": setting_string("turnstile_site_key", "")
        ,"turnstile_secret_key_configured": values.iter().any(|row| row.0 == "turnstile_secret_key_encrypted" && !row.1.is_empty())
    }})))
}

#[derive(Deserialize)]
struct SettingsInput {
    site_name: String,
    #[serde(default = "default_theme")]
    default_theme: String,
    #[serde(default)]
    site_subtitle: String,
    #[serde(default)]
    site_logo: String,
    #[serde(default)]
    contact_info: String,
    #[serde(default)]
    doc_url: String,
    #[serde(default)]
    home_content: String,
    audit_retention_days: i64,
    retry_attempts: usize,
    model_cache_seconds: u64,
    cooldown_5xx_seconds: i64,
    cooldown_429_seconds: i64,
    #[serde(default)]
    registration_enabled: bool,
    #[serde(default)]
    email_verification_enabled: bool,
    #[serde(default = "enabled_by_default")]
    password_reset_enabled: bool,
    #[serde(default = "enabled_by_default")]
    channel_monitor_enabled: bool,
    #[serde(default = "default_monitor_interval")]
    channel_monitor_default_interval_seconds: i64,
    #[serde(default)]
    turnstile_enabled: bool,
    #[serde(default)]
    turnstile_site_key: String,
    #[serde(default)]
    turnstile_secret_key: String,
}

fn enabled_by_default() -> bool {
    true
}

fn default_monitor_interval() -> i64 {
    300
}

fn default_theme() -> String {
    "light".into()
}

async fn update_settings(
    State(state): State<AppState>,
    Json(input): Json<SettingsInput>,
) -> ApiResult<Json<Value>> {
    if input.site_name.trim().is_empty()
        || input.site_name.chars().count() > 80
        || input.site_subtitle.chars().count() > 200
        || input.contact_info.chars().count() > 500
        || input.home_content.len() > 500_000
        || !valid_site_logo(&input.site_logo)
        || !valid_optional_http_url(&input.doc_url)
        || !(1..=3650).contains(&input.audit_retention_days)
        || !matches!(input.default_theme.trim(), "light" | "dark")
    {
        return Err(ApiError::bad_request(
            "INVALID_SETTINGS",
            "site name or audit retention is invalid",
        ));
    }
    let runtime = RuntimeSettings {
        retry_attempts: input.retry_attempts,
        model_cache_seconds: input.model_cache_seconds,
        cooldown_5xx_seconds: input.cooldown_5xx_seconds,
        cooldown_429_seconds: input.cooldown_429_seconds,
    };
    runtime.validate()?;
    if input.email_verification_enabled && !crate::mail::is_configured(&state).await? {
        return Err(ApiError::bad_request(
            "MAIL_NOT_CONFIGURED",
            "configure Webhook or SMTP mail delivery before enabling email verification",
        ));
    }
    if !(30..=86_400).contains(&input.channel_monitor_default_interval_seconds) {
        return Err(ApiError::bad_request(
            "INVALID_MONITOR_INTERVAL",
            "channel monitor default interval must be 30-86400 seconds",
        ));
    }
    let existing_turnstile_secret: Option<String> = sqlx::query_scalar(
        "SELECT value FROM app_settings WHERE key = 'turnstile_secret_key_encrypted'",
    )
    .fetch_optional(&state.pool)
    .await?;
    let supplied_turnstile_secret = input.turnstile_secret_key.trim().to_string();
    if input.turnstile_site_key.trim().chars().count() > 256
        || supplied_turnstile_secret.chars().count() > 512
        || (input.turnstile_enabled
            && (input.turnstile_site_key.trim().is_empty()
                || (supplied_turnstile_secret.is_empty()
                    && existing_turnstile_secret
                        .as_deref()
                        .is_none_or(str::is_empty))))
    {
        return Err(ApiError::bad_request(
            "TURNSTILE_NOT_CONFIGURED",
            "site key and secret key are required before enabling turnstile",
        ));
    }
    let values = [
        ("site_name", input.site_name.trim().to_string()),
        ("default_theme", input.default_theme.trim().to_string()),
        ("site_subtitle", input.site_subtitle.trim().to_string()),
        ("site_logo", input.site_logo.trim().to_string()),
        ("contact_info", input.contact_info.trim().to_string()),
        ("doc_url", input.doc_url.trim().to_string()),
        ("home_content", input.home_content.trim().to_string()),
        (
            "audit_retention_days",
            input.audit_retention_days.to_string(),
        ),
        ("retry_attempts", runtime.retry_attempts.to_string()),
        (
            "model_cache_seconds",
            runtime.model_cache_seconds.to_string(),
        ),
        (
            "cooldown_5xx_seconds",
            runtime.cooldown_5xx_seconds.to_string(),
        ),
        (
            "cooldown_429_seconds",
            runtime.cooldown_429_seconds.to_string(),
        ),
        (
            "registration_enabled",
            input.registration_enabled.to_string(),
        ),
        (
            "email_verification_enabled",
            input.email_verification_enabled.to_string(),
        ),
        (
            "password_reset_enabled",
            input.password_reset_enabled.to_string(),
        ),
        (
            "channel_monitor_enabled",
            input.channel_monitor_enabled.to_string(),
        ),
        (
            "channel_monitor_default_interval_seconds",
            input.channel_monitor_default_interval_seconds.to_string(),
        ),
        ("turnstile_enabled", input.turnstile_enabled.to_string()),
        (
            "turnstile_site_key",
            input.turnstile_site_key.trim().to_string(),
        ),
    ];
    let mut transaction = state.pool.begin().await?;
    for (key, value) in values {
        sqlx::query(
            "INSERT INTO app_settings (key, value) VALUES (?, ?) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = CURRENT_TIMESTAMP",
        )
        .bind(key)
        .bind(value)
        .execute(&mut *transaction)
        .await?;
    }
    if !supplied_turnstile_secret.is_empty() {
        sqlx::query(
            "INSERT INTO app_settings (key, value) VALUES ('turnstile_secret_key_encrypted', ?) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = CURRENT_TIMESTAMP",
        )
        .bind(state.crypto.encrypt(supplied_turnstile_secret.as_bytes())?)
        .execute(&mut *transaction)
        .await?;
    }
    sqlx::query(
        "DELETE FROM audit_logs WHERE datetime(created_at) < datetime('now', '-' || ? || ' days')",
    )
    .bind(input.audit_retention_days)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    *state.runtime_settings.write().await = runtime;
    settings(State(state)).await
}

fn valid_optional_http_url(value: &str) -> bool {
    let value = value.trim();
    if value.is_empty() {
        return true;
    }
    if value.len() > 2048 {
        return false;
    }
    url::Url::parse(value).ok().is_some_and(|url| {
        matches!(url.scheme(), "http" | "https")
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
    })
}

fn valid_site_logo(value: &str) -> bool {
    let value = value.trim();
    if value.is_empty() {
        return true;
    }
    if value.len() > 256 * 1024 {
        return false;
    }
    (value.starts_with('/') && !value.starts_with("//"))
        || valid_optional_http_url(value)
        || [
            "data:image/png;base64,",
            "data:image/jpeg;base64,",
            "data:image/webp;base64,",
            "data:image/gif;base64,",
        ]
        .iter()
        .any(|prefix| value.starts_with(prefix))
}

async fn list_accounts(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let rows = sqlx::query_as::<_, AccountRow>(
        "SELECT accounts.id, accounts.name, accounts.kind, accounts.base_url, \
         accounts.encrypted_credentials, accounts.priority, accounts.concurrency, \
         accounts.enabled, accounts.cooldown_until, accounts.last_used_at, accounts.last_error, \
         accounts.proxy_id, proxies.name AS proxy_name, CASE WHEN proxies.id IS NULL THEN NULL \
         WHEN proxies.enabled = 1 AND (proxies.expires_at IS NULL OR \
         datetime(proxies.expires_at) > CURRENT_TIMESTAMP) THEN 1 WHEN proxies.fallback_mode = 'direct' \
         THEN 1 WHEN proxies.fallback_mode = 'proxy' AND backup_proxies.enabled = 1 AND \
         (backup_proxies.expires_at IS NULL OR datetime(backup_proxies.expires_at) > CURRENT_TIMESTAMP) \
         THEN 1 ELSE 0 END AS proxy_active, CASE WHEN proxies.enabled = 1 AND \
         (proxies.expires_at IS NULL OR datetime(proxies.expires_at) > CURRENT_TIMESTAMP) \
         THEN proxies.encrypted_url WHEN proxies.fallback_mode = 'proxy' AND backup_proxies.enabled = 1 \
         AND (backup_proxies.expires_at IS NULL OR datetime(backup_proxies.expires_at) > CURRENT_TIMESTAMP) \
         THEN backup_proxies.encrypted_url ELSE NULL END AS encrypted_proxy_url, \
         accounts.parent_account_id, accounts.quota_dimension, \
         accounts.created_at, accounts.updated_at FROM accounts \
         LEFT JOIN proxies ON proxies.id = accounts.proxy_id \
         LEFT JOIN proxies AS backup_proxies ON backup_proxies.id = proxies.backup_proxy_id \
         ORDER BY accounts.priority ASC, accounts.id ASC",
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(json!({
        "data": rows.iter().map(AccountRow::public).collect::<Vec<_>>()
    })))
}

#[derive(Deserialize)]
struct CreateAccountInput {
    name: String,
    #[serde(default = "default_api_key_kind")]
    kind: String,
    #[serde(default)]
    base_url: String,
    api_key: String,
    #[serde(default = "default_priority")]
    priority: i32,
    #[serde(default = "default_concurrency")]
    concurrency: i32,
    #[serde(default)]
    proxy_id: Option<i64>,
}

fn default_api_key_kind() -> String {
    "api_key".into()
}
fn default_priority() -> i32 {
    50
}
fn default_concurrency() -> i32 {
    3
}

async fn create_account(
    State(state): State<AppState>,
    Json(input): Json<CreateAccountInput>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    if input.kind != "api_key" {
        return Err(ApiError::bad_request(
            "INVALID_ACCOUNT_KIND",
            "OAuth accounts must be created through OAuth import or PKCE",
        ));
    }
    validate_account_fields(&input.name, input.priority, input.concurrency)?;
    if input.api_key.trim().is_empty() {
        return Err(ApiError::bad_request(
            "API_KEY_REQUIRED",
            "api_key is required",
        ));
    }
    let base_url = normalize_base_url(&input.base_url, "api_key")?;
    validate_proxy_id(&state, input.proxy_id).await?;
    let credentials = Credentials {
        api_key: Some(input.api_key.trim().to_string()),
        ..Default::default()
    };
    let encrypted = encrypt_credentials(&state, &credentials)?;
    let result = sqlx::query(
        "INSERT INTO accounts (name, kind, base_url, encrypted_credentials, priority, concurrency, proxy_id) \
         VALUES (?, 'api_key', ?, ?, ?, ?, ?)",
    )
    .bind(input.name.trim())
    .bind(base_url)
    .bind(encrypted)
    .bind(input.priority)
    .bind(input.concurrency)
    .bind(input.proxy_id)
    .execute(&state.pool)
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"data": {"id": result.last_insert_rowid()}})),
    ))
}

#[derive(Deserialize)]
struct UpdateAccountInput {
    name: Option<String>,
    base_url: Option<String>,
    api_key: Option<String>,
    priority: Option<i32>,
    concurrency: Option<i32>,
    enabled: Option<bool>,
    #[serde(default, deserialize_with = "crate::models::deserialize_nullable")]
    proxy_id: Option<Option<i64>>,
}

async fn update_account(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(input): Json<UpdateAccountInput>,
) -> ApiResult<Json<Value>> {
    let row = get_account_row(&state, id).await?;
    if row.parent_account_id.is_some() {
        if input.base_url.is_some() || input.api_key.is_some() || input.proxy_id.is_some() {
            return Err(ApiError::bad_request(
                "SPARK_SHADOW_CREDENTIALS_INHERITED",
                "Spark shadow credentials, Base URL, and proxy are inherited from the parent account",
            ));
        }
        let name = input.name.unwrap_or_else(|| row.name.clone());
        let priority = input.priority.unwrap_or(row.priority);
        let concurrency = input.concurrency.unwrap_or(row.concurrency);
        validate_account_fields(&name, priority, concurrency)?;
        sqlx::query(
            "UPDATE accounts SET name = ?, priority = ?, concurrency = ?, enabled = ?, \
             cooldown_until = CASE WHEN ? = 1 THEN NULL ELSE cooldown_until END, \
             last_error = CASE WHEN ? = 1 THEN NULL ELSE last_error END, \
             updated_at = CURRENT_TIMESTAMP WHERE id = ?",
        )
        .bind(name.trim())
        .bind(priority)
        .bind(concurrency)
        .bind(input.enabled.unwrap_or(row.enabled))
        .bind(input.enabled.unwrap_or(row.enabled))
        .bind(input.enabled.unwrap_or(row.enabled))
        .bind(id)
        .execute(&state.pool)
        .await?;
        return Ok(Json(json!({"data": {"id": id}})));
    }
    let name = input.name.unwrap_or_else(|| row.name.clone());
    let priority = input.priority.unwrap_or(row.priority);
    let concurrency = input.concurrency.unwrap_or(row.concurrency);
    validate_account_fields(&name, priority, concurrency)?;
    let base_url = match input.base_url {
        Some(value) => normalize_base_url(&value, &row.kind)?,
        None => row.base_url.clone(),
    };
    let mut account = state.resolve_account(row.clone()).await?;
    if let Some(api_key) = input.api_key {
        if row.kind != "api_key" || api_key.trim().is_empty() {
            return Err(ApiError::bad_request(
                "INVALID_API_KEY",
                "api_key is invalid",
            ));
        }
        account.credentials.api_key = Some(api_key.trim().to_string());
    }
    let encrypted = encrypt_credentials(&state, &account.credentials)?;
    let proxy_changed = input.proxy_id.is_some();
    let proxy_id = input.proxy_id.unwrap_or(row.proxy_id);
    validate_proxy_id(&state, proxy_id).await?;
    sqlx::query(
        "UPDATE accounts SET name = ?, base_url = ?, encrypted_credentials = ?, priority = ?, \
         concurrency = ?, enabled = ?, proxy_id = ?, \
         cooldown_until = CASE WHEN ? = 1 THEN NULL ELSE cooldown_until END, \
         last_error = CASE WHEN ? = 1 THEN NULL ELSE last_error END, \
         updated_at = CURRENT_TIMESTAMP WHERE id = ?",
    )
    .bind(name.trim())
    .bind(base_url)
    .bind(encrypted)
    .bind(priority)
    .bind(concurrency)
    .bind(input.enabled.unwrap_or(row.enabled))
    .bind(proxy_id)
    .bind(input.enabled.unwrap_or(row.enabled))
    .bind(input.enabled.unwrap_or(row.enabled))
    .bind(id)
    .execute(&state.pool)
    .await?;
    if proxy_changed {
        sqlx::query(
            "UPDATE accounts SET proxy_id = ?, updated_at = CURRENT_TIMESTAMP \
             WHERE parent_account_id = ?",
        )
        .bind(proxy_id)
        .bind(id)
        .execute(&state.pool)
        .await?;
    }
    Ok(Json(json!({"data": {"id": id}})))
}

async fn delete_account(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    let result = sqlx::query("DELETE FROM accounts WHERE id = ?")
        .bind(id)
        .execute(&state.pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found("account not found"));
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct AccountBatchIdsInput {
    account_ids: Vec<i64>,
}

#[derive(Deserialize)]
struct BulkUpdateAccountsInput {
    account_ids: Vec<i64>,
    enabled: Option<bool>,
    schedulable: Option<bool>,
    priority: Option<i32>,
    concurrency: Option<i32>,
    #[serde(default, deserialize_with = "crate::models::deserialize_nullable")]
    proxy_id: Option<Option<i64>>,
    group_ids: Option<Vec<i64>>,
}

async fn bulk_update_accounts(
    State(state): State<AppState>,
    Json(input): Json<BulkUpdateAccountsInput>,
) -> ApiResult<Json<Value>> {
    let ids = normalize_account_ids(input.account_ids, 500)?;
    let enabled = input.enabled.or(input.schedulable);
    if input.priority.is_none()
        && input.concurrency.is_none()
        && input.proxy_id.is_none()
        && input.group_ids.is_none()
        && enabled.is_none()
    {
        return Err(ApiError::bad_request(
            "NO_ACCOUNT_UPDATES",
            "at least one account field is required",
        ));
    }
    if input.priority.is_some_and(|value| value < 0)
        || input
            .concurrency
            .is_some_and(|value| !(1..=1000).contains(&value))
    {
        return Err(ApiError::bad_request(
            "INVALID_ACCOUNT",
            "priority or concurrency is invalid",
        ));
    }
    if let Some(proxy_id) = input.proxy_id.flatten() {
        validate_proxy_id(&state, Some(proxy_id)).await?;
    }
    let group_ids = input.group_ids.map(normalize_group_ids).transpose()?;
    if let Some(group_ids) = &group_ids {
        validate_group_ids(&state, group_ids).await?;
    }

    let mut transaction = state.pool.begin().await?;
    let mut success_ids = Vec::new();
    let mut errors = Vec::new();
    for id in ids {
        let parent_account_id: Option<Option<i64>> =
            sqlx::query_scalar("SELECT parent_account_id FROM accounts WHERE id = ?")
                .bind(id)
                .fetch_optional(&mut *transaction)
                .await?;
        let Some(parent_account_id) = parent_account_id else {
            errors.push((id, "account not found".to_string()));
            continue;
        };
        if parent_account_id.is_some() && input.proxy_id.is_some() {
            errors.push((
                id,
                "Spark shadow proxy is inherited from its parent".to_string(),
            ));
            continue;
        }
        if let Some(value) = enabled {
            sqlx::query(
                "UPDATE accounts SET enabled = ?, cooldown_until = CASE WHEN ? = 1 THEN NULL \
                 ELSE cooldown_until END, last_error = CASE WHEN ? = 1 THEN NULL ELSE last_error END \
                 WHERE id = ?",
            )
            .bind(value)
            .bind(value)
            .bind(value)
            .bind(id)
            .execute(&mut *transaction)
            .await?;
        }
        if let Some(value) = input.priority {
            sqlx::query("UPDATE accounts SET priority = ? WHERE id = ?")
                .bind(value)
                .bind(id)
                .execute(&mut *transaction)
                .await?;
        }
        if let Some(value) = input.concurrency {
            sqlx::query("UPDATE accounts SET concurrency = ? WHERE id = ?")
                .bind(value)
                .bind(id)
                .execute(&mut *transaction)
                .await?;
        }
        if let Some(value) = input.proxy_id {
            sqlx::query("UPDATE accounts SET proxy_id = ? WHERE id = ?")
                .bind(value)
                .bind(id)
                .execute(&mut *transaction)
                .await?;
            sqlx::query("UPDATE accounts SET proxy_id = ? WHERE parent_account_id = ?")
                .bind(value)
                .bind(id)
                .execute(&mut *transaction)
                .await?;
        }
        if let Some(group_ids) = &group_ids {
            sqlx::query("DELETE FROM account_groups WHERE account_id = ?")
                .bind(id)
                .execute(&mut *transaction)
                .await?;
            for group_id in group_ids {
                sqlx::query("INSERT INTO account_groups (account_id, group_id) VALUES (?, ?)")
                    .bind(id)
                    .bind(group_id)
                    .execute(&mut *transaction)
                    .await?;
            }
        }
        sqlx::query("UPDATE accounts SET updated_at = CURRENT_TIMESTAMP WHERE id = ?")
            .bind(id)
            .execute(&mut *transaction)
            .await?;
        success_ids.push(id);
    }
    transaction.commit().await?;
    Ok(Json(
        json!({"data": account_batch_result(success_ids, errors)}),
    ))
}

async fn batch_clear_accounts(
    State(state): State<AppState>,
    Json(input): Json<AccountBatchIdsInput>,
) -> ApiResult<Json<Value>> {
    let ids = normalize_account_ids(input.account_ids, 500)?;
    let mut transaction = state.pool.begin().await?;
    let mut success_ids = Vec::new();
    let mut errors = Vec::new();
    for id in ids {
        let result = sqlx::query(
            "UPDATE accounts SET enabled = 1, cooldown_until = NULL, last_error = NULL, \
             updated_at = CURRENT_TIMESTAMP WHERE id = ?",
        )
        .bind(id)
        .execute(&mut *transaction)
        .await?;
        if result.rows_affected() == 0 {
            errors.push((id, "account not found".to_string()));
        } else {
            success_ids.push(id);
        }
    }
    transaction.commit().await?;
    Ok(Json(
        json!({"data": account_batch_result(success_ids, errors)}),
    ))
}

async fn batch_delete_accounts(
    State(state): State<AppState>,
    Json(input): Json<AccountBatchIdsInput>,
) -> ApiResult<Json<Value>> {
    let ids = normalize_account_ids(input.account_ids, 500)?;
    let mut transaction = state.pool.begin().await?;
    let mut success_ids = Vec::new();
    let mut errors = Vec::new();
    for id in ids {
        let result = sqlx::query("DELETE FROM accounts WHERE id = ?")
            .bind(id)
            .execute(&mut *transaction)
            .await?;
        if result.rows_affected() == 0 {
            errors.push((id, "account not found".to_string()));
        } else {
            success_ids.push(id);
        }
    }
    transaction.commit().await?;
    Ok(Json(
        json!({"data": account_batch_result(success_ids, errors)}),
    ))
}

async fn batch_refresh_accounts(
    State(state): State<AppState>,
    Json(input): Json<AccountBatchIdsInput>,
) -> ApiResult<Json<Value>> {
    let ids = normalize_account_ids(input.account_ids, 100)?;
    let mut success_ids = Vec::new();
    let mut errors = Vec::new();
    for id in ids {
        let row = match get_account_row(&state, id).await {
            Ok(row) => row,
            Err(error) => {
                errors.push((id, error.message));
                continue;
            }
        };
        if row.kind != "oauth" {
            errors.push((id, "account is not an OAuth account".to_string()));
            continue;
        }
        let mut account = match state.resolve_account(row).await {
            Ok(account) => account,
            Err(error) => {
                errors.push((id, error.message));
                continue;
            }
        };
        match oauth::refresh_account_forced(&state, &mut account).await {
            Ok(()) => success_ids.push(id),
            Err(error) => errors.push((id, error.message)),
        }
    }
    Ok(Json(
        json!({"data": account_batch_result(success_ids, errors)}),
    ))
}

async fn recover_account(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<Value>> {
    let result = sqlx::query(
        "UPDATE accounts SET enabled = 1, cooldown_until = NULL, last_error = NULL, \
         updated_at = CURRENT_TIMESTAMP WHERE id = ?",
    )
    .bind(id)
    .execute(&state.pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found("account not found"));
    }
    Ok(Json(json!({"data": {"id": id, "recovered": true}})))
}

fn normalize_account_ids(ids: Vec<i64>, limit: usize) -> ApiResult<Vec<i64>> {
    normalize_related_ids(ids, limit, "INVALID_ACCOUNT_BATCH")
}

fn normalize_group_ids(ids: Vec<i64>) -> ApiResult<Vec<i64>> {
    if ids.len() > 100 || ids.iter().any(|id| *id <= 0) {
        return Err(ApiError::bad_request(
            "INVALID_ACCOUNT_GROUPS",
            "group IDs must contain at most 100 positive values",
        ));
    }
    let mut unique = Vec::with_capacity(ids.len());
    for id in ids {
        if !unique.contains(&id) {
            unique.push(id);
        }
    }
    Ok(unique)
}

fn normalize_related_ids(ids: Vec<i64>, limit: usize, code: &'static str) -> ApiResult<Vec<i64>> {
    if ids.is_empty() || ids.len() > limit || ids.iter().any(|id| *id <= 0) {
        return Err(ApiError::bad_request(
            code,
            format!("IDs must contain 1 to {limit} positive values"),
        ));
    }
    let mut unique = Vec::with_capacity(ids.len());
    for id in ids {
        if !unique.contains(&id) {
            unique.push(id);
        }
    }
    Ok(unique)
}

async fn validate_group_ids(state: &AppState, ids: &[i64]) -> ApiResult<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let placeholders = std::iter::repeat_n("?", ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let query = format!("SELECT COUNT(*) FROM groups WHERE id IN ({placeholders})");
    let mut query = sqlx::query_scalar::<_, i64>(&query);
    for id in ids {
        query = query.bind(id);
    }
    if query.fetch_one(&state.pool).await? != ids.len() as i64 {
        return Err(ApiError::bad_request(
            "INVALID_ACCOUNT_GROUPS",
            "one or more selected groups do not exist",
        ));
    }
    Ok(())
}

fn account_batch_result(success_ids: Vec<i64>, errors: Vec<(i64, String)>) -> Value {
    let failed_ids = errors.iter().map(|item| item.0).collect::<Vec<_>>();
    let results = success_ids
        .iter()
        .map(|id| json!({"account_id": id, "success": true}))
        .chain(
            errors
                .iter()
                .map(|(id, error)| json!({"account_id": id, "success": false, "error": error})),
        )
        .collect::<Vec<_>>();
    json!({
        "total": success_ids.len() + errors.len(),
        "success": success_ids.len(),
        "failed": errors.len(),
        "success_ids": success_ids,
        "failed_ids": failed_ids,
        "errors": errors.into_iter().map(|(id, error)| {
            json!({"account_id": id, "error": error})
        }).collect::<Vec<_>>(),
        "results": results,
    })
}

async fn test_account(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<Value>> {
    let mut account = state
        .resolve_account(get_account_row(&state, id).await?)
        .await?;
    let result = gateway::probe_account(&state, &mut account).await?;
    Ok(Json(json!({"data": result})))
}

async fn refresh_account(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<Value>> {
    let mut account = state
        .resolve_account(get_account_row(&state, id).await?)
        .await?;
    if account.row.kind != "oauth" {
        return Err(ApiError::bad_request(
            "NOT_OAUTH_ACCOUNT",
            "account is not an OAuth account",
        ));
    }
    oauth::refresh_account_forced(&state, &mut account).await?;
    Ok(Json(json!({"data": {"id": id, "refreshed": true}})))
}

#[derive(Deserialize)]
struct ImportOAuthInput {
    content: String,
    #[serde(default)]
    name: String,
    #[serde(default = "default_priority")]
    priority: i32,
    #[serde(default = "default_concurrency")]
    concurrency: i32,
    #[serde(default)]
    proxy_id: Option<i64>,
}

async fn import_oauth(
    State(state): State<AppState>,
    Json(input): Json<ImportOAuthInput>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let credentials = oauth::parse_import(&input.content)?;
    validate_proxy_id(&state, input.proxy_id).await?;
    let name = if input.name.trim().is_empty() {
        credentials
            .email
            .clone()
            .unwrap_or_else(|| "OpenAI OAuth".into())
    } else {
        input.name.trim().to_string()
    };
    let id = oauth::insert_oauth_account(
        &state,
        &name,
        credentials,
        input.priority,
        input.concurrency,
    )
    .await?;
    if let Some(proxy_id) = input.proxy_id {
        sqlx::query("UPDATE accounts SET proxy_id = ? WHERE id = ?")
            .bind(proxy_id)
            .bind(id)
            .execute(&state.pool)
            .await?;
    }
    Ok((StatusCode::CREATED, Json(json!({"data": {"id": id}}))))
}

#[derive(Debug, Deserialize, Default)]
struct OAuthStartInput {
    account_id: Option<i64>,
}

async fn start_oauth(
    State(state): State<AppState>,
    Json(input): Json<OAuthStartInput>,
) -> ApiResult<Json<Value>> {
    if let Some(account_id) = input.account_id {
        let account = get_account_row(&state, account_id).await?;
        if account.kind != "oauth" || account.parent_account_id.is_some() {
            return Err(ApiError::bad_request(
                "NOT_OAUTH_ACCOUNT",
                "only OAuth accounts can be re-authorized",
            ));
        }
    }
    let started = oauth::start_flow(&state, input.account_id).await?;
    Ok(Json(json!({"data": started})))
}

async fn list_keys(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    Ok(Json(json!({
        "data": key_policy::list_keys(&state.pool, None).await?
    })))
}

#[derive(Deserialize)]
struct CreateKeyInput {
    name: String,
    custom_key: Option<String>,
    user_id: Option<i64>,
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
    let expires_at = crate::user::expires_from_days(input.expires_in_days)?;
    let quota_tokens = crate::user::validate_quota(input.quota_tokens.unwrap_or(0))?;
    let quota_cost_microusd = key_policy::validate_microusd(
        input.quota_cost_microusd.unwrap_or(0),
        "quota_cost_microusd",
    )?;
    let allowed_models = crate::user::normalize_allowed_models(input.allowed_models)?;
    let owner_id = input.user_id.unwrap_or(session.user_id);
    let owner_exists: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE id = ? AND enabled = 1")
            .bind(owner_id)
            .fetch_one(&state.pool)
            .await?;
    if owner_exists == 0 {
        return Err(ApiError::bad_request(
            "USER_NOT_FOUND",
            "key owner was not found",
        ));
    }
    let group_id = crate::user::validate_group_id(&state, input.group_id).await?;
    if let Some(group_id) = group_id {
        crate::groups::ensure_user_group_access(&state, owner_id, group_id).await?;
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
    .bind(owner_id)
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
            "id": result.last_insert_rowid(), "name": input.name.trim(), "token": token,
            "token_prefix": prefix, "expires_at": expires_at, "quota_tokens": quota_tokens,
            "quota_cost_microusd": quota_cost_microusd, "allowed_models": allowed_models,
            "group_id": group_id, "ip_whitelist": ip_whitelist, "ip_blacklist": ip_blacklist,
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
    Path(id): Path<i64>,
    Json(input): Json<UpdateKeyInput>,
) -> ApiResult<Json<Value>> {
    let owner: Option<(Option<i64>,)> = sqlx::query_as("SELECT user_id FROM api_keys WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?;
    let Some((owner_id,)) = owner else {
        return Err(ApiError::not_found("API key not found"));
    };
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
        sqlx::query(
            "UPDATE api_keys SET expires_at = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
        )
        .bind(crate::user::validate_expiry(&expires_at)?)
        .bind(id)
        .execute(&state.pool)
        .await?;
    }
    if let Some(quota_tokens) = input.quota_tokens {
        sqlx::query(
            "UPDATE api_keys SET quota_tokens = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
        )
        .bind(crate::user::validate_quota(quota_tokens)?)
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
        let models = crate::user::normalize_allowed_models(allowed_models)?;
        sqlx::query(
            "UPDATE api_keys SET allowed_models = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
        )
        .bind(serde_json::to_string(&models).unwrap())
        .bind(id)
        .execute(&state.pool)
        .await?;
    }
    if let Some(group_id) = input.group_id {
        let group_id = crate::user::validate_group_id(&state, Some(group_id)).await?;
        if let (Some(owner_id), Some(group_id)) = (owner_id, group_id) {
            crate::groups::ensure_user_group_access(&state, owner_id, group_id).await?;
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
    Json(input): Json<BatchKeyInput>,
) -> ApiResult<Json<Value>> {
    Ok(Json(json!({
        "data": key_policy::batch_action(&state.pool, None, input.ids, &input.action).await?
    })))
}

async fn delete_key(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    let result = sqlx::query("DELETE FROM api_keys WHERE id = ?")
        .bind(id)
        .execute(&state.pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found("API key not found"));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn get_account_row(state: &AppState, id: i64) -> ApiResult<AccountRow> {
    sqlx::query_as::<_, AccountRow>(
        "SELECT accounts.id, accounts.name, accounts.kind, accounts.base_url, \
         accounts.encrypted_credentials, accounts.priority, accounts.concurrency, \
         accounts.enabled, accounts.cooldown_until, accounts.last_used_at, accounts.last_error, \
         accounts.proxy_id, proxies.name AS proxy_name, CASE WHEN proxies.id IS NULL THEN NULL \
         WHEN proxies.enabled = 1 AND (proxies.expires_at IS NULL OR \
         datetime(proxies.expires_at) > CURRENT_TIMESTAMP) THEN 1 WHEN proxies.fallback_mode = 'direct' \
         THEN 1 WHEN proxies.fallback_mode = 'proxy' AND backup_proxies.enabled = 1 AND \
         (backup_proxies.expires_at IS NULL OR datetime(backup_proxies.expires_at) > CURRENT_TIMESTAMP) \
         THEN 1 ELSE 0 END AS proxy_active, CASE WHEN proxies.enabled = 1 AND \
         (proxies.expires_at IS NULL OR datetime(proxies.expires_at) > CURRENT_TIMESTAMP) \
         THEN proxies.encrypted_url WHEN proxies.fallback_mode = 'proxy' AND backup_proxies.enabled = 1 \
         AND (backup_proxies.expires_at IS NULL OR datetime(backup_proxies.expires_at) > CURRENT_TIMESTAMP) \
         THEN backup_proxies.encrypted_url ELSE NULL END AS encrypted_proxy_url, \
         accounts.parent_account_id, accounts.quota_dimension, \
         accounts.created_at, accounts.updated_at FROM accounts \
         LEFT JOIN proxies ON proxies.id = accounts.proxy_id \
         LEFT JOIN proxies AS backup_proxies ON backup_proxies.id = proxies.backup_proxy_id \
         WHERE accounts.id = ?",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| ApiError::not_found("account not found"))
}

async fn validate_proxy_id(state: &AppState, id: Option<i64>) -> ApiResult<()> {
    let Some(id) = id else { return Ok(()) };
    let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM proxies WHERE id = ?")
        .bind(id)
        .fetch_one(&state.pool)
        .await?;
    if exists == 0 {
        return Err(ApiError::bad_request(
            "INVALID_PROXY",
            "selected proxy does not exist",
        ));
    }
    Ok(())
}

fn validate_account_fields(name: &str, priority: i32, concurrency: i32) -> ApiResult<()> {
    if name.trim().is_empty() || priority < 0 || !(1..=1000).contains(&concurrency) {
        return Err(ApiError::bad_request(
            "INVALID_ACCOUNT",
            "name, priority, or concurrency is invalid",
        ));
    }
    Ok(())
}

fn encrypt_credentials(state: &AppState, credentials: &Credentials) -> ApiResult<String> {
    state.crypto.encrypt(
        &serde_json::to_vec(credentials)
            .map_err(|_| ApiError::internal("credential serialization failed"))?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;

    async fn insert_api_account(state: &AppState, name: &str) -> i64 {
        let encrypted = encrypt_credentials(
            state,
            &Credentials {
                api_key: Some(format!("sk-{name}")),
                ..Default::default()
            },
        )
        .unwrap();
        sqlx::query(
            "INSERT INTO accounts (name, kind, base_url, encrypted_credentials) \
             VALUES (?, 'api_key', 'https://api.openai.com', ?)",
        )
        .bind(name)
        .bind(encrypted)
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid()
    }

    #[tokio::test]
    async fn bulk_update_replaces_fields_and_group_bindings() {
        let (_directory, state) = test_support::state().await;
        let account_id = insert_api_account(&state, "primary").await;
        sqlx::query(
            "UPDATE accounts SET enabled = 0, cooldown_until = datetime('now', '+1 hour'), \
             last_error = 'rate limited' WHERE id = ?",
        )
        .bind(account_id)
        .execute(&state.pool)
        .await
        .unwrap();
        let proxy = state.crypto.encrypt(b"http://127.0.0.1:3128").unwrap();
        let proxy_id =
            sqlx::query("INSERT INTO proxies (name, encrypted_url) VALUES ('office', ?)")
                .bind(proxy)
                .execute(&state.pool)
                .await
                .unwrap()
                .last_insert_rowid();
        let group_id = sqlx::query("INSERT INTO groups (name) VALUES ('paid')")
            .execute(&state.pool)
            .await
            .unwrap()
            .last_insert_rowid();

        let Json(value) = bulk_update_accounts(
            State(state.clone()),
            Json(BulkUpdateAccountsInput {
                account_ids: vec![account_id, account_id, 999_999],
                enabled: Some(true),
                schedulable: None,
                priority: Some(7),
                concurrency: Some(9),
                proxy_id: Some(Some(proxy_id)),
                group_ids: Some(vec![group_id, group_id]),
            }),
        )
        .await
        .unwrap();
        assert_eq!(value["data"]["total"], 2);
        assert_eq!(value["data"]["success"], 1);
        assert_eq!(value["data"]["failed"], 1);
        let row: (bool, i32, i32, Option<i64>, Option<String>, Option<String>) = sqlx::query_as(
            "SELECT enabled, priority, concurrency, proxy_id, cooldown_until, last_error \
                 FROM accounts WHERE id = ?",
        )
        .bind(account_id)
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(row, (true, 7, 9, Some(proxy_id), None, None));
        let bound: Vec<i64> =
            sqlx::query_scalar("SELECT group_id FROM account_groups WHERE account_id = ?")
                .bind(account_id)
                .fetch_all(&state.pool)
                .await
                .unwrap();
        assert_eq!(bound, vec![group_id]);

        let _ = bulk_update_accounts(
            State(state.clone()),
            Json(BulkUpdateAccountsInput {
                account_ids: vec![account_id],
                enabled: None,
                schedulable: None,
                priority: None,
                concurrency: None,
                proxy_id: None,
                group_ids: Some(Vec::new()),
            }),
        )
        .await
        .unwrap();
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM account_groups WHERE account_id = ?")
                .bind(account_id)
                .fetch_one(&state.pool)
                .await
                .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn batch_recovery_and_delete_report_missing_accounts() {
        let (_directory, state) = test_support::state().await;
        let recover_id = insert_api_account(&state, "recover").await;
        let delete_id = insert_api_account(&state, "delete").await;
        sqlx::query(
            "UPDATE accounts SET enabled = 0, cooldown_until = datetime('now', '+1 hour'), \
             last_error = 'temporary' WHERE id = ?",
        )
        .bind(recover_id)
        .execute(&state.pool)
        .await
        .unwrap();
        let Json(recovered) = batch_clear_accounts(
            State(state.clone()),
            Json(AccountBatchIdsInput {
                account_ids: vec![recover_id, 999_999],
            }),
        )
        .await
        .unwrap();
        assert_eq!(recovered["data"]["success"], 1);
        assert_eq!(recovered["data"]["failed"], 1);
        let state_row: (bool, Option<String>, Option<String>) =
            sqlx::query_as("SELECT enabled, cooldown_until, last_error FROM accounts WHERE id = ?")
                .bind(recover_id)
                .fetch_one(&state.pool)
                .await
                .unwrap();
        assert_eq!(state_row, (true, None, None));

        let Json(deleted) = batch_delete_accounts(
            State(state.clone()),
            Json(AccountBatchIdsInput {
                account_ids: vec![delete_id, 999_999],
            }),
        )
        .await
        .unwrap();
        assert_eq!(deleted["data"]["success"], 1);
        assert_eq!(deleted["data"]["failed"], 1);
        let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM accounts WHERE id = ?")
            .bind(delete_id)
            .fetch_one(&state.pool)
            .await
            .unwrap();
        assert_eq!(exists, 0);
    }

    #[tokio::test]
    async fn batch_refresh_rejects_non_oauth_and_missing_accounts_per_item() {
        let (_directory, state) = test_support::state().await;
        let account_id = insert_api_account(&state, "api-key").await;
        let Json(value) = batch_refresh_accounts(
            State(state),
            Json(AccountBatchIdsInput {
                account_ids: vec![account_id, 999_999],
            }),
        )
        .await
        .unwrap();
        assert_eq!(value["data"]["total"], 2);
        assert_eq!(value["data"]["success"], 0);
        assert_eq!(value["data"]["failed"], 2);
        assert_eq!(
            value["data"]["errors"][0]["error"],
            "account is not an OAuth account"
        );
    }

    #[test]
    fn account_batches_require_bounded_positive_ids() {
        assert_eq!(
            normalize_account_ids(Vec::new(), 10).unwrap_err().code,
            "INVALID_ACCOUNT_BATCH"
        );
        assert_eq!(
            normalize_account_ids(vec![1, -2], 10).unwrap_err().code,
            "INVALID_ACCOUNT_BATCH"
        );
        assert_eq!(
            normalize_account_ids(vec![2, 2, 3], 10).unwrap(),
            vec![2, 3]
        );
        assert!(normalize_group_ids(Vec::new()).unwrap().is_empty());
    }
}
