use std::{collections::HashSet, time::Duration};

use axum::{Json, Router, extract::State, routing::post};
use chrono::DateTime;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use url::Url;

use crate::{
    error::{ApiError, ApiResult},
    models::{Credentials, normalize_account_base_url},
    state::AppState,
};

const RESPONSE_LIMIT: usize = 5 * 1024 * 1024;

pub fn admin_router() -> Router<AppState> {
    Router::new()
        .route("/accounts/sync/crs/preview", post(preview))
        .route("/accounts/sync/crs", post(sync))
}

#[derive(Clone, Deserialize)]
struct SyncInput {
    base_url: String,
    username: String,
    password: String,
    #[serde(default = "default_true")]
    sync_proxies: bool,
    #[serde(default, alias = "selected_account_ids")]
    selected_new_account_ids: Vec<String>,
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize)]
struct LoginResponse {
    success: bool,
    #[serde(default)]
    token: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    error: String,
}

#[derive(Deserialize)]
struct ExportResponse {
    success: bool,
    #[serde(default)]
    message: String,
    #[serde(default)]
    error: String,
    data: ExportData,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportData {
    #[serde(default, rename = "openaiOAuthAccounts")]
    open_ai_oauth_accounts: Vec<CrsAccount>,
    #[serde(default, rename = "openaiResponsesAccounts")]
    open_ai_responses_accounts: Vec<CrsAccount>,
    #[serde(default)]
    claude_accounts: Vec<CrsAccount>,
    #[serde(default)]
    claude_console_accounts: Vec<CrsAccount>,
    #[serde(default, rename = "geminiOAuthAccounts")]
    gemini_oauth_accounts: Vec<Value>,
    #[serde(default, rename = "geminiApiKeyAccounts")]
    gemini_api_key_accounts: Vec<Value>,
}

impl ExportData {
    fn unsupported_count(&self) -> usize {
        self.claude_accounts
            .iter()
            .filter(|account| claude_account_type(account).is_none())
            .count()
            + self.gemini_oauth_accounts.len()
            + self.gemini_api_key_accounts.len()
    }

    fn supported(&self) -> Vec<CrsTarget<'_>> {
        let mut targets = Vec::with_capacity(
            self.claude_accounts.len()
                + self.claude_console_accounts.len()
                + self.open_ai_oauth_accounts.len()
                + self.open_ai_responses_accounts.len(),
        );
        targets.extend(self.claude_accounts.iter().filter_map(|account| {
            claude_account_type(account).map(|account_type| CrsTarget {
                account,
                platform: "anthropic",
                kind: "oauth",
                account_type,
            })
        }));
        targets.extend(
            self.claude_console_accounts
                .iter()
                .map(|account| CrsTarget {
                    account,
                    platform: "anthropic",
                    kind: "api_key",
                    account_type: "api_key",
                }),
        );
        targets.extend(self.open_ai_oauth_accounts.iter().map(|account| CrsTarget {
            account,
            platform: "openai",
            kind: "oauth",
            account_type: "oauth",
        }));
        targets.extend(
            self.open_ai_responses_accounts
                .iter()
                .map(|account| CrsTarget {
                    account,
                    platform: "openai",
                    kind: "api_key",
                    account_type: "api_key",
                }),
        );
        targets
    }
}

#[derive(Clone, Copy)]
struct CrsTarget<'a> {
    account: &'a CrsAccount,
    platform: &'static str,
    kind: &'static str,
    account_type: &'static str,
}

