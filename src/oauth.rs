use std::time::{Duration, Instant};

use base64::{
    Engine as _,
    engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD},
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{
    crypto::random_token,
    error::{ApiError, ApiResult},
    models::{Account, Credentials, DEFAULT_OAUTH_BASE_URL},
    state::{AppState, OAuthFlow},
};

pub const OPENAI_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub const OPENAI_AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
pub const OPENAI_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
pub const OPENAI_REDIRECT_URI: &str = "http://localhost:1455/auth/callback";

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct OAuthStart {
    pub auth_url: String,
    pub expires_in: i64,
}

#[derive(Debug, Serialize)]
pub struct OAuthComplete {
    pub account_id: i64,
    pub reauthorized: bool,
}

pub async fn start_flow(state: &AppState, account_id: Option<i64>) -> ApiResult<OAuthStart> {
    let flow_state = random_token(32)?;
    let verifier = random_token(64)?;
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));

    let mut url = url::Url::parse(OPENAI_AUTHORIZE_URL)
        .map_err(|_| ApiError::internal("OAuth URL is invalid"))?;
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", OPENAI_CLIENT_ID)
        .append_pair("redirect_uri", OPENAI_REDIRECT_URI)
        .append_pair("scope", "openid profile email offline_access")
        .append_pair("state", &flow_state)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("id_token_add_organizations", "true")
        .append_pair("codex_cli_simplified_flow", "true");

    let mut flows = state.oauth_flows.lock().await;
    flows.retain(|_, flow| flow.created_at.elapsed() < Duration::from_secs(600));
    flows.insert(
        flow_state,
        OAuthFlow {
            verifier,
            created_at: Instant::now(),
            account_id,
        },
    );

    Ok(OAuthStart {
        auth_url: url.to_string(),
        expires_in: 600,
    })
}

pub async fn complete_flow(
    state: &AppState,
    code: &str,
    flow_state: &str,
) -> ApiResult<OAuthComplete> {
    let flow = state
        .oauth_flows
        .lock()
        .await
        .remove(flow_state)
        .ok_or_else(|| {
            ApiError::bad_request("OAUTH_STATE_INVALID", "OAuth state is invalid or expired")
        })?;
    if flow.created_at.elapsed() > Duration::from_secs(600) {
        return Err(ApiError::bad_request(
            "OAUTH_STATE_EXPIRED",
            "OAuth state has expired",
        ));
    }

    let token = state
        .client
        .post(OPENAI_TOKEN_URL)
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", OPENAI_CLIENT_ID),
            ("code", code),
            ("redirect_uri", OPENAI_REDIRECT_URI),
            ("code_verifier", flow.verifier.as_str()),
        ])
        .send()
        .await?;
    if !token.status().is_success() {
        tracing::warn!(status = %token.status(), "OAuth code exchange rejected");
        return Err(ApiError::new(
            http::StatusCode::BAD_GATEWAY,
            "OAUTH_EXCHANGE_FAILED",
            "OpenAI rejected the OAuth code exchange",
        ));
    }
    let token: TokenResponse = token.json().await?;
    persist_completed_flow(state, token, flow.account_id).await
}

async fn persist_completed_flow(
    state: &AppState,
    token: TokenResponse,
    account_id: Option<i64>,
) -> ApiResult<OAuthComplete> {
    let credentials = credentials_from_token(token, None);
    if let Some(account_id) = account_id {
        replace_oauth_credentials(state, account_id, credentials).await?;
        return Ok(OAuthComplete {
            account_id,
            reauthorized: true,
        });
    }
    let name = credentials
        .email
        .clone()
        .unwrap_or_else(|| "OpenAI OAuth".into());
    Ok(OAuthComplete {
        account_id: insert_oauth_account(state, &name, credentials, 50, 3).await?,
        reauthorized: false,
    })
}

