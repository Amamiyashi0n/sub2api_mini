use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use reqwest::Client;
use serde_json::Value;
use sqlx::SqlitePool;
use tokio::sync::{Mutex, Notify, OwnedSemaphorePermit, RwLock, Semaphore};

use crate::{
    config::Config,
    crypto::Crypto,
    error::{ApiError, ApiResult},
    models::{Account, AccountRow},
};

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub crypto: Crypto,
    pub config: Config,
    pub client: Client,
    pub scheduler: Arc<Scheduler>,
    pub oauth_flows: Arc<Mutex<HashMap<String, OAuthFlow>>>,
    pub claude_oauth_flows: Arc<Mutex<HashMap<String, ClaudeOAuthFlow>>>,
    pub oauth_refresh_locks: Arc<Mutex<HashMap<i64, Arc<Mutex<()>>>>>,
    pub model_cache: Arc<Mutex<HashMap<i64, CachedModels>>>,
    pub vertex_tokens: Arc<Mutex<HashMap<i64, CachedVertexToken>>>,
    pub login_attempts: Arc<Mutex<HashMap<String, Vec<Instant>>>>,
    pub runtime_settings: Arc<RwLock<RuntimeSettings>>,
    pub started_at: Instant,
    pub active_requests: Arc<AtomicUsize>,
    pub prompt_audit_slots: Arc<DynamicSlots>,
    pub tls_clients: Arc<Mutex<HashMap<String, Client>>>,
}

