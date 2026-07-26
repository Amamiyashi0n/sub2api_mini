use std::collections::HashMap;

use axum::{
    Json, Router,
    extract::{Query, State},
    routing::get,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sqlx::FromRow;

use crate::{
    error::{ApiError, ApiResult},
    models::{Credentials, DEFAULT_OAUTH_BASE_URL, normalize_base_url},
    state::AppState,
};

const DATA_TYPE: &str = "sub2api-data";
const LEGACY_DATA_TYPE: &str = "sub2api-bundle";
const DATA_VERSION: i64 = 1;
const DATA_LIMIT: usize = 500;

pub fn admin_router() -> Router<AppState> {
    Router::new().route("/accounts/data", get(export_data).post(import_data))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DataPayload {
    #[serde(default, rename = "type")]
    data_type: String,
    #[serde(default)]
    version: i64,
    #[serde(default)]
    exported_at: String,
    #[serde(default)]
    proxies: Vec<DataProxy>,
    #[serde(default)]
    accounts: Vec<DataAccount>,
    #[serde(default)]
    skipped_shadows: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DataProxy {
    #[serde(default)]
    proxy_key: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    protocol: String,
    #[serde(default)]
    host: String,
    #[serde(default)]
    port: u16,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    password: Option<String>,
    #[serde(default)]
    status: String,
    #[serde(default)]
    expires_at: Option<i64>,
    #[serde(default)]
    fallback_mode: String,
    #[serde(default)]
    backup_proxy_name: String,
    #[serde(default = "default_expiry_warn_days")]
    expiry_warn_days: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DataAccount {
    #[serde(default)]
    name: String,
    #[serde(default)]
    platform: String,
    #[serde(default, rename = "type")]
    account_type: String,
    #[serde(default)]
    credentials: Map<String, Value>,
    #[serde(default)]
    extra: Map<String, Value>,
    #[serde(default)]
    proxy_key: Option<String>,
    #[serde(default)]
    concurrency: i32,
    #[serde(default)]
    priority: i32,
}

#[derive(Debug, Deserialize)]
struct ImportRequest {
    data: DataPayload,
}

#[derive(Debug, Serialize, Default)]
struct ImportResult {
    proxy_created: usize,
    proxy_reused: usize,
    proxy_failed: usize,
    account_created: usize,
    account_failed: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    errors: Vec<ImportError>,
}

#[derive(Debug, Serialize)]
struct ImportError {
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    proxy_key: Option<String>,
    message: String,
}

#[derive(Debug, FromRow)]
struct AccountBackupRow {
    id: i64,
    name: String,
    kind: String,
    base_url: String,
    encrypted_credentials: String,
    priority: i32,
    concurrency: i32,
    enabled: bool,
    proxy_id: Option<i64>,
}

#[derive(Debug, Deserialize, Default)]
struct ExportQuery {
    ids: Option<String>,
}

#[derive(Debug, FromRow)]
struct ProxyBackupRow {
    id: i64,
    name: String,
    encrypted_url: String,
    enabled: bool,
    fallback_mode: String,
    backup_proxy_id: Option<i64>,
    expiry_warn_days: i64,
    expires_at: Option<String>,
}

fn default_expiry_warn_days() -> i64 {
    7
}

async fn export_data(
    State(state): State<AppState>,
    Query(query): Query<ExportQuery>,
) -> ApiResult<Json<Value>> {
    let selected_ids = query.ids.as_deref().map(parse_export_ids).transpose()?;
    let proxy_rows = sqlx::query_as::<_, ProxyBackupRow>(
        "SELECT id, name, encrypted_url, enabled, fallback_mode, backup_proxy_id, \
         expiry_warn_days, expires_at FROM proxies ORDER BY id ASC",
    )
    .fetch_all(&state.pool)
    .await?;
    let mut proxy_names = HashMap::new();
    let mut proxy_keys = HashMap::new();
    for row in &proxy_rows {
        proxy_names.insert(row.id, row.name.clone());
        proxy_keys.insert(row.id, format!("proxy-{}", row.id));
    }

    let mut proxies = Vec::with_capacity(proxy_rows.len());
    for row in proxy_rows {
        let raw = decrypt_text(&state, &row.encrypted_url, "stored proxy URL is malformed")?;
        let url = validate_proxy_url(&raw)?;
        proxies.push(DataProxy {
            proxy_key: proxy_keys.get(&row.id).cloned().unwrap_or_default(),
            name: row.name,
            protocol: url.scheme().to_string(),
            host: url.host_str().unwrap_or_default().to_string(),
            port: url.port().unwrap_or_default(),
            username: decode_url_component(url.username())?,
            password: url
                .password()
                .map(decode_url_component)
                .transpose()?
                .flatten(),
            status: if row.enabled { "active" } else { "inactive" }.into(),
            expires_at: row
                .expires_at
                .as_deref()
                .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
                .map(|value| value.timestamp()),
            fallback_mode: row.fallback_mode,
            backup_proxy_name: row
                .backup_proxy_id
                .and_then(|id| proxy_names.get(&id).cloned())
                .unwrap_or_default(),
            expiry_warn_days: row.expiry_warn_days,
        });
    }

    let skipped_shadows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM accounts WHERE parent_account_id IS NOT NULL")
            .fetch_one(&state.pool)
            .await?;
    let rows = sqlx::query_as::<_, AccountBackupRow>(
        "SELECT id, name, kind, base_url, encrypted_credentials, priority, concurrency, enabled, \
         proxy_id FROM accounts WHERE parent_account_id IS NULL ORDER BY id ASC",
    )
    .fetch_all(&state.pool)
    .await?;
    let mut accounts = Vec::with_capacity(rows.len());
    for row in rows {
        if selected_ids
            .as_ref()
            .is_some_and(|ids| !ids.contains(&row.id))
        {
            continue;
        }
        let raw = state.crypto.decrypt(&row.encrypted_credentials)?;
        let credentials: Credentials = serde_json::from_slice(&raw)
            .map_err(|_| ApiError::internal("stored account credentials are malformed"))?;
        let mut credential_map = match serde_json::to_value(credentials)
            .map_err(|_| ApiError::internal("credential serialization failed"))?
        {
            Value::Object(map) => map,
            _ => Map::new(),
        };
        credential_map.insert("base_url".into(), Value::String(row.base_url));
        let mut extra = Map::new();
        extra.insert("mini_enabled".into(), Value::Bool(row.enabled));
        accounts.push(DataAccount {
            name: row.name,
            platform: "openai".into(),
            account_type: if row.kind == "oauth" {
                "oauth"
            } else {
                "apikey"
            }
            .into(),
            credentials: credential_map,
            extra,
            proxy_key: row.proxy_id.and_then(|id| proxy_keys.get(&id).cloned()),
            concurrency: row.concurrency,
            priority: row.priority,
        });
    }

    Ok(Json(json!({"data": DataPayload {
        data_type: DATA_TYPE.into(),
        version: DATA_VERSION,
        exported_at: Utc::now().to_rfc3339(),
        proxies,
        accounts,
        skipped_shadows,
    }})))
}

fn parse_export_ids(raw: &str) -> ApiResult<std::collections::HashSet<i64>> {
    let ids = raw
        .split(',')
        .filter(|value| !value.trim().is_empty())
        .map(|value| value.trim().parse::<i64>())
        .collect::<Result<std::collections::HashSet<_>, _>>()
        .map_err(|_| ApiError::bad_request("INVALID_ACCOUNT_IDS", "account ids are invalid"))?;
    if ids.is_empty() || ids.len() > DATA_LIMIT || ids.iter().any(|id| *id <= 0) {
        return Err(ApiError::bad_request(
            "INVALID_ACCOUNT_IDS",
            "account ids are invalid",
        ));
    }
    Ok(ids)
}

async fn import_data(
    State(state): State<AppState>,
    Json(input): Json<ImportRequest>,
) -> ApiResult<Json<Value>> {
    validate_header(&input.data)?;
    let mut result = ImportResult::default();
    let mut proxy_ids = existing_proxy_maps(&state).await?;
    let mut pending_fallbacks = Vec::new();

    for item in input.data.proxies {
        let key = if item.proxy_key.trim().is_empty() {
            build_proxy_key(&item)
        } else {
            item.proxy_key.trim().to_string()
        };
        match import_proxy(&state, &item, &mut proxy_ids).await {
            Ok((id, created)) => {
                if created {
                    result.proxy_created += 1;
                } else {
                    result.proxy_reused += 1;
                }
                proxy_ids.by_key.insert(key, id);
                proxy_ids.by_name.entry(item.name.clone()).or_insert(id);
                pending_fallbacks.push((id, item));
            }
            Err(error) => {
                result.proxy_failed += 1;
                result.errors.push(ImportError {
                    kind: "proxy",
                    name: nonempty(&item.name),
                    proxy_key: nonempty(&key),
                    message: error.message,
                });
            }
        }
    }

    for (id, item) in pending_fallbacks {
        if let Some(message) = apply_proxy_fallback(&state, id, &item, &proxy_ids).await? {
            result.errors.push(ImportError {
                kind: "proxy",
                name: nonempty(&item.name),
                proxy_key: nonempty(&item.proxy_key),
                message,
            });
        }
    }

    for item in input.data.accounts {
        match import_account(&state, &item, &proxy_ids.by_key).await {
            Ok(()) => result.account_created += 1,
            Err(error) => {
                result.account_failed += 1;
                result.errors.push(ImportError {
                    kind: "account",
                    name: nonempty(&item.name),
                    proxy_key: item.proxy_key.clone().filter(|value| !value.is_empty()),
                    message: error.message,
                });
            }
        }
    }

    Ok(Json(json!({"data": result})))
}

struct ProxyMaps {
    by_url: HashMap<String, i64>,
    by_key: HashMap<String, i64>,
    by_name: HashMap<String, i64>,
}

async fn existing_proxy_maps(state: &AppState) -> ApiResult<ProxyMaps> {
    let rows = sqlx::query_as::<_, (i64, String, String)>(
        "SELECT id, name, encrypted_url FROM proxies ORDER BY id ASC",
    )
    .fetch_all(&state.pool)
    .await?;
    let mut maps = ProxyMaps {
        by_url: HashMap::new(),
        by_key: HashMap::new(),
        by_name: HashMap::new(),
    };
    for (id, name, encrypted) in rows {
        if let Ok(raw) = state.crypto.decrypt(&encrypted)
            && let Ok(raw) = String::from_utf8(raw)
            && let Ok(url) = validate_proxy_url(&raw)
        {
            maps.by_url.insert(url.to_string(), id);
        }
        maps.by_name.entry(name).or_insert(id);
    }
    Ok(maps)
}

async fn import_proxy(
    state: &AppState,
    item: &DataProxy,
    maps: &mut ProxyMaps,
) -> ApiResult<(i64, bool)> {
    validate_proxy_name(&item.name)?;
    let url = proxy_url(item)?;
    if let Some(id) = maps.by_url.get(url.as_str()).copied() {
        sqlx::query(
            "UPDATE proxies SET enabled = ?, expires_at = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
        )
        .bind(normalize_proxy_enabled(&item.status)?)
        .bind(proxy_expiry(item.expires_at)?)
        .bind(id)
        .execute(&state.pool)
        .await?;
        return Ok((id, false));
    }
    let encrypted = state.crypto.encrypt(url.as_str().as_bytes())?;
    let id = sqlx::query(
        "INSERT INTO proxies (name, encrypted_url, enabled, fallback_mode, expiry_warn_days, expires_at) \
         VALUES (?, ?, ?, 'none', ?, ?)",
    )
    .bind(item.name.trim())
    .bind(encrypted)
    .bind(normalize_proxy_enabled(&item.status)?)
    .bind(validate_expiry_warn_days(item.expiry_warn_days)?)
    .bind(proxy_expiry(item.expires_at)?)
    .execute(&state.pool)
    .await?
    .last_insert_rowid();
    maps.by_url.insert(url.to_string(), id);
    Ok((id, true))
}

async fn apply_proxy_fallback(
    state: &AppState,
    id: i64,
    item: &DataProxy,
    maps: &ProxyMaps,
) -> ApiResult<Option<String>> {
    let mode = match item.fallback_mode.trim() {
        "" | "none" => "none",
        "direct" => "direct",
        "proxy" => "proxy",
        value => {
            disable_proxy_fallback(state, id).await?;
            return Ok(Some(format!("fallback_mode is invalid: {value}")));
        }
    };
    let backup_id = if mode == "proxy" {
        maps.by_name.get(item.backup_proxy_name.trim()).copied()
    } else {
        None
    };
    if mode == "proxy" && backup_id.is_none() {
        disable_proxy_fallback(state, id).await?;
        return Ok(Some(format!(
            "backup_proxy_name '{}' was not found; fallback was disabled",
            item.backup_proxy_name
        )));
    }
    if backup_id == Some(id) || creates_proxy_cycle(state, id, backup_id).await? {
        disable_proxy_fallback(state, id).await?;
        return Ok(Some(
            "proxy fallback cycle was rejected; fallback was disabled".into(),
        ));
    }
    sqlx::query(
        "UPDATE proxies SET fallback_mode = ?, backup_proxy_id = ?, expiry_warn_days = ?, \
         updated_at = CURRENT_TIMESTAMP WHERE id = ?",
    )
    .bind(mode)
    .bind(backup_id)
    .bind(validate_expiry_warn_days(item.expiry_warn_days)?)
    .bind(id)
    .execute(&state.pool)
    .await?;
    Ok(None)
}

async fn disable_proxy_fallback(state: &AppState, id: i64) -> ApiResult<()> {
    sqlx::query(
        "UPDATE proxies SET fallback_mode = 'none', backup_proxy_id = NULL, \
         updated_at = CURRENT_TIMESTAMP WHERE id = ?",
    )
    .bind(id)
    .execute(&state.pool)
    .await?;
    Ok(())
}

async fn creates_proxy_cycle(state: &AppState, id: i64, backup_id: Option<i64>) -> ApiResult<bool> {
    let Some(backup_id) = backup_id else {
        return Ok(false);
    };
    let found: i64 = sqlx::query_scalar(
        "WITH RECURSIVE chain(id) AS (SELECT ? UNION SELECT proxies.backup_proxy_id \
         FROM proxies JOIN chain ON proxies.id = chain.id WHERE proxies.backup_proxy_id IS NOT NULL) \
         SELECT COUNT(*) FROM chain WHERE id = ?",
    )
    .bind(backup_id)
    .bind(id)
    .fetch_one(&state.pool)
    .await?;
    Ok(found > 0)
}

async fn import_account(
    state: &AppState,
    item: &DataAccount,
    proxy_ids: &HashMap<String, i64>,
) -> ApiResult<()> {
    validate_account(item)?;
    let proxy_id = match item.proxy_key.as_deref().map(str::trim) {
        Some("") | None => None,
        Some(key) => Some(*proxy_ids.get(key).ok_or_else(|| {
            ApiError::bad_request(
                "PROXY_NOT_FOUND",
                format!("proxy_key '{key}' was not found"),
            )
        })?),
    };
    let mut credentials: Credentials =
        serde_json::from_value(Value::Object(item.credentials.clone()))
            .map_err(|_| ApiError::bad_request("INVALID_CREDENTIALS", "credentials are invalid"))?;
    let base_url = item
        .credentials
        .get("base_url")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let kind = match item.account_type.trim().to_ascii_lowercase().as_str() {
        "apikey" | "api_key" => {
            if credentials
                .api_key
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
            {
                return Err(ApiError::bad_request(
                    "API_KEY_REQUIRED",
                    "credentials.api_key is required",
                ));
            }
            credentials.api_key = credentials.api_key.map(|value| value.trim().to_string());
            "api_key"
        }
        "oauth" => {
            let no_access = credentials
                .access_token
                .as_deref()
                .is_none_or(|value| value.trim().is_empty());
            let no_refresh = credentials
                .refresh_token
                .as_deref()
                .is_none_or(|value| value.trim().is_empty());
            if no_access && no_refresh {
                return Err(ApiError::bad_request(
                    "OAUTH_TOKEN_REQUIRED",
                    "credentials must contain access_token or refresh_token",
                ));
            }
            "oauth"
        }
        value => {
            return Err(ApiError::bad_request(
                "UNSUPPORTED_ACCOUNT_TYPE",
                format!("OpenAI account type '{value}' is not supported"),
            ));
        }
    };
    let base_url = if kind == "oauth" {
        DEFAULT_OAUTH_BASE_URL.to_string()
    } else {
        normalize_base_url(base_url, kind)?
    };
    let encrypted = state.crypto.encrypt(
        &serde_json::to_vec(&credentials)
            .map_err(|_| ApiError::internal("credential serialization failed"))?,
    )?;
    sqlx::query(
        "INSERT INTO accounts (name, kind, base_url, encrypted_credentials, priority, concurrency, \
         enabled, proxy_id) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(item.name.trim())
    .bind(kind)
    .bind(base_url)
    .bind(encrypted)
    .bind(item.priority)
    .bind(if item.concurrency == 0 {
        3
    } else {
        item.concurrency
    })
    .bind(
        item.extra
            .get("mini_enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true),
    )
    .bind(proxy_id)
    .execute(&state.pool)
    .await?;
    Ok(())
}

fn validate_header(data: &DataPayload) -> ApiResult<()> {
    if !data.data_type.is_empty()
        && data.data_type != DATA_TYPE
        && data.data_type != LEGACY_DATA_TYPE
    {
        return Err(ApiError::bad_request(
            "UNSUPPORTED_DATA_TYPE",
            format!("unsupported data type: {}", data.data_type),
        ));
    }
    if data.version != 0 && data.version != DATA_VERSION {
        return Err(ApiError::bad_request(
            "UNSUPPORTED_DATA_VERSION",
            format!("unsupported data version: {}", data.version),
        ));
    }
    if data.proxies.len() > DATA_LIMIT || data.accounts.len() > DATA_LIMIT {
        return Err(ApiError::bad_request(
            "DATA_IMPORT_TOO_LARGE",
            format!("a data package may contain at most {DATA_LIMIT} proxies and accounts"),
        ));
    }
    Ok(())
}

fn validate_account(item: &DataAccount) -> ApiResult<()> {
    if item.name.trim().is_empty() || item.name.chars().count() > 120 {
        return Err(ApiError::bad_request(
            "INVALID_ACCOUNT_NAME",
            "account name must contain 1 to 120 characters",
        ));
    }
    if !item.platform.trim().eq_ignore_ascii_case("openai") {
        return Err(ApiError::bad_request(
            "UNSUPPORTED_ACCOUNT_PLATFORM",
            format!(
                "account platform '{}' is not supported by Mini",
                item.platform
            ),
        ));
    }
    if item.priority < 0 || !(0..=1000).contains(&item.concurrency) {
        return Err(ApiError::bad_request(
            "INVALID_ACCOUNT",
            "priority or concurrency is invalid",
        ));
    }
    if item.credentials.is_empty() {
        return Err(ApiError::bad_request(
            "CREDENTIALS_REQUIRED",
            "account credentials are required",
        ));
    }
    Ok(())
}

fn validate_proxy_name(value: &str) -> ApiResult<()> {
    if value.trim().is_empty() || value.chars().count() > 80 {
        return Err(ApiError::bad_request(
            "INVALID_PROXY_NAME",
            "proxy name must contain 1 to 80 characters",
        ));
    }
    Ok(())
}

fn normalize_proxy_enabled(value: &str) -> ApiResult<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "active" | "enabled" => Ok(true),
        "inactive" | "disabled" | "expired" => Ok(false),
        value => Err(ApiError::bad_request(
            "INVALID_PROXY_STATUS",
            format!("proxy status is invalid: {value}"),
        )),
    }
}

fn validate_expiry_warn_days(value: i64) -> ApiResult<i64> {
    if !(0..=365).contains(&value) {
        return Err(ApiError::bad_request(
            "INVALID_EXPIRY_WARNING",
            "expiry_warn_days must be between 0 and 365",
        ));
    }
    Ok(value)
}

fn proxy_expiry(value: Option<i64>) -> ApiResult<Option<String>> {
    value
        .map(|timestamp| {
            DateTime::<Utc>::from_timestamp(timestamp, 0)
                .map(|value| value.to_rfc3339())
                .ok_or_else(|| {
                    ApiError::bad_request("INVALID_PROXY_EXPIRY", "proxy expires_at is invalid")
                })
        })
        .transpose()
}

fn proxy_url(item: &DataProxy) -> ApiResult<url::Url> {
    let protocol = item.protocol.trim().to_ascii_lowercase();
    let host = item.host.trim();
    if host.is_empty() || item.port == 0 {
        return Err(ApiError::bad_request(
            "INVALID_PROXY_URL",
            "proxy host and port are required",
        ));
    }
    let host = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    let mut url = validate_proxy_url(&format!("{protocol}://{host}:{}", item.port))?;
    if let Some(username) = item.username.as_deref().filter(|value| !value.is_empty()) {
        url.set_username(username)
            .map_err(|_| ApiError::bad_request("INVALID_PROXY_URL", "proxy username is invalid"))?;
    }
    if let Some(password) = item.password.as_deref().filter(|value| !value.is_empty()) {
        url.set_password(Some(password))
            .map_err(|_| ApiError::bad_request("INVALID_PROXY_URL", "proxy password is invalid"))?;
    }
    Ok(url)
}

fn validate_proxy_url(value: &str) -> ApiResult<url::Url> {
    let url = url::Url::parse(value.trim())
        .map_err(|_| ApiError::bad_request("INVALID_PROXY_URL", "proxy URL is invalid"))?;
    if !matches!(url.scheme(), "http" | "https" | "socks5" | "socks5h")
        || url.host_str().is_none()
        || url.port().is_none()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ApiError::bad_request(
            "INVALID_PROXY_URL",
            "proxy URL must use http, https, socks5, or socks5h and include host and port",
        ));
    }
    Ok(url)
}

fn build_proxy_key(item: &DataProxy) -> String {
    format!(
        "{}|{}|{}|{}|{}",
        item.protocol.trim(),
        item.host.trim(),
        item.port,
        item.username.as_deref().unwrap_or_default().trim(),
        item.password.as_deref().unwrap_or_default().trim()
    )
}

fn decrypt_text(state: &AppState, value: &str, message: &'static str) -> ApiResult<String> {
    String::from_utf8(state.crypto.decrypt(value)?).map_err(|_| ApiError::internal(message))
}

fn decode_url_component(value: &str) -> ApiResult<Option<String>> {
    if value.is_empty() {
        return Ok(None);
    }
    percent_encoding::percent_decode_str(value)
        .decode_utf8()
        .map(|value| Some(value.into_owned()))
        .map_err(|_| ApiError::internal("stored proxy credentials are malformed"))
}

fn nonempty(value: &str) -> Option<String> {
    (!value.trim().is_empty()).then(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;

    fn sample_payload() -> DataPayload {
        DataPayload {
            data_type: DATA_TYPE.into(),
            version: DATA_VERSION,
            exported_at: String::new(),
            proxies: vec![DataProxy {
                proxy_key: "office".into(),
                name: "Office".into(),
                protocol: "http".into(),
                host: "127.0.0.1".into(),
                port: 3128,
                username: Some("alice".into()),
                password: Some("proxy-secret".into()),
                status: "active".into(),
                expires_at: None,
                fallback_mode: "none".into(),
                backup_proxy_name: String::new(),
                expiry_warn_days: 7,
            }],
            accounts: vec![DataAccount {
                name: "Primary".into(),
                platform: "openai".into(),
                account_type: "apikey".into(),
                credentials: Map::from_iter([
                    ("api_key".into(), Value::String("sk-upstream-secret".into())),
                    (
                        "base_url".into(),
                        Value::String("https://api.openai.com".into()),
                    ),
                ]),
                extra: Map::new(),
                proxy_key: Some("office".into()),
                concurrency: 4,
                priority: 20,
            }],
            skipped_shadows: 0,
        }
    }

    #[tokio::test]
    async fn imports_original_bundle_and_encrypts_all_secrets() {
        let (_directory, state) = test_support::state().await;
        let Json(value) = import_data(
            State(state.clone()),
            Json(ImportRequest {
                data: sample_payload(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(value["data"]["proxy_created"], 1);
        assert_eq!(value["data"]["account_created"], 1);
        let stored_proxy: String = sqlx::query_scalar("SELECT encrypted_url FROM proxies")
            .fetch_one(&state.pool)
            .await
            .unwrap();
        let stored_account: String =
            sqlx::query_scalar("SELECT encrypted_credentials FROM accounts")
                .fetch_one(&state.pool)
                .await
                .unwrap();
        assert!(!stored_proxy.contains("proxy-secret"));
        assert!(!stored_account.contains("sk-upstream-secret"));
        let proxy_id: Option<i64> = sqlx::query_scalar("SELECT proxy_id FROM accounts")
            .fetch_one(&state.pool)
            .await
            .unwrap();
        assert!(proxy_id.is_some());
    }

    #[tokio::test]
    async fn reports_unsupported_accounts_without_rolling_back_valid_items() {
        let (_directory, state) = test_support::state().await;
        let mut payload = sample_payload();
        let mut invalid = payload.accounts[0].clone();
        invalid.name = "Claude".into();
        invalid.platform = "anthropic".into();
        payload.accounts.push(invalid);
        let Json(value) = import_data(State(state.clone()), Json(ImportRequest { data: payload }))
            .await
            .unwrap();
        assert_eq!(value["data"]["account_created"], 1);
        assert_eq!(value["data"]["account_failed"], 1);
        assert_eq!(value["data"]["errors"][0]["kind"], "account");
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM accounts")
            .fetch_one(&state.pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn exported_bundle_round_trips_openai_credentials() {
        let (_directory, state) = test_support::state().await;
        let _ = import_data(
            State(state.clone()),
            Json(ImportRequest {
                data: sample_payload(),
            }),
        )
        .await
        .unwrap();
        let Json(value) = export_data(State(state), Query(ExportQuery::default()))
            .await
            .unwrap();
        assert_eq!(value["data"]["type"], DATA_TYPE);
        assert_eq!(value["data"]["accounts"][0]["type"], "apikey");
        assert_eq!(
            value["data"]["accounts"][0]["credentials"]["api_key"],
            "sk-upstream-secret"
        );
        assert_eq!(value["data"]["proxies"][0]["password"], "proxy-secret");
    }

    #[tokio::test]
    async fn export_skips_linked_spark_shadows() {
        let (_directory, state) = test_support::state().await;
        let credentials = Credentials {
            access_token: Some("parent-access".into()),
            refresh_token: Some("parent-refresh".into()),
            ..Default::default()
        };
        let encrypted = state
            .crypto
            .encrypt(&serde_json::to_vec(&credentials).unwrap())
            .unwrap();
        let parent_id = sqlx::query(
            "INSERT INTO accounts (name, kind, base_url, encrypted_credentials) \
             VALUES ('parent', 'oauth', ?, ?)",
        )
        .bind(DEFAULT_OAUTH_BASE_URL)
        .bind(&encrypted)
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        sqlx::query(
            "INSERT INTO accounts (name, kind, base_url, encrypted_credentials, parent_account_id, \
             quota_dimension) VALUES ('shadow', 'oauth', ?, ?, ?, 'spark')",
        )
        .bind(DEFAULT_OAUTH_BASE_URL)
        .bind(encrypted)
        .bind(parent_id)
        .execute(&state.pool)
        .await
        .unwrap();

        let Json(value) = export_data(State(state), Query(ExportQuery::default()))
            .await
            .unwrap();
        assert_eq!(value["data"]["accounts"].as_array().unwrap().len(), 1);
        assert_eq!(value["data"]["accounts"][0]["name"], "parent");
        assert_eq!(value["data"]["skipped_shadows"], 1);
    }

    #[test]
    fn rejects_oversized_or_future_bundles() {
        let mut payload = sample_payload();
        payload.version = 2;
        assert_eq!(
            validate_header(&payload).unwrap_err().code,
            "UNSUPPORTED_DATA_VERSION"
        );
        payload.version = 1;
        payload.accounts = vec![payload.accounts[0].clone(); DATA_LIMIT + 1];
        assert_eq!(
            validate_header(&payload).unwrap_err().code,
            "DATA_IMPORT_TOO_LARGE"
        );
    }
}