async fn replace_oauth_credentials(
    state: &AppState,
    account_id: i64,
    mut credentials: Credentials,
) -> ApiResult<()> {
    let stored: Option<(String, String)> =
        sqlx::query_as("SELECT kind, encrypted_credentials FROM accounts WHERE id = ?")
            .bind(account_id)
            .fetch_optional(&state.pool)
            .await?;
    let encrypted_current = match stored {
        Some((kind, encrypted)) if kind == "oauth" => encrypted,
        Some(_) => {
            return Err(ApiError::bad_request(
                "NOT_OAUTH_ACCOUNT",
                "only OAuth accounts can be re-authorized",
            ));
        }
        None => return Err(ApiError::not_found("OAuth account not found")),
    };
    let current: Credentials =
        serde_json::from_slice(&state.crypto.decrypt(&encrypted_current)?)
            .map_err(|_| ApiError::internal("stored OAuth credential is malformed"))?;
    credentials.refresh_token = credentials.refresh_token.or(current.refresh_token);
    credentials.id_token = credentials.id_token.or(current.id_token);
    credentials.email = credentials.email.or(current.email);
    credentials.chatgpt_account_id = credentials
        .chatgpt_account_id
        .or(current.chatgpt_account_id);
    let encrypted = state.crypto.encrypt(
        &serde_json::to_vec(&credentials)
            .map_err(|_| ApiError::internal("credential serialization failed"))?,
    )?;
    sqlx::query(
        "UPDATE accounts SET encrypted_credentials = ?, cooldown_until = NULL, last_error = NULL, \
         updated_at = CURRENT_TIMESTAMP WHERE id = ?",
    )
    .bind(encrypted)
    .bind(account_id)
    .execute(&state.pool)
    .await?;
    state.model_cache.lock().await.remove(&account_id);
    Ok(())
}

pub fn parse_import(content: &str) -> ApiResult<Credentials> {
    let value: Value = serde_json::from_str(content)
        .map_err(|_| ApiError::bad_request("INVALID_AUTH_JSON", "content is not valid JSON"))?;
    let tokens = value.get("tokens").unwrap_or(&value);
    let access_token = string_field(tokens, &["access_token", "accessToken"])
        .or_else(|| string_field(&value, &["access_token", "accessToken"]))
        .ok_or_else(|| {
            ApiError::bad_request("ACCESS_TOKEN_REQUIRED", "access_token is required")
        })?;
    let refresh_token = string_field(tokens, &["refresh_token", "refreshToken"])
        .or_else(|| string_field(&value, &["refresh_token", "refreshToken"]));
    let id_token = string_field(tokens, &["id_token", "idToken"])
        .or_else(|| string_field(&value, &["id_token", "idToken"]));
    let account_id = string_field(tokens, &["account_id", "accountId"])
        .or_else(|| string_field(&value, &["account_id", "accountId"]));

    let mut credentials = Credentials {
        access_token: Some(access_token.clone()),
        refresh_token,
        id_token: id_token.clone(),
        client_id: Some(OPENAI_CLIENT_ID.into()),
        chatgpt_account_id: account_id,
        ..Default::default()
    };
    enrich_from_jwt(
        &mut credentials,
        id_token.as_deref().unwrap_or(&access_token),
    );
    if credentials.expires_at.is_none() {
        enrich_from_jwt(&mut credentials, &access_token);
    }
    Ok(credentials)
}

pub async fn insert_oauth_account(
    state: &AppState,
    name: &str,
    credentials: Credentials,
    priority: i32,
    concurrency: i32,
) -> ApiResult<i64> {
    if name.trim().is_empty() || concurrency < 1 || priority < 0 {
        return Err(ApiError::bad_request(
            "INVALID_ACCOUNT",
            "name, priority, or concurrency is invalid",
        ));
    }
    let encrypted = state.crypto.encrypt(
        &serde_json::to_vec(&credentials)
            .map_err(|_| ApiError::internal("credential serialization failed"))?,
    )?;
    let result = sqlx::query(
        "INSERT INTO accounts (name, kind, base_url, encrypted_credentials, priority, concurrency) \
         VALUES (?, 'oauth', ?, ?, ?, ?)",
    )
    .bind(name.trim())
    .bind(DEFAULT_OAUTH_BASE_URL)
    .bind(encrypted)
    .bind(priority)
    .bind(concurrency)
    .execute(&state.pool)
    .await?;
    Ok(result.last_insert_rowid())
}

pub async fn refresh_if_needed(state: &AppState, account: &mut Account) -> ApiResult<()> {
    if account.row.kind != "oauth" {
        return Ok(());
    }
    let expires_at = account.credentials.expires_at.unwrap_or(0);
    if expires_at > Utc::now().timestamp() + 300 {
        return Ok(());
    }
    refresh_account(state, account).await
}

