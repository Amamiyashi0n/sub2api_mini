use axum::{Json, Router, extract::State, routing::get};
use serde_json::{Value, json};

use crate::{error::ApiResult, state::AppState};

pub fn router() -> Router<AppState> {
    Router::new().route("/setup/status", get(status))
}

async fn status(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let database_ok: bool = sqlx::query_scalar::<_, i64>("SELECT 1")
        .fetch_one(&state.pool)
        .await
        .is_ok();
    let migration_version: i64 =
        sqlx::query_scalar("SELECT COALESCE(MAX(version),0) FROM _sqlx_migrations")
            .fetch_one(&state.pool)
            .await?;
    let journal_mode: String = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(&state.pool)
        .await?;
    let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
        .fetch_one(&state.pool)
        .await?;
    let admin_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE role='admin' AND enabled=1")
            .fetch_one(&state.pool)
            .await?;
    let data_directory = state
        .config
        .database_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("/data/sub2api_mini"));
    let data_directory_ready = std::fs::metadata(data_directory)
        .map(|metadata| metadata.is_dir() && !metadata.permissions().readonly())
        .unwrap_or(false);
    Ok(Json(json!({"data": {
        "needs_setup": false,
        "step": "complete",
        "initialization_mode": "environment_file",
        "version": env!("CARGO_PKG_VERSION"),
        "checks": {
            "configuration_loaded": true,
            "master_key_loaded": true,
            "database_connected": database_ok,
            "data_directory_ready": data_directory_ready,
            "sqlite_wal": journal_mode.eq_ignore_ascii_case("wal"),
            "foreign_keys": foreign_keys == 1,
            "admin_configured": admin_count == 1,
            "single_process_runtime": true,
            "redis_required": false
        },
        "database": {
            "engine": "SQLite",
            "migration_version": migration_version,
            "journal_mode": journal_mode,
            "max_connections": 4
        },
        "listeners": {
            "main": state.config.bind.to_string(),
            "oauth_callback": state.config.callback_bind.to_string()
        }
    }})))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;

    #[tokio::test]
    async fn reports_initialized_sqlite_runtime_without_secrets() {
        let (_directory, state) = test_support::state().await;
        let Json(value) = status(State(state)).await.unwrap();
        assert_eq!(value["data"]["needs_setup"], false);
        assert_eq!(value["data"]["checks"]["database_connected"], true);
        assert_eq!(value["data"]["checks"]["redis_required"], false);
        assert_eq!(value["data"]["database"]["max_connections"], 4);
        let serialized = value.to_string();
        assert!(!serialized.contains("test-password"));
        assert!(!serialized.contains("CQkJCQkJCQkJ"));
    }
}
