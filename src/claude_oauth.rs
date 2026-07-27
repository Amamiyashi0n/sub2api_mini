use std::time::{Duration, Instant};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::Utc;
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::{
    crypto::random_token,
    error::{ApiError, ApiResult},
    models::{Account, Credentials},
    state::{AppState, ClaudeOAuthFlow},
};

pub const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const AUTHORIZE_URL: &str = "https://claude.ai/oauth/authorize";
const TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const REDIRECT_URI: &str = "https://platform.claude.com/oauth/code/callback";
const OAUTH_SCOPE: &str = "org:create_api_key user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload";
const SETUP_TOKEN_SCOPE: &str = "user:inference";

#[derive(Debug, serde::Serialize)]
pub struct StartResult {
    pub auth_url: String,
    pub session_id: String,
    pub expires_in: i64,
}

pub struct ExchangedToken {
    pub credentials: Credentials,
    pub account_type: &'static str,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    token_type: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    organization: Option<Organization>,
    #[serde(default)]
    account: Option<TokenAccount>,
}

#[derive(Debug, Deserialize)]
struct Organization {
    uuid: String,
}

#[derive(Debug, Deserialize)]
struct TokenAccount {
    uuid: String,
    #[serde(default)]
    email_address: Option<String>,
}

pub async fn start_flow(state: &AppState, setup_token: bool) -> ApiResult<StartResult> {
    let flow_state = random_token(32)?;
    let session_id = random_token(16)?;
    let verifier = random_token(32)?;
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let scope = if setup_token {
        SETUP_TOKEN_SCOPE
    } else {
        OAUTH_SCOPE
    };
    let mut url = url::Url::parse(AUTHORIZE_URL)
        .map_err(|_| ApiError::internal("Claude OAuth URL is invalid"))?;
    url.query_pairs_mut()
        .append_pair("code", "true")
        .append_pair("client_id", CLIENT_ID)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", REDIRECT_URI)
        .append_pair("scope", scope)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", &flow_state);

    let mut flows = state.claude_oauth_flows.lock().await;
    flows.retain(|_, flow| flow.created_at.elapsed() < Duration::from_secs(1800));
    flows.insert(
        session_id.clone(),
        ClaudeOAuthFlow {
            verifier,
            state: flow_state,
            setup_token,
            created_at: Instant::now(),
        },
    );
    Ok(StartResult {
        auth_url: url.to_string(),
        session_id,
        expires_in: 1800,
    })
}

pub async fn exchange_code(
    state: &AppState,
    session_id: &str,
    code: &str,
) -> ApiResult<ExchangedToken> {
    let flow = state
        .claude_oauth_flows
        .lock()
        .await
        .get(session_id)
        .cloned()
        .ok_or_else(|| {
            ApiError::bad_request(
                "CLAUDE_OAUTH_SESSION_INVALID",
                "Claude OAuth session is invalid or expired",
            )
        })?;
    if flow.created_at.elapsed() > Duration::from_secs(1800) {
        return Err(ApiError::bad_request(
            "CLAUDE_OAUTH_SESSION_EXPIRED",
            "Claude OAuth session has expired",
        ));
    }
    let (authorization_code, returned_state) = code
        .trim()
        .split_once('#')
        .map_or((code.trim(), None), |(code, state)| {
            (code.trim(), Some(state.trim()))
        });
    if authorization_code.is_empty() {
        return Err(ApiError::bad_request(
            "CLAUDE_OAUTH_CODE_REQUIRED",
            "Claude authorization code is required",
        ));
    }
    if returned_state.is_some_and(|value| value != flow.state) {
        return Err(ApiError::bad_request(
            "CLAUDE_OAUTH_STATE_INVALID",
            "Claude OAuth state does not match",
        ));
    }
    let mut body = json!({
        "code": authorization_code,
        "grant_type": "authorization_code",
        "client_id": CLIENT_ID,
        "redirect_uri": REDIRECT_URI,
        "code_verifier": flow.verifier,
    });
    if let Some(state) = returned_state {
        body["state"] = json!(state);
    }
    let response = state
        .client
        .post(TOKEN_URL)
        .header("accept", "application/json, text/plain, */*")
        .header("user-agent", "axios/1.13.6")
        .json(&body)
        .send()
        .await?;
    if !response.status().is_success() {
        tracing::warn!(status = %response.status(), "Claude OAuth code exchange rejected");
        return Err(ApiError::new(
            http::StatusCode::BAD_GATEWAY,
            "CLAUDE_OAUTH_EXCHANGE_FAILED",
            "Claude rejected the OAuth code exchange",
        ));
    }
    let token: TokenResponse = response.json().await?;
    state.claude_oauth_flows.lock().await.remove(session_id);
    Ok(ExchangedToken {
        credentials: credentials_from_token(token, None),
        account_type: if flow.setup_token {
            "setup_token"
        } else {
            "oauth"
        },
    })
}

pub async fn refresh_if_needed(state: &AppState, account: &mut Account) -> ApiResult<()> {
    if account.credentials.expires_at.unwrap_or(i64::MAX) > Utc::now().timestamp() + 300 {
        return Ok(());
    }
    refresh_account(state, account, false).await
}

