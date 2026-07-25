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
    pub oauth_refresh_locks: Arc<Mutex<HashMap<i64, Arc<Mutex<()>>>>>,
    pub model_cache: Arc<Mutex<HashMap<i64, CachedModels>>>,
    pub vertex_tokens: Arc<Mutex<HashMap<i64, CachedVertexToken>>>,
    pub login_attempts: Arc<Mutex<HashMap<String, Vec<Instant>>>>,
    pub runtime_settings: Arc<RwLock<RuntimeSettings>>,
    pub started_at: Instant,
    pub active_requests: Arc<AtomicUsize>,
    pub prompt_audit_slots: Arc<DynamicSlots>,
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
            oauth_refresh_locks: Arc::new(Mutex::new(HashMap::new())),
            model_cache: Arc::new(Mutex::new(HashMap::new())),
            vertex_tokens: Arc::new(Mutex::new(HashMap::new())),
            login_attempts: Arc::new(Mutex::new(HashMap::new())),
            runtime_settings: Arc::new(RwLock::new(RuntimeSettings::default())),
            started_at: Instant::now(),
            active_requests: Arc::new(AtomicUsize::new(0)),
            prompt_audit_slots: Arc::new(DynamicSlots::default()),
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

    pub fn client_for_account(&self, account: &Account) -> ApiResult<Client> {
        if account.row.proxy_id.is_none() {
            return Ok(self.client.clone());
        }
        if account.row.proxy_active != Some(true) {
            return Err(ApiError::new(
                http::StatusCode::SERVICE_UNAVAILABLE,
                "PROXY_UNAVAILABLE",
                "the account proxy is disabled or expired",
            ));
        }
        match account.proxy_url.as_deref() {
            Some(proxy_url) => build_http_client(Some(proxy_url)),
            None => Ok(self.client.clone()),
        }
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
             CURRENT_TIMESTAMP) THEN backup_proxies.encrypted_url ELSE NULL END \
             FROM accounts LEFT JOIN proxies ON proxies.id = accounts.proxy_id \
             LEFT JOIN proxies AS backup_proxies ON backup_proxies.id = proxies.backup_proxy_id \
             WHERE accounts.id = ? AND accounts.kind = 'oauth' AND accounts.parent_account_id IS NULL",
        )
        .bind(parent_id)
        .fetch_optional(&self.pool)
        .await?;
        let (credentials, base_url, proxy_id, proxy_name, proxy_active, proxy_url) =
            parent.ok_or_else(|| ApiError::not_found("Spark parent account not found"))?;
        row.encrypted_credentials = credentials;
        row.base_url = base_url;
        row.proxy_id = proxy_id;
        row.proxy_name = proxy_name;
        row.proxy_active = proxy_active;
        row.encrypted_proxy_url = proxy_url;
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
    pub async fn select(
        &self,
        state: &AppState,
        excluded: &HashSet<i64>,
        group_id: Option<i64>,
    ) -> ApiResult<ScheduledAccount> {
        let rows = sqlx::query_as::<_, AccountRow>(
            "SELECT accounts.id, accounts.name, accounts.kind, accounts.base_url, \
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
             accounts.parent_account_id, accounts.quota_dimension, \
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