fn claude_account_type(account: &CrsAccount) -> Option<&'static str> {
    match account.auth_type.trim().to_ascii_lowercase().as_str() {
        "" | "oauth" => Some("oauth"),
        "setup-token" | "setup_token" => Some("setup_token"),
        _ => None,
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CrsAccount {
    #[serde(default)]
    kind: String,
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    auth_type: String,
    #[serde(default)]
    is_active: bool,
    #[serde(default)]
    schedulable: bool,
    #[serde(default)]
    priority: i32,
    #[serde(default)]
    max_concurrent_tasks: i32,
    #[serde(default)]
    status: String,
    #[serde(default)]
    proxy: Option<CrsProxy>,
    #[serde(default)]
    credentials: Map<String, Value>,
    #[serde(default)]
    extra: Map<String, Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CrsProxy {
    #[serde(default)]
    protocol: String,
    #[serde(default)]
    host: String,
    #[serde(default)]
    port: i32,
    #[serde(default)]
    username: String,
    #[serde(default)]
    password: String,
}

async fn preview(
    State(state): State<AppState>,
    Json(input): Json<SyncInput>,
) -> ApiResult<Json<Value>> {
    let exported = fetch_export(&state, &input).await?;
    let existing: Vec<String> = sqlx::query_scalar(
        "SELECT crs_account_id FROM accounts WHERE crs_account_id IS NOT NULL \
         AND parent_account_id IS NULL",
    )
    .fetch_all(&state.pool)
    .await?;
    let existing = existing.into_iter().collect::<HashSet<_>>();
    let mut new_accounts = Vec::new();
    let mut existing_accounts = Vec::new();
    for target in exported.data.supported() {
        let value = preview_value(target);
        if existing.contains(target.account.id.trim()) {
            existing_accounts.push(value);
        } else {
            new_accounts.push(value);
        }
    }
    Ok(Json(json!({"data": {
        "new_accounts": new_accounts,
        "existing_accounts": existing_accounts,
        "unsupported_count": exported.data.unsupported_count()
    }})))
}

fn preview_value(target: CrsTarget<'_>) -> Value {
    let account = target.account;
    json!({
        "crs_account_id": account.id,
        "kind": if account.kind.trim().is_empty() { target.kind } else { account.kind.as_str() },
        "name": account_name(account),
        "platform": target.platform,
        "type": target.account_type,
    })
}

async fn sync(
    State(state): State<AppState>,
    Json(input): Json<SyncInput>,
) -> ApiResult<Json<Value>> {
    let exported = fetch_export(&state, &input).await?;
    let selected = input
        .selected_new_account_ids
        .iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<HashSet<_>>();
    let mut created = 0;
    let mut updated = 0;
    let mut skipped = exported.data.unsupported_count();
    let mut failed = 0;
    let mut items = Vec::new();
    for target in exported.data.supported() {
        let source = target.account;
        let existing_id: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM accounts WHERE crs_account_id = ? AND parent_account_id IS NULL",
        )
        .bind(source.id.trim())
        .fetch_optional(&state.pool)
        .await?;
        if existing_id.is_none() && !selected.contains(source.id.trim()) {
            skipped += 1;
            items.push(sync_item(source, "skipped", Some("not selected")));
            continue;
        }
        match sync_account(&state, target, existing_id, input.sync_proxies).await {
            Ok("created") => {
                created += 1;
                items.push(sync_item(source, "created", None));
            }
            Ok(_) => {
                updated += 1;
                items.push(sync_item(source, "updated", None));
            }
            Err(error) => {
                failed += 1;
                items.push(sync_item(source, "failed", Some(&error.message)));
            }
        }
    }
    state.model_cache.lock().await.clear();
    state.tls_clients.lock().await.clear();
    Ok(Json(json!({"data": {
        "created": created, "updated": updated, "skipped": skipped, "failed": failed,
        "items": items
    }})))
}

fn sync_item(account: &CrsAccount, action: &str, error: Option<&str>) -> Value {
    json!({
        "crs_account_id": account.id,
        "kind": account.kind,
        "name": account_name(account),
        "action": action,
        "error": error,
    })
}

async fn sync_account(
    state: &AppState,
    target: CrsTarget<'_>,
    existing_id: Option<i64>,
    sync_proxies: bool,
) -> ApiResult<&'static str> {
    let source = target.account;
    let crs_id = source.id.trim();
    if crs_id.is_empty() || crs_id.chars().count() > 200 {
        return Err(ApiError::bad_request(
            "INVALID_CRS_ACCOUNT",
            "CRS account id is invalid",
        ));
    }
    if existing_id.is_some() && !(target.platform == "openai" && target.kind == "oauth") {
        let shadows: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM accounts WHERE parent_account_id = ?")
                .bind(existing_id)
                .fetch_one(&state.pool)
                .await?;
        if shadows != 0 {
            return Err(ApiError::bad_request(
                "CRS_SPARK_PARENT_CONFLICT",
                "delete the Spark shadow before changing its OpenAI OAuth parent",
            ));
        }
    }
    let existing = if let Some(id) = existing_id {
        Some(load_existing(state, id).await?)
    } else {
        None
    };
    let existing_credentials = existing
        .as_ref()
        .filter(|account| account.platform == target.platform && account.kind == target.kind)
        .map(|account| account.credentials.clone())
        .unwrap_or_default();
    let credentials = credentials_from_crs(target, existing_credentials)?;
    let source_base_url = source
        .credentials
        .get("base_url")
        .and_then(Value::as_str)
        .map(clean_crs_base_url)
        .filter(|value| !value.is_empty());
    let base_url = match (source_base_url, existing.as_ref()) {
        (Some(value), _) => normalize_account_base_url(&value, target.kind, target.platform)?,
        (None, Some(account)) if account.platform == target.platform => account.base_url.clone(),
        (None, _) => normalize_account_base_url("", target.kind, target.platform)?,
    };
    let encrypted = state.crypto.encrypt(
        &serde_json::to_vec(&credentials)
            .map_err(|_| ApiError::internal("credential serialization failed"))?,
    )?;
    let proxy_id = if sync_proxies {
        sync_proxy(state, source.proxy.as_ref(), &account_name(source)).await?
    } else {
        None
    };
    let enabled =
        source.is_active && source.schedulable && !source.status.eq_ignore_ascii_case("error");
    let last_error = source
        .status
        .eq_ignore_ascii_case("error")
        .then_some("CRS account status is error");
    let priority = if (1..=100).contains(&source.priority) {
        source.priority
    } else if let Some(account) = &existing {
        account.priority
    } else {
        50
    };
    let notes = source
        .description
        .trim()
        .chars()
        .take(1000)
        .collect::<String>();
    let concurrency = if source.max_concurrent_tasks > 0 {
        source.max_concurrent_tasks
    } else if let Some(account) = &existing {
        account.concurrency
    } else {
        3
    };
    if let Some(id) = existing_id {
        let proxy_sql = if sync_proxies && proxy_id.is_some() {
            proxy_id
        } else {
            current_proxy(state, id).await?
        };
        sqlx::query(
            "UPDATE accounts SET name = ?, kind = ?, platform = ?, account_type = ?, \
             base_url = ?, encrypted_credentials = ?, priority = ?, concurrency = ?, \
             enabled = ?, proxy_id = ?, notes = ?, \
             cooldown_until = NULL, last_error = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
        )
        .bind(account_name(source))
        .bind(target.kind)
        .bind(target.platform)
        .bind(target.account_type)
        .bind(base_url)
        .bind(encrypted)
        .bind(priority)
        .bind(concurrency)
        .bind(enabled)
        .bind(proxy_sql)
        .bind(notes)
        .bind(last_error)
        .bind(id)
        .execute(&state.pool)
        .await?;
        if target.platform == "openai" && target.kind == "oauth" && sync_proxies {
            sqlx::query(
                "UPDATE accounts SET proxy_id = ?, updated_at = CURRENT_TIMESTAMP \
                 WHERE parent_account_id = ?",
            )
            .bind(proxy_sql)
            .bind(id)
            .execute(&state.pool)
            .await?;
        }
        Ok("updated")
    } else {
        sqlx::query(
            "INSERT INTO accounts (name, kind, platform, account_type, base_url, \
             encrypted_credentials, priority, concurrency, enabled, proxy_id, notes, \
             crs_account_id, last_error) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(account_name(source))
        .bind(target.kind)
        .bind(target.platform)
        .bind(target.account_type)
        .bind(base_url)
        .bind(encrypted)
        .bind(priority)
        .bind(concurrency)
        .bind(enabled)
        .bind(proxy_id)
        .bind(notes)
        .bind(crs_id)
        .bind(last_error)
        .execute(&state.pool)
        .await?;
        Ok("created")
    }
}

async fn current_proxy(state: &AppState, id: i64) -> ApiResult<Option<i64>> {
    Ok(
        sqlx::query_scalar("SELECT proxy_id FROM accounts WHERE id = ?")
            .bind(id)
            .fetch_one(&state.pool)
            .await?,
    )
}

struct ExistingSyncAccount {
    kind: String,
    platform: String,
    base_url: String,
    priority: i32,
    concurrency: i32,
    credentials: Credentials,
}

async fn load_existing(state: &AppState, id: i64) -> ApiResult<ExistingSyncAccount> {
    let row: (String, String, String, i32, i32, String) = sqlx::query_as(
        "SELECT kind, platform, base_url, priority, concurrency, \
         encrypted_credentials FROM accounts WHERE id = ?",
    )
    .bind(id)
    .fetch_one(&state.pool)
    .await?;
    let credentials = serde_json::from_slice::<Credentials>(&state.crypto.decrypt(&row.5)?)
        .map_err(|_| ApiError::internal("stored account credentials are malformed"))?;
    Ok(ExistingSyncAccount {
        kind: row.0,
        platform: row.1,
        base_url: row.2,
        priority: row.3,
        concurrency: row.4,
        credentials,
    })
}

fn clean_crs_base_url(value: &str) -> String {
    let value = value.trim().trim_end_matches('/');
    value.strip_suffix("/v1").unwrap_or(value).to_string()
}

fn credentials_from_crs(
    target: CrsTarget<'_>,
    mut credentials: Credentials,
) -> ApiResult<Credentials> {
    let source = target.account;
    let string = |key: &str| {
        source
            .credentials
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    if target.kind == "api_key" {
        credentials.api_key = string("api_key").or(credentials.api_key);
        if credentials.api_key.is_none() {
            return Err(ApiError::bad_request(
                "CRS_API_KEY_REQUIRED",
                "CRS account is missing api_key",
            ));
        }
        credentials.access_token = None;
        credentials.refresh_token = None;
        credentials.id_token = None;
        credentials.expires_at = None;
        credentials.email = None;
        credentials.chatgpt_account_id = None;
        credentials.client_id = None;
        credentials.token_type = None;
        credentials.scope = None;
        credentials.org_uuid = None;
        credentials.account_uuid = None;
        return Ok(credentials);
    }
    credentials.access_token = string("access_token").or(credentials.access_token);
    credentials.refresh_token = string("refresh_token").or(credentials.refresh_token);
    if credentials.access_token.is_none() && credentials.refresh_token.is_none() {
        return Err(ApiError::bad_request(
            "CRS_OAUTH_TOKEN_REQUIRED",
            "CRS account is missing OAuth tokens",
        ));
    }
    credentials.id_token = string("id_token").or(credentials.id_token);
    credentials.email = string("email")
        .or_else(|| {
            source
                .extra
                .get("crs_email")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .or(credentials.email);
    credentials.chatgpt_account_id = string("chatgpt_account_id")
        .or_else(|| string("account_id"))
        .or(credentials.chatgpt_account_id);
    credentials.client_id = string("client_id").or(credentials.client_id);
    credentials.token_type = string("token_type").or(credentials.token_type);
    credentials.scope = string("scope").or(credentials.scope);
    credentials.expires_at = source
        .credentials
        .get("expires_at")
        .and_then(parse_timestamp)
        .or(credentials.expires_at);
    credentials.api_key = None;
    if target.platform == "anthropic" {
        credentials.org_uuid = string("org_uuid").or(credentials.org_uuid);
        credentials.account_uuid = string("account_uuid").or(credentials.account_uuid);
        credentials.chatgpt_account_id = None;
        credentials.id_token = None;
    } else {
        if credentials.token_type.is_none() {
            credentials.token_type = Some("Bearer".into());
        }
        credentials.org_uuid = None;
        credentials.account_uuid = None;
    }
    Ok(credentials)
}

fn parse_timestamp(value: &Value) -> Option<i64> {
    value.as_i64().or_else(|| {
        value.as_str().and_then(|raw| {
            raw.parse::<i64>().ok().or_else(|| {
                DateTime::parse_from_rfc3339(raw)
                    .ok()
                    .map(|value| value.timestamp())
            })
        })
    })
}

async fn sync_proxy(
    state: &AppState,
    proxy: Option<&CrsProxy>,
    account_name: &str,
) -> ApiResult<Option<i64>> {
    let Some(proxy) = proxy else { return Ok(None) };
    let protocol = match proxy.protocol.trim().to_ascii_lowercase().as_str() {
        "socks" | "socks5h" => "socks5",
        "http" => "http",
        "https" => "https",
        "socks5" => "socks5",
        _ => return Ok(None),
    };
    if proxy.host.trim().is_empty() || !(1..=65535).contains(&proxy.port) {
        return Ok(None);
    }
    let host = if proxy.host.contains(':') && !proxy.host.starts_with('[') {
        format!("[{}]", proxy.host.trim())
    } else {
        proxy.host.trim().to_string()
    };
    let mut url = Url::parse(&format!("{protocol}://{host}:{}", proxy.port))
        .map_err(|_| ApiError::bad_request("INVALID_CRS_PROXY", "CRS proxy is invalid"))?;
    if !proxy.username.trim().is_empty() {
        url.set_username(proxy.username.trim()).map_err(|_| {
            ApiError::bad_request("INVALID_CRS_PROXY", "CRS proxy username is invalid")
        })?;
    }
    if !proxy.password.is_empty() {
        url.set_password(Some(&proxy.password)).map_err(|_| {
            ApiError::bad_request("INVALID_CRS_PROXY", "CRS proxy password is invalid")
        })?;
    }
    let rows: Vec<(i64, String)> =
        sqlx::query_as("SELECT id, encrypted_url FROM proxies ORDER BY id")
            .fetch_all(&state.pool)
            .await?;
    for (id, encrypted) in rows {
        if state
            .crypto
            .decrypt(&encrypted)
            .ok()
            .and_then(|raw| String::from_utf8(raw).ok())
            .is_some_and(|raw| raw == url.as_str())
        {
            return Ok(Some(id));
        }
    }
    let encrypted = state.crypto.encrypt(url.as_str().as_bytes())?;
    let name = format!("crs-{}", account_name.trim())
        .chars()
        .take(80)
        .collect::<String>();
    let id = sqlx::query(
        "INSERT INTO proxies (name, encrypted_url, enabled, fallback_mode, expiry_warn_days) \
         VALUES (?, ?, 1, 'none', 7)",
    )
    .bind(if name == "crs-" { "crs-proxy" } else { &name })
    .bind(encrypted)
    .execute(&state.pool)
    .await?
    .last_insert_rowid();
    Ok(Some(id))
}

fn account_name(account: &CrsAccount) -> String {
    let name = account.name.trim();
    if name.is_empty() {
        format!("CRS {}", account.id.trim())
    } else {
        name.chars().take(120).collect()
    }
}

async fn fetch_export(state: &AppState, input: &SyncInput) -> ApiResult<ExportResponse> {
    let base = validate_base_url(&input.base_url)?;
    if input.username.trim().is_empty() || input.password.is_empty() {
        return Err(ApiError::bad_request(
            "CRS_CREDENTIALS_REQUIRED",
            "CRS username and password are required",
        ));
    }
    let login_url = format!("{base}/web/auth/login");
    let response = state
        .client
        .post(login_url)
        .timeout(Duration::from_secs(20))
        .json(&json!({"username": input.username.trim(), "password": input.password}))
        .send()
        .await
        .map_err(|error| upstream_error("CRS_LOGIN_FAILED", "CRS login failed", error))?;
    let status = response.status();
    let body = limited_body(response, 1024 * 1024).await?;
    if !status.is_success() {
        return Err(ApiError::bad_request(
            "CRS_LOGIN_FAILED",
            format!("CRS login returned HTTP {status}"),
        ));
    }
    let login: LoginResponse = serde_json::from_slice(&body).map_err(|_| {
        ApiError::bad_request("CRS_LOGIN_FAILED", "CRS login returned invalid JSON")
    })?;
    if !login.success || login.token.trim().is_empty() {
        return Err(ApiError::bad_request(
            "CRS_LOGIN_FAILED",
            nonempty_message(&login.message, &login.error, "CRS login was rejected"),
        ));
    }
    let response = state
        .client
        .get(format!(
            "{base}/admin/sync/export-accounts?include_secrets=true"
        ))
        .timeout(Duration::from_secs(20))
        .bearer_auth(login.token.trim())
        .send()
        .await
        .map_err(|error| upstream_error("CRS_EXPORT_FAILED", "CRS export failed", error))?;
    let status = response.status();
    let body = limited_body(response, RESPONSE_LIMIT).await?;
    if !status.is_success() {
        return Err(ApiError::bad_request(
            "CRS_EXPORT_FAILED",
            format!("CRS export returned HTTP {status}"),
        ));
    }
    let exported: ExportResponse = serde_json::from_slice(&body).map_err(|_| {
        ApiError::bad_request("CRS_EXPORT_FAILED", "CRS export returned invalid JSON")
    })?;
    if !exported.success {
        return Err(ApiError::bad_request(
            "CRS_EXPORT_FAILED",
            nonempty_message(
                &exported.message,
                &exported.error,
                "CRS export was rejected",
            ),
        ));
    }
    Ok(exported)
}

async fn limited_body(response: reqwest::Response, limit: usize) -> ApiResult<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(ApiError::bad_request(
            "CRS_RESPONSE_TOO_LARGE",
            "CRS response is too large",
        ));
    }
    let body = response.bytes().await.map_err(|error| {
        upstream_error("CRS_RESPONSE_FAILED", "failed to read CRS response", error)
    })?;
    if body.len() > limit {
        return Err(ApiError::bad_request(
            "CRS_RESPONSE_TOO_LARGE",
            "CRS response is too large",
        ));
    }
    Ok(body.to_vec())
}

fn validate_base_url(raw: &str) -> ApiResult<String> {
    let url = Url::parse(raw.trim())
        .map_err(|_| ApiError::bad_request("INVALID_CRS_URL", "CRS URL is invalid"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ApiError::bad_request(
            "INVALID_CRS_URL",
            "CRS URL must be an HTTP(S) origin without credentials, query, or fragment",
        ));
    }
    Ok(raw.trim().trim_end_matches('/').to_string())
}

fn upstream_error(code: &'static str, message: &'static str, error: reqwest::Error) -> ApiError {
    ApiError::bad_request(code, format!("{message}: {error}"))
}

fn nonempty_message(primary: &str, secondary: &str, fallback: &str) -> String {
    [primary, secondary, fallback]
        .into_iter()
        .find(|value| !value.trim().is_empty())
        .unwrap_or(fallback)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::{get as axum_get, post as axum_post};

    #[test]
    fn validates_crs_origins_and_timestamps() {
        assert!(validate_base_url("http://127.0.0.1:3000/").is_ok());
        assert!(validate_base_url("file:///tmp/crs").is_err());
        assert!(validate_base_url("https://user@example.com").is_err());
        assert_eq!(
            parse_timestamp(&json!("2026-07-26T00:00:00Z")),
            Some(1_785_024_000)
        );
        assert_eq!(
            clean_crs_base_url("https://api.anthropic.com/v1/"),
            "https://api.anthropic.com"
        );
    }

    #[tokio::test]
    async fn syncs_openai_oauth_and_api_key_from_crs_protocol() {
        let upstream = Router::new()
            .route(
                "/web/auth/login",
                axum_post(|| async { Json(json!({"success": true, "token": "admin-token"})) }),
            )
            .route(
                "/admin/sync/export-accounts",
                axum_get(|| async {
                    Json(json!({"success": true, "data": {
                        "openaiOAuthAccounts": [{
                            "kind": "openai-oauth", "id": "oauth-1", "name": "CRS OAuth",
                            "description": "oauth notes", "isActive": true, "schedulable": true,
                            "priority": 20, "status": "active",
                            "credentials": {"access_token": "access", "refresh_token": "refresh",
                                "email": "oauth@example.com", "expires_at": "2026-07-26T00:00:00Z"}
                        }],
                        "openaiResponsesAccounts": [{
                            "kind": "openai-responses", "id": "key-1", "name": "CRS Key",
                            "isActive": true, "schedulable": true, "priority": 30,
                            "credentials": {"api_key": "sk-upstream", "base_url": "https://api.openai.com/v1"}
                        }]
                    }}))
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
        let (_directory, state) = crate::test_support::state().await;
        let input = SyncInput {
            base_url: format!("http://{address}"),
            username: "admin".into(),
            password: "secret".into(),
            sync_proxies: false,
            selected_new_account_ids: vec!["oauth-1".into(), "key-1".into()],
        };
        let Json(result) = sync(State(state.clone()), Json(input)).await.unwrap();
        assert_eq!(
            result.pointer("/data/created").and_then(Value::as_i64),
            Some(2)
        );
        let rows: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT crs_account_id, kind, notes FROM accounts ORDER BY crs_account_id",
        )
        .fetch_all(&state.pool)
        .await
        .unwrap();
        assert_eq!(
            rows,
            vec![
                ("key-1".into(), "api_key".into(), "".into()),
                ("oauth-1".into(), "oauth".into(), "oauth notes".into()),
            ]
        );
        server.abort();
    }

    #[tokio::test]
    async fn previews_and_syncs_claude_and_openai_account_matrix() {
        let upstream = Router::new()
            .route(
                "/web/auth/login",
                axum_post(|| async { Json(json!({"success": true, "token": "admin-token"})) }),
            )
            .route(
                "/admin/sync/export-accounts",
                axum_get(|| async {
                    Json(json!({"success": true, "data": {
                        "claudeAccounts": [{
                            "kind": "claude", "id": "claude-oauth", "name": "Claude OAuth",
                            "authType": "oauth", "description": "synced Claude account",
                            "isActive": true, "schedulable": true, "priority": 12,
                            "status": "active", "credentials": {
                                "access_token": "new-claude-access", "expires_at": "2026-07-26T00:00:00Z",
                                "org_uuid": "org-1", "account_uuid": "account-1"
                            }
                        }, {
                            "kind": "claude", "id": "claude-setup", "name": "Claude Setup",
                            "authType": "setup-token", "isActive": true, "schedulable": true,
                            "priority": 14, "credentials": {"access_token": "setup-access"}
                        }, {
                            "kind": "claude", "id": "claude-unknown", "name": "Unsupported Claude",
                            "authType": "cookie", "credentials": {"access_token": "ignored"}
                        }],
                        "claudeConsoleAccounts": [{
                            "kind": "claude-console", "id": "claude-key", "name": "Claude Console",
                            "isActive": true, "schedulable": true, "priority": 16,
                            "maxConcurrentTasks": 8, "credentials": {
                                "api_key": "sk-ant-key", "base_url": "https://api.anthropic.com/v1/"
                            }
                        }],
                        "openaiOAuthAccounts": [{
                            "kind": "openai-oauth", "id": "openai-oauth", "name": "OpenAI OAuth",
                            "isActive": true, "schedulable": true, "priority": 18,
                            "credentials": {"access_token": "openai-access"}
                        }],
                        "geminiOAuthAccounts": [{"id": "gemini-1"}]
                    }}))
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
        let (_directory, state) = crate::test_support::state().await;
        let existing = Credentials {
            access_token: Some("old-claude-access".into()),
            refresh_token: Some("preserved-refresh".into()),
            email: Some("preserved@example.com".into()),
            ..Default::default()
        };
        let encrypted = state
            .crypto
            .encrypt(&serde_json::to_vec(&existing).unwrap())
            .unwrap();
        sqlx::query(
            "INSERT INTO accounts (name, kind, platform, account_type, base_url, \
             encrypted_credentials, priority, concurrency, crs_account_id) \
             VALUES ('Old Claude', 'oauth', 'anthropic', 'oauth', \
             'https://api.anthropic.com', ?, 50, 6, 'claude-oauth')",
        )
        .bind(encrypted)
        .execute(&state.pool)
        .await
        .unwrap();
        let input = SyncInput {
            base_url: format!("http://{address}"),
            username: "admin".into(),
            password: "secret".into(),
            sync_proxies: false,
            selected_new_account_ids: vec![
                "claude-setup".into(),
                "claude-key".into(),
                "openai-oauth".into(),
            ],
        };
        let Json(previewed) = preview(State(state.clone()), Json(input.clone()))
            .await
            .unwrap();
        assert_eq!(
            previewed
                .pointer("/data/existing_accounts/0/type")
                .and_then(Value::as_str),
            Some("oauth")
        );
        assert_eq!(
            previewed
                .pointer("/data/new_accounts/0/platform")
                .and_then(Value::as_str),
            Some("anthropic")
        );
        assert_eq!(
            previewed
                .pointer("/data/new_accounts/0/type")
                .and_then(Value::as_str),
            Some("setup_token")
        );
        assert_eq!(
            previewed
                .pointer("/data/unsupported_count")
                .and_then(Value::as_u64),
            Some(2)
        );

        let Json(result) = sync(State(state.clone()), Json(input)).await.unwrap();
        assert_eq!(
            result.pointer("/data/created").and_then(Value::as_i64),
            Some(3)
        );
        assert_eq!(
            result.pointer("/data/updated").and_then(Value::as_i64),
            Some(1)
        );
        assert_eq!(
            result.pointer("/data/skipped").and_then(Value::as_i64),
            Some(2)
        );
        let rows: Vec<(String, String, String, String, String, i32)> = sqlx::query_as(
            "SELECT crs_account_id, platform, kind, account_type, base_url, concurrency \
             FROM accounts ORDER BY crs_account_id",
        )
        .fetch_all(&state.pool)
        .await
        .unwrap();
        assert_eq!(
            rows,
            vec![
                (
                    "claude-key".into(),
                    "anthropic".into(),
                    "api_key".into(),
                    "api_key".into(),
                    "https://api.anthropic.com".into(),
                    8,
                ),
                (
                    "claude-oauth".into(),
                    "anthropic".into(),
                    "oauth".into(),
                    "oauth".into(),
                    "https://api.anthropic.com".into(),
                    6,
                ),
                (
                    "claude-setup".into(),
                    "anthropic".into(),
                    "oauth".into(),
                    "setup_token".into(),
                    "https://api.anthropic.com".into(),
                    3,
                ),
                (
                    "openai-oauth".into(),
                    "openai".into(),
                    "oauth".into(),
                    "oauth".into(),
                    "https://chatgpt.com/backend-api/codex".into(),
                    3,
                ),
            ]
        );
        let encrypted: String = sqlx::query_scalar(
            "SELECT encrypted_credentials FROM accounts WHERE crs_account_id = 'claude-oauth'",
        )
        .fetch_one(&state.pool)
        .await
        .unwrap();
        let credentials: Credentials =
            serde_json::from_slice(&state.crypto.decrypt(&encrypted).unwrap()).unwrap();
        assert_eq!(
            credentials.access_token.as_deref(),
            Some("new-claude-access")
        );
        assert_eq!(
            credentials.refresh_token.as_deref(),
            Some("preserved-refresh")
        );
        assert_eq!(credentials.email.as_deref(), Some("preserved@example.com"));
        assert_eq!(credentials.org_uuid.as_deref(), Some("org-1"));
        assert_eq!(credentials.account_uuid.as_deref(), Some("account-1"));
        server.abort();
    }
}
