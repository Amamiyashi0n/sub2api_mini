use std::{
    collections::BTreeMap,
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

use serde_json::json;
use sqlx::SqlitePool;
use tokio::sync::mpsc;
use tracing::{
    Event, Subscriber,
    field::{Field, Visit},
};
use tracing_subscriber::{
    EnvFilter, Layer, Registry,
    layer::{Context, SubscriberExt},
    reload,
    util::SubscriberInitExt,
};

use crate::error::{ApiError, ApiResult};

static FILTER_HANDLE: OnceLock<reload::Handle<EnvFilter, Registry>> = OnceLock::new();
static DB_ENABLED: AtomicBool = AtomicBool::new(true);

#[derive(Debug)]
pub struct RuntimeLogEvent {
    level: String,
    target: String,
    message: String,
    request_id: Option<String>,
    fields_json: String,
}

struct RuntimeLogLayer {
    sender: mpsc::Sender<RuntimeLogEvent>,
}

impl<S> Layer<S> for RuntimeLogLayer
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        if !DB_ENABLED.load(Ordering::Relaxed) {
            return;
        }
        let metadata = event.metadata();
        if !metadata.target().starts_with("sub2api_mini") {
            return;
        }
        let mut visitor = SafeVisitor::default();
        event.record(&mut visitor);
        let message = visitor
            .message
            .unwrap_or_else(|| metadata.name().to_string());
        let fields_json = serde_json::to_string(&visitor.fields).unwrap_or_else(|_| "{}".into());
        let _ = self.sender.try_send(RuntimeLogEvent {
            level: metadata.level().as_str().to_ascii_lowercase(),
            target: metadata.target().chars().take(160).collect(),
            message: message.chars().take(1000).collect(),
            request_id: visitor
                .request_id
                .map(|value| value.chars().take(128).collect()),
            fields_json,
        });
    }
}

#[derive(Default)]
struct SafeVisitor {
    message: Option<String>,
    request_id: Option<String>,
    fields: BTreeMap<String, String>,
}

impl Visit for SafeVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        let name = field.name();
        if name.contains("password")
            || name.contains("secret")
            || name.contains("token")
            || name.contains("credential")
            || name.contains("body")
            || name.contains("prompt")
        {
            return;
        }
        let mut value = format!("{value:?}");
        if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
            value = value[1..value.len() - 1].to_string();
        }
        value = value.chars().take(500).collect();
        match name {
            "message" => self.message = Some(value),
            "request_id" => self.request_id = Some(value),
            "account_id" | "subscription_id" | "provider" | "code" | "status" | "error" => {
                self.fields.insert(name.to_string(), value);
            }
            _ => {}
        }
    }
}

pub fn init() -> mpsc::Receiver<RuntimeLogEvent> {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "sub2api_mini=info,tower_http=info".into());
    let (filter_layer, handle) = reload::Layer::new(filter);
    let _ = FILTER_HANDLE.set(handle);
    let (sender, receiver) = mpsc::channel(256);
    Registry::default()
        .with(filter_layer)
        .with(tracing_subscriber::fmt::layer().json())
        .with(RuntimeLogLayer { sender })
        .init();
    receiver
}

pub async fn configure_from_database(pool: &SqlitePool) -> ApiResult<()> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT key,value FROM app_settings WHERE key IN \
         ('runtime_log_level','runtime_log_db_enabled')",
    )
    .fetch_all(pool)
    .await?;
    let value = |key: &str| {
        rows.iter()
            .find(|row| row.0 == key)
            .map(|row| row.1.as_str())
    };
    set_level(value("runtime_log_level").unwrap_or("info"))?;
    set_db_enabled(
        value("runtime_log_db_enabled")
            .unwrap_or("true")
            .parse()
            .unwrap_or(true),
    );
    Ok(())
}

pub fn set_level(level: &str) -> ApiResult<()> {
    let level = level.trim().to_ascii_lowercase();
    if !matches!(
        level.as_str(),
        "trace" | "debug" | "info" | "warn" | "error"
    ) {
        return Err(ApiError::bad_request(
            "INVALID_LOG_LEVEL",
            "runtime log level is invalid",
        ));
    }
    let filter = EnvFilter::new(format!("sub2api_mini={level},tower_http=info"));
    FILTER_HANDLE
        .get()
        .ok_or_else(|| ApiError::internal("runtime log filter is not initialized"))?
        .reload(filter)
        .map_err(|_| ApiError::internal("runtime log filter could not be reloaded"))
}

pub fn set_db_enabled(enabled: bool) {
    DB_ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn db_enabled() -> bool {
    DB_ENABLED.load(Ordering::Relaxed)
}

pub fn start_sink(pool: SqlitePool, mut receiver: mpsc::Receiver<RuntimeLogEvent>) {
    tokio::spawn(async move {
        while let Some(event) = receiver.recv().await {
            let _ = sqlx::query(
                "INSERT INTO runtime_logs(level,target,message,request_id,fields_json) \
                 VALUES(?,?,?,?,?)",
            )
            .bind(event.level)
            .bind(event.target)
            .bind(event.message)
            .bind(event.request_id)
            .bind(event.fields_json)
            .execute(&pool)
            .await;
        }
    });
}

pub fn safe_config_json(level: &str) -> serde_json::Value {
    json!({"level": level, "db_enabled": db_enabled()})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_runtime_level() {
        assert_eq!(set_level("verbose").unwrap_err().code, "INVALID_LOG_LEVEL");
    }
}
