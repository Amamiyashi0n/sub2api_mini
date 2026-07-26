use std::{collections::HashSet, time::Duration};

use axum::{Json, Router, extract::State, routing::post};
use chrono::DateTime;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use url::Url;

use crate::{
    error::{ApiError, ApiResult},
    models::{Credentials, DEFAULT_OAUTH_BASE_URL, normalize_base_url},
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
    claude_accounts: Vec<Value>,
    #[serde(default)]
    claude_console_accounts: Vec<Value>,
    #[serde(default)]
    gemini_oauth_accounts: Vec<Value>,
    #[serde(default)]
    gemini_api_key_accounts: Vec<Value>,
}

impl ExportData {
    fn unsupported_count(&self) -> usize {
        self.claude_accounts.len()
            + self.claude_console_accounts.len()
            + self.gemini_oauth_accounts.len()
            + self.gemini_api_key_accounts.len()
    }

    fn supported(&self) -> impl Iterator<Item = (&CrsAccount, &'static str)> {
        self.open_ai_oauth_accounts
            .iter()
            .map(|account| (account, "oauth"))
            .chain(
                self.open_ai_responses_accounts
                    .iter()
                    .map(|account| (account, "api_key")),
            )
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
    is_active: bool,
    #[serde(default)]
    schedulable: bool,
    #[serde(default)]
    priority: i32,
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
    for (account, kind) in exported.data.supported() {
        let value = preview_value(account, kind);
        if existing.contains(account.id.trim()) {
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

fn preview_value(account: &CrsAccount, kind: &str) -> Value {
    json!({
        "crs_account_id": account.id,
        "kind": if account.kind.trim().is_empty() { kind } else { account.kind.as_str() },
        "name": account_name(account),
        "platform": "openai",
        "type": kind,
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
    for (source, kind) in exported.data.supported() {
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
        match sync_account(&state, source, kind, existing_id, input.sync_proxies).await {
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
    source: &CrsAccount,
    kind: &str,
    existing_id: Option<i64>,
    sync_proxies: bool,
) -> ApiResult<&'static str> {
    let crs_id = source.id.trim();
    if crs_id.is_empty() || crs_id.chars().count() > 200 {
        return Err(ApiError::bad_request(
            "INVALID_CRS_ACCOUNT",
            "CRS account id is invalid",
        ));
    }
    if kind == "api_key" && existing_id.is_some() {
        let shadows: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM accounts WHERE parent_account_id = ?")
                .bind(existing_id)
                .fetch_one(&state.pool)
                .await?;
        if shadows != 0 {
            return Err(ApiError::bad_request(
                "CRS_SPARK_PARENT_CONFLICT",
                "delete the Spark shadow before changing its OAuth parent to API key",
            ));
        }
    }
    let existing_credentials = if let Some(id) = existing_id {
        let encrypted: String =
            sqlx::query_scalar("SELECT encrypted_credentials FROM accounts WHERE id = ?")
                .bind(id)
                .fetch_one(&state.pool)
                .await?;
        serde_json::from_slice::<Credentials>(&state.crypto.decrypt(&encrypted)?)
            .map_err(|_| ApiError::internal("stored account credentials are malformed"))?
    } else {
        Credentials::default()
    };
    let credentials = credentials_from_crs(source, kind, existing_credentials)?;
    let base_url = if kind == "oauth" {
        DEFAULT_OAUTH_BASE_URL.to_string()
    } else {
        normalize_base_url(
            source
                .credentials
                .get("base_url")
                .and_then(Value::as_str)
                .unwrap_or("https://api.openai.com"),
            "api_key",
        )?
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
    } else {
        50
    };
    let notes = source
        .description
        .trim()
        .chars()
        .take(1000)
        .collect::<String>();
    if let Some(id) = existing_id {
        let proxy_sql = if sync_proxies {
            proxy_id
        } else {
            current_proxy(state, id).await?
        };
        sqlx::query(
            "UPDATE accounts SET name = ?, kind = ?, base_url = ?, encrypted_credentials = ?, \
             priority = ?, concurrency = 3, enabled = ?, proxy_id = ?, notes = ?, \
             cooldown_until = NULL, last_error = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
        )
        .bind(account_name(source))
        .bind(kind)
        .bind(base_url)
        .bind(encrypted)
        .bind(priority)
        .bind(enabled)
        .bind(proxy_sql)
        .bind(notes)
        .bind(last_error)
        .bind(id)
        .execute(&state.pool)
        .await?;
        if kind == "oauth" && sync_proxies {
            sqlx::query(
                "UPDATE accounts SET proxy_id = ?, updated_at = CURRENT_TIMESTAMP \
                 WHERE parent_account_id = ?",
            )
            .bind(proxy_id)
            .bind(id)
            .execute(&state.pool)
            .await?;
        }
        Ok("updated")
    } else {
        sqlx::query(
            "INSERT INTO accounts (name, kind, base_url, encrypted_credentials, priority, \
             concurrency, enabled, proxy_id, notes, crs_account_id, last_error) \
             VALUES (?, ?, ?, ?, ?, 3, ?, ?, ?, ?, ?)",
        )
        .bind(account_name(source))
        .bind(kind)
        .bind(base_url)
        .bind(encrypted)
        .bind(priority)
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

fn credentials_from_crs(
    source: &CrsAccount,
    kind: &str,
    mut credentials: Credentials,
) -> ApiResult<Credentials> {
    let string = |key: &str| {
        source
            .credentials
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    if kind == "api_key" {
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
    credentials.expires_at = source
        .credentials
        .get("expires_at")
        .and_then(parse_timestamp)
        .or(credentials.expires_at);
    credentials.api_key = None;
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
}
