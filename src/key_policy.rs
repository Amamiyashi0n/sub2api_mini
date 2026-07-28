use std::net::IpAddr;

use chrono::{DateTime, Utc};
use ipnet::IpNet;
use serde::Serialize;
use serde_json::{Value, json};
use sqlx::{FromRow, SqlitePool};

use crate::{
    crypto::{random_token, token_hash},
    error::{ApiError, ApiResult},
    models::ApiKeyRow,
    state::AppState,
};

const MAX_POLICY_ITEMS: usize = 100;
const MAX_MICROUSD: i64 = 1_000_000_000_000_000;

#[derive(Debug, FromRow)]
pub struct KeyListRow {
    id: i64,
    user_id: Option<i64>,
    name: String,
    token_prefix: String,
    enabled: bool,
    last_used_at: Option<String>,
    last_used_ip: Option<String>,
    created_at: String,
    updated_at: String,
    expires_at: Option<String>,
    quota_tokens: i64,
    quota_cost_microusd: i64,
    quota_reset_at: Option<String>,
    allowed_models: String,
    group_id: Option<i64>,
    group_name: Option<String>,
    owner_username: Option<String>,
    ip_whitelist: String,
    ip_blacklist: String,
    rate_limit_5h_microusd: i64,
    rate_limit_1d_microusd: i64,
    rate_limit_7d_microusd: i64,
    rate_usage_reset_at: Option<String>,
    used_tokens: i64,
    used_cost_microusd: i64,
    usage_5h_microusd: i64,
    usage_1d_microusd: i64,
    usage_7d_microusd: i64,
}

#[derive(Debug, Serialize)]
struct KeyView {
    id: i64,
    user_id: Option<i64>,
    name: String,
    token_prefix: String,
    enabled: bool,
    status: &'static str,
    last_used_at: Option<String>,
    last_used_ip: Option<String>,
    created_at: String,
    updated_at: String,
    expires_at: Option<String>,
    quota_tokens: i64,
    quota_cost_microusd: i64,
    quota_reset_at: Option<String>,
    allowed_models: Vec<String>,
    group_id: Option<i64>,
    group_name: Option<String>,
    owner_username: Option<String>,
    ip_whitelist: Vec<String>,
    ip_blacklist: Vec<String>,
    rate_limit_5h_microusd: i64,
    rate_limit_1d_microusd: i64,
    rate_limit_7d_microusd: i64,
    rate_usage_reset_at: Option<String>,
    used_tokens: i64,
    used_cost_microusd: i64,
    usage_5h_microusd: i64,
    usage_1d_microusd: i64,
    usage_7d_microusd: i64,
}

impl KeyListRow {
    fn into_view(self) -> ApiResult<KeyView> {
        let allowed_models = serde_json::from_str(&self.allowed_models)
            .map_err(|_| ApiError::internal("stored API key model policy is malformed"))?;
        let ip_whitelist = stored_network_strings(&self.ip_whitelist)?;
        let ip_blacklist = stored_network_strings(&self.ip_blacklist)?;
        let expired = self
            .expires_at
            .as_deref()
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
            .is_some_and(|value| value <= Utc::now());
        let exhausted = (self.quota_tokens > 0 && self.used_tokens >= self.quota_tokens)
            || (self.quota_cost_microusd > 0
                && self.used_cost_microusd >= self.quota_cost_microusd)
            || (self.rate_limit_5h_microusd > 0
                && self.usage_5h_microusd >= self.rate_limit_5h_microusd)
            || (self.rate_limit_1d_microusd > 0
                && self.usage_1d_microusd >= self.rate_limit_1d_microusd)
            || (self.rate_limit_7d_microusd > 0
                && self.usage_7d_microusd >= self.rate_limit_7d_microusd);
        let status = if !self.enabled {
            "inactive"
        } else if expired {
            "expired"
        } else if exhausted {
            "quota_exhausted"
        } else {
            "active"
        };
        Ok(KeyView {
            id: self.id,
            user_id: self.user_id,
            name: self.name,
            token_prefix: self.token_prefix,
            enabled: self.enabled,
            status,
            last_used_at: self.last_used_at,
            last_used_ip: self.last_used_ip,
            created_at: self.created_at,
            updated_at: self.updated_at,
            expires_at: self.expires_at,
            quota_tokens: self.quota_tokens,
            quota_cost_microusd: self.quota_cost_microusd,
            quota_reset_at: self.quota_reset_at,
            allowed_models,
            group_id: self.group_id,
            group_name: self.group_name,
            owner_username: self.owner_username,
            ip_whitelist,
            ip_blacklist,
            rate_limit_5h_microusd: self.rate_limit_5h_microusd,
            rate_limit_1d_microusd: self.rate_limit_1d_microusd,
            rate_limit_7d_microusd: self.rate_limit_7d_microusd,
            rate_usage_reset_at: self.rate_usage_reset_at,
            used_tokens: self.used_tokens,
            used_cost_microusd: self.used_cost_microusd,
            usage_5h_microusd: self.usage_5h_microusd,
            usage_1d_microusd: self.usage_1d_microusd,
            usage_7d_microusd: self.usage_7d_microusd,
        })
    }
}