impl AppState {
    pub fn new(pool: SqlitePool, crypto: Crypto, config: Config) -> ApiResult<Self> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .pool_idle_timeout(Duration::from_secs(90))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| ApiError::config("failed to initialize HTTP client"))?;
        Ok(Self {
            pool,
            crypto,
            config,
            client,
            scheduler: Arc::new(Scheduler::default()),
            oauth_flows: Arc::new(Mutex::new(HashMap::new())),
            claude_oauth_flows: Arc::new(Mutex::new(HashMap::new())),
            oauth_refresh_locks: Arc::new(Mutex::new(HashMap::new())),
            model_cache: Arc::new(Mutex::new(HashMap::new())),
            vertex_tokens: Arc::new(Mutex::new(HashMap::new())),
            login_attempts: Arc::new(Mutex::new(HashMap::new())),
            runtime_settings: Arc::new(RwLock::new(RuntimeSettings::default())),
            started_at: Instant::now(),
            active_requests: Arc::new(AtomicUsize::new(0)),
            prompt_audit_slots: Arc::new(DynamicSlots::default()),
            tls_clients: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub async fn load_runtime_settings(&self) -> ApiResult<()> {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT key, value FROM app_settings WHERE key IN \
             ('retry_attempts', 'model_cache_seconds', 'cooldown_5xx_seconds', 'cooldown_429_seconds')",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut settings = RuntimeSettings::default();
        for (key, value) in rows {
            match key.as_str() {
                "retry_attempts" => settings.retry_attempts = value.parse().unwrap_or(3),
                "model_cache_seconds" => {
                    settings.model_cache_seconds = value.parse().unwrap_or(300)
                }
                "cooldown_5xx_seconds" => {
                    settings.cooldown_5xx_seconds = value.parse().unwrap_or(15)
                }
                "cooldown_429_seconds" => {
                    settings.cooldown_429_seconds = value.parse().unwrap_or(60)
                }
                _ => {}
            }
        }
        settings.validate()?;
        *self.runtime_settings.write().await = settings;
        Ok(())
    }

    pub async fn client_for_account(&self, account: &Account) -> ApiResult<Client> {
        if account.row.proxy_active != Some(true) {
            if account.row.proxy_id.is_none() {
                return crate::tls_fingerprint::client_for_account(self, account).await;
            }
            return Err(ApiError::new(
                http::StatusCode::SERVICE_UNAVAILABLE,
                "PROXY_UNAVAILABLE",
                "the account proxy is disabled or expired",
            ));
        }
        crate::tls_fingerprint::client_for_account(self, account).await
    }

    pub async fn client_for_connection(
        &self,
        proxy_id: Option<i64>,
        tls_fingerprint_profile_id: Option<i64>,
    ) -> ApiResult<Client> {
        let proxy_url = self.effective_proxy_url(proxy_id).await?;
        crate::tls_fingerprint::client_for_settings(
            self,
            tls_fingerprint_profile_id,
            proxy_url.as_deref(),
        )
        .await
    }

    async fn effective_proxy_url(&self, proxy_id: Option<i64>) -> ApiResult<Option<String>> {
        let Some(proxy_id) = proxy_id else {
            return Ok(None);
        };
        let selection: Option<(bool, Option<String>)> = sqlx::query_as(
            "SELECT CASE WHEN proxies.enabled = 1 AND (proxies.expires_at IS NULL OR \
             datetime(proxies.expires_at) > CURRENT_TIMESTAMP) THEN 1 \
             WHEN proxies.fallback_mode = 'direct' THEN 1 \
             WHEN proxies.fallback_mode = 'proxy' AND backup_proxies.enabled = 1 AND \
             (backup_proxies.expires_at IS NULL OR datetime(backup_proxies.expires_at) > \
             CURRENT_TIMESTAMP) THEN 1 ELSE 0 END, \
             CASE WHEN proxies.enabled = 1 AND (proxies.expires_at IS NULL OR \
             datetime(proxies.expires_at) > CURRENT_TIMESTAMP) THEN proxies.encrypted_url \
             WHEN proxies.fallback_mode = 'proxy' AND backup_proxies.enabled = 1 AND \
             (backup_proxies.expires_at IS NULL OR datetime(backup_proxies.expires_at) > \
             CURRENT_TIMESTAMP) THEN backup_proxies.encrypted_url ELSE NULL END \
             FROM proxies LEFT JOIN proxies AS backup_proxies \
             ON backup_proxies.id = proxies.backup_proxy_id WHERE proxies.id = ?",
        )
        .bind(proxy_id)
        .fetch_optional(&self.pool)
        .await?;
        let (available, encrypted_url) = selection.ok_or_else(|| {
            ApiError::new(
                http::StatusCode::SERVICE_UNAVAILABLE,
                "PROXY_UNAVAILABLE",
                "the selected proxy no longer exists",
            )
        })?;
        if !available {
            return Err(ApiError::new(
                http::StatusCode::SERVICE_UNAVAILABLE,
                "PROXY_UNAVAILABLE",
                "the selected proxy is disabled or expired",
            ));
        }
        encrypted_url
            .map(|value| {
                String::from_utf8(self.crypto.decrypt(&value)?)
                    .map_err(|_| ApiError::internal("stored proxy URL is malformed"))
            })
            .transpose()
    }

    pub async fn resolve_account(&self, mut row: AccountRow) -> ApiResult<Account> {
        let Some(parent_id) = row.parent_account_id else {
            return row.decrypt(&self.crypto);
        };
        let parent: Option<(
            String,
            String,
            Option<i64>,
            Option<String>,
            Option<bool>,
            Option<String>,
            Option<i64>,
        )> = sqlx::query_as(
            "SELECT accounts.encrypted_credentials, accounts.base_url, accounts.proxy_id, \
             proxies.name, CASE WHEN proxies.id IS NULL THEN NULL WHEN proxies.enabled = 1 \
             AND (proxies.expires_at IS NULL OR datetime(proxies.expires_at) > CURRENT_TIMESTAMP) \
             THEN 1 WHEN proxies.fallback_mode = 'direct' THEN 1 WHEN proxies.fallback_mode = 'proxy' \
             AND backup_proxies.enabled = 1 AND (backup_proxies.expires_at IS NULL OR \
             datetime(backup_proxies.expires_at) > CURRENT_TIMESTAMP) THEN 1 ELSE 0 END, \
             CASE WHEN proxies.enabled = 1 AND (proxies.expires_at IS NULL OR \
             datetime(proxies.expires_at) > CURRENT_TIMESTAMP) THEN proxies.encrypted_url \
             WHEN proxies.fallback_mode = 'proxy' AND backup_proxies.enabled = 1 AND \
             (backup_proxies.expires_at IS NULL OR datetime(backup_proxies.expires_at) > \
             CURRENT_TIMESTAMP) THEN backup_proxies.encrypted_url ELSE NULL END, \
             accounts.tls_fingerprint_profile_id \
             FROM accounts LEFT JOIN proxies ON proxies.id = accounts.proxy_id \
             LEFT JOIN proxies AS backup_proxies ON backup_proxies.id = proxies.backup_proxy_id \
             WHERE accounts.id = ? AND accounts.kind = 'oauth' AND accounts.parent_account_id IS NULL",
        )
        .bind(parent_id)
        .fetch_optional(&self.pool)
        .await?;
        let (credentials, base_url, proxy_id, proxy_name, proxy_active, proxy_url, tls_profile_id) =
            parent.ok_or_else(|| ApiError::not_found("Spark parent account not found"))?;
        row.encrypted_credentials = credentials;
        row.base_url = base_url;
        row.proxy_id = proxy_id;
        row.proxy_name = proxy_name;
        row.proxy_active = proxy_active;
        row.encrypted_proxy_url = proxy_url;
        row.tls_fingerprint_profile_id = tls_profile_id;
        row.decrypt(&self.crypto)
    }
}

#[derive(Default)]
pub struct DynamicSlots {
    active: AtomicUsize,
    notify: Notify,
}

impl DynamicSlots {
    pub async fn acquire(self: &Arc<Self>, limit: usize) -> DynamicSlotPermit {
        let limit = limit.max(1);
        loop {
            let notified = self.notify.notified();
            let mut active = self.active.load(Ordering::Acquire);
            while active < limit {
                match self.active.compare_exchange_weak(
                    active,
                    active + 1,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(_) => return DynamicSlotPermit(self.clone()),
                    Err(current) => active = current,
                }
            }
            notified.await;
        }
    }

    pub fn active(&self) -> usize {
        self.active.load(Ordering::Acquire)
    }
}

pub struct DynamicSlotPermit(Arc<DynamicSlots>);

impl Drop for DynamicSlotPermit {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::AcqRel);
        self.0.notify.notify_one();
    }
}