pub async fn refresh_account(state: &AppState, account: &mut Account) -> ApiResult<()> {
    refresh_account_inner(state, account, false).await
}

pub async fn refresh_account_forced(state: &AppState, account: &mut Account) -> ApiResult<()> {
    refresh_account_inner(state, account, true).await
}

async fn refresh_account_inner(
    state: &AppState,
    account: &mut Account,
    force: bool,
) -> ApiResult<()> {
    let credential_account_id = account.row.parent_account_id.unwrap_or(account.row.id);
    let lock = {
        let mut locks = state.oauth_refresh_locks.lock().await;
        locks
            .entry(credential_account_id)
            .or_insert_with(|| std::sync::Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    };
    let _guard = lock.lock().await;

    let encrypted: String = sqlx::query_scalar(
        "SELECT encrypted_credentials FROM accounts WHERE id = ? AND kind = 'oauth'",
    )
    .bind(credential_account_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| ApiError::not_found("OAuth account not found"))?;
    let mut current: Credentials = serde_json::from_slice(&state.crypto.decrypt(&encrypted)?)
        .map_err(|_| ApiError::internal("stored OAuth credential is malformed"))?;
    if !force && current.expires_at.unwrap_or(0) > Utc::now().timestamp() + 300 {
        account.credentials = current;
        return Ok(());
    }
    let refresh_token = current.refresh_token.clone().ok_or_else(|| {
        ApiError::new(
            http::StatusCode::BAD_GATEWAY,
            "OAUTH_REFRESH_REQUIRED",
            "OAuth access token expired and no refresh token is available",
        )
    })?;

    let client = state.client_for_account(account).await?;
    let response = client
        .post(OPENAI_TOKEN_URL)
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token.as_str()),
            ("client_id", OPENAI_CLIENT_ID),
            ("scope", "openid profile email"),
        ])
        .send()
        .await?;
    if !response.status().is_success() {
        sqlx::query(
            "UPDATE accounts SET last_error = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
        )
        .bind(format!("OAuth refresh failed: {}", response.status()))
        .bind(credential_account_id)
        .execute(&state.pool)
        .await?;
        return Err(ApiError::new(
            http::StatusCode::BAD_GATEWAY,
            "OAUTH_REFRESH_FAILED",
            "OpenAI rejected the OAuth token refresh",
        ));
    }
    let token: TokenResponse = response.json().await?;
    let old_refresh = current.refresh_token.clone();
    let mut updated = credentials_from_token(token, old_refresh);
    if updated.email.is_none() {
        updated.email = current.email.take();
    }
    if updated.chatgpt_account_id.is_none() {
        updated.chatgpt_account_id = current.chatgpt_account_id.take();
    }
    let encrypted = state.crypto.encrypt(
        &serde_json::to_vec(&updated)
            .map_err(|_| ApiError::internal("credential serialization failed"))?,
    )?;
    sqlx::query(
        "UPDATE accounts SET encrypted_credentials = ?, last_error = NULL, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
    )
    .bind(encrypted)
    .bind(credential_account_id)
    .execute(&state.pool)
    .await?;
    account.credentials = updated;
    Ok(())
}

fn credentials_from_token(token: TokenResponse, previous_refresh: Option<String>) -> Credentials {
    let mut credentials = Credentials {
        access_token: Some(token.access_token.clone()),
        refresh_token: token.refresh_token.or(previous_refresh),
        id_token: token.id_token.clone(),
        expires_at: Some(Utc::now().timestamp() + token.expires_in.unwrap_or(3600)),
        client_id: Some(OPENAI_CLIENT_ID.into()),
        ..Default::default()
    };
    enrich_from_jwt(
        &mut credentials,
        token.id_token.as_deref().unwrap_or(&token.access_token),
    );
    credentials
}