pub async fn list_keys(pool: &SqlitePool, owner_id: Option<i64>) -> ApiResult<Vec<Value>> {
    let rows = sqlx::query_as::<_, KeyListRow>(
        "SELECT keys.id, keys.user_id, keys.name, keys.token_prefix, keys.enabled, \
         keys.last_used_at, keys.last_used_ip, keys.created_at, keys.updated_at, keys.expires_at, \
         keys.quota_tokens, keys.quota_cost_microusd, keys.quota_reset_at, keys.allowed_models, \
         keys.group_id, groups.name AS group_name, users.username AS owner_username, \
         keys.ip_whitelist, keys.ip_blacklist, keys.rate_limit_5h_microusd, \
         keys.rate_limit_1d_microusd, keys.rate_limit_7d_microusd, keys.rate_usage_reset_at, \
         COALESCE((SELECT SUM(COALESCE(log.total_tokens, 0)) FROM usage_logs log \
           WHERE log.api_key_id = keys.id AND (keys.quota_reset_at IS NULL OR \
           datetime(log.created_at) >= datetime(keys.quota_reset_at))), 0) AS used_tokens, \
         COALESCE((SELECT SUM(log.cost_microusd) FROM usage_logs log \
           WHERE log.api_key_id = keys.id AND (keys.quota_reset_at IS NULL OR \
           datetime(log.created_at) >= datetime(keys.quota_reset_at))), 0) AS used_cost_microusd, \
         COALESCE((SELECT SUM(log.cost_microusd) FROM usage_logs log WHERE log.api_key_id = keys.id \
           AND datetime(log.created_at) >= datetime('now', '-5 hours') AND \
           (keys.rate_usage_reset_at IS NULL OR datetime(log.created_at) >= datetime(keys.rate_usage_reset_at))), 0) AS usage_5h_microusd, \
         COALESCE((SELECT SUM(log.cost_microusd) FROM usage_logs log WHERE log.api_key_id = keys.id \
           AND datetime(log.created_at) >= datetime('now', '-1 day') AND \
           (keys.rate_usage_reset_at IS NULL OR datetime(log.created_at) >= datetime(keys.rate_usage_reset_at))), 0) AS usage_1d_microusd, \
         COALESCE((SELECT SUM(log.cost_microusd) FROM usage_logs log WHERE log.api_key_id = keys.id \
           AND datetime(log.created_at) >= datetime('now', '-7 days') AND \
           (keys.rate_usage_reset_at IS NULL OR datetime(log.created_at) >= datetime(keys.rate_usage_reset_at))), 0) AS usage_7d_microusd \
         FROM api_keys keys LEFT JOIN users ON users.id = keys.user_id \
         LEFT JOIN groups ON groups.id = keys.group_id \
         WHERE (? IS NULL OR keys.user_id = ?) ORDER BY keys.id DESC",
    )
    .bind(owner_id)
    .bind(owner_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|row| {
            serde_json::to_value(row.into_view()?)
                .map_err(|_| ApiError::internal("cannot serialize API key policy"))
        })
        .collect()
}

pub fn validate_microusd(value: i64, field: &str) -> ApiResult<i64> {
    if !(0..=MAX_MICROUSD).contains(&value) {
        return Err(ApiError::bad_request(
            "INVALID_KEY_COST_LIMIT",
            format!("{field} must be between 0 and {MAX_MICROUSD} microusd"),
        ));
    }
    Ok(value)
}