impl AppState {
    pub fn active_request_count(&self) -> usize {
        self.active_requests.load(Ordering::Relaxed)
    }

    pub fn track_request(&self) -> ActiveRequestGuard {
        self.active_requests.fetch_add(1, Ordering::Relaxed);
        ActiveRequestGuard(self.active_requests.clone())
    }
}

pub struct ActiveRequestGuard(Arc<AtomicUsize>);

impl Drop for ActiveRequestGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

pub fn build_http_client(proxy_url: Option<&str>) -> ApiResult<Client> {
    let mut builder = Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(120))
        .pool_idle_timeout(Duration::from_secs(90))
        .redirect(reqwest::redirect::Policy::none());
    if let Some(proxy_url) = proxy_url {
        let proxy = reqwest::Proxy::all(proxy_url)
            .map_err(|_| ApiError::bad_request("INVALID_PROXY_URL", "proxy URL is invalid"))?;
        builder = builder.proxy(proxy);
    }
    builder
        .build()
        .map_err(|_| ApiError::config("failed to initialize HTTP client"))
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct RuntimeSettings {
    pub retry_attempts: usize,
    pub model_cache_seconds: u64,
    pub cooldown_5xx_seconds: i64,
    pub cooldown_429_seconds: i64,
}

impl Default for RuntimeSettings {
    fn default() -> Self {
        Self {
            retry_attempts: 3,
            model_cache_seconds: 300,
            cooldown_5xx_seconds: 15,
            cooldown_429_seconds: 60,
        }
    }
}