fn string_field(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn enrich_from_jwt(credentials: &mut Credentials, token: &str) {
    let Some(payload) = token.split('.').nth(1) else {
        return;
    };
    let decoded = URL_SAFE_NO_PAD
        .decode(payload)
        .or_else(|_| URL_SAFE.decode(payload));
    let Ok(decoded) = decoded else { return };
    let Ok(value) = serde_json::from_slice::<Value>(&decoded) else {
        return;
    };
    if credentials.email.is_none() {
        credentials.email = value
            .get("email")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
    }
    if credentials.expires_at.is_none() {
        credentials.expires_at = value.get("exp").and_then(Value::as_i64);
    }
    if credentials.chatgpt_account_id.is_none() {
        credentials.chatgpt_account_id = value
            .get("https://api.openai.com/auth")
            .and_then(|claims| claims.get("chatgpt_account_id"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;

    #[test]
    fn imports_codex_auth_json() {
        let credentials = parse_import(
            r#"{"tokens":{"access_token":"access","refresh_token":"refresh","account_id":"acct"}}"#,
        )
        .unwrap();
        assert_eq!(credentials.access_token.as_deref(), Some("access"));
        assert_eq!(credentials.refresh_token.as_deref(), Some("refresh"));
        assert_eq!(credentials.chatgpt_account_id.as_deref(), Some("acct"));
    }

    #[test]
    fn rejects_missing_access_token() {
        assert!(parse_import(r#"{"tokens":{"refresh_token":"refresh"}}"#).is_err());
    }

    #[tokio::test]
    async fn browser_flow_targets_an_existing_account_and_preserves_configuration() {
        let (_directory, state) = test_support::state().await;
        let old = Credentials {
            access_token: Some("old-access".into()),
            refresh_token: Some("old-refresh".into()),
            email: Some("old@example.com".into()),
            chatgpt_account_id: Some("acct-old".into()),
            ..Default::default()
        };
        let account_id = insert_oauth_account(&state, "browser oauth", old, 7, 9)
            .await
            .unwrap();
        let group_id = sqlx::query("INSERT INTO groups (name) VALUES ('oauth group')")
            .execute(&state.pool)
            .await
            .unwrap()
            .last_insert_rowid();
        sqlx::query("INSERT INTO account_groups (account_id, group_id) VALUES (?, ?)")
            .bind(account_id)
            .bind(group_id)
            .execute(&state.pool)
            .await
            .unwrap();
        sqlx::query(
            "UPDATE accounts SET last_error = 'expired', \
             cooldown_until = datetime('now', '+1 hour') WHERE id = ?",
        )
        .bind(account_id)
        .execute(&state.pool)
        .await
        .unwrap();

        let started = start_flow(&state, Some(account_id)).await.unwrap();
        let url = url::Url::parse(&started.auth_url).unwrap();
        let flow_state = url
            .query_pairs()
            .find(|pair| pair.0 == "state")
            .unwrap()
            .1
            .into_owned();
        assert_eq!(
            state
                .oauth_flows
                .lock()
                .await
                .get(&flow_state)
                .unwrap()
                .account_id,
            Some(account_id)
        );
        assert!(url.query_pairs().any(|pair| pair.0 == "code_challenge"));

        let completed = persist_completed_flow(
            &state,
            TokenResponse {
                access_token: "new-access".into(),
                refresh_token: None,
                id_token: None,
                expires_in: Some(3600),
            },
            Some(account_id),
        )
        .await
        .unwrap();
        assert_eq!(completed.account_id, account_id);
        assert!(completed.reauthorized);
        let (name, priority, concurrency, encrypted, cooldown, error): (
            String,
            i32,
            i32,
            String,
            Option<String>,
            Option<String>,
        ) = sqlx::query_as(
            "SELECT name, priority, concurrency, encrypted_credentials, cooldown_until, \
             last_error FROM accounts WHERE id = ?",
        )
        .bind(account_id)
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(
            (name.as_str(), priority, concurrency),
            ("browser oauth", 7, 9)
        );
        assert!(cooldown.is_none());
        assert!(error.is_none());
        let updated: Credentials =
            serde_json::from_slice(&state.crypto.decrypt(&encrypted).unwrap()).unwrap();
        assert_eq!(updated.access_token.as_deref(), Some("new-access"));
        assert_eq!(updated.refresh_token.as_deref(), Some("old-refresh"));
        assert_eq!(updated.email.as_deref(), Some("old@example.com"));
        assert_eq!(updated.chatgpt_account_id.as_deref(), Some("acct-old"));
        let copied_group: i64 =
            sqlx::query_scalar("SELECT group_id FROM account_groups WHERE account_id = ?")
                .bind(account_id)
                .fetch_one(&state.pool)
                .await
                .unwrap();
        assert_eq!(copied_group, group_id);
        let account_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM accounts")
            .fetch_one(&state.pool)
            .await
            .unwrap();
        assert_eq!(account_count, 1);
    }
}