pub async fn refresh_account(
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
        "SELECT encrypted_credentials FROM accounts \
         WHERE id = ? AND kind = 'oauth' AND platform = 'anthropic'",
    )
    .bind(credential_account_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| ApiError::not_found("Claude OAuth account not found"))?;
    let current: Credentials = serde_json::from_slice(&state.crypto.decrypt(&encrypted)?)
        .map_err(|_| ApiError::internal("stored Claude OAuth credential is malformed"))?;
    if !force && current.expires_at.unwrap_or(i64::MAX) > Utc::now().timestamp() + 300 {
        account.credentials = current;
        return Ok(());
    }
    let refresh_token = current.refresh_token.as_deref().ok_or_else(|| {
        ApiError::new(
            http::StatusCode::BAD_GATEWAY,
            "CLAUDE_OAUTH_REFRESH_REQUIRED",
            "Claude access token expired and no refresh token is available",
        )
    })?;
    let client = state.client_for_account(account).await?;
    let response = client
        .post(TOKEN_URL)
        .header("accept", "application/json, text/plain, */*")
        .header("user-agent", "axios/1.13.6")
        .json(&json!({
            "grant_type": "refresh_token",
            "refresh_token": refresh_token,
            "client_id": CLIENT_ID,
        }))
        .send()
        .await?;
    if !response.status().is_success() {
        sqlx::query(
            "UPDATE accounts SET last_error = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
        )
        .bind(format!(
            "Claude OAuth refresh failed: {}",
            response.status()
        ))
        .bind(credential_account_id)
        .execute(&state.pool)
        .await?;
        return Err(ApiError::new(
            http::StatusCode::BAD_GATEWAY,
            "CLAUDE_OAUTH_REFRESH_FAILED",
            "Claude rejected the OAuth token refresh",
        ));
    }
    let updated = credentials_from_token(response.json().await?, Some(current));
    let encrypted = state.crypto.encrypt(
        &serde_json::to_vec(&updated)
            .map_err(|_| ApiError::internal("credential serialization failed"))?,
    )?;
    sqlx::query(
        "UPDATE accounts SET encrypted_credentials = ?, last_error = NULL, \
         updated_at = CURRENT_TIMESTAMP WHERE id = ?",
    )
    .bind(encrypted)
    .bind(credential_account_id)
    .execute(&state.pool)
    .await?;
    account.credentials = updated;
    Ok(())
}

fn credentials_from_token(token: TokenResponse, previous: Option<Credentials>) -> Credentials {
    let previous = previous.unwrap_or_default();
    Credentials {
        access_token: Some(token.access_token),
        refresh_token: token.refresh_token.or(previous.refresh_token),
        expires_at: Some(Utc::now().timestamp() + token.expires_in.unwrap_or(28_800)),
        email: token
            .account
            .as_ref()
            .and_then(|account| account.email_address.clone())
            .or(previous.email),
        client_id: Some(CLIENT_ID.into()),
        token_type: token.token_type.or(previous.token_type),
        scope: token.scope.or(previous.scope),
        org_uuid: token
            .organization
            .map(|organization| organization.uuid)
            .or(previous.org_uuid),
        account_uuid: token
            .account
            .map(|account| account.uuid)
            .or(previous.account_uuid),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;

    #[test]
    fn token_refresh_preserves_identity_and_rotates_tokens() {
        let credentials = credentials_from_token(
            TokenResponse {
                access_token: "new-access".into(),
                token_type: Some("Bearer".into()),
                expires_in: Some(60),
                refresh_token: None,
                scope: None,
                organization: None,
                account: None,
            },
            Some(Credentials {
                refresh_token: Some("old-refresh".into()),
                email: Some("claude@example.com".into()),
                org_uuid: Some("org-1".into()),
                ..Default::default()
            }),
        );
        assert_eq!(credentials.access_token.as_deref(), Some("new-access"));
        assert_eq!(credentials.refresh_token.as_deref(), Some("old-refresh"));
        assert_eq!(credentials.email.as_deref(), Some("claude@example.com"));
        assert_eq!(credentials.org_uuid.as_deref(), Some("org-1"));
    }

    #[tokio::test]
    async fn start_flow_uses_distinct_oauth_and_setup_token_scopes() {
        let (_directory, state) = test_support::state().await;
        let oauth = start_flow(&state, false).await.unwrap();
        let setup = start_flow(&state, true).await.unwrap();
        let oauth_url = url::Url::parse(&oauth.auth_url).unwrap();
        let setup_url = url::Url::parse(&setup.auth_url).unwrap();
        let oauth_scope = oauth_url
            .query_pairs()
            .find(|pair| pair.0 == "scope")
            .unwrap()
            .1
            .into_owned();
        let setup_scope = setup_url
            .query_pairs()
            .find(|pair| pair.0 == "scope")
            .unwrap()
            .1
            .into_owned();
        assert!(oauth_scope.contains("user:sessions:claude_code"));
        assert_eq!(setup_scope, SETUP_TOKEN_SCOPE);
        assert!(
            state
                .claude_oauth_flows
                .lock()
                .await
                .get(&setup.session_id)
                .unwrap()
                .setup_token
        );
    }

    #[tokio::test]
    async fn invalid_code_state_keeps_the_session_for_retry() {
        let (_directory, state) = test_support::state().await;
        let started = start_flow(&state, false).await.unwrap();
        let error = exchange_code(&state, &started.session_id, "code#wrong-state")
            .await
            .err()
            .unwrap();
        assert_eq!(error.code, "CLAUDE_OAUTH_STATE_INVALID");
        assert!(
            state
                .claude_oauth_flows
                .lock()
                .await
                .contains_key(&started.session_id)
        );
    }
}
