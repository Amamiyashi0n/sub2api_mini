use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};
use sqlx::FromRow;

use crate::{
    crypto::Crypto,
    error::{ApiError, ApiResult},
};

pub const DEFAULT_API_BASE_URL: &str = "https://api.openai.com";
pub const DEFAULT_OAUTH_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";
pub const DEFAULT_ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com";
pub const DEFAULT_GEMINI_BASE_URL: &str = "https://generativelanguage.googleapis.com";
pub const DEFAULT_GEMINI_OAUTH_BASE_URL: &str = "https://cloudcode-pa.googleapis.com";
pub const DEFAULT_ANTIGRAVITY_BASE_URL: &str = "https://cloudcode-pa.googleapis.com";
pub const DEFAULT_GROK_API_BASE_URL: &str = "https://api.x.ai";
pub const DEFAULT_GROK_OAUTH_BASE_URL: &str = "https://cli-chat-proxy.grok.com/v1";

pub fn deserialize_nullable<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Some(Option::<T>::deserialize(deserializer)?))
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Credentials {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chatgpt_account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub org_uuid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_uuid: Option<String>,
    #[serde(default, flatten)]
    pub provider: Map<String, Value>,
}

impl Credentials {
    pub fn provider_str(&self, key: &str) -> Option<&str> {
        self.provider
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }
}

#[derive(Debug, Clone, FromRow)]
pub struct AccountRow {
    pub id: i64,
    pub name: String,
    pub kind: String,
    pub platform: String,
    pub account_type: String,
    pub base_url: String,
    pub encrypted_credentials: String,
    pub priority: i32,
    pub concurrency: i32,
    pub enabled: bool,
    pub cooldown_until: Option<String>,
    pub last_used_at: Option<String>,
    pub last_error: Option<String>,
    pub proxy_id: Option<i64>,
    pub proxy_name: Option<String>,
    pub proxy_active: Option<bool>,
    pub encrypted_proxy_url: Option<String>,
    pub parent_account_id: Option<i64>,
    pub quota_dimension: String,
    pub notes: String,
    pub crs_account_id: Option<String>,
    pub tls_fingerprint_profile_id: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct Account {
    pub row: AccountRow,
    pub credentials: Credentials,
    pub proxy_url: Option<String>,
}

impl AccountRow {
    pub fn decrypt(self, crypto: &Crypto) -> ApiResult<Account> {
        let plaintext = crypto.decrypt(&self.encrypted_credentials)?;
        let credentials = serde_json::from_slice(&plaintext)
            .map_err(|_| ApiError::internal("stored credential JSON is malformed"))?;
        let proxy_url = self
            .encrypted_proxy_url
            .as_deref()
            .map(|value| crypto.decrypt(value))
            .transpose()?
            .map(String::from_utf8)
            .transpose()
            .map_err(|_| ApiError::internal("stored proxy URL is malformed"))?;
        Ok(Account {
            row: self,
            credentials,
            proxy_url,
        })
    }