impl RuntimeSettings {
    pub fn validate(&self) -> ApiResult<()> {
        if !(1..=5).contains(&self.retry_attempts)
            || !(30..=3600).contains(&self.model_cache_seconds)
            || !(1..=600).contains(&self.cooldown_5xx_seconds)
            || !(1..=3600).contains(&self.cooldown_429_seconds)
        {
            return Err(ApiError::bad_request(
                "INVALID_RUNTIME_SETTINGS",
                "runtime settings are outside the supported range",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct OAuthFlow {
    pub verifier: String,
    pub created_at: Instant,
    pub account_id: Option<i64>,
    pub name: Option<String>,
    pub priority: i32,
    pub concurrency: i32,
    pub proxy_id: Option<i64>,
    pub notes: String,
    pub tls_fingerprint_profile_id: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct ClaudeOAuthFlow {
    pub verifier: String,
    pub state: String,
    pub setup_token: bool,
    pub proxy_id: Option<i64>,
    pub tls_fingerprint_profile_id: Option<i64>,
    pub created_at: Instant,
}

#[derive(Debug, Clone)]
pub struct CachedModels {
    pub value: Value,
    pub created_at: Instant,
}

#[derive(Debug, Clone)]
pub struct CachedVertexToken {
    pub token: String,
    pub credential_fingerprint: String,
    pub expires_at: Instant,
}

#[derive(Default)]
pub struct Scheduler {
    cursors: Mutex<HashMap<i32, usize>>,
    semaphores: Mutex<HashMap<i64, (i32, Arc<Semaphore>)>>,
}

pub struct ScheduledAccount {
    pub account: Account,
    pub _permit: OwnedSemaphorePermit,
}

impl Scheduler {
    pub async fn active_for(&self, account_id: i64, concurrency: i32) -> usize {
        self.semaphores
            .lock()
            .await
            .get(&account_id)
            .map(|(_, semaphore)| {
                (concurrency as usize).saturating_sub(semaphore.available_permits())
            })
            .unwrap_or_default()
    }

    pub async fn select(
        &self,
        state: &AppState,
        excluded: &HashSet<i64>,
        group_id: Option<i64>,
        platform: &str,
    ) -> ApiResult<ScheduledAccount> {
        let rows = sqlx::query_as::<_, AccountRow>(
            "SELECT accounts.id, accounts.name, accounts.kind, accounts.platform, \
             accounts.account_type, accounts.base_url, \
             accounts.encrypted_credentials, accounts.priority, accounts.concurrency, \
             accounts.enabled, accounts.cooldown_until, accounts.last_used_at, \
             accounts.last_error, accounts.proxy_id, proxies.name AS proxy_name, \
             CASE WHEN proxies.id IS NULL THEN NULL WHEN proxies.enabled = 1 \
             AND (proxies.expires_at IS NULL OR datetime(proxies.expires_at) > CURRENT_TIMESTAMP) \
             THEN 1 WHEN proxies.fallback_mode = 'direct' THEN 1 WHEN proxies.fallback_mode = 'proxy' \
             AND backup_proxies.enabled = 1 AND (backup_proxies.expires_at IS NULL OR \
             datetime(backup_proxies.expires_at) > CURRENT_TIMESTAMP) THEN 1 ELSE 0 END AS proxy_active, \
             CASE WHEN proxies.enabled = 1 AND (proxies.expires_at IS NULL OR \
             datetime(proxies.expires_at) > CURRENT_TIMESTAMP) THEN proxies.encrypted_url \
             WHEN proxies.fallback_mode = 'proxy' AND backup_proxies.enabled = 1 AND \
             (backup_proxies.expires_at IS NULL OR datetime(backup_proxies.expires_at) > CURRENT_TIMESTAMP) \
             THEN backup_proxies.encrypted_url ELSE NULL END AS encrypted_proxy_url, \
             accounts.parent_account_id, accounts.quota_dimension, accounts.notes, \
             accounts.crs_account_id, accounts.tls_fingerprint_profile_id, \
             accounts.created_at, accounts.updated_at FROM accounts \
             LEFT JOIN proxies ON proxies.id = accounts.proxy_id \
             LEFT JOIN proxies AS backup_proxies ON backup_proxies.id = proxies.backup_proxy_id \
             WHERE accounts.enabled = 1 AND (accounts.proxy_id IS NULL OR \
             (proxies.enabled = 1 AND (proxies.expires_at IS NULL OR \
             datetime(proxies.expires_at) > CURRENT_TIMESTAMP)) OR proxies.fallback_mode = 'direct' OR \
             (proxies.fallback_mode = 'proxy' AND backup_proxies.enabled = 1 AND \
             (backup_proxies.expires_at IS NULL OR datetime(backup_proxies.expires_at) > CURRENT_TIMESTAMP))) \
             AND (cooldown_until IS NULL OR datetime(cooldown_until) <= CURRENT_TIMESTAMP) \
             AND (? IS NULL OR EXISTS (SELECT 1 FROM account_groups \
             WHERE account_groups.account_id = accounts.id AND account_groups.group_id = ?)) \
             ORDER BY accounts.priority ASC, accounts.id ASC",
        )
        .bind(group_id)
        .bind(group_id)
        .fetch_all(&state.pool)
        .await?;
        let rows = rows
            .into_iter()
            .filter(|row| match platform {
                "openai_responses" => matches!(row.platform.as_str(), "openai" | "grok"),
                "openai_chat" | "openai_models" => {
                    matches!(row.platform.as_str(), "openai" | "grok")
                        || (row.platform == "gemini" && row.account_type == "api_key")
                        || (platform == "openai_models"
                            && row.platform == "antigravity"
                            && row.account_type == "upstream")
                }
                "anthropic_messages" => {
                    row.platform == "anthropic"
                        || (row.platform == "antigravity" && row.account_type == "upstream")
                }
                value => row.platform == value,
            })
            .collect::<Vec<_>>();

        let mut priorities = Vec::new();
        for row in &rows {
            if !priorities.contains(&row.priority) {
                priorities.push(row.priority);
            }
        }

        for priority in priorities {
            let group: Vec<_> = rows
                .iter()
                .filter(|row| row.priority == priority && !excluded.contains(&row.id))
                .cloned()
                .collect();
            if group.is_empty() {
                continue;
            }

            let start = {
                let mut cursors = self.cursors.lock().await;
                let cursor = cursors.entry(priority).or_default();
                let start = *cursor % group.len();
                *cursor = (*cursor + 1) % group.len();
                start
            };

            for offset in 0..group.len() {
                let row = group[(start + offset) % group.len()].clone();
                let semaphore = {
                    let mut semaphores = self.semaphores.lock().await;
                    let entry = semaphores.entry(row.id).or_insert_with(|| {
                        (
                            row.concurrency,
                            Arc::new(Semaphore::new(row.concurrency as usize)),
                        )
                    });
                    if entry.0 != row.concurrency {
                        *entry = (
                            row.concurrency,
                            Arc::new(Semaphore::new(row.concurrency as usize)),
                        );
                    }
                    entry.1.clone()
                };

                if let Ok(permit) = semaphore.try_acquire_owned() {
                    return Ok(ScheduledAccount {
                        account: state.resolve_account(row).await?,
                        _permit: permit,
                    });
                }
            }
        }

        Err(ApiError::new(
            http::StatusCode::SERVICE_UNAVAILABLE,
            "NO_UPSTREAM_ACCOUNT",
            "no upstream account is currently available",
        ))
    }
}

#[cfg(test)]
mod tests {
    use crate::test_support;

    #[tokio::test]
    async fn pending_connection_honors_backup_and_direct_proxy_fallbacks() {
        let (_directory, state) = test_support::state().await;
        let backup_url = state.crypto.encrypt(b"http://127.0.0.1:4128").unwrap();
        let backup_id =
            sqlx::query("INSERT INTO proxies (name, encrypted_url) VALUES ('oauth backup', ?)")
                .bind(backup_url)
                .execute(&state.pool)
                .await
                .unwrap()
                .last_insert_rowid();
        let primary_url = state.crypto.encrypt(b"http://127.0.0.1:3128").unwrap();
        let primary_id = sqlx::query(
            "INSERT INTO proxies (name, encrypted_url, fallback_mode, backup_proxy_id) \
             VALUES ('oauth primary', ?, 'proxy', ?)",
        )
        .bind(primary_url)
        .bind(backup_id)
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();

        assert_eq!(
            state.effective_proxy_url(Some(primary_id)).await.unwrap(),
            Some("http://127.0.0.1:3128".into())
        );
        sqlx::query("UPDATE proxies SET enabled = 0 WHERE id = ?")
            .bind(primary_id)
            .execute(&state.pool)
            .await
            .unwrap();
        assert_eq!(
            state.effective_proxy_url(Some(primary_id)).await.unwrap(),
            Some("http://127.0.0.1:4128".into())
        );

        sqlx::query(
            "UPDATE proxies SET fallback_mode = 'direct', backup_proxy_id = NULL WHERE id = ?",
        )
        .bind(primary_id)
        .execute(&state.pool)
        .await
        .unwrap();
        assert_eq!(
            state.effective_proxy_url(Some(primary_id)).await.unwrap(),
            None
        );
        assert!(
            state
                .client_for_connection(Some(primary_id), None)
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn pending_connection_rejects_an_unavailable_proxy() {
        let (_directory, state) = test_support::state().await;
        let encrypted = state.crypto.encrypt(b"socks5h://127.0.0.1:1080").unwrap();
        let proxy_id = sqlx::query(
            "INSERT INTO proxies (name, encrypted_url, enabled) VALUES ('offline', ?, 0)",
        )
        .bind(encrypted)
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();

        let error = state
            .client_for_connection(Some(proxy_id), None)
            .await
            .unwrap_err();
        assert_eq!(error.code, "PROXY_UNAVAILABLE");
        let missing = state
            .client_for_connection(Some(proxy_id + 100), None)
            .await
            .unwrap_err();
        assert_eq!(missing.code, "PROXY_UNAVAILABLE");
    }
}