pub async fn issue_token(pool: &SqlitePool, custom: Option<String>) -> ApiResult<String> {
    let token = match custom {
        Some(value) if !value.trim().is_empty() => {
            let value = value.trim();
            if !(20..=200).contains(&value.len())
                || !value.starts_with("sk-")
                || !value.is_ascii()
                || value
                    .bytes()
                    .any(|byte| !(byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')))
            {
                return Err(ApiError::bad_request(
                    "INVALID_CUSTOM_KEY",
                    "custom_key must start with sk-, contain 20-200 ASCII letters, numbers, hyphens or underscores",
                ));
            }
            value.to_string()
        }
        _ => format!("sk-mini_{}", random_token(32)?),
    };
    let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM api_keys WHERE token_hash = ?")
        .bind(token_hash(&token))
        .fetch_one(pool)
        .await?;
    if exists > 0 {
        return Err(ApiError::bad_request(
            "API_KEY_EXISTS",
            "the API key value is already in use",
        ));
    }
    Ok(token)
}

pub fn normalize_networks(values: Vec<String>, field: &str) -> ApiResult<Vec<String>> {
    let mut result = values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(|value| canonical_network(&value, field))
        .collect::<ApiResult<Vec<_>>>()?;
    if result.len() > MAX_POLICY_ITEMS {
        return Err(ApiError::bad_request(
            "INVALID_IP_POLICY",
            format!("{field} supports at most {MAX_POLICY_ITEMS} entries"),
        ));
    }
    result.sort();
    result.dedup();
    Ok(result)
}

fn canonical_network(value: &str, field: &str) -> ApiResult<String> {
    if value.contains('/') {
        value
            .parse::<IpNet>()
            .map(|network| network.trunc().to_string())
            .map_err(|_| {
                ApiError::bad_request(
                    "INVALID_IP_POLICY",
                    format!("{field} contains an invalid IP address or CIDR"),
                )
            })
    } else {
        value
            .parse::<IpAddr>()
            .map(|ip| ip.to_string())
            .map_err(|_| {
                ApiError::bad_request(
                    "INVALID_IP_POLICY",
                    format!("{field} contains an invalid IP address or CIDR"),
                )
            })
    }
}

fn stored_network_strings(value: &str) -> ApiResult<Vec<String>> {
    let values: Vec<String> = serde_json::from_str(value)
        .map_err(|_| ApiError::internal("stored API key IP policy is malformed"))?;
    normalize_networks(values, "stored IP policy")
        .map_err(|_| ApiError::internal("stored API key IP policy is malformed"))
}

fn matches_network(ip: IpAddr, value: &str) -> bool {
    if value.contains('/') {
        value
            .parse::<IpNet>()
            .is_ok_and(|network| network.contains(&ip))
    } else {
        value.parse::<IpAddr>() == Ok(ip)
    }
}

pub fn ip_is_allowed(ip: IpAddr, whitelist: &[String], blacklist: &[String]) -> bool {
    if blacklist.iter().any(|entry| matches_network(ip, entry)) {
        return false;
    }
    whitelist.is_empty() || whitelist.iter().any(|entry| matches_network(ip, entry))
}

pub async fn enforce(state: &AppState, row: &ApiKeyRow, peer_ip: Option<IpAddr>) -> ApiResult<()> {
    if let Some(expires_at) = row.expires_at.as_deref() {
        let expires_at = DateTime::parse_from_rfc3339(expires_at)
            .map_err(|_| ApiError::internal("stored API key expiry is malformed"))?;
        if expires_at <= Utc::now() {
            return Err(ApiError::new(
                http::StatusCode::FORBIDDEN,
                "API_KEY_EXPIRED",
                "API key has expired",
            ));
        }
    }

    let whitelist = stored_network_strings(&row.ip_whitelist)?;
    let blacklist = stored_network_strings(&row.ip_blacklist)?;
    if !whitelist.is_empty() || !blacklist.is_empty() {
        let peer_ip = peer_ip.ok_or_else(|| {
            ApiError::new(
                http::StatusCode::FORBIDDEN,
                "CLIENT_IP_UNAVAILABLE",
                "client IP is unavailable for this API key policy",
            )
        })?;
        if !ip_is_allowed(peer_ip, &whitelist, &blacklist) {
            return Err(ApiError::new(
                http::StatusCode::FORBIDDEN,
                "API_KEY_IP_FORBIDDEN",
                "client IP is not allowed by this API key",
            ));
        }
    }

    if row.quota_tokens == 0
        && row.quota_cost_microusd == 0
        && row.rate_limit_5h_microusd == 0
        && row.rate_limit_1d_microusd == 0
        && row.rate_limit_7d_microusd == 0
    {
        return Ok(());
    }

    let usage: (i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT \
         COALESCE(SUM(CASE WHEN ? IS NULL OR datetime(created_at) >= datetime(?) \
           THEN COALESCE(total_tokens, 0) ELSE 0 END), 0), \
         COALESCE(SUM(CASE WHEN ? IS NULL OR datetime(created_at) >= datetime(?) \
           THEN cost_microusd ELSE 0 END), 0), \
         COALESCE(SUM(CASE WHEN datetime(created_at) >= datetime('now', '-5 hours') AND \
           (? IS NULL OR datetime(created_at) >= datetime(?)) THEN cost_microusd ELSE 0 END), 0), \
         COALESCE(SUM(CASE WHEN datetime(created_at) >= datetime('now', '-1 day') AND \
           (? IS NULL OR datetime(created_at) >= datetime(?)) THEN cost_microusd ELSE 0 END), 0), \
         COALESCE(SUM(CASE WHEN datetime(created_at) >= datetime('now', '-7 days') AND \
           (? IS NULL OR datetime(created_at) >= datetime(?)) THEN cost_microusd ELSE 0 END), 0) \
         FROM usage_logs WHERE api_key_id = ?",
    )
    .bind(&row.quota_reset_at)
    .bind(&row.quota_reset_at)
    .bind(&row.quota_reset_at)
    .bind(&row.quota_reset_at)
    .bind(&row.rate_usage_reset_at)
    .bind(&row.rate_usage_reset_at)
    .bind(&row.rate_usage_reset_at)
    .bind(&row.rate_usage_reset_at)
    .bind(&row.rate_usage_reset_at)
    .bind(&row.rate_usage_reset_at)
    .bind(row.id)
    .fetch_one(&state.pool)
    .await?;

    if row.quota_tokens > 0 && usage.0 >= row.quota_tokens {
        return Err(limit_error(
            "API_KEY_QUOTA_EXHAUSTED",
            "API key token quota has been exhausted",
        ));
    }
    if row.quota_cost_microusd > 0 && usage.1 >= row.quota_cost_microusd {
        return Err(limit_error(
            "API_KEY_COST_QUOTA_EXHAUSTED",
            "API key cost quota has been exhausted",
        ));
    }
    for (limit, used, code, message) in [
        (
            row.rate_limit_5h_microusd,
            usage.2,
            "API_KEY_RATE_5H_EXCEEDED",
            "API key five-hour cost limit has been exhausted",
        ),
        (
            row.rate_limit_1d_microusd,
            usage.3,
            "API_KEY_RATE_1D_EXCEEDED",
            "API key daily cost limit has been exhausted",
        ),
        (
            row.rate_limit_7d_microusd,
            usage.4,
            "API_KEY_RATE_7D_EXCEEDED",
            "API key seven-day cost limit has been exhausted",
        ),
    ] {
        if limit > 0 && used >= limit {
            return Err(limit_error(code, message));
        }
    }
    Ok(())
}

fn limit_error(code: &'static str, message: &'static str) -> ApiError {
    ApiError::new(http::StatusCode::TOO_MANY_REQUESTS, code, message)
}

pub async fn batch_action(
    pool: &SqlitePool,
    owner_id: Option<i64>,
    ids: Vec<i64>,
    action: &str,
) -> ApiResult<Value> {
    let mut ids = ids.into_iter().filter(|id| *id > 0).collect::<Vec<_>>();
    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() || ids.len() > 200 {
        return Err(ApiError::bad_request(
            "INVALID_KEY_SELECTION",
            "select between 1 and 200 API keys",
        ));
    }
    if !matches!(
        action,
        "enable" | "disable" | "delete" | "reset_quota" | "reset_rate_limit"
    ) {
        return Err(ApiError::bad_request(
            "INVALID_KEY_BATCH_ACTION",
            "unsupported API key batch action",
        ));
    }

    let mut transaction = pool.begin().await?;
    let mut affected_ids = Vec::new();
    for id in ids {
        let result = match (action, owner_id) {
            ("delete", Some(owner)) => {
                sqlx::query("DELETE FROM api_keys WHERE id = ? AND user_id = ?")
                    .bind(id)
                    .bind(owner)
                    .execute(&mut *transaction)
                    .await?
            }
            ("delete", None) => {
                sqlx::query("DELETE FROM api_keys WHERE id = ?")
                    .bind(id)
                    .execute(&mut *transaction)
                    .await?
            }
            (_, Some(owner)) => {
                sqlx::query(batch_update_sql(action, true))
                    .bind(id)
                    .bind(owner)
                    .execute(&mut *transaction)
                    .await?
            }
            (_, None) => {
                sqlx::query(batch_update_sql(action, false))
                    .bind(id)
                    .execute(&mut *transaction)
                    .await?
            }
        };
        if result.rows_affected() > 0 {
            affected_ids.push(id);
        }
    }
    transaction.commit().await?;
    let affected = affected_ids.len();
    Ok(json!({"affected_ids": affected_ids, "affected": affected}))
}

fn batch_update_sql(action: &str, owned: bool) -> &'static str {
    match (action, owned) {
        ("enable", true) => {
            "UPDATE api_keys SET enabled = 1, updated_at = CURRENT_TIMESTAMP WHERE id = ? AND user_id = ?"
        }
        ("enable", false) => {
            "UPDATE api_keys SET enabled = 1, updated_at = CURRENT_TIMESTAMP WHERE id = ?"
        }
        ("disable", true) => {
            "UPDATE api_keys SET enabled = 0, updated_at = CURRENT_TIMESTAMP WHERE id = ? AND user_id = ?"
        }
        ("disable", false) => {
            "UPDATE api_keys SET enabled = 0, updated_at = CURRENT_TIMESTAMP WHERE id = ?"
        }
        ("reset_quota", true) => {
            "UPDATE api_keys SET quota_reset_at = CURRENT_TIMESTAMP, updated_at = CURRENT_TIMESTAMP WHERE id = ? AND user_id = ?"
        }
        ("reset_quota", false) => {
            "UPDATE api_keys SET quota_reset_at = CURRENT_TIMESTAMP, updated_at = CURRENT_TIMESTAMP WHERE id = ?"
        }
        ("reset_rate_limit", true) => {
            "UPDATE api_keys SET rate_usage_reset_at = CURRENT_TIMESTAMP, updated_at = CURRENT_TIMESTAMP WHERE id = ? AND user_id = ?"
        }
        ("reset_rate_limit", false) => {
            "UPDATE api_keys SET rate_usage_reset_at = CURRENT_TIMESTAMP, updated_at = CURRENT_TIMESTAMP WHERE id = ?"
        }
        _ => unreachable!("batch action was validated"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unrestricted_key() -> ApiKeyRow {
        ApiKeyRow {
            id: 1,
            user_id: None,
            name: "unrestricted".into(),
            token_prefix: "sk-mini-test".into(),
            token_hash: "hash".into(),
            enabled: true,
            last_used_at: None,
            created_at: "2026-01-01T00:00:00Z".into(),
            expires_at: None,
            quota_tokens: 0,
            quota_cost_microusd: 0,
            quota_reset_at: None,
            allowed_models: "[]".into(),
            group_id: None,
            ip_whitelist: "[]".into(),
            ip_blacklist: "[]".into(),
            rate_limit_5h_microusd: 0,
            rate_limit_1d_microusd: 0,
            rate_limit_7d_microusd: 0,
            rate_usage_reset_at: None,
        }
    }

    #[test]
    fn normalizes_and_matches_ip_policies() {
        let whitelist = normalize_networks(
            vec!["192.168.1.42".into(), "10.12.34.56/8".into()],
            "ip_whitelist",
        )
        .unwrap();
        let blacklist = normalize_networks(vec!["10.1.0.0/16".into()], "ip_blacklist").unwrap();
        assert_eq!(whitelist, vec!["10.0.0.0/8", "192.168.1.42"]);
        assert!(ip_is_allowed(
            "10.2.3.4".parse().unwrap(),
            &whitelist,
            &blacklist
        ));
        assert!(!ip_is_allowed(
            "10.1.2.3".parse().unwrap(),
            &whitelist,
            &blacklist
        ));
        assert!(!ip_is_allowed(
            "172.16.0.1".parse().unwrap(),
            &whitelist,
            &blacklist
        ));
        assert!(normalize_networks(vec!["not-an-ip".into()], "ip_whitelist").is_err());
    }

    #[tokio::test]
    async fn validates_custom_tokens_without_storing_plaintext() {
        let (_directory, state) = crate::test_support::state().await;
        let custom = "sk-custom_key_1234567890";
        assert_eq!(
            issue_token(&state.pool, Some(custom.into())).await.unwrap(),
            custom
        );
        sqlx::query(
            "INSERT INTO api_keys (name, token_prefix, token_hash) VALUES ('custom', 'sk-custom', ?)",
        )
        .bind(token_hash(custom))
        .execute(&state.pool)
        .await
        .unwrap();
        assert!(issue_token(&state.pool, Some(custom.into())).await.is_err());
        assert!(issue_token(&state.pool, Some("weak".into())).await.is_err());
    }

    #[tokio::test]
    async fn unrestricted_keys_do_not_query_usage_logs() {
        let (_directory, state) = crate::test_support::state().await;
        state.pool.close().await;

        enforce(&state, &unrestricted_key(), None).await.unwrap();
    }
}