    pub fn public(&self) -> AccountPublic {
        AccountPublic {
            id: self.id,
            name: self.name.clone(),
            kind: self.kind.clone(),
            platform: self.platform.clone(),
            account_type: self.account_type.clone(),
            base_url: self.base_url.clone(),
            priority: self.priority,
            concurrency: self.concurrency,
            enabled: self.enabled,
            cooldown_until: self.cooldown_until.clone(),
            last_used_at: self.last_used_at.clone(),
            last_error: self.last_error.clone(),
            proxy_id: self.proxy_id,
            proxy_name: self.proxy_name.clone(),
            proxy_active: self.proxy_active,
            parent_account_id: self.parent_account_id,
            quota_dimension: self.quota_dimension.clone(),
            notes: self.notes.clone(),
            crs_account_id: self.crs_account_id.clone(),
            tls_fingerprint_profile_id: self.tls_fingerprint_profile_id,
            created_at: self.created_at.clone(),
            updated_at: self.updated_at.clone(),
            credential_hint: if self.parent_account_id.is_some() {
                "Inherited OAuth token".into()
            } else if self.account_type == "setup_token" {
                "Setup Token".into()
            } else if self.account_type == "bedrock" {
                match self.kind.as_str() {
                    "bedrock" => "AWS credential".into(),
                    _ => "Bedrock credential".into(),
                }
            } else if self.account_type == "service_account" {
                "Service Account".into()
            } else if self.account_type == "upstream" {
                "Upstream API key".into()
            } else if self.kind == "oauth" {
                "OAuth token".into()
            } else {
                "API key".into()
            },
        }
    }
}

#[derive(Debug, Serialize)]
pub struct AccountPublic {
    pub id: i64,
    pub name: String,
    pub kind: String,
    pub platform: String,
    pub account_type: String,
    pub base_url: String,
    pub priority: i32,
    pub concurrency: i32,
    pub enabled: bool,
    pub cooldown_until: Option<String>,
    pub last_used_at: Option<String>,
    pub last_error: Option<String>,
    pub proxy_id: Option<i64>,
    pub proxy_name: Option<String>,
    pub proxy_active: Option<bool>,
    pub parent_account_id: Option<i64>,
    pub quota_dimension: String,
    pub notes: String,
    pub crs_account_id: Option<String>,
    pub tls_fingerprint_profile_id: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
    pub credential_hint: String,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct ApiKeyRow {
    pub id: i64,
    pub user_id: Option<i64>,
    pub name: String,
    pub token_prefix: String,
    #[serde(skip)]
    #[allow(dead_code)]
    pub token_hash: String,
    pub enabled: bool,
    pub last_used_at: Option<String>,
    pub created_at: String,
    pub expires_at: Option<String>,
    pub quota_tokens: i64,
    pub quota_cost_microusd: i64,
    pub quota_reset_at: Option<String>,
    pub allowed_models: String,
    pub group_id: Option<i64>,
    pub ip_whitelist: String,
    pub ip_blacklist: String,
    pub rate_limit_5h_microusd: i64,
    pub rate_limit_1d_microusd: i64,
    pub rate_limit_7d_microusd: i64,
    pub rate_usage_reset_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ApiKeyContext {
    pub id: i64,
    pub user_id: Option<i64>,
    pub allowed_models: Vec<String>,
    pub group_id: Option<i64>,
}

#[derive(Debug, Serialize, FromRow)]
pub struct UsageLog {
    pub id: i64,
    pub request_id: String,
    pub api_key_id: Option<i64>,
    pub account_id: Option<i64>,
    pub user_id: Option<i64>,
    pub endpoint: String,
    pub model: Option<String>,
    pub status_code: i32,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub total_tokens: Option<i64>,
    pub cached_input_tokens: i64,
    pub cache_write_tokens: i64,
    pub image_input_tokens: i64,
    pub image_output_tokens: i64,
    pub reasoning_tokens: i64,
    pub billing_model: Option<String>,
    pub mapped_model: Option<String>,
    pub model_mapping_chain: String,
    pub request_type: String,
    pub stream: bool,
    pub service_tier: Option<String>,
    pub cost_microusd: i64,
    pub duration_ms: i64,
    pub ttft_ms: Option<i64>,
    pub upstream_attempts: i64,
    pub account_switches: i64,
    pub error_summary: Option<String>,
    pub created_at: String,
}

pub fn normalize_base_url(value: &str, kind: &str) -> ApiResult<String> {
    let default = if kind == "oauth" {
        DEFAULT_OAUTH_BASE_URL
    } else {
        DEFAULT_API_BASE_URL
    };
    let value = if value.trim().is_empty() {
        default
    } else {
        value.trim()
    };
    let mut url = url::Url::parse(value)
        .map_err(|_| ApiError::bad_request("INVALID_BASE_URL", "base_url is invalid"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(ApiError::bad_request(
            "INVALID_BASE_URL",
            "base_url must use http or https",
        ));
    }
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.as_str().trim_end_matches('/').to_string())
}

pub fn normalize_account_base_url(value: &str, kind: &str, platform: &str) -> ApiResult<String> {
    let default = match platform {
        "anthropic" => DEFAULT_ANTHROPIC_BASE_URL,
        "gemini" => {
            if kind == "oauth" {
                DEFAULT_GEMINI_OAUTH_BASE_URL
            } else {
                DEFAULT_GEMINI_BASE_URL
            }
        }
        "antigravity" => DEFAULT_ANTIGRAVITY_BASE_URL,
        "grok" => {
            if kind == "oauth" {
                DEFAULT_GROK_OAUTH_BASE_URL
            } else {
                DEFAULT_GROK_API_BASE_URL
            }
        }
        "openai" => {
            if kind == "oauth" {
                DEFAULT_OAUTH_BASE_URL
            } else {
                DEFAULT_API_BASE_URL
            }
        }
        _ => {
            return Err(ApiError::bad_request(
                "INVALID_ACCOUNT_PLATFORM",
                "platform must be anthropic, openai, gemini, antigravity, or grok",
            ));
        }
    };
    normalize_base_url(
        if value.trim().is_empty() {
            default
        } else {
            value
        },
        if !matches!(platform, "openai" | "grok") {
            "api_key"
        } else {
            kind
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Deserialize)]
    struct NullablePatch {
        #[serde(default, deserialize_with = "deserialize_nullable")]
        value: Option<Option<i64>>,
    }

    #[test]
    fn nullable_patch_distinguishes_missing_null_and_value() {
        let missing: NullablePatch = serde_json::from_str("{}").unwrap();
        let null: NullablePatch = serde_json::from_str(r#"{"value":null}"#).unwrap();
        let value: NullablePatch = serde_json::from_str(r#"{"value":7}"#).unwrap();
        assert_eq!(missing.value, None);
        assert_eq!(null.value, Some(None));
        assert_eq!(value.value, Some(Some(7)));
    }
}
