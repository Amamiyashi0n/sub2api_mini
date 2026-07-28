use std::{path::Path, str::FromStr, time::Duration};

use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};

use crate::{
    crypto::{hash_password, verify_password},
    error::{ApiError, ApiResult},
};

pub async fn connect(
    path: &Path,
    admin_username: &str,
    admin_password: &str,
) -> ApiResult<SqlitePool> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|_| ApiError::config("cannot create database directory"))?;
    }

    let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))
        .map_err(|_| ApiError::config("invalid SQLite path"))?
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .statement_cache_capacity(32)
        .pragma("cache_size", "-1024")
        .busy_timeout(Duration::from_secs(5));

    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .idle_timeout(Duration::from_secs(30))
        .connect_with(options)
        .await?;

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .map_err(|error| {
            tracing::error!(%error, "database migration failed");
            ApiError::internal("database migration failed")
        })?;

    let admin_username = admin_username.trim();
    if admin_username.is_empty() {
        return Err(ApiError::config(
            "SUB2API_MINI_ADMIN_USERNAME cannot be empty",
        ));
    }

    let existing: Option<(i64, String, String)> = sqlx::query_as(
        "SELECT id, username, password_hash FROM users WHERE role = 'admin' ORDER BY id LIMIT 1",
    )
    .fetch_optional(&pool)
    .await?;
    if let Some((id, username, password_hash)) = existing {
        let credentials_changed =
            username != admin_username || !verify_password(admin_password, &password_hash);
        if credentials_changed {
            sqlx::query(
                "UPDATE users SET username = ?, display_name = ?, password_hash = ?, enabled = 1, \
                 updated_at = CURRENT_TIMESTAMP WHERE id = ?",
            )
            .bind(admin_username)
            .bind(admin_username)
            .bind(hash_password(admin_password)?)
            .bind(id)
            .execute(&pool)
            .await
            .map_err(|error| match error {
                sqlx::Error::Database(ref database) if database.is_unique_violation() => {
                    ApiError::config("SUB2API_MINI_ADMIN_USERNAME is already in use")
                }
                other => other.into(),
            })?;
            sqlx::query("DELETE FROM auth_sessions WHERE user_id = ?")
                .bind(id)
                .execute(&pool)
                .await?;
        }
    } else {
        let password_hash = hash_password(admin_password)?;
        sqlx::query(
            "INSERT INTO users (username, display_name, password_hash, role) VALUES (?, ?, ?, 'admin')",
        )
        .bind(admin_username)
        .bind(admin_username)
        .bind(password_hash)
        .execute(&pool)
        .await?;
    }

    sqlx::query("DELETE FROM auth_sessions WHERE datetime(expires_at) <= CURRENT_TIMESTAMP")
        .execute(&pool)
        .await?;

    Ok(pool)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn removed_feature_schema_stays_absent() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("removed-rewards.sqlite3");
        let pool = connect(&path, "admin", "test-password").await.unwrap();
        let tables: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN (\
             'promo_codes', 'promo_usages', 'affiliate_profiles', 'affiliate_invites', \
             'affiliate_rebates', 'affiliate_transfers', 'invitation_codes', \
             'invitation_uses', 'payment_provider_instances', 'payment_orders', \
             'payment_events', 'payment_refunds', 'wxpay_payment_oauth_flows', \
             'external_auth_providers', 'external_auth_identities', \
             'external_oauth_flows', 'external_oauth_pending', \
             'user_external_attributes')",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let settings: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM app_settings \
             WHERE key IN ('promo_code_enabled', 'invitation_required') \
             OR key LIKE 'affiliate_%' OR key LIKE 'payment_%'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(tables, 0);
        assert_eq!(settings, 0);
    }

    #[tokio::test]
    async fn configured_admin_credentials_replace_existing_values() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("admin.sqlite3");

        let pool = connect(&path, "admin", "old-password").await.unwrap();
        pool.close().await;
        let pool = connect(&path, "replacement-admin", "replacement-password")
            .await
            .unwrap();
        let (username, display_name, password_hash): (String, String, String) = sqlx::query_as(
            "SELECT username, display_name, password_hash FROM users WHERE role = 'admin'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        assert_eq!(username, "replacement-admin");
        assert_eq!(display_name, "replacement-admin");
        assert!(verify_password("replacement-password", &password_hash));
        assert!(!verify_password("old-password", &password_hash));
    }
}
