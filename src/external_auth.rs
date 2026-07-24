use std::{
    collections::HashSet,
    time::{Duration as StdDuration, Instant},
};

use axum::{
    Json, Router,
    extract::{Extension, Path, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{Duration, Utc};
use ring::signature::{
    ECDSA_P256_SHA256_FIXED, RSA_PKCS1_2048_8192_SHA256, RSA_PSS_2048_8192_SHA256,
    RsaPublicKeyComponents, UnparsedPublicKey,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{Sqlite, Transaction};

use crate::{
    auth::{self, AuthSession},
    crypto::{hash_password, random_token, token_hash, verify_password},
    error::{ApiError, ApiResult},
    state::AppState,
};

const SUPPORTED_PROVIDERS: &[&str] = &[
    "oidc",
    "github",
    "google",
    "linuxdo",
    "dingtalk",
    "wechat_open",
    "wechat_mp",
];

pub fn auth_router() -> Router<AppState> {
    Router::new()
        .route("/oauth/providers", get(public_providers))
        .route("/oauth/{provider}/start", get(start_login))
        .route("/oauth/{provider}/callback", get(callback))
        .route("/oauth/pending/inspect", post(inspect_pending))
        .route("/oauth/pending/bind", post(bind_pending))
        .route("/oauth/pending/register", post(register_pending))
}

pub fn user_router() -> Router<AppState> {
    Router::new()
        .route("/auth-identities", get(list_identities))
        .route("/auth-identities/{provider}/start", post(start_binding))
        .route(
            "/auth-identities/{provider}",
            axum::routing::delete(unbind_identity),
        )
}

pub fn admin_router() -> Router<AppState> {
    Router::new()
        .route("/auth-providers", get(admin_providers))
        .route(
            "/auth-providers/{provider}",
            get(admin_provider).put(update_provider),
        )
}

#[derive(Clone, Debug)]
struct ProviderConfig {
    provider: String,
    name: String,
    enabled: bool,
    client_id: String,
    client_secret: Option<String>,
    authorize_url: String,
    token_url: String,
    userinfo_url: String,
    scopes: String,
    subject_path: String,
    email_path: String,
    display_name_path: String,
    token_auth_method: String,
    use_pkce: bool,
    profile_mode: String,
    emails_url: String,
    issuer_url: String,
    discovery_url: String,
    jwks_url: String,
    validate_id_token: bool,
    allowed_signing_algs: String,
    clock_skew_seconds: i64,
    require_email_verified: bool,
    dingtalk_app_type: String,
    dingtalk_corp_policy: String,
    dingtalk_internal_corp_id: String,
    dingtalk_bypass_registration: bool,
    dingtalk_sync_corp_email: bool,
    dingtalk_sync_display_name: bool,
    dingtalk_sync_dept: bool,
    dingtalk_require_email: bool,
    dingtalk_email_attr_key: String,
    dingtalk_email_attr_name: String,
    dingtalk_name_attr_key: String,
    dingtalk_name_attr_name: String,
    dingtalk_dept_attr_key: String,
    dingtalk_dept_attr_name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct PendingProfile {
    subject: String,
    display_name: Option<String>,
    email: Option<String>,
    email_verified: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    dingtalk: Option<DingTalkProfile>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct DingTalkProfile {
    #[serde(default)]
    corp_id: String,
    #[serde(default)]
    user_id: String,
    #[serde(default)]
    staff_name: String,
    #[serde(default)]
    nickname: String,
    #[serde(default)]
    corporate_email: Option<String>,
    #[serde(default)]
    department_ids: Vec<i64>,
    #[serde(default)]
    department_path: Option<String>,
}

#[derive(sqlx::FromRow)]
struct ProviderRow {
    name: String,
    enabled: bool,
    client_id: String,
    encrypted_client_secret: Option<String>,
    authorize_url: String,
    token_url: String,
    userinfo_url: String,
    scopes: String,
    subject_path: String,
    email_path: String,
    display_name_path: String,
    token_auth_method: String,
    use_pkce: bool,
    profile_mode: String,
    emails_url: String,
    issuer_url: String,
    discovery_url: String,
    jwks_url: String,
    validate_id_token: bool,
    allowed_signing_algs: String,
    clock_skew_seconds: i64,
    require_email_verified: bool,
    dingtalk_app_type: String,
    dingtalk_corp_policy: String,
    dingtalk_internal_corp_id: String,
    dingtalk_bypass_registration: bool,
    dingtalk_sync_corp_email: bool,
    dingtalk_sync_display_name: bool,
    dingtalk_sync_dept: bool,
    dingtalk_require_email: bool,
    dingtalk_email_attr_key: String,
    dingtalk_email_attr_name: String,
    dingtalk_name_attr_key: String,
    dingtalk_name_attr_name: String,
    dingtalk_dept_attr_key: String,
    dingtalk_dept_attr_name: String,
}

fn normalize_provider(provider: &str) -> ApiResult<String> {
    let provider = provider.trim().to_ascii_lowercase();
    if !SUPPORTED_PROVIDERS.contains(&provider.as_str()) {
        return Err(ApiError::not_found("OAuth provider was not found"));
    }
    Ok(provider)
}

fn identity_provider(provider: &str) -> &str {
    if provider == "wechat_mp" {
        "wechat_open"
    } else {
        provider
    }
}

async fn load_provider(
    state: &AppState,
    provider: &str,
    require_enabled: bool,
) -> ApiResult<ProviderConfig> {
    let provider = normalize_provider(provider)?;
    let row: Option<ProviderRow> = sqlx::query_as(
        "SELECT name, enabled, client_id, encrypted_client_secret, authorize_url, token_url, \
         userinfo_url, scopes, subject_path, email_path, display_name_path, token_auth_method, use_pkce, \
         profile_mode, emails_url, issuer_url, discovery_url, jwks_url, validate_id_token, \
         allowed_signing_algs, clock_skew_seconds, require_email_verified, dingtalk_app_type, \
         dingtalk_corp_policy, dingtalk_internal_corp_id, dingtalk_bypass_registration, \
         dingtalk_sync_corp_email, dingtalk_sync_display_name, dingtalk_sync_dept, \
         dingtalk_require_email, dingtalk_email_attr_key, dingtalk_email_attr_name, \
         dingtalk_name_attr_key, dingtalk_name_attr_name, dingtalk_dept_attr_key, \
         dingtalk_dept_attr_name \
         FROM external_auth_providers WHERE provider = ?",
    )
    .bind(&provider)
    .fetch_optional(&state.pool)
    .await?;
    let row = row.ok_or_else(|| ApiError::not_found("OAuth provider is not configured"))?;
    if require_enabled && !row.enabled {
        return Err(ApiError::not_found("OAuth login is disabled"));
    }
    let client_secret = row
        .encrypted_client_secret
        .as_deref()
        .map(|encrypted| {
            state.crypto.decrypt(encrypted).and_then(|bytes| {
                String::from_utf8(bytes)
                    .map_err(|_| ApiError::internal("stored OAuth secret is malformed"))
            })
        })
        .transpose()?;
    Ok(ProviderConfig {
        provider,
        name: row.name,
        enabled: row.enabled,
        client_id: row.client_id,
        client_secret,
        authorize_url: row.authorize_url,
        token_url: row.token_url,
        userinfo_url: row.userinfo_url,
        scopes: row.scopes,
        subject_path: row.subject_path,
        email_path: row.email_path,
        display_name_path: row.display_name_path,
        token_auth_method: row.token_auth_method,
        use_pkce: row.use_pkce,
        profile_mode: row.profile_mode,
        emails_url: row.emails_url,
        issuer_url: row.issuer_url,
        discovery_url: row.discovery_url,
        jwks_url: row.jwks_url,
        validate_id_token: row.validate_id_token,
        allowed_signing_algs: row.allowed_signing_algs,
        clock_skew_seconds: row.clock_skew_seconds,
        require_email_verified: row.require_email_verified,
        dingtalk_app_type: row.dingtalk_app_type,
        dingtalk_corp_policy: row.dingtalk_corp_policy,
        dingtalk_internal_corp_id: row.dingtalk_internal_corp_id,
        dingtalk_bypass_registration: row.dingtalk_bypass_registration,
        dingtalk_sync_corp_email: row.dingtalk_sync_corp_email,
        dingtalk_sync_display_name: row.dingtalk_sync_display_name,
        dingtalk_sync_dept: row.dingtalk_sync_dept,
        dingtalk_require_email: row.dingtalk_require_email,
        dingtalk_email_attr_key: row.dingtalk_email_attr_key,
        dingtalk_email_attr_name: row.dingtalk_email_attr_name,
        dingtalk_name_attr_key: row.dingtalk_name_attr_key,
        dingtalk_name_attr_name: row.dingtalk_name_attr_name,
        dingtalk_dept_attr_key: row.dingtalk_dept_attr_key,
        dingtalk_dept_attr_name: row.dingtalk_dept_attr_name,
    })
}

async fn public_providers(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT provider, name FROM external_auth_providers WHERE enabled = 1 ORDER BY provider",
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(json!({"data": rows.into_iter().map(|row| json!({
        "provider": row.0, "name": row.1
    })).collect::<Vec<_>>() })))
}

async fn start_login(
    State(state): State<AppState>,
    Path(provider): Path<String>,
) -> ApiResult<Redirect> {
    let authorization_url = create_flow(&state, &provider, "login", None).await?;
    Ok(Redirect::temporary(&authorization_url))
}

async fn start_binding(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
    Path(provider): Path<String>,
) -> ApiResult<Json<Value>> {
    let authorization_url = create_flow(&state, &provider, "bind", Some(session.user_id)).await?;
    Ok(Json(
        json!({"data": {"authorization_url": authorization_url}}),
    ))
}

async fn create_flow(
    state: &AppState,
    provider: &str,
    intent: &str,
    user_id: Option<i64>,
) -> ApiResult<String> {
    let config = load_provider(state, provider, true).await?;
    let recent_flows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM external_oauth_flows WHERE consumed_at IS NULL \
         AND datetime(created_at) > datetime('now', '-1 minute')",
    )
    .fetch_one(&state.pool)
    .await?;
    if recent_flows >= 100 {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "OAUTH_RATE_LIMITED",
            "too many OAuth authorization attempts",
        ));
    }
    let flow_state = random_token(32)?;
    let verifier = if config.use_pkce {
        Some(random_token(64)?)
    } else {
        None
    };
    let encrypted_verifier = verifier
        .as_deref()
        .map(|value| state.crypto.encrypt(value.as_bytes()))
        .transpose()?;
    let nonce = (config.provider == "oidc" && config.validate_id_token)
        .then(|| random_token(32))
        .transpose()?;
    let encrypted_nonce = nonce
        .as_deref()
        .map(|value| state.crypto.encrypt(value.as_bytes()))
        .transpose()?;
    sqlx::query(
        "DELETE FROM external_oauth_flows WHERE consumed_at IS NOT NULL \
         OR datetime(expires_at) <= CURRENT_TIMESTAMP",
    )
    .execute(&state.pool)
    .await?;
    sqlx::query(
        "INSERT INTO external_oauth_flows \
         (provider, state_hash, intent, user_id, encrypted_verifier, encrypted_nonce, expires_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&config.provider)
    .bind(token_hash(&flow_state))
    .bind(intent)
    .bind(user_id)
    .bind(encrypted_verifier)
    .bind(encrypted_nonce)
    .bind((Utc::now() + Duration::minutes(10)).to_rfc3339())
    .execute(&state.pool)
    .await?;

    let mut url = url::Url::parse(&config.authorize_url)
        .map_err(|_| ApiError::internal("stored OIDC authorize URL is invalid"))?;
    let redirect_uri = callback_url(state, &config.provider);
    let mut query = url.query_pairs_mut();
    query.append_pair("response_type", "code");
    if config.provider.starts_with("wechat_") {
        query.append_pair("appid", &config.client_id);
    } else {
        query.append_pair("client_id", &config.client_id);
    }
    query
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("scope", &config.scopes)
        .append_pair("state", &flow_state);
    if let Some(verifier) = verifier {
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        query
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256");
    }
    if let Some(nonce) = nonce {
        query.append_pair("nonce", &nonce);
    }
    if config.provider == "dingtalk" {
        query.append_pair("prompt", "consent");
    }
    drop(query);
    if config.provider.starts_with("wechat_") {
        url.set_fragment(Some("wechat_redirect"));
    }
    Ok(url.to_string())
}

#[derive(Deserialize)]
struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

async fn callback(
    State(state): State<AppState>,
    Path(provider): Path<String>,
    Query(query): Query<CallbackQuery>,
) -> Response {
    if query
        .error
        .as_deref()
        .is_some_and(|value| !value.is_empty())
    {
        return error_redirect(&state, "PROVIDER_DENIED");
    }
    match complete_callback(&state, &provider, query).await {
        Ok(response) => response,
        Err(error) => {
            tracing::warn!(provider, code = error.code, message = %error.message, "user OAuth callback failed");
            error_redirect(&state, error.code)
        }
    }
}

async fn complete_callback(
    state: &AppState,
    provider: &str,
    query: CallbackQuery,
) -> ApiResult<Response> {
    let provider = normalize_provider(provider)?;
    let code = query
        .code
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError::bad_request("OAUTH_CODE_REQUIRED", "OAuth code is required"))?;
    let flow_state = query
        .state
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError::bad_request("OAUTH_STATE_REQUIRED", "OAuth state is required"))?;
    if code.len() > 4096 || flow_state.len() > 256 {
        return Err(ApiError::bad_request(
            "OAUTH_CALLBACK_INVALID",
            "OAuth callback parameters are invalid",
        ));
    }
    let mut transaction = state.pool.begin().await?;
    let flow: Option<(i64, String, Option<i64>, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT id, intent, user_id, encrypted_verifier, encrypted_nonce FROM external_oauth_flows \
         WHERE provider = ? AND state_hash = ? AND consumed_at IS NULL \
         AND datetime(expires_at) > CURRENT_TIMESTAMP",
    )
    .bind(&provider)
    .bind(token_hash(flow_state))
    .fetch_optional(&mut *transaction)
    .await?;
    let flow = flow.ok_or_else(|| {
        ApiError::bad_request("OAUTH_STATE_INVALID", "OAuth state is invalid or expired")
    })?;
    let consumed = sqlx::query(
        "UPDATE external_oauth_flows SET consumed_at = CURRENT_TIMESTAMP \
         WHERE id = ? AND consumed_at IS NULL",
    )
    .bind(flow.0)
    .execute(&mut *transaction)
    .await?;
    if consumed.rows_affected() != 1 {
        return Err(ApiError::bad_request(
            "OAUTH_STATE_INVALID",
            "OAuth state is invalid or expired",
        ));
    }
    transaction.commit().await?;

    let config = load_provider(state, &provider, true).await?;
    let identity_provider = identity_provider(&provider);
    let verifier = flow
        .3
        .as_deref()
        .map(|encrypted| {
            state.crypto.decrypt(encrypted).and_then(|bytes| {
                String::from_utf8(bytes)
                    .map_err(|_| ApiError::internal("stored OAuth verifier is malformed"))
            })
        })
        .transpose()?;
    let expected_nonce = flow
        .4
        .as_deref()
        .map(|encrypted| {
            state.crypto.decrypt(encrypted).and_then(|bytes| {
                String::from_utf8(bytes)
                    .map_err(|_| ApiError::internal("stored OAuth nonce is malformed"))
            })
        })
        .transpose()?;
    let profile = if config.provider.starts_with("wechat_") {
        fetch_wechat_profile(state, &config, code).await?
    } else {
        let token = exchange_code(state, &config, code, verifier.as_deref()).await?;
        let id_token_subject = if config.provider == "oidc" && config.validate_id_token {
            Some(
                validate_oidc_id_token(
                    state,
                    &config,
                    token.id_token.as_deref().ok_or_else(|| {
                        ApiError::new(
                            StatusCode::BAD_GATEWAY,
                            "OAUTH_ID_TOKEN_MISSING",
                            "OIDC token response has no ID token",
                        )
                    })?,
                    expected_nonce
                        .as_deref()
                        .ok_or_else(|| ApiError::internal("OIDC flow has no validation nonce"))?,
                )
                .await?,
            )
        } else {
            None
        };
        let profile = if config.provider == "dingtalk" {
            fetch_dingtalk_profile(
                state,
                &config,
                &token.access_token,
                token.dingtalk_corp_id.as_deref().unwrap_or_default(),
            )
            .await?
        } else {
            fetch_profile(state, &config, &token.access_token).await?
        };
        if id_token_subject
            .as_deref()
            .is_some_and(|subject| subject != profile.subject)
        {
            return Err(ApiError::new(
                StatusCode::BAD_GATEWAY,
                "OAUTH_SUBJECT_MISMATCH",
                "OIDC ID token and UserInfo subjects do not match",
            ));
        }
        profile
    };
    let subject = profile.subject.clone();
    if subject.len() > 512 {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "OAUTH_SUBJECT_INVALID",
            "OIDC user subject is too long",
        ));
    }
    if flow.1 == "bind" {
        let user_id = flow
            .2
            .ok_or_else(|| ApiError::internal("OAuth bind flow has no user"))?;
        bind_identity(state, identity_provider, user_id, &profile).await?;
        sync_dingtalk_attributes(state, &config, user_id, &profile).await?;
        return Ok(Redirect::to(&ui_target(state, "#/profile?identity=bound")).into_response());
    }

    let user: Option<(i64, String, String, String)> = sqlx::query_as(
        "SELECT users.id, users.username, users.display_name, users.role \
         FROM external_auth_identities JOIN users ON users.id = external_auth_identities.user_id \
         WHERE external_auth_identities.provider = ? AND external_auth_identities.subject = ? \
         AND users.enabled = 1 AND users.deleted_at IS NULL",
    )
    .bind(identity_provider)
    .bind(&subject)
    .fetch_optional(&state.pool)
    .await?;
    let Some(user) = user else {
        let pending_token = create_pending(state, &provider, profile).await?;
        let encoded: String =
            url::form_urlencoded::byte_serialize(pending_token.as_bytes()).collect();
        return Ok(Redirect::to(&ui_target(
            state,
            &format!("#/oauth-result?status=pending&provider={provider}&token={encoded}"),
        ))
        .into_response());
    };
    sqlx::query(
        "UPDATE external_auth_identities SET display_name = ?, email = ?, email_verified = ?, \
         last_login_at = CURRENT_TIMESTAMP, updated_at = CURRENT_TIMESTAMP \
         WHERE provider = ? AND subject = ?",
    )
    .bind(profile.display_name.as_deref())
    .bind(profile.email.as_deref())
    .bind(profile.email_verified)
    .bind(identity_provider)
    .bind(&subject)
    .execute(&state.pool)
    .await?;
    sync_dingtalk_attributes(state, &config, user.0, &profile).await?;
    let session_response =
        auth::create_session_response(state, user.0, &user.1, &user.2, &user.3).await?;
    let cookie = session_response
        .headers()
        .get(header::SET_COOKIE)
        .cloned()
        .ok_or_else(|| ApiError::internal("OAuth session cookie was not created"))?;
    let mut response =
        Redirect::to(&ui_target(state, "#/overview?oauth_login=success")).into_response();
    response.headers_mut().insert(header::SET_COOKIE, cookie);
    Ok(response)
}

struct OAuthToken {
    access_token: String,
    id_token: Option<String>,
    dingtalk_corp_id: Option<String>,
}

async fn exchange_code(
    state: &AppState,
    config: &ProviderConfig,
    code: &str,
    verifier: Option<&str>,
) -> ApiResult<OAuthToken> {
    if config.provider == "dingtalk" {
        return exchange_dingtalk_code(state, config, code).await;
    }
    let mut form = vec![
        ("grant_type", "authorization_code".to_string()),
        ("client_id", config.client_id.clone()),
        ("code", code.to_string()),
        ("redirect_uri", callback_url(state, &config.provider)),
    ];
    if let Some(verifier) = verifier {
        form.push(("code_verifier", verifier.to_string()));
    }
    let mut request = state.client.post(&config.token_url);
    match config.token_auth_method.as_str() {
        "client_secret_post" => form.push((
            "client_secret",
            config.client_secret.clone().unwrap_or_default(),
        )),
        "client_secret_basic" => {
            request = request.basic_auth(&config.client_id, config.client_secret.as_deref())
        }
        "none" => {}
        _ => {
            return Err(ApiError::internal(
                "stored OAuth token auth method is invalid",
            ));
        }
    }
    let response = request.form(&form).send().await?;
    if !response.status().is_success() {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "OAUTH_EXCHANGE_FAILED",
            "OAuth provider rejected the authorization code",
        ));
    }
    let bytes = response.bytes().await?;
    if bytes.len() > 64 * 1024 {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "OAUTH_TOKEN_INVALID",
            "OAuth token response is too large",
        ));
    }
    let json_value = serde_json::from_slice::<Value>(&bytes).ok();
    let access_token = json_value
        .as_ref()
        .and_then(|value| {
            value
                .get("access_token")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        })
        .or_else(|| {
            url::form_urlencoded::parse(&bytes)
                .find(|pair| pair.0 == "access_token")
                .map(|pair| pair.1.trim().to_string())
                .filter(|value| !value.is_empty())
        })
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::BAD_GATEWAY,
                "OAUTH_TOKEN_INVALID",
                "OAuth token response has no access token",
            )
        })?;
    if access_token.len() > 16 * 1024 {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "OAUTH_TOKEN_INVALID",
            "OAuth access token is too large",
        ));
    }
    let id_token = json_value
        .as_ref()
        .and_then(|value| value.get("id_token"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            url::form_urlencoded::parse(&bytes)
                .find(|pair| pair.0 == "id_token")
                .map(|pair| pair.1.trim().to_string())
                .filter(|value| !value.is_empty())
        });
    if id_token
        .as_ref()
        .is_some_and(|value| value.len() > 64 * 1024)
    {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "OAUTH_TOKEN_INVALID",
            "OIDC ID token is too large",
        ));
    }
    Ok(OAuthToken {
        access_token,
        id_token,
        dingtalk_corp_id: None,
    })
}

async fn exchange_dingtalk_code(
    state: &AppState,
    config: &ProviderConfig,
    code: &str,
) -> ApiResult<OAuthToken> {
    let response = state
        .client
        .post(&config.token_url)
        .json(&json!({
            "clientId": config.client_id,
            "clientSecret": config.client_secret.as_deref().unwrap_or_default(),
            "code": code,
            "grantType": "authorization_code"
        }))
        .send()
        .await?;
    if !response.status().is_success() {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "OAUTH_EXCHANGE_FAILED",
            "DingTalk rejected the authorization code",
        ));
    }
    let bytes = response.bytes().await?;
    if bytes.len() > 64 * 1024 {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "OAUTH_TOKEN_INVALID",
            "DingTalk token response is too large",
        ));
    }
    let value = serde_json::from_slice::<Value>(&bytes).map_err(|_| {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            "OAUTH_TOKEN_INVALID",
            "DingTalk token response is invalid",
        )
    })?;
    reject_dingtalk_error(&value, "token exchange")?;
    let access_token = value
        .get("accessToken")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::BAD_GATEWAY,
                "OAUTH_TOKEN_INVALID",
                "DingTalk token response has no access token",
            )
        })?;
    if access_token.len() > 16 * 1024 {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "OAUTH_TOKEN_INVALID",
            "DingTalk access token is too large",
        ));
    }
    let corp_id = value
        .get("corpId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.chars().take(256).collect::<String>());
    Ok(OAuthToken {
        access_token,
        id_token: None,
        dingtalk_corp_id: corp_id,
    })
}

async fn fetch_wechat_profile(
    state: &AppState,
    config: &ProviderConfig,
    code: &str,
) -> ApiResult<PendingProfile> {
    let mut token_url = url::Url::parse(&config.token_url)
        .map_err(|_| ApiError::internal("stored WeChat token URL is invalid"))?;
    token_url
        .query_pairs_mut()
        .append_pair("appid", &config.client_id)
        .append_pair(
            "secret",
            config.client_secret.as_deref().unwrap_or_default(),
        )
        .append_pair("code", code)
        .append_pair("grant_type", "authorization_code");
    let token_value = bounded_oauth_json(
        state.client.get(token_url).send().await?,
        "WeChat token response",
    )
    .await?;
    reject_wechat_error(&token_value, "token exchange")?;
    let access_token = token_value
        .get("access_token")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::BAD_GATEWAY,
                "OAUTH_TOKEN_INVALID",
                "WeChat token response has no access token",
            )
        })?;
    let openid = token_value
        .get("openid")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::BAD_GATEWAY,
                "OAUTH_SUBJECT_MISSING",
                "WeChat token response has no OpenID",
            )
        })?;
    if access_token.len() > 16 * 1024 || openid.len() > 512 {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "OAUTH_TOKEN_INVALID",
            "WeChat token response fields are too large",
        ));
    }

    let mut userinfo_url = url::Url::parse(&config.userinfo_url)
        .map_err(|_| ApiError::internal("stored WeChat UserInfo URL is invalid"))?;
    userinfo_url
        .query_pairs_mut()
        .append_pair("access_token", access_token)
        .append_pair("openid", openid)
        .append_pair("lang", "zh_CN");
    let profile = bounded_oauth_json(
        state.client.get(userinfo_url).send().await?,
        "WeChat user profile",
    )
    .await?;
    reject_wechat_error(&profile, "UserInfo request")?;
    let subject = value_string(&profile, "unionid")
        .or_else(|| value_string(&token_value, "unionid"))
        .unwrap_or_else(|| openid.to_string());
    if subject.len() > 512 {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "OAUTH_SUBJECT_INVALID",
            "WeChat user subject is too long",
        ));
    }
    Ok(PendingProfile {
        subject,
        display_name: value_string(&profile, &config.display_name_path)
            .map(|value| value.chars().take(200).collect()),
        email: None,
        email_verified: false,
        dingtalk: None,
    })
}

async fn bounded_oauth_json(response: reqwest::Response, label: &str) -> ApiResult<Value> {
    if !response.status().is_success() {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "OAUTH_UPSTREAM_FAILED",
            format!("{label} request failed"),
        ));
    }
    let bytes = response.bytes().await?;
    if bytes.len() > 1024 * 1024 {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "OAUTH_UPSTREAM_INVALID",
            format!("{label} is too large"),
        ));
    }
    serde_json::from_slice(&bytes).map_err(|_| {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            "OAUTH_UPSTREAM_INVALID",
            format!("{label} is invalid"),
        )
    })
}

fn reject_wechat_error(value: &Value, operation: &str) -> ApiResult<()> {
    if value.get("errcode").and_then(Value::as_i64).unwrap_or(0) != 0 {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "OAUTH_UPSTREAM_FAILED",
            format!("WeChat {operation} failed"),
        ));
    }
    Ok(())
}

fn reject_dingtalk_error(value: &Value, operation: &str) -> ApiResult<()> {
    let numeric_code = value.get("errcode").and_then(Value::as_i64).unwrap_or(0);
    let text_code = value
        .get("code")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if numeric_code == 0 && text_code.is_empty() {
        return Ok(());
    }
    let code = if numeric_code != 0 {
        numeric_code.to_string()
    } else {
        text_code.to_string()
    };
    if matches!(code.as_str(), "60011" | "60121") {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "DINGTALK_CORP_REJECTED",
            "DingTalk account is not a member of the configured enterprise",
        ));
    }
    Err(ApiError::new(
        StatusCode::BAD_GATEWAY,
        "DINGTALK_UPSTREAM_FAILED",
        format!("DingTalk {operation} failed"),
    ))
}

async fn fetch_oidc_jwks(state: &AppState, url: &str, force: bool) -> ApiResult<Value> {
    if !force {
        let cache = state.oidc_jwks.lock().await;
        if let Some(cached) = cache.get(url)
            && cached.cached_at.elapsed() < StdDuration::from_secs(300)
        {
            return Ok(cached.value.clone());
        }
    }
    let response = state
        .client
        .get(url)
        .header(header::ACCEPT, "application/json")
        .header(header::USER_AGENT, "sub2api-mini")
        .timeout(StdDuration::from_secs(10))
        .send()
        .await?;
    if !response.status().is_success() {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "OAUTH_JWKS_FAILED",
            "OIDC JWKS request failed",
        ));
    }
    let bytes = response.bytes().await?;
    if bytes.len() > 256 * 1024 {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "OAUTH_JWKS_INVALID",
            "OIDC JWKS response is too large",
        ));
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            "OAUTH_JWKS_INVALID",
            "OIDC JWKS response is invalid",
        )
    })?;
    let keys = value.get("keys").and_then(Value::as_array).ok_or_else(|| {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            "OAUTH_JWKS_INVALID",
            "OIDC JWKS response has no keys",
        )
    })?;
    if keys.is_empty() || keys.len() > 64 {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "OAUTH_JWKS_INVALID",
            "OIDC JWKS key count is invalid",
        ));
    }
    state.oidc_jwks.lock().await.insert(
        url.to_string(),
        crate::state::CachedOidcJwks {
            value: value.clone(),
            cached_at: Instant::now(),
        },
    );
    Ok(value)
}

fn oidc_key<'a>(jwks: &'a Value, kid: &str, alg: &str) -> Option<&'a Value> {
    jwks.get("keys")?.as_array()?.iter().find(|key| {
        key.get("kid").and_then(Value::as_str) == Some(kid)
            && key
                .get("alg")
                .and_then(Value::as_str)
                .is_none_or(|value| value == alg)
            && key
                .get("use")
                .and_then(Value::as_str)
                .is_none_or(|value| value == "sig")
    })
}

fn oidc_b64(value: &Value, field: &str, maximum: usize) -> ApiResult<Vec<u8>> {
    let value = value
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= maximum * 2)
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::BAD_GATEWAY,
                "OAUTH_JWK_INVALID",
                "OIDC signing key is malformed",
            )
        })?;
    let decoded = URL_SAFE_NO_PAD.decode(value).map_err(|_| {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            "OAUTH_JWK_INVALID",
            "OIDC signing key is malformed",
        )
    })?;
    if decoded.is_empty() || decoded.len() > maximum {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "OAUTH_JWK_INVALID",
            "OIDC signing key size is invalid",
        ));
    }
    Ok(decoded)
}

fn verify_oidc_signature(
    key: &Value,
    alg: &str,
    signing_input: &[u8],
    signature: &[u8],
) -> ApiResult<()> {
    let verified = match alg {
        "RS256" | "PS256" if key.get("kty").and_then(Value::as_str) == Some("RSA") => {
            let n = oidc_b64(key, "n", 1024)?;
            let e = oidc_b64(key, "e", 8)?;
            let components = RsaPublicKeyComponents { n: &n, e: &e };
            if alg == "RS256" {
                components.verify(&RSA_PKCS1_2048_8192_SHA256, signing_input, signature)
            } else {
                components.verify(&RSA_PSS_2048_8192_SHA256, signing_input, signature)
            }
        }
        "ES256" if key.get("kty").and_then(Value::as_str) == Some("EC") => {
            if key.get("crv").and_then(Value::as_str) != Some("P-256") {
                return Err(ApiError::new(
                    StatusCode::BAD_GATEWAY,
                    "OAUTH_JWK_INVALID",
                    "OIDC EC signing key uses an unsupported curve",
                ));
            }
            let x = oidc_b64(key, "x", 32)?;
            let y = oidc_b64(key, "y", 32)?;
            if x.len() != 32 || y.len() != 32 {
                return Err(ApiError::new(
                    StatusCode::BAD_GATEWAY,
                    "OAUTH_JWK_INVALID",
                    "OIDC EC signing key size is invalid",
                ));
            }
            let mut public_key = Vec::with_capacity(65);
            public_key.push(4);
            public_key.extend_from_slice(&x);
            public_key.extend_from_slice(&y);
            UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, public_key)
                .verify(signing_input, signature)
        }
        _ => {
            return Err(ApiError::new(
                StatusCode::BAD_GATEWAY,
                "OAUTH_JWK_INVALID",
                "OIDC signing key does not match the token algorithm",
            ));
        }
    };
    verified.map_err(|_| {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            "OAUTH_ID_TOKEN_SIGNATURE_INVALID",
            "OIDC ID token signature is invalid",
        )
    })
}

fn oidc_audience_matches(value: Option<&Value>, client_id: &str) -> bool {
    match value {
        Some(Value::String(value)) => value == client_id,
        Some(Value::Array(values)) => values.iter().any(|value| value.as_str() == Some(client_id)),
        _ => false,
    }
}

async fn validate_oidc_id_token(
    state: &AppState,
    config: &ProviderConfig,
    token: &str,
    expected_nonce: &str,
) -> ApiResult<String> {
    let mut segments = token.split('.');
    let encoded_header = segments.next().unwrap_or_default();
    let encoded_claims = segments.next().unwrap_or_default();
    let encoded_signature = segments.next().unwrap_or_default();
    if segments.next().is_some()
        || encoded_header.is_empty()
        || encoded_claims.is_empty()
        || encoded_signature.is_empty()
        || token.len() > 64 * 1024
    {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "OAUTH_ID_TOKEN_INVALID",
            "OIDC ID token is malformed",
        ));
    }
    let header_bytes = URL_SAFE_NO_PAD.decode(encoded_header).map_err(|_| {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            "OAUTH_ID_TOKEN_INVALID",
            "OIDC ID token header is malformed",
        )
    })?;
    let claims_bytes = URL_SAFE_NO_PAD.decode(encoded_claims).map_err(|_| {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            "OAUTH_ID_TOKEN_INVALID",
            "OIDC ID token claims are malformed",
        )
    })?;
    let signature = URL_SAFE_NO_PAD.decode(encoded_signature).map_err(|_| {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            "OAUTH_ID_TOKEN_INVALID",
            "OIDC ID token signature is malformed",
        )
    })?;
    if header_bytes.len() > 16 * 1024 || claims_bytes.len() > 48 * 1024 {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "OAUTH_ID_TOKEN_INVALID",
            "OIDC ID token fields are too large",
        ));
    }
    let header_value: Value = serde_json::from_slice(&header_bytes).map_err(|_| {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            "OAUTH_ID_TOKEN_INVALID",
            "OIDC ID token header is invalid",
        )
    })?;
    let claims: Value = serde_json::from_slice(&claims_bytes).map_err(|_| {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            "OAUTH_ID_TOKEN_INVALID",
            "OIDC ID token claims are invalid",
        )
    })?;
    let alg = header_value
        .get("alg")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let kid = header_value
        .get("kid")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 256)
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::BAD_GATEWAY,
                "OAUTH_ID_TOKEN_INVALID",
                "OIDC ID token has no signing key ID",
            )
        })?;
    let allowed = config
        .allowed_signing_algs
        .split(',')
        .map(str::trim)
        .any(|value| value == alg);
    if !allowed || !matches!(alg, "RS256" | "PS256" | "ES256") {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "OAUTH_ID_TOKEN_ALG_REJECTED",
            "OIDC ID token signing algorithm is not allowed",
        ));
    }
    let mut jwks = fetch_oidc_jwks(state, &config.jwks_url, false).await?;
    let key = if let Some(key) = oidc_key(&jwks, kid, alg) {
        key
    } else {
        jwks = fetch_oidc_jwks(state, &config.jwks_url, true).await?;
        oidc_key(&jwks, kid, alg).ok_or_else(|| {
            ApiError::new(
                StatusCode::BAD_GATEWAY,
                "OAUTH_SIGNING_KEY_NOT_FOUND",
                "OIDC signing key was not found",
            )
        })?
    };
    let signing_input = format!("{encoded_header}.{encoded_claims}");
    verify_oidc_signature(key, alg, signing_input.as_bytes(), &signature)?;

    let now = Utc::now().timestamp();
    let skew = config.clock_skew_seconds;
    let issuer_matches =
        claims.get("iss").and_then(Value::as_str) == Some(config.issuer_url.as_str());
    let expires_at = claims.get("exp").and_then(Value::as_i64).unwrap_or(0);
    let not_before = claims.get("nbf").and_then(Value::as_i64);
    let issued_at = claims.get("iat").and_then(Value::as_i64);
    if !issuer_matches
        || !oidc_audience_matches(claims.get("aud"), &config.client_id)
        || expires_at < now - skew
        || not_before.is_some_and(|value| value > now + skew)
        || issued_at.is_some_and(|value| value > now + skew)
        || claims.get("nonce").and_then(Value::as_str) != Some(expected_nonce)
    {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "OAUTH_ID_TOKEN_CLAIMS_INVALID",
            "OIDC ID token claims are invalid",
        ));
    }
    claims
        .get("sub")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.len() <= 512)
        .map(ToOwned::to_owned)
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::BAD_GATEWAY,
                "OAUTH_SUBJECT_MISSING",
                "OIDC ID token has no subject",
            )
        })
}

#[derive(Default)]
struct DingTalkStaff {
    user_id: String,
    name: String,
    nickname: String,
    email: Option<String>,
    department_ids: Vec<i64>,
}

async fn fetch_dingtalk_profile(
    state: &AppState,
    config: &ProviderConfig,
    user_token: &str,
    corp_id: &str,
) -> ApiResult<PendingProfile> {
    let personal = bounded_oauth_json(
        state
            .client
            .get(&config.userinfo_url)
            .header(header::ACCEPT, "application/json")
            .header("x-acs-dingtalk-access-token", user_token)
            .send()
            .await?,
        "DingTalk user profile",
    )
    .await?;
    reject_dingtalk_error(&personal, "user profile request")?;
    let subject = value_string(&personal, &config.subject_path).ok_or_else(|| {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            "OAUTH_SUBJECT_MISSING",
            "DingTalk user profile does not contain a union ID",
        )
    })?;
    if subject.len() > 512 {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "OAUTH_SUBJECT_INVALID",
            "DingTalk union ID is too long",
        ));
    }
    let personal_nick = value_string(&personal, &config.display_name_path)
        .unwrap_or_default()
        .chars()
        .take(200)
        .collect::<String>();

    let enterprise = fetch_dingtalk_staff(state, config, &subject).await;
    let (staff, app_token) = match enterprise {
        Ok(value) => value,
        Err(error) if config.dingtalk_corp_policy == "internal_only" => return Err(error),
        Err(error) => {
            tracing::debug!(
                code = error.code,
                "DingTalk enterprise lookup unavailable; using personal profile"
            );
            (DingTalkStaff::default(), String::new())
        }
    };
    let corporate_email = staff
        .email
        .as_deref()
        .and_then(|value| auth::normalize_email(value).ok());
    let email = corporate_email.clone().or_else(|| {
        (!config.dingtalk_require_email).then(|| {
            format!(
                "dingtalk-{}@dingtalk-connect.invalid",
                subject.to_ascii_lowercase()
            )
        })
    });
    let department_path = if config.dingtalk_corp_policy == "internal_only"
        && config.dingtalk_sync_dept
        && !app_token.is_empty()
    {
        match resolve_dingtalk_department_path(state, config, &app_token, &staff.department_ids)
            .await
        {
            Ok(path) => path,
            Err(error) => {
                tracing::warn!(
                    code = error.code,
                    "DingTalk department synchronization failed"
                );
                None
            }
        }
    } else {
        None
    };
    let display_name = [&staff.name, &personal_nick, &staff.nickname]
        .into_iter()
        .find(|value| !value.trim().is_empty())
        .map(|value| value.trim().chars().take(200).collect::<String>());
    Ok(PendingProfile {
        subject,
        display_name,
        email,
        email_verified: corporate_email.is_some(),
        dingtalk: Some(DingTalkProfile {
            corp_id: corp_id.trim().chars().take(256).collect(),
            user_id: staff.user_id,
            staff_name: staff.name,
            nickname: if personal_nick.is_empty() {
                staff.nickname
            } else {
                personal_nick
            },
            corporate_email,
            department_ids: staff.department_ids,
            department_path,
        }),
    })
}

async fn fetch_dingtalk_staff(
    state: &AppState,
    config: &ProviderConfig,
    union_id: &str,
) -> ApiResult<(DingTalkStaff, String)> {
    let mut app_token_url = url::Url::parse(&config.token_url)
        .map_err(|_| ApiError::internal("stored DingTalk token URL is invalid"))?;
    let path = app_token_url
        .path()
        .replace("/oauth2/userAccessToken", "/oauth2/accessToken");
    app_token_url.set_path(&path);
    app_token_url.set_query(None);
    let token = bounded_oauth_json(
        state
            .client
            .post(app_token_url)
            .json(&json!({
                "appKey": config.client_id,
                "appSecret": config.client_secret.as_deref().unwrap_or_default()
            }))
            .send()
            .await?,
        "DingTalk application token",
    )
    .await?;
    reject_dingtalk_error(&token, "application token request")?;
    let app_token = token
        .get("accessToken")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.len() <= 16 * 1024)
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::BAD_GATEWAY,
                "DINGTALK_UPSTREAM_INVALID",
                "DingTalk application token response is invalid",
            )
        })?
        .to_string();

    let user_id_value = dingtalk_oapi_post(
        state,
        config,
        "/topapi/user/getbyunionid",
        &app_token,
        json!({"unionid": union_id}),
        "enterprise member lookup",
    )
    .await?;
    let user_id = value_string(&user_id_value, "result.userid").ok_or_else(|| {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            "DINGTALK_UPSTREAM_INVALID",
            "DingTalk enterprise member response has no user ID",
        )
    })?;
    let staff_value = dingtalk_oapi_post(
        state,
        config,
        "/topapi/v2/user/get",
        &app_token,
        json!({"userid": user_id}),
        "staff profile request",
    )
    .await?;
    let result = staff_value.get("result").ok_or_else(|| {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            "DINGTALK_UPSTREAM_INVALID",
            "DingTalk staff profile is invalid",
        )
    })?;
    let returned_user_id = value_string(result, "userid").unwrap_or_else(|| user_id.clone());
    let extension_email = result
        .get("extension")
        .and_then(Value::as_str)
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .as_ref()
        .and_then(|extension| value_string(extension, "企业邮箱"));
    let email = value_string(result, "org_email")
        .or_else(|| value_string(result, "email"))
        .or(extension_email);
    let department_ids = result
        .get("dept_id_list")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_i64)
                .filter(|value| *value > 0)
                .take(100)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Ok((
        DingTalkStaff {
            user_id: returned_user_id,
            name: value_string(result, "name").unwrap_or_default(),
            nickname: value_string(result, "nickname").unwrap_or_default(),
            email,
            department_ids,
        },
        app_token,
    ))
}

fn dingtalk_oapi_url(config: &ProviderConfig, path: &str, app_token: &str) -> ApiResult<url::Url> {
    let mut url = url::Url::parse(&config.userinfo_url)
        .map_err(|_| ApiError::internal("stored DingTalk UserInfo URL is invalid"))?;
    if let Some(host) = url.host_str().map(ToOwned::to_owned)
        && let Some(suffix) = host.strip_prefix("api.")
    {
        url.set_host(Some(&format!("oapi.{suffix}")))
            .map_err(|_| ApiError::internal("stored DingTalk UserInfo host is invalid"))?;
    }
    url.set_path(path);
    url.set_query(None);
    url.query_pairs_mut().append_pair("access_token", app_token);
    Ok(url)
}

async fn dingtalk_oapi_post(
    state: &AppState,
    config: &ProviderConfig,
    path: &str,
    app_token: &str,
    body: Value,
    operation: &str,
) -> ApiResult<Value> {
    let value = bounded_oauth_json(
        state
            .client
            .post(dingtalk_oapi_url(config, path, app_token)?)
            .json(&body)
            .send()
            .await?,
        &format!("DingTalk {operation}"),
    )
    .await?;
    reject_dingtalk_error(&value, operation)?;
    Ok(value)
}

async fn resolve_dingtalk_department_path(
    state: &AppState,
    config: &ProviderConfig,
    app_token: &str,
    department_ids: &[i64],
) -> ApiResult<Option<String>> {
    let Some(mut current) = department_ids
        .iter()
        .copied()
        .find(|value| *value > 1)
        .or_else(|| department_ids.first().copied())
    else {
        return Ok(None);
    };
    let mut visited = HashSet::new();
    let mut parts = Vec::new();
    for _ in 0..50 {
        if current < 1 || !visited.insert(current) {
            break;
        }
        let value = dingtalk_oapi_post(
            state,
            config,
            "/topapi/v2/department/get",
            app_token,
            json!({"dept_id": current, "language": "zh_CN"}),
            "department request",
        )
        .await?;
        let result = value.get("result").ok_or_else(|| {
            ApiError::new(
                StatusCode::BAD_GATEWAY,
                "DINGTALK_UPSTREAM_INVALID",
                "DingTalk department response is invalid",
            )
        })?;
        if let Some(name) = value_string(result, "name") {
            parts.push(name);
        }
        let parent = result.get("parent_id").and_then(Value::as_i64).unwrap_or(0);
        if parent < 1 || parent == current {
            break;
        }
        current = parent;
    }
    parts.reverse();
    if !parts.is_empty() {
        parts.remove(0);
    }
    Ok(Some(parts.join("/")))
}

async fn fetch_profile(
    state: &AppState,
    config: &ProviderConfig,
    access_token: &str,
) -> ApiResult<PendingProfile> {
    let mut request = state
        .client
        .get(&config.userinfo_url)
        .header(header::ACCEPT, "application/json")
        .header(header::USER_AGENT, "sub2api-mini");
    if config.provider == "dingtalk" {
        request = request.header("x-acs-dingtalk-access-token", access_token);
    } else {
        request = request.bearer_auth(access_token);
    }
    let response = request.send().await?;
    if !response.status().is_success() {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "OAUTH_USERINFO_FAILED",
            "OAuth user profile request failed",
        ));
    }
    let bytes = response.bytes().await?;
    if bytes.len() > 1024 * 1024 {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "OAUTH_USERINFO_INVALID",
            "OAuth user profile is too large",
        ));
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            "OAUTH_USERINFO_INVALID",
            "OAuth user profile is invalid",
        )
    })?;
    let subject = value_string(&value, &config.subject_path).ok_or_else(|| {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            "OAUTH_SUBJECT_MISSING",
            "OAuth user profile does not contain a subject",
        )
    })?;
    let display_name = value_string(&value, &config.display_name_path)
        .or_else(|| {
            (config.profile_mode == "github")
                .then(|| value_string(&value, "login"))
                .flatten()
        })
        .map(|value| value.chars().take(200).collect::<String>());
    let mut email = value_string(&value, &config.email_path)
        .and_then(|email| auth::normalize_email(&email).ok());
    let email_verified = match config.profile_mode.as_str() {
        "github" => {
            email = Some(fetch_github_email(state, config, access_token).await?);
            true
        }
        "google" => {
            let verified = value
                .get("email_verified")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if email.is_none() || !verified {
                return Err(ApiError::new(
                    StatusCode::BAD_GATEWAY,
                    "OAUTH_EMAIL_UNVERIFIED",
                    "Google did not return a verified email address",
                ));
            }
            true
        }
        "linuxdo" => false,
        _ => value
            .get("email_verified")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    };
    if config.require_email_verified && (email.is_none() || !email_verified) {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "OAUTH_EMAIL_UNVERIFIED",
            "OIDC provider did not return a verified email address",
        ));
    }
    Ok(PendingProfile {
        subject,
        display_name,
        email,
        email_verified,
        dingtalk: None,
    })
}

async fn fetch_github_email(
    state: &AppState,
    config: &ProviderConfig,
    access_token: &str,
) -> ApiResult<String> {
    let response = state
        .client
        .get(&config.emails_url)
        .header(header::ACCEPT, "application/vnd.github+json")
        .header(header::USER_AGENT, "sub2api-mini")
        .bearer_auth(access_token)
        .send()
        .await?;
    if !response.status().is_success() {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "OAUTH_EMAIL_FAILED",
            "GitHub verified email request failed",
        ));
    }
    let bytes = response.bytes().await?;
    if bytes.len() > 1024 * 1024 {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "OAUTH_EMAIL_INVALID",
            "GitHub email response is too large",
        ));
    }
    let rows: Vec<Value> = serde_json::from_slice(&bytes).map_err(|_| {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            "OAUTH_EMAIL_INVALID",
            "GitHub email response is invalid",
        )
    })?;
    for primary_only in [true, false] {
        for row in &rows {
            if row.get("verified").and_then(Value::as_bool) != Some(true)
                || (primary_only && row.get("primary").and_then(Value::as_bool) != Some(true))
            {
                continue;
            }
            if let Some(email) = row
                .get("email")
                .and_then(Value::as_str)
                .and_then(|email| auth::normalize_email(email).ok())
            {
                return Ok(email);
            }
        }
    }
    Err(ApiError::new(
        StatusCode::BAD_GATEWAY,
        "OAUTH_EMAIL_UNVERIFIED",
        "GitHub did not return a verified email address",
    ))
}

fn value_string(value: &Value, path: &str) -> Option<String> {
    let value = path
        .split('.')
        .filter(|part| !part.is_empty())
        .try_fold(value, |current, part| current.get(part))?;
    match value {
        Value::String(value) => Some(value.trim().to_string()).filter(|value| !value.is_empty()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

async fn create_pending(
    state: &AppState,
    provider: &str,
    profile: PendingProfile,
) -> ApiResult<String> {
    let provider = normalize_provider(provider)?;
    let token = random_token(32)?;
    let serialized = serde_json::to_vec(&profile)
        .map_err(|_| ApiError::internal("OIDC pending profile serialization failed"))?;
    let encrypted_profile = state.crypto.encrypt(&serialized)?;
    sqlx::query(
        "DELETE FROM external_oauth_pending WHERE consumed_at IS NOT NULL \
         OR datetime(expires_at) <= CURRENT_TIMESTAMP",
    )
    .execute(&state.pool)
    .await?;
    sqlx::query(
        "INSERT INTO external_oauth_pending \
         (provider, token_hash, encrypted_profile, expires_at) VALUES (?, ?, ?, ?)",
    )
    .bind(provider)
    .bind(token_hash(&token))
    .bind(encrypted_profile)
    .bind((Utc::now() + Duration::minutes(10)).to_rfc3339())
    .execute(&state.pool)
    .await?;
    Ok(token)
}

#[derive(Deserialize)]
struct PendingTokenInput {
    token: String,
}

async fn load_pending(state: &AppState, token: &str) -> ApiResult<(i64, String, PendingProfile)> {
    let token = token.trim();
    if token.is_empty() || token.len() > 256 {
        return Err(pending_invalid());
    }
    let row: Option<(i64, String, String)> = sqlx::query_as(
        "SELECT id, provider, encrypted_profile FROM external_oauth_pending \
         WHERE token_hash = ? AND consumed_at IS NULL \
         AND datetime(expires_at) > CURRENT_TIMESTAMP",
    )
    .bind(token_hash(token))
    .fetch_optional(&state.pool)
    .await?;
    let (id, provider, encrypted_profile) = row.ok_or_else(pending_invalid)?;
    normalize_provider(&provider)?;
    let decrypted = state.crypto.decrypt(&encrypted_profile)?;
    let profile = serde_json::from_slice(&decrypted)
        .map_err(|_| ApiError::internal("stored OIDC pending profile is malformed"))?;
    Ok((id, provider, profile))
}

fn pending_invalid() -> ApiError {
    ApiError::bad_request(
        "OAUTH_PENDING_INVALID",
        "OAuth continuation is invalid, expired, or already used",
    )
}

async fn inspect_pending(
    State(state): State<AppState>,
    Json(input): Json<PendingTokenInput>,
) -> ApiResult<Json<Value>> {
    let (_, provider, profile) = load_pending(&state, &input.token).await?;
    let config = load_provider(&state, &provider, true).await?;
    let registration_enabled = auth::bool_setting(&state, "registration_enabled", false).await?
        || dingtalk_registration_bypass(&config);
    let email_verification_enabled =
        auth::bool_setting(&state, "email_verification_enabled", false).await?;
    Ok(Json(json!({"data": {
        "provider": provider,
        "provider_name": config.name,
        "display_name": profile.display_name,
        "suggested_email": profile.email,
        "email_hint": profile.email.as_deref().map(mask_email),
        "provider_email_verified": profile.email_verified,
        "registration_enabled": registration_enabled,
        "email_verification_required": email_verification_enabled && !profile.email_verified,
        "email_verification_enabled": email_verification_enabled
    }})))
}

fn mask_email(email: &str) -> String {
    let Some((local, domain)) = email.split_once('@') else {
        return "***".into();
    };
    let prefix = local.chars().take(2).collect::<String>();
    format!("{prefix}***@{domain}")
}

#[derive(Deserialize)]
struct BindPendingInput {
    token: String,
    #[serde(alias = "email")]
    identifier: String,
    password: String,
    totp_code: Option<String>,
}

async fn bind_pending(
    State(state): State<AppState>,
    Json(input): Json<BindPendingInput>,
) -> ApiResult<Response> {
    let (pending_id, provider, profile) = load_pending(&state, &input.token).await?;
    let config = load_provider(&state, &provider, true).await?;
    let identity_provider = identity_provider(&provider);
    let identifier = input.identifier.trim();
    let user: Option<(i64, String, String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT id, username, password_hash, display_name, role, totp_secret FROM users \
         WHERE (username = ? COLLATE NOCASE OR email = ? COLLATE NOCASE) \
         AND enabled = 1 AND deleted_at IS NULL",
    )
    .bind(identifier)
    .bind(identifier)
    .fetch_optional(&state.pool)
    .await?;
    let user = user.ok_or_else(|| ApiError::unauthorized("invalid username or password"))?;
    if !verify_password(&input.password, &user.2) {
        return Err(ApiError::unauthorized("invalid username or password"));
    }
    if user.5.is_some() {
        let code = input
            .totp_code
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                ApiError::new(
                    StatusCode::UNAUTHORIZED,
                    "TOTP_REQUIRED",
                    "two-factor authentication code is required",
                )
            })?;
        if !crate::totp::verify_user_code(&state, user.0, code, true).await? {
            return Err(ApiError::new(
                StatusCode::UNAUTHORIZED,
                "TOTP_INVALID",
                "two-factor authentication code is invalid",
            ));
        }
    }

    let mut transaction = state.pool.begin().await?;
    consume_pending(&mut transaction, pending_id).await?;
    bind_identity_transaction(&mut transaction, identity_provider, user.0, &profile).await?;
    transaction.commit().await?;
    sync_dingtalk_attributes(&state, &config, user.0, &profile).await?;
    auth::create_session_response(&state, user.0, &user.1, &user.3, &user.4).await
}

#[derive(Deserialize)]
struct RegisterPendingInput {
    token: String,
    email: String,
    password: String,
    verify_code: Option<String>,
}

async fn register_pending(
    State(state): State<AppState>,
    Json(input): Json<RegisterPendingInput>,
) -> ApiResult<Response> {
    let (pending_id, provider, profile) = load_pending(&state, &input.token).await?;
    let config = load_provider(&state, &provider, true).await?;
    let identity_provider = identity_provider(&provider);
    if !auth::bool_setting(&state, "registration_enabled", false).await?
        && !dingtalk_registration_bypass(&config)
    {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "REGISTRATION_DISABLED",
            "registration is disabled",
        ));
    }
    let email = auth::normalize_email(&input.email)?;
    auth::validate_password(&input.password)?;
    let provider_email_verified = profile.email_verified
        && profile
            .email
            .as_deref()
            .is_some_and(|value| value.eq_ignore_ascii_case(&email));
    let verify_email = auth::bool_setting(&state, "email_verification_enabled", false).await?;
    let username = auth::username_for_email(&email);
    let display_name = profile
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.chars().take(200).collect::<String>())
        .unwrap_or_else(|| email.split('@').next().unwrap_or("user").to_string());
    let password_hash = hash_password(&input.password)?;
    let mut transaction = state.pool.begin().await?;

    consume_pending(&mut transaction, pending_id).await?;
    if verify_email && !provider_email_verified {
        let verify_code = input
            .verify_code
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                ApiError::bad_request(
                    "EMAIL_VERIFICATION_REQUIRED",
                    "email verification code is required",
                )
            })?;
        let consumed = sqlx::query(
            "UPDATE auth_challenges SET consumed_at = CURRENT_TIMESTAMP \
             WHERE email = ? COLLATE NOCASE AND purpose = 'email_verification' \
             AND token_hash = ? AND consumed_at IS NULL \
             AND datetime(expires_at) > CURRENT_TIMESTAMP",
        )
        .bind(&email)
        .bind(token_hash(verify_code))
        .execute(&mut *transaction)
        .await?;
        if consumed.rows_affected() != 1 {
            return Err(ApiError::bad_request(
                "INVALID_VERIFICATION_CODE",
                "verification code is invalid or expired",
            ));
        }
    }

    ensure_identity_available(&mut transaction, identity_provider, None, &profile.subject).await?;
    let user_id = sqlx::query(
        "INSERT INTO users \
         (username, display_name, password_hash, role, email, email_verified) \
         VALUES (?, ?, ?, 'user', ?, ?)",
    )
    .bind(&username)
    .bind(&display_name)
    .bind(password_hash)
    .bind(&email)
    .bind(provider_email_verified || verify_email)
    .execute(&mut *transaction)
    .await
    .map_err(|error| match error {
        sqlx::Error::Database(ref database) if database.is_unique_violation() => {
            ApiError::bad_request("EMAIL_EXISTS", "email is already registered")
        }
        other => other.into(),
    })?
    .last_insert_rowid();
    insert_identity(&mut transaction, identity_provider, user_id, &profile).await?;
    transaction.commit().await?;
    sync_dingtalk_attributes(&state, &config, user_id, &profile).await?;
    auth::create_session_response(&state, user_id, &username, &display_name, "user").await
}

fn dingtalk_registration_bypass(config: &ProviderConfig) -> bool {
    config.provider == "dingtalk"
        && config.dingtalk_corp_policy == "internal_only"
        && config.dingtalk_bypass_registration
}

async fn sync_dingtalk_attributes(
    state: &AppState,
    config: &ProviderConfig,
    user_id: i64,
    profile: &PendingProfile,
) -> ApiResult<()> {
    if config.provider != "dingtalk" || config.dingtalk_corp_policy != "internal_only" {
        return Ok(());
    }
    let Some(dingtalk) = profile.dingtalk.as_ref() else {
        return Ok(());
    };
    let mut values = Vec::with_capacity(3);
    if config.dingtalk_sync_corp_email
        && let Some(email) = dingtalk.corporate_email.as_deref()
    {
        values.push((
            config.dingtalk_email_attr_key.as_str(),
            config.dingtalk_email_attr_name.as_str(),
            email,
        ));
    }
    if config.dingtalk_sync_display_name && !dingtalk.staff_name.trim().is_empty() {
        values.push((
            config.dingtalk_name_attr_key.as_str(),
            config.dingtalk_name_attr_name.as_str(),
            dingtalk.staff_name.trim(),
        ));
    }
    if config.dingtalk_sync_dept
        && let Some(path) = dingtalk.department_path.as_deref()
    {
        values.push((
            config.dingtalk_dept_attr_key.as_str(),
            config.dingtalk_dept_attr_name.as_str(),
            path,
        ));
    }
    if values.is_empty() {
        return Ok(());
    }
    let mut transaction = state.pool.begin().await?;
    for (key, name, value) in values {
        sqlx::query(
            "INSERT INTO user_external_attributes \
             (user_id, provider, attribute_key, attribute_name, value) VALUES (?, 'dingtalk', ?, ?, ?) \
             ON CONFLICT(user_id, provider, attribute_key) DO UPDATE SET \
             attribute_name = excluded.attribute_name, value = excluded.value, \
             updated_at = CURRENT_TIMESTAMP",
        )
        .bind(user_id)
        .bind(key)
        .bind(name)
        .bind(value)
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;
    Ok(())
}

async fn consume_pending(
    transaction: &mut Transaction<'_, Sqlite>,
    pending_id: i64,
) -> ApiResult<()> {
    let result = sqlx::query(
        "UPDATE external_oauth_pending SET consumed_at = CURRENT_TIMESTAMP \
         WHERE id = ? AND consumed_at IS NULL AND datetime(expires_at) > CURRENT_TIMESTAMP",
    )
    .bind(pending_id)
    .execute(&mut **transaction)
    .await?;
    if result.rows_affected() != 1 {
        return Err(pending_invalid());
    }
    Ok(())
}

async fn ensure_identity_available(
    transaction: &mut Transaction<'_, Sqlite>,
    provider: &str,
    user_id: Option<i64>,
    subject: &str,
) -> ApiResult<()> {
    let owner: Option<i64> = sqlx::query_scalar(
        "SELECT user_id FROM external_auth_identities WHERE provider = ? AND subject = ?",
    )
    .bind(provider)
    .bind(subject)
    .fetch_optional(&mut **transaction)
    .await?;
    if owner.is_some_and(|owner| Some(owner) != user_id) {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "OAUTH_IDENTITY_IN_USE",
            "this OIDC identity is already bound to another account",
        ));
    }
    if let Some(user_id) = user_id {
        let current_subject: Option<String> = sqlx::query_scalar(
            "SELECT subject FROM external_auth_identities WHERE user_id = ? AND provider = ?",
        )
        .bind(user_id)
        .bind(provider)
        .fetch_optional(&mut **transaction)
        .await?;
        if current_subject
            .as_deref()
            .is_some_and(|current| current != subject)
        {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "OAUTH_PROVIDER_ALREADY_BOUND",
                "unbind the current OIDC identity before binding another one",
            ));
        }
    }
    Ok(())
}

async fn insert_identity(
    transaction: &mut Transaction<'_, Sqlite>,
    provider: &str,
    user_id: i64,
    profile: &PendingProfile,
) -> ApiResult<()> {
    sqlx::query(
        "INSERT INTO external_auth_identities \
         (user_id, provider, subject, display_name, email, email_verified) VALUES (?, ?, ?, ?, ?, ?) \
         ON CONFLICT(provider, subject) DO UPDATE SET display_name = excluded.display_name, \
         email = excluded.email, email_verified = excluded.email_verified, updated_at = CURRENT_TIMESTAMP",
    )
    .bind(user_id)
    .bind(provider)
    .bind(&profile.subject)
    .bind(&profile.display_name)
    .bind(&profile.email)
    .bind(profile.email_verified)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn bind_identity_transaction(
    transaction: &mut Transaction<'_, Sqlite>,
    provider: &str,
    user_id: i64,
    profile: &PendingProfile,
) -> ApiResult<()> {
    ensure_identity_available(transaction, provider, Some(user_id), &profile.subject).await?;
    insert_identity(transaction, provider, user_id, profile).await
}

async fn bind_identity(
    state: &AppState,
    provider: &str,
    user_id: i64,
    profile: &PendingProfile,
) -> ApiResult<()> {
    let subject = &profile.subject;
    let active: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM users WHERE id = ? AND enabled = 1 AND deleted_at IS NULL",
    )
    .bind(user_id)
    .fetch_one(&state.pool)
    .await?;
    if active != 1 {
        return Err(ApiError::forbidden("local account is disabled"));
    }
    let owner: Option<i64> = sqlx::query_scalar(
        "SELECT user_id FROM external_auth_identities WHERE provider = ? AND subject = ?",
    )
    .bind(provider)
    .bind(subject)
    .fetch_optional(&state.pool)
    .await?;
    if owner.is_some_and(|owner| owner != user_id) {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "OAUTH_IDENTITY_IN_USE",
            "this OIDC identity is already bound to another account",
        ));
    }
    let current_subject: Option<String> = sqlx::query_scalar(
        "SELECT subject FROM external_auth_identities WHERE user_id = ? AND provider = ?",
    )
    .bind(user_id)
    .bind(provider)
    .fetch_optional(&state.pool)
    .await?;
    if current_subject
        .as_deref()
        .is_some_and(|current| current != subject)
    {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "OAUTH_PROVIDER_ALREADY_BOUND",
            "unbind the current OIDC identity before binding another one",
        ));
    }
    sqlx::query(
        "INSERT INTO external_auth_identities \
         (user_id, provider, subject, display_name, email, email_verified) VALUES (?, ?, ?, ?, ?, ?) \
         ON CONFLICT(provider, subject) DO UPDATE SET display_name = excluded.display_name, \
         email = excluded.email, email_verified = excluded.email_verified, updated_at = CURRENT_TIMESTAMP",
    )
    .bind(user_id)
    .bind(provider)
    .bind(subject)
    .bind(profile.display_name.as_deref())
    .bind(profile.email.as_deref())
    .bind(profile.email_verified)
    .execute(&state.pool)
    .await?;
    Ok(())
}

async fn list_identities(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
) -> ApiResult<Json<Value>> {
    let providers: Vec<(String, String, bool)> = sqlx::query_as(
        "SELECT provider, name, enabled FROM external_auth_providers ORDER BY provider",
    )
    .fetch_all(&state.pool)
    .await?;
    let identities: Vec<(
        String,
        String,
        Option<String>,
        Option<String>,
        bool,
        Option<String>,
    )> = sqlx::query_as(
        "SELECT provider, subject, display_name, email, email_verified, last_login_at \
             FROM external_auth_identities WHERE user_id = ? ORDER BY provider",
    )
    .bind(session.user_id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(json!({"data": {
        "providers": providers.into_iter().map(|row| json!({
            "provider": row.0, "name": row.1, "enabled": row.2
        })).collect::<Vec<_>>(),
        "identities": identities.into_iter().map(|row| json!({
            "provider": row.0, "subject_hint": mask_subject(&row.1), "display_name": row.2,
            "email": row.3, "email_verified": row.4, "last_login_at": row.5
        })).collect::<Vec<_>>()
    }})))
}

fn mask_subject(subject: &str) -> String {
    let chars = subject.chars().collect::<Vec<_>>();
    if chars.len() <= 8 {
        return "********".into();
    }
    format!(
        "{}…{}",
        chars[..4].iter().collect::<String>(),
        chars[chars.len() - 4..].iter().collect::<String>()
    )
}

async fn unbind_identity(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
    Path(provider): Path<String>,
) -> ApiResult<StatusCode> {
    let provider = normalize_provider(&provider)?;
    let identity_provider = identity_provider(&provider);
    let result =
        sqlx::query("DELETE FROM external_auth_identities WHERE user_id = ? AND provider = ?")
            .bind(session.user_id)
            .bind(identity_provider)
            .execute(&state.pool)
            .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found("OAuth identity is not bound"));
    }
    sqlx::query("DELETE FROM auth_sessions WHERE user_id = ? AND id != ?")
        .bind(session.user_id)
        .bind(session.id)
        .execute(&state.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn admin_providers(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let mut providers = Vec::with_capacity(SUPPORTED_PROVIDERS.len());
    for provider in SUPPORTED_PROVIDERS {
        let config = load_provider(&state, provider, false).await?;
        let mut value = provider_json(&config);
        value["callback_url"] = json!(callback_url(&state, provider));
        providers.push(value);
    }
    Ok(Json(json!({"data": providers})))
}

async fn admin_provider(
    State(state): State<AppState>,
    Path(provider): Path<String>,
) -> ApiResult<Json<Value>> {
    let config = load_provider(&state, &provider, false).await?;
    let mut value = provider_json(&config);
    value["callback_url"] = json!(callback_url(&state, &config.provider));
    Ok(Json(json!({"data": value})))
}

fn provider_json(config: &ProviderConfig) -> Value {
    json!({
        "provider": config.provider, "name": config.name, "enabled": config.enabled,
        "client_id": config.client_id, "has_client_secret": config.client_secret.is_some(),
        "authorize_url": config.authorize_url, "token_url": config.token_url,
        "userinfo_url": config.userinfo_url, "scopes": config.scopes,
        "subject_path": config.subject_path, "email_path": config.email_path,
        "display_name_path": config.display_name_path,
        "token_auth_method": config.token_auth_method, "use_pkce": config.use_pkce,
        "profile_mode": config.profile_mode, "emails_url": config.emails_url,
        "issuer_url":config.issuer_url,"discovery_url":config.discovery_url,
        "jwks_url":config.jwks_url,"validate_id_token":config.validate_id_token,
        "allowed_signing_algs":config.allowed_signing_algs,
        "clock_skew_seconds":config.clock_skew_seconds,
        "require_email_verified":config.require_email_verified,
        "dingtalk_app_type":config.dingtalk_app_type,
        "dingtalk_corp_policy":config.dingtalk_corp_policy,
        "dingtalk_internal_corp_id":config.dingtalk_internal_corp_id,
        "dingtalk_bypass_registration":config.dingtalk_bypass_registration,
        "dingtalk_sync_corp_email":config.dingtalk_sync_corp_email,
        "dingtalk_sync_display_name":config.dingtalk_sync_display_name,
        "dingtalk_sync_dept":config.dingtalk_sync_dept,
        "dingtalk_require_email":config.dingtalk_require_email,
        "dingtalk_email_attr_key":config.dingtalk_email_attr_key,
        "dingtalk_email_attr_name":config.dingtalk_email_attr_name,
        "dingtalk_name_attr_key":config.dingtalk_name_attr_key,
        "dingtalk_name_attr_name":config.dingtalk_name_attr_name,
        "dingtalk_dept_attr_key":config.dingtalk_dept_attr_key,
        "dingtalk_dept_attr_name":config.dingtalk_dept_attr_name,
        "callback_url": Value::Null
    })
}

#[derive(Default, Deserialize)]
struct ProviderInput {
    #[serde(default)]
    enabled: bool,
    name: String,
    client_id: String,
    client_secret: Option<String>,
    #[serde(default)]
    clear_client_secret: bool,
    authorize_url: String,
    token_url: String,
    userinfo_url: String,
    #[serde(default)]
    emails_url: String,
    scopes: String,
    subject_path: String,
    email_path: String,
    display_name_path: String,
    token_auth_method: String,
    #[serde(default = "enabled")]
    use_pkce: bool,
    #[serde(default)]
    issuer_url: String,
    #[serde(default)]
    discovery_url: String,
    #[serde(default)]
    jwks_url: String,
    #[serde(default)]
    validate_id_token: bool,
    #[serde(default = "default_oidc_algs")]
    allowed_signing_algs: String,
    #[serde(default = "default_clock_skew")]
    clock_skew_seconds: i64,
    #[serde(default)]
    require_email_verified: bool,
    #[serde(default = "default_dingtalk_app_type")]
    dingtalk_app_type: String,
    #[serde(default = "default_dingtalk_corp_policy")]
    dingtalk_corp_policy: String,
    #[serde(default)]
    dingtalk_internal_corp_id: String,
    #[serde(default)]
    dingtalk_bypass_registration: bool,
    #[serde(default)]
    dingtalk_sync_corp_email: bool,
    #[serde(default)]
    dingtalk_sync_display_name: bool,
    #[serde(default)]
    dingtalk_sync_dept: bool,
    #[serde(default)]
    dingtalk_require_email: bool,
    #[serde(default = "default_dingtalk_email_attr_key")]
    dingtalk_email_attr_key: String,
    #[serde(default = "default_dingtalk_email_attr_name")]
    dingtalk_email_attr_name: String,
    #[serde(default = "default_dingtalk_name_attr_key")]
    dingtalk_name_attr_key: String,
    #[serde(default = "default_dingtalk_name_attr_name")]
    dingtalk_name_attr_name: String,
    #[serde(default = "default_dingtalk_dept_attr_key")]
    dingtalk_dept_attr_key: String,
    #[serde(default = "default_dingtalk_dept_attr_name")]
    dingtalk_dept_attr_name: String,
}

fn enabled() -> bool {
    true
}

fn default_oidc_algs() -> String {
    "RS256,ES256,PS256".into()
}

fn default_clock_skew() -> i64 {
    120
}

fn default_dingtalk_app_type() -> String {
    "public".into()
}

fn default_dingtalk_corp_policy() -> String {
    "none".into()
}

fn default_dingtalk_email_attr_key() -> String {
    "dingtalk_email".into()
}

fn default_dingtalk_email_attr_name() -> String {
    "DingTalk corporate email".into()
}

fn default_dingtalk_name_attr_key() -> String {
    "dingtalk_name".into()
}

fn default_dingtalk_name_attr_name() -> String {
    "DingTalk display name".into()
}

fn default_dingtalk_dept_attr_key() -> String {
    "dingtalk_department".into()
}

fn default_dingtalk_dept_attr_name() -> String {
    "DingTalk department".into()
}

async fn update_provider(
    State(state): State<AppState>,
    Path(provider): Path<String>,
    Json(input): Json<ProviderInput>,
) -> ApiResult<Json<Value>> {
    let provider = normalize_provider(&provider)?;
    let current: Option<String> = sqlx::query_scalar(
        "SELECT encrypted_client_secret FROM external_auth_providers WHERE provider = ?",
    )
    .bind(&provider)
    .fetch_one(&state.pool)
    .await?;
    let encrypted_secret = if input.clear_client_secret {
        None
    } else if let Some(secret) = input
        .client_secret
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(state.crypto.encrypt(secret.as_bytes())?)
    } else {
        current
    };
    let mut normalized = ProviderInput {
        name: input.name.trim().to_string(),
        client_id: input.client_id.trim().to_string(),
        authorize_url: input.authorize_url.trim().to_string(),
        token_url: input.token_url.trim().to_string(),
        userinfo_url: input.userinfo_url.trim().to_string(),
        emails_url: input.emails_url.trim().to_string(),
        scopes: input.scopes.trim().to_string(),
        subject_path: input.subject_path.trim().to_string(),
        email_path: input.email_path.trim().to_string(),
        display_name_path: input.display_name_path.trim().to_string(),
        token_auth_method: input.token_auth_method.trim().to_ascii_lowercase(),
        issuer_url: input.issuer_url.trim().to_string(),
        discovery_url: input.discovery_url.trim().to_string(),
        jwks_url: input.jwks_url.trim().to_string(),
        allowed_signing_algs: input
            .allowed_signing_algs
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join(","),
        dingtalk_app_type: match input.dingtalk_app_type.trim().to_ascii_lowercase() {
            value if value.is_empty() => default_dingtalk_app_type(),
            value => value,
        },
        dingtalk_corp_policy: match input.dingtalk_corp_policy.trim().to_ascii_lowercase() {
            value if value.is_empty() => default_dingtalk_corp_policy(),
            value => value,
        },
        dingtalk_internal_corp_id: input.dingtalk_internal_corp_id.trim().to_string(),
        dingtalk_email_attr_key: input.dingtalk_email_attr_key.trim().to_string(),
        dingtalk_email_attr_name: input.dingtalk_email_attr_name.trim().to_string(),
        dingtalk_name_attr_key: input.dingtalk_name_attr_key.trim().to_string(),
        dingtalk_name_attr_name: input.dingtalk_name_attr_name.trim().to_string(),
        dingtalk_dept_attr_key: input.dingtalk_dept_attr_key.trim().to_string(),
        dingtalk_dept_attr_name: input.dingtalk_dept_attr_name.trim().to_string(),
        client_secret: None,
        clear_client_secret: false,
        ..input
    };
    if provider == "oidc" && !normalized.discovery_url.is_empty() {
        resolve_oidc_discovery(&state, &mut normalized).await?;
    }
    if provider == "dingtalk" && normalized.dingtalk_corp_policy != "internal_only" {
        normalized.dingtalk_bypass_registration = false;
        normalized.dingtalk_sync_corp_email = false;
        normalized.dingtalk_sync_display_name = false;
        normalized.dingtalk_sync_dept = false;
    }
    validate_provider(&provider, &normalized, encrypted_secret.is_some())?;
    if provider == "github" && normalized.enabled && normalized.emails_url.is_empty() {
        return Err(ApiError::bad_request(
            "INVALID_OAUTH_CONFIG",
            "enabled GitHub login requires a verified emails endpoint",
        ));
    }
    sqlx::query(
        "UPDATE external_auth_providers SET name = ?, enabled = ?, client_id = ?, \
         encrypted_client_secret = ?, authorize_url = ?, token_url = ?, userinfo_url = ?, \
         scopes = ?, subject_path = ?, email_path = ?, display_name_path = ?, \
         token_auth_method = ?, use_pkce = ?, emails_url = ?, issuer_url = ?, discovery_url = ?, \
         jwks_url = ?, validate_id_token = ?, allowed_signing_algs = ?, clock_skew_seconds = ?, \
         require_email_verified = ?, dingtalk_app_type = ?, dingtalk_corp_policy = ?, \
         dingtalk_internal_corp_id = ?, dingtalk_bypass_registration = ?, \
         dingtalk_sync_corp_email = ?, dingtalk_sync_display_name = ?, dingtalk_sync_dept = ?, \
         dingtalk_require_email = ?, dingtalk_email_attr_key = ?, dingtalk_email_attr_name = ?, \
         dingtalk_name_attr_key = ?, dingtalk_name_attr_name = ?, dingtalk_dept_attr_key = ?, \
         dingtalk_dept_attr_name = ?, updated_at = CURRENT_TIMESTAMP WHERE provider = ?",
    )
    .bind(&normalized.name)
    .bind(normalized.enabled)
    .bind(&normalized.client_id)
    .bind(encrypted_secret)
    .bind(&normalized.authorize_url)
    .bind(&normalized.token_url)
    .bind(&normalized.userinfo_url)
    .bind(&normalized.scopes)
    .bind(&normalized.subject_path)
    .bind(&normalized.email_path)
    .bind(&normalized.display_name_path)
    .bind(&normalized.token_auth_method)
    .bind(normalized.use_pkce)
    .bind(&normalized.emails_url)
    .bind(&normalized.issuer_url)
    .bind(&normalized.discovery_url)
    .bind(&normalized.jwks_url)
    .bind(normalized.validate_id_token)
    .bind(&normalized.allowed_signing_algs)
    .bind(normalized.clock_skew_seconds)
    .bind(normalized.require_email_verified)
    .bind(&normalized.dingtalk_app_type)
    .bind(&normalized.dingtalk_corp_policy)
    .bind(&normalized.dingtalk_internal_corp_id)
    .bind(normalized.dingtalk_bypass_registration)
    .bind(normalized.dingtalk_sync_corp_email)
    .bind(normalized.dingtalk_sync_display_name)
    .bind(normalized.dingtalk_sync_dept)
    .bind(normalized.dingtalk_require_email)
    .bind(&normalized.dingtalk_email_attr_key)
    .bind(&normalized.dingtalk_email_attr_name)
    .bind(&normalized.dingtalk_name_attr_key)
    .bind(&normalized.dingtalk_name_attr_name)
    .bind(&normalized.dingtalk_dept_attr_key)
    .bind(&normalized.dingtalk_dept_attr_name)
    .bind(&provider)
    .execute(&state.pool)
    .await?;
    let config = load_provider(&state, &provider, false).await?;
    let mut value = provider_json(&config);
    value["callback_url"] = json!(callback_url(&state, &provider));
    Ok(Json(json!({"data": value})))
}

async fn resolve_oidc_discovery(state: &AppState, input: &mut ProviderInput) -> ApiResult<()> {
    validate_endpoint(&input.discovery_url)?;
    let response = state
        .client
        .get(&input.discovery_url)
        .header(header::ACCEPT, "application/json")
        .header(header::USER_AGENT, "sub2api-mini")
        .timeout(StdDuration::from_secs(10))
        .send()
        .await?;
    if !response.status().is_success() {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "OIDC_DISCOVERY_FAILED",
            "OIDC discovery request failed",
        ));
    }
    let bytes = response.bytes().await?;
    if bytes.len() > 256 * 1024 {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "OIDC_DISCOVERY_INVALID",
            "OIDC discovery document is too large",
        ));
    }
    let document: Value = serde_json::from_slice(&bytes).map_err(|_| {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            "OIDC_DISCOVERY_INVALID",
            "OIDC discovery document is invalid",
        )
    })?;
    let field = |name: &str| {
        document
            .get(name)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty() && value.len() <= 2048)
            .map(ToOwned::to_owned)
            .ok_or_else(|| {
                ApiError::new(
                    StatusCode::BAD_GATEWAY,
                    "OIDC_DISCOVERY_INVALID",
                    format!("OIDC discovery document has no {name}"),
                )
            })
    };
    let issuer = field("issuer")?;
    let authorize_url = field("authorization_endpoint")?;
    let token_url = field("token_endpoint")?;
    let userinfo_url = field("userinfo_endpoint")?;
    let jwks_url = field("jwks_uri")?;
    for endpoint in [
        &issuer,
        &authorize_url,
        &token_url,
        &userinfo_url,
        &jwks_url,
    ] {
        validate_endpoint(endpoint)?;
    }
    if !input.issuer_url.is_empty() && input.issuer_url != issuer {
        return Err(ApiError::bad_request(
            "OIDC_ISSUER_MISMATCH",
            "configured OIDC issuer does not match discovery",
        ));
    }
    input.issuer_url = issuer;
    if input.authorize_url.is_empty() {
        input.authorize_url = authorize_url;
    }
    if input.token_url.is_empty() {
        input.token_url = token_url;
    }
    if input.userinfo_url.is_empty() {
        input.userinfo_url = userinfo_url;
    }
    if input.jwks_url.is_empty() {
        input.jwks_url = jwks_url;
    }
    Ok(())
}

fn validate_provider(provider: &str, input: &ProviderInput, has_secret: bool) -> ApiResult<()> {
    if input.name.is_empty()
        || input.name.chars().count() > 80
        || input.client_id.len() > 512
        || input.scopes.len() > 512
        || input.authorize_url.len() > 2048
        || input.token_url.len() > 2048
        || input.userinfo_url.len() > 2048
        || input.emails_url.len() > 2048
        || input.subject_path.len() > 128
        || input.email_path.len() > 128
        || input.display_name_path.len() > 128
        || input.issuer_url.len() > 2048
        || input.discovery_url.len() > 2048
        || input.jwks_url.len() > 2048
        || input.allowed_signing_algs.len() > 128
        || !(0..=600).contains(&input.clock_skew_seconds)
        || !matches!(
            input.token_auth_method.as_str(),
            "client_secret_post" | "client_secret_basic" | "none"
        )
        || input.subject_path.is_empty()
        || input.email_path.is_empty()
        || input.display_name_path.is_empty()
    {
        return Err(ApiError::bad_request(
            "INVALID_OAUTH_CONFIG",
            "OAuth provider fields are invalid",
        ));
    }
    if provider == "dingtalk" {
        let valid_key = |value: &str| {
            !value.is_empty()
                && value.len() <= 64
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        };
        if !matches!(input.dingtalk_app_type.as_str(), "public" | "internal")
            || !matches!(
                input.dingtalk_corp_policy.as_str(),
                "none" | "internal_only"
            )
            || (input.dingtalk_corp_policy == "internal_only"
                && input.dingtalk_app_type != "internal")
            || input.dingtalk_internal_corp_id.len() > 256
            || (input.dingtalk_sync_corp_email
                && (!valid_key(&input.dingtalk_email_attr_key)
                    || input.dingtalk_email_attr_name.is_empty()
                    || input.dingtalk_email_attr_name.chars().count() > 80))
            || (input.dingtalk_sync_display_name
                && (!valid_key(&input.dingtalk_name_attr_key)
                    || input.dingtalk_name_attr_name.is_empty()
                    || input.dingtalk_name_attr_name.chars().count() > 80))
            || (input.dingtalk_sync_dept
                && (!valid_key(&input.dingtalk_dept_attr_key)
                    || input.dingtalk_dept_attr_name.is_empty()
                    || input.dingtalk_dept_attr_name.chars().count() > 80))
        {
            return Err(ApiError::bad_request(
                "INVALID_DINGTALK_CONFIG",
                "DingTalk enterprise policy or synchronization fields are invalid",
            ));
        }
    }
    let algorithms = input
        .allowed_signing_algs
        .split(',')
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if algorithms.is_empty()
        || algorithms.len() > 3
        || algorithms
            .iter()
            .any(|value| !matches!(*value, "RS256" | "PS256" | "ES256"))
    {
        return Err(ApiError::bad_request(
            "INVALID_OIDC_ALGORITHMS",
            "OIDC signing algorithms must be RS256, PS256, or ES256",
        ));
    }
    if input.validate_id_token
        && (input.issuer_url.is_empty()
            || input.jwks_url.is_empty()
            || !input
                .scopes
                .split_whitespace()
                .any(|scope| scope == "openid"))
    {
        return Err(ApiError::bad_request(
            "INVALID_OIDC_SECURITY",
            "ID token validation requires issuer, JWKS URL, and the openid scope",
        ));
    }
    for value in [&input.issuer_url, &input.discovery_url, &input.jwks_url] {
        if !value.is_empty() {
            validate_endpoint(value)?;
        }
    }
    if input.enabled {
        if input.client_id.is_empty()
            || input.scopes.is_empty()
            || (input.token_auth_method != "none" && !has_secret)
        {
            return Err(ApiError::bad_request(
                "INVALID_OAUTH_CONFIG",
                "enabled OAuth provider requires a client ID, scopes, and client secret",
            ));
        }
        for value in [&input.authorize_url, &input.token_url, &input.userinfo_url] {
            validate_endpoint(value)?;
        }
        if !input.emails_url.is_empty() {
            validate_endpoint(&input.emails_url)?;
        }
    }
    Ok(())
}

fn validate_endpoint(value: &str) -> ApiResult<()> {
    let url = url::Url::parse(value)
        .map_err(|_| ApiError::bad_request("INVALID_OIDC_URL", "OIDC endpoint URL is invalid"))?;
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "::1"));
    if (url.scheme() != "https" && !(url.scheme() == "http" && loopback))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.host_str().is_none()
        || url.fragment().is_some()
    {
        return Err(ApiError::bad_request(
            "INVALID_OIDC_URL",
            "OIDC endpoints must use HTTPS or loopback HTTP",
        ));
    }
    Ok(())
}

fn callback_url(state: &AppState, provider: &str) -> String {
    format!(
        "{}/api/auth/oauth/{}/callback",
        state.config.public_ui_url.trim_end_matches('/'),
        provider
    )
}

fn ui_target(state: &AppState, hash: &str) -> String {
    format!(
        "{}/{}",
        state.config.public_ui_url.trim_end_matches('/'),
        hash
    )
}

fn error_redirect(state: &AppState, code: &str) -> Response {
    let encoded: String = url::form_urlencoded::byte_serialize(code.as_bytes()).collect();
    Redirect::to(&ui_target(
        state,
        &format!("#/oauth-result?status=error&code={encoded}"),
    ))
    .into_response()
}

pub async fn public_provider_summary(state: &AppState) -> ApiResult<Vec<Value>> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT provider, name FROM external_auth_providers WHERE enabled = 1 ORDER BY provider",
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| json!({"provider": row.0, "name": row.1}))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        body::Bytes,
        http::HeaderMap,
        routing::{get, post},
    };
    use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
    use ring::{
        rand::SystemRandom,
        signature::{RSA_PKCS1_SHA256, RsaKeyPair},
    };
    use std::collections::HashMap;
    use std::sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::sync::Mutex;

    use crate::test_support;

    const TEST_RSA_N: &str = "yrzaX1hMqpbuMb2-shW0tSwrYijDQKYgjJpw6wAhMn85DEhH3uj_TDV-egBU0H4e0QC2W1cyOA89eajn5eYttzGcUl81CUuRj3tQ_UTgM_5s1SMtT_H9l5j7vcyqx8abOskhd5EIMV3kRE16oBOCMDPcqYYrFbWCVcopW2meNL6ZJG8XnRnuSF6diXNny9jPsZAwB5n8LqRMcJJs7g-6SABY_iswHIXxpQK7P_9ZxNjTO_SirVcTPSXlVUqxGv9SZllTR7UI_zHTRDxV_IJtBAVSN6JbHkQDsV4zf_n92VWuKacDy01WDA2GeU1kAWqHjffOXSOikvv7tMa_8brBCw";
    // Static PKCS#8 fixture used only to sign local OIDC test tokens.
    const TEST_RSA_PRIVATE_KEY_PKCS8: &str = r#"
MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQDKvNpfWEyqlu4x
vb6yFbS1LCtiKMNApiCMmnDrACEyfzkMSEfe6P9MNX56AFTQfh7RALZbVzI4Dz15
qOfl5i23MZxSXzUJS5GPe1D9ROAz/mzVIy1P8f2XmPu9zKrHxps6ySF3kQgxXeRE
TXqgE4IwM9yphisVtYJVyilbaZ40vpkkbxedGe5IXp2Jc2fL2M+xkDAHmfwupExw
kmzuD7pIAFj+KzAchfGlArs//1nE2NM79KKtVxM9JeVVSrEa/1JmWVNHtQj/MdNE
PFX8gm0EBVI3olseRAOxXjN/+f3ZVa4ppwPLTVYMDYZ5TWQBaoeN985dI6KS+/u0
xr/xusELAgMBAAECggEAUTwi9BlZfvFDOEMjahAwfHfaWlajBgCLAkvP+xnuM2Gu
5jEAO115DnxQ1WnUkkY26uAyMZ9azAOvSlRXt1Ln9oO2c3sasULKbIepCBLVE4Ba
83xI58O7LUdrd73OoIYAJSn6cwJ2GfHZSVUSUZn/jHj7biIImYZFV0LOF4bWkaMm
YpJI8zufmF+xVzmd4RYqPS3tdDf7KBycMYhVdMRb/J4p0QiaefB7+mHZo/ySppgZ
GU5DdtcEWnRoqmvP7pBnxJMpOpehh1btlul4wGcdiKwVQfhV7v12iIQl9X3BGwL9
5fV3CVYrrHdbslp52L0J5CGvk9ZBItxNWvZOMyqWQQKBgQD3R6kX7+S/fKURi5il
yEKaB2wPZ0ATHqg9qg27V1owaFoEnmKaUCjdtXttjttGgZTSg2OQnAyjxPSesD9B
W0M8D+VvhAFUUttrC0QMrvLOxMbtjJwyIreTyGjSpeqUaveSpcj0vDaawUU7TcSd
ftCePObqfUXR4BbaiA/5zyYS6QKBgQDR4xWDdroCk2xmD3OHOQJPNmdAJvJiuZMe
vg0lWmfalUQUCnCKOyhWoKnaP/TxqFAiFjL4DB6JCSDUKvKc662s7PGNS9LwdklV
uJvLVPimogpbBDB4YeBguMYhtBUKbsGw5Pho1hsW023dCE0InSDiox1pBsPpzO3w
tiJtSnbz0wKBgQC1efkGIT/OrIp0Wu/XUyZV5naOw2bJ4Wj1gHT9dXkyJ5NQ6nBQ
8d1cARGpcPtKPlVbPaP3gB7inews/goeS/0G+l+WvNlA6mIvqB/z8v0tdErOEbCc
NtBle+I6HhwPeoVhMZxOyEaGwqqtgEB4mZY/W1DY7MEt6vi6vrqCyl0V6QKBgCog
bBcA44DU2jL58vQ3KxF/F+Y2avwJx0+qUbUnmiSzRQDIv6HfEc+hW7YklCNU5xCQ
aBaFSDO1E1PCcwOwAiHtROZZS7Nb6og8D3kWSvoXGAEArEHdU03WiF4HaRm49UNu
EbXpE3LXaPuuSNfrwcf7eVG1O+lXaoKf6/UHtyxlAoGBAJorY9BR3J6O/00JUcZU
YYPymsdRyh8VxWdcq8ax6fOkHM45NhMOU98HgPuFl9FU275JDIQrCTpZ3memOy/O
atfKmm4K7OeYDBfPvjyv8SFBy4m9p2GpVgupD7vVEJHYCBCZEo1WAy6YjCfYGF0B
WZtqjYx/IsKdOsqNFpU4ap8H
"#;

    fn oidc_test_token(issuer: &str, nonce: &str, subject: &str) -> String {
        let header = URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&json!({"alg":"RS256","kid":"oidc-test-key","typ":"JWT"})).unwrap(),
        );
        let claims = URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&json!({
                "iss":issuer,"aud":"oidc-secure-client","sub":subject,"nonce":nonce,
                "iat":Utc::now().timestamp(),"exp":(Utc::now()+Duration::minutes(5)).timestamp()
            }))
            .unwrap(),
        );
        let input = format!("{header}.{claims}");
        let encoded = TEST_RSA_PRIVATE_KEY_PKCS8.lines().collect::<String>();
        let der = BASE64_STANDARD.decode(encoded).unwrap();
        let key = RsaKeyPair::from_pkcs8(&der).unwrap();
        let mut signature = vec![0; key.public().modulus_len()];
        key.sign(
            &RSA_PKCS1_SHA256,
            &SystemRandom::new(),
            input.as_bytes(),
            &mut signature,
        )
        .unwrap();
        format!("{input}.{}", URL_SAFE_NO_PAD.encode(signature))
    }

    async fn configure_provider(state: &AppState, base_url: &str) {
        let _ = update_provider(
            State(state.clone()),
            Path("oidc".into()),
            Json(ProviderInput {
                enabled: true,
                name: "Test OIDC".into(),
                client_id: "test-client".into(),
                client_secret: Some("provider-secret".into()),
                clear_client_secret: false,
                authorize_url: format!("{base_url}/authorize"),
                token_url: format!("{base_url}/token"),
                userinfo_url: format!("{base_url}/userinfo"),
                emails_url: String::new(),
                scopes: "openid email profile".into(),
                subject_path: "account.id".into(),
                email_path: "account.email".into(),
                display_name_path: "account.name".into(),
                token_auth_method: "client_secret_post".into(),
                use_pkce: true,
                issuer_url: String::new(),
                discovery_url: String::new(),
                jwks_url: String::new(),
                validate_id_token: false,
                allowed_signing_algs: default_oidc_algs(),
                clock_skew_seconds: 120,
                require_email_verified: false,
                dingtalk_app_type: "public".into(),
                dingtalk_corp_policy: "none".into(),
                ..ProviderInput::default()
            }),
        )
        .await
        .unwrap();
    }

    fn state_from_authorize_url(value: &str) -> String {
        url::Url::parse(value)
            .unwrap()
            .query_pairs()
            .find(|pair| pair.0 == "state")
            .unwrap()
            .1
            .into_owned()
    }

    fn pending_token_from_response(response: &Response) -> String {
        let location = response
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap();
        let fragment = url::Url::parse(location)
            .unwrap()
            .fragment()
            .unwrap()
            .to_string();
        let query = fragment.split_once('?').unwrap().1;
        url::form_urlencoded::parse(query.as_bytes())
            .find(|pair| pair.0 == "token")
            .unwrap()
            .1
            .into_owned()
    }

    #[tokio::test]
    async fn oidc_bind_and_login_encrypt_secrets_and_consume_state_once() {
        let (_directory, state) = test_support::state().await;
        let exchanges = Arc::new(Mutex::new(Vec::<String>::new()));
        let exchange_capture = exchanges.clone();
        let mock = Router::new()
            .route("/authorize", get(|| async { StatusCode::NO_CONTENT }))
            .route(
                "/token",
                post(move |body: String| {
                    let exchange_capture = exchange_capture.clone();
                    async move {
                        exchange_capture.lock().await.push(body);
                        Json(json!({"access_token": "oidc-access"}))
                    }
                }),
            )
            .route(
                "/userinfo",
                get(|headers: HeaderMap| async move {
                    if headers
                        .get(header::AUTHORIZATION)
                        .and_then(|value| value.to_str().ok())
                        != Some("Bearer oidc-access")
                    {
                        return (
                            StatusCode::UNAUTHORIZED,
                            Json(json!({"error": "unauthorized"})),
                        );
                    }
                    (
                        StatusCode::OK,
                        Json(json!({"account": {
                        "id": "external-subject-123456", "email": "oidc@example.com",
                        "name": "OIDC Person"
                    }, "email_verified": true})),
                    )
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, mock).await });
        configure_provider(&state, &base_url).await;

        let stored_secret: String = sqlx::query_scalar(
            "SELECT encrypted_client_secret FROM external_auth_providers WHERE provider = 'oidc'",
        )
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert!(!stored_secret.contains("provider-secret"));
        assert_eq!(
            state.crypto.decrypt(&stored_secret).unwrap(),
            b"provider-secret"
        );

        let user_id: i64 = sqlx::query_scalar("SELECT id FROM users WHERE role = 'admin'")
            .fetch_one(&state.pool)
            .await
            .unwrap();
        let unbound_url = create_flow(&state, "oidc", "login", None).await.unwrap();
        let unbound_state = state_from_authorize_url(&unbound_url);
        let unbound = complete_callback(
            &state,
            "oidc",
            CallbackQuery {
                code: Some("unbound-code".into()),
                state: Some(unbound_state),
                error: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(unbound.status(), StatusCode::SEE_OTHER);
        let pending_token = pending_token_from_response(&unbound);
        let Json(inspected) = inspect_pending(
            State(state.clone()),
            Json(PendingTokenInput {
                token: pending_token.clone(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(inspected["data"]["provider_name"], "Test OIDC");
        assert_eq!(inspected["data"]["suggested_email"], "oidc@example.com");
        let bound = bind_pending(
            State(state.clone()),
            Json(BindPendingInput {
                token: pending_token.clone(),
                identifier: "admin".into(),
                password: "test-password".into(),
                totp_code: None,
            }),
        )
        .await
        .unwrap();
        assert!(bound.headers().contains_key(header::SET_COOKIE));
        let reused_pending = inspect_pending(
            State(state.clone()),
            Json(PendingTokenInput {
                token: pending_token,
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(reused_pending.code, "OAUTH_PENDING_INVALID");

        let bind_url = create_flow(&state, "oidc", "bind", Some(user_id))
            .await
            .unwrap();
        let bind_state = state_from_authorize_url(&bind_url);
        let stored: (String, String) = sqlx::query_as(
            "SELECT state_hash, encrypted_verifier FROM external_oauth_flows WHERE intent = 'bind'",
        )
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(stored.0, token_hash(&bind_state));
        assert_ne!(stored.0, bind_state);

        let response = complete_callback(
            &state,
            "oidc",
            CallbackQuery {
                code: Some("bind-code".into()),
                state: Some(bind_state.clone()),
                error: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert!(
            response
                .headers()
                .get(header::LOCATION)
                .unwrap()
                .to_str()
                .unwrap()
                .ends_with("#/profile?identity=bound")
        );
        let identity: (i64, String, Option<String>) = sqlx::query_as(
            "SELECT user_id, subject, email FROM external_auth_identities WHERE provider = 'oidc'",
        )
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(identity.0, user_id);
        assert_eq!(identity.1, "external-subject-123456");
        assert_eq!(identity.2.as_deref(), Some("oidc@example.com"));

        let reused = complete_callback(
            &state,
            "oidc",
            CallbackQuery {
                code: Some("bind-code".into()),
                state: Some(bind_state),
                error: None,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(reused.code, "OAUTH_STATE_INVALID");

        let login_url = create_flow(&state, "oidc", "login", None).await.unwrap();
        let login_state = state_from_authorize_url(&login_url);
        let response = complete_callback(
            &state,
            "oidc",
            CallbackQuery {
                code: Some("login-code".into()),
                state: Some(login_state),
                error: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert!(response.headers().contains_key(header::SET_COOKIE));
        let exchange_bodies = exchanges.lock().await;
        assert_eq!(exchange_bodies.len(), 3);
        for body in exchange_bodies.iter() {
            assert!(body.contains("client_secret=provider-secret"));
            assert!(body.contains("code_verifier="));
        }
        let verifier = url::form_urlencoded::parse(exchange_bodies[1].as_bytes())
            .find(|pair| pair.0 == "code_verifier")
            .unwrap()
            .1
            .into_owned();
        assert_ne!(stored.1, verifier);
        assert_eq!(
            state.crypto.decrypt(&stored.1).unwrap(),
            verifier.as_bytes()
        );
        server.abort();
    }

    #[tokio::test]
    async fn github_uses_verified_email_and_provider_scoped_state() {
        let (_directory, state) = test_support::state().await;
        let mock = Router::new()
            .route("/authorize", get(|| async { StatusCode::NO_CONTENT }))
            .route(
                "/token",
                post(|| async { Json(json!({"access_token": "github-access"})) }),
            )
            .route(
                "/userinfo",
                get(|headers: HeaderMap| async move {
                    assert_eq!(
                        headers
                            .get(header::AUTHORIZATION)
                            .and_then(|value| value.to_str().ok()),
                        Some("Bearer github-access")
                    );
                    Json(json!({"id": 4242, "login": "octo", "name": "Octo User"}))
                }),
            )
            .route(
                "/emails",
                get(|| async {
                    Json(json!([
                        {"email": "old@example.com", "primary": false, "verified": true},
                        {"email": "octo@example.com", "primary": true, "verified": true}
                    ]))
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, mock).await });
        let _ = update_provider(
            State(state.clone()),
            Path("github".into()),
            Json(ProviderInput {
                enabled: true,
                name: "GitHub Test".into(),
                client_id: "github-client".into(),
                client_secret: Some("github-secret".into()),
                clear_client_secret: false,
                authorize_url: format!("{base_url}/authorize"),
                token_url: format!("{base_url}/token"),
                userinfo_url: format!("{base_url}/userinfo"),
                emails_url: format!("{base_url}/emails"),
                scopes: "read:user user:email".into(),
                subject_path: "id".into(),
                email_path: "email".into(),
                display_name_path: "name".into(),
                token_auth_method: "client_secret_post".into(),
                use_pkce: false,
                issuer_url: String::new(),
                discovery_url: String::new(),
                jwks_url: String::new(),
                validate_id_token: false,
                allowed_signing_algs: default_oidc_algs(),
                clock_skew_seconds: 120,
                require_email_verified: false,
                ..ProviderInput::default()
            }),
        )
        .await
        .unwrap();

        let authorization_url = create_flow(&state, "github", "login", None).await.unwrap();
        assert!(authorization_url.contains("/authorize?"));
        assert!(authorization_url.contains("scope=read%3Auser+user%3Aemail"));
        let flow_state = state_from_authorize_url(&authorization_url);
        let wrong_provider = complete_callback(
            &state,
            "google",
            CallbackQuery {
                code: Some("code".into()),
                state: Some(flow_state.clone()),
                error: None,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(wrong_provider.code, "OAUTH_STATE_INVALID");

        let response = complete_callback(
            &state,
            "github",
            CallbackQuery {
                code: Some("code".into()),
                state: Some(flow_state),
                error: None,
            },
        )
        .await
        .unwrap();
        let token = pending_token_from_response(&response);
        let Json(pending) =
            inspect_pending(State(state.clone()), Json(PendingTokenInput { token }))
                .await
                .unwrap();
        assert_eq!(pending["data"]["provider"], "github");
        assert_eq!(pending["data"]["provider_name"], "GitHub Test");
        assert_eq!(pending["data"]["suggested_email"], "octo@example.com");
        assert_eq!(pending["data"]["provider_email_verified"], true);

        let configs = admin_providers(State(state.clone())).await.unwrap().0;
        assert_eq!(configs["data"].as_array().unwrap().len(), 7);
        let disabled: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM external_auth_providers WHERE provider IN ('oidc','google','linuxdo','dingtalk','wechat_open','wechat_mp') AND enabled=0",
        )
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(disabled, 6);
        server.abort();
    }

    #[tokio::test]
    async fn wechat_open_uses_query_protocol_unionid_and_scoped_state() {
        let (_directory, state) = test_support::state().await;
        let mock = Router::new()
            .route(
                "/token",
                get(|Query(query): Query<HashMap<String, String>>| async move {
                    assert_eq!(query.get("appid").map(String::as_str), Some("wechat-app"));
                    assert_eq!(
                        query.get("secret").map(String::as_str),
                        Some("wechat-secret")
                    );
                    assert_eq!(query.get("code").map(String::as_str), Some("wechat-code"));
                    assert_eq!(
                        query.get("grant_type").map(String::as_str),
                        Some("authorization_code")
                    );
                    Json(json!({
                        "access_token": "wechat-access", "openid": "open-id-123",
                        "unionid": "union-id-456", "scope": "snsapi_login"
                    }))
                }),
            )
            .route(
                "/userinfo",
                get(|Query(query): Query<HashMap<String, String>>| async move {
                    assert_eq!(
                        query.get("access_token").map(String::as_str),
                        Some("wechat-access")
                    );
                    assert_eq!(query.get("openid").map(String::as_str), Some("open-id-123"));
                    Json(json!({
                        "openid": "open-id-123", "unionid": "union-id-456",
                        "nickname": "WeChat User"
                    }))
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, mock).await });
        let _ = update_provider(
            State(state.clone()),
            Path("wechat_open".into()),
            Json(ProviderInput {
                enabled: true,
                name: "WeChat Open Test".into(),
                client_id: "wechat-app".into(),
                client_secret: Some("wechat-secret".into()),
                clear_client_secret: false,
                authorize_url: format!("{base_url}/authorize"),
                token_url: format!("{base_url}/token"),
                userinfo_url: format!("{base_url}/userinfo"),
                emails_url: String::new(),
                scopes: "snsapi_login".into(),
                subject_path: "unionid".into(),
                email_path: "email".into(),
                display_name_path: "nickname".into(),
                token_auth_method: "client_secret_post".into(),
                use_pkce: false,
                issuer_url: String::new(),
                discovery_url: String::new(),
                jwks_url: String::new(),
                validate_id_token: false,
                allowed_signing_algs: default_oidc_algs(),
                clock_skew_seconds: 120,
                require_email_verified: false,
                ..ProviderInput::default()
            }),
        )
        .await
        .unwrap();

        let authorization_url = create_flow(&state, "wechat_open", "login", None)
            .await
            .unwrap();
        let parsed = url::Url::parse(&authorization_url).unwrap();
        assert_eq!(parsed.fragment(), Some("wechat_redirect"));
        assert_eq!(
            parsed
                .query_pairs()
                .find(|pair| pair.0 == "appid")
                .map(|pair| pair.1.into_owned()),
            Some("wechat-app".into())
        );
        assert!(!parsed.query_pairs().any(|pair| pair.0 == "client_id"));
        let flow_state = state_from_authorize_url(&authorization_url);
        let wrong_mode = complete_callback(
            &state,
            "wechat_mp",
            CallbackQuery {
                code: Some("wechat-code".into()),
                state: Some(flow_state.clone()),
                error: None,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(wrong_mode.code, "OAUTH_STATE_INVALID");

        let response = complete_callback(
            &state,
            "wechat_open",
            CallbackQuery {
                code: Some("wechat-code".into()),
                state: Some(flow_state),
                error: None,
            },
        )
        .await
        .unwrap();
        let token = pending_token_from_response(&response);
        let Json(pending) = inspect_pending(
            State(state.clone()),
            Json(PendingTokenInput {
                token: token.clone(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(pending["data"]["provider"], "wechat_open");
        assert_eq!(pending["data"]["display_name"], "WeChat User");
        assert_eq!(pending["data"]["suggested_email"], Value::Null);
        assert_eq!(pending["data"]["provider_email_verified"], false);

        let bound = bind_pending(
            State(state.clone()),
            Json(BindPendingInput {
                token,
                identifier: "admin".into(),
                password: "test-password".into(),
                totp_code: None,
            }),
        )
        .await
        .unwrap();
        assert!(bound.headers().contains_key(header::SET_COOKIE));
        sqlx::query(
            "UPDATE external_auth_providers SET enabled=1,client_id='wechat-app', \
             encrypted_client_secret=(SELECT encrypted_client_secret FROM external_auth_providers WHERE provider='wechat_open'), \
             authorize_url=?,token_url=?,userinfo_url=? WHERE provider='wechat_mp'",
        )
        .bind(format!("{base_url}/authorize"))
        .bind(format!("{base_url}/token"))
        .bind(format!("{base_url}/userinfo"))
        .execute(&state.pool)
        .await
        .unwrap();
        let mp_url = create_flow(&state, "wechat_mp", "login", None)
            .await
            .unwrap();
        let mp_state = state_from_authorize_url(&mp_url);
        let mp_login = complete_callback(
            &state,
            "wechat_mp",
            CallbackQuery {
                code: Some("wechat-code".into()),
                state: Some(mp_state),
                error: None,
            },
        )
        .await
        .unwrap();
        assert!(mp_login.headers().contains_key(header::SET_COOKIE));
        assert!(
            mp_login
                .headers()
                .get(header::LOCATION)
                .unwrap()
                .to_str()
                .unwrap()
                .contains("oauth_login=success")
        );
        let identities: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM external_auth_identities WHERE provider='wechat_open'",
        )
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(identities, 1);
        server.abort();
    }

    #[tokio::test]
    async fn dingtalk_uses_json_exchange_and_custom_userinfo_header() {
        let (_directory, state) = test_support::state().await;
        let mock = Router::new()
            .route(
                "/token",
                post(|Json(body): Json<Value>| async move {
                    if body.get("appKey").is_some() {
                        assert_eq!(body["appKey"], "dingtalk-client");
                        assert_eq!(body["appSecret"], "dingtalk-secret");
                        return Json(json!({"accessToken": "dingtalk-app", "expireIn": 7200}));
                    }
                    assert_eq!(body["clientId"], "dingtalk-client");
                    assert_eq!(body["clientSecret"], "dingtalk-secret");
                    assert_eq!(body["code"], "dingtalk-code");
                    assert_eq!(body["grantType"], "authorization_code");
                    Json(json!({"accessToken": "dingtalk-access", "expireIn": 7200}))
                }),
            )
            .route(
                "/userinfo",
                get(|headers: HeaderMap| async move {
                    assert_eq!(
                        headers
                            .get("x-acs-dingtalk-access-token")
                            .and_then(|value| value.to_str().ok()),
                        Some("dingtalk-access")
                    );
                    assert!(headers.get(header::AUTHORIZATION).is_none());
                    Json(json!({"unionId": "ding-union-123", "nick": "Ding User"}))
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, mock).await });
        let _ = update_provider(
            State(state.clone()),
            Path("dingtalk".into()),
            Json(ProviderInput {
                enabled: true,
                name: "DingTalk Test".into(),
                client_id: "dingtalk-client".into(),
                client_secret: Some("dingtalk-secret".into()),
                clear_client_secret: false,
                authorize_url: format!("{base_url}/authorize"),
                token_url: format!("{base_url}/token"),
                userinfo_url: format!("{base_url}/userinfo"),
                emails_url: String::new(),
                scopes: "openid".into(),
                subject_path: "unionId".into(),
                email_path: "email".into(),
                display_name_path: "nick".into(),
                token_auth_method: "client_secret_post".into(),
                use_pkce: false,
                issuer_url: String::new(),
                discovery_url: String::new(),
                jwks_url: String::new(),
                validate_id_token: false,
                allowed_signing_algs: default_oidc_algs(),
                clock_skew_seconds: 120,
                require_email_verified: false,
                dingtalk_app_type: "public".into(),
                dingtalk_corp_policy: "none".into(),
                ..ProviderInput::default()
            }),
        )
        .await
        .unwrap();

        let authorization_url = create_flow(&state, "dingtalk", "login", None)
            .await
            .unwrap();
        assert!(authorization_url.contains("prompt=consent"));
        let flow_state = state_from_authorize_url(&authorization_url);
        let response = complete_callback(
            &state,
            "dingtalk",
            CallbackQuery {
                code: Some("dingtalk-code".into()),
                state: Some(flow_state),
                error: None,
            },
        )
        .await
        .unwrap();
        let token = pending_token_from_response(&response);
        let Json(pending) = inspect_pending(State(state), Json(PendingTokenInput { token }))
            .await
            .unwrap();
        assert_eq!(pending["data"]["provider"], "dingtalk");
        assert_eq!(pending["data"]["display_name"], "Ding User");
        assert_eq!(
            pending["data"]["suggested_email"],
            "dingtalk-ding-union-123@dingtalk-connect.invalid"
        );
        assert_eq!(pending["data"]["provider_email_verified"], false);
        server.abort();
    }

    #[tokio::test]
    async fn dingtalk_internal_members_bypass_registration_and_sync_attributes() {
        use std::sync::atomic::AtomicBool;

        let (_directory, state) = test_support::state().await;
        let reject_member = Arc::new(AtomicBool::new(false));
        let app_token_calls = Arc::new(AtomicUsize::new(0));
        let member_calls = Arc::new(AtomicUsize::new(0));
        let app_token_counter = app_token_calls.clone();
        let member_counter = member_calls.clone();
        let reject_member_request = reject_member.clone();
        let mock = Router::new()
            .route(
                "/v1.0/oauth2/userAccessToken",
                post(|Json(body): Json<Value>| async move {
                    assert_eq!(body["clientId"], "enterprise-client");
                    assert_eq!(body["code"], "enterprise-code");
                    Json(json!({
                        "accessToken": "enterprise-user-token",
                        "corpId": "corp-from-oauth",
                        "expireIn": 7200
                    }))
                }),
            )
            .route(
                "/v1.0/oauth2/accessToken",
                post(move |Json(body): Json<Value>| {
                    let app_token_counter = app_token_counter.clone();
                    async move {
                        app_token_counter.fetch_add(1, Ordering::SeqCst);
                        assert_eq!(body["appKey"], "enterprise-client");
                        assert_eq!(body["appSecret"], "enterprise-secret");
                        Json(json!({"accessToken": "enterprise-app-token", "expireIn": 7200}))
                    }
                }),
            )
            .route(
                "/v1.0/contact/users/me",
                get(|headers: HeaderMap| async move {
                    assert_eq!(
                        headers
                            .get("x-acs-dingtalk-access-token")
                            .and_then(|value| value.to_str().ok()),
                        Some("enterprise-user-token")
                    );
                    Json(json!({"unionId": "enterprise-union", "nick": "Personal Nick"}))
                }),
            )
            .route(
                "/topapi/user/getbyunionid",
                post(
                    move |Query(query): Query<HashMap<String, String>>,
                          Json(body): Json<Value>| {
                        let member_counter = member_counter.clone();
                        let reject_member_request = reject_member_request.clone();
                        async move {
                            member_counter.fetch_add(1, Ordering::SeqCst);
                            assert_eq!(query.get("access_token").map(String::as_str), Some("enterprise-app-token"));
                            assert_eq!(body["unionid"], "enterprise-union");
                            if reject_member_request.load(Ordering::SeqCst) {
                                Json(json!({"errcode": 60011, "errmsg": "not in organization"}))
                            } else {
                                Json(json!({"errcode": 0, "result": {"userid": "staff-100"}}))
                            }
                        }
                    },
                ),
            )
            .route(
                "/topapi/v2/user/get",
                post(
                    |Query(query): Query<HashMap<String, String>>, Json(body): Json<Value>| async move {
                        assert_eq!(query.get("access_token").map(String::as_str), Some("enterprise-app-token"));
                        assert_eq!(body["userid"], "staff-100");
                        Json(json!({
                            "errcode": 0,
                            "result": {
                                "userid": "staff-100",
                                "name": "Enterprise Name",
                                "nickname": "Staff Nick",
                                "email": "",
                                "org_email": "",
                                "extension": "{\"企业邮箱\":\"enterprise@example.com\"}",
                                "dept_id_list": [3]
                            }
                        }))
                    },
                ),
            )
            .route(
                "/topapi/v2/department/get",
                post(|Json(body): Json<Value>| async move {
                    let id = body["dept_id"].as_i64().unwrap();
                    let (name, parent) = match id {
                        3 => ("Platform", 2),
                        2 => ("Research", 1),
                        1 => ("Example Corp", 0),
                        _ => panic!("unexpected department ID"),
                    };
                    Json(json!({
                        "errcode": 0,
                        "result": {"dept_id": id, "name": name, "parent_id": parent}
                    }))
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
        let Json(updated) = update_provider(
            State(state.clone()),
            Path("dingtalk".into()),
            Json(ProviderInput {
                enabled: true,
                name: "Enterprise DingTalk".into(),
                client_id: "enterprise-client".into(),
                client_secret: Some("enterprise-secret".into()),
                authorize_url: format!("{base_url}/authorize"),
                token_url: format!("{base_url}/v1.0/oauth2/userAccessToken"),
                userinfo_url: format!("{base_url}/v1.0/contact/users/me"),
                scopes: "openid".into(),
                subject_path: "unionId".into(),
                email_path: "email".into(),
                display_name_path: "nick".into(),
                token_auth_method: "client_secret_post".into(),
                allowed_signing_algs: default_oidc_algs(),
                dingtalk_app_type: "internal".into(),
                dingtalk_corp_policy: "internal_only".into(),
                dingtalk_internal_corp_id: "corp-note".into(),
                dingtalk_bypass_registration: true,
                dingtalk_sync_corp_email: true,
                dingtalk_sync_display_name: true,
                dingtalk_sync_dept: true,
                dingtalk_require_email: true,
                dingtalk_email_attr_key: "corp_email".into(),
                dingtalk_email_attr_name: "Corporate email".into(),
                dingtalk_name_attr_key: "corp_name".into(),
                dingtalk_name_attr_name: "Corporate name".into(),
                dingtalk_dept_attr_key: "corp_department".into(),
                dingtalk_dept_attr_name: "Corporate department".into(),
                ..ProviderInput::default()
            }),
        )
        .await
        .unwrap();
        assert_eq!(updated["data"]["dingtalk_corp_policy"], "internal_only");
        assert_eq!(updated["data"]["dingtalk_bypass_registration"], true);

        let authorization_url = create_flow(&state, "dingtalk", "login", None)
            .await
            .unwrap();
        let response = complete_callback(
            &state,
            "dingtalk",
            CallbackQuery {
                code: Some("enterprise-code".into()),
                state: Some(state_from_authorize_url(&authorization_url)),
                error: None,
            },
        )
        .await
        .unwrap();
        let token = pending_token_from_response(&response);
        let Json(pending) = inspect_pending(
            State(state.clone()),
            Json(PendingTokenInput {
                token: token.clone(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(pending["data"]["registration_enabled"], true);
        assert_eq!(pending["data"]["suggested_email"], "enterprise@example.com");
        assert_eq!(pending["data"]["provider_email_verified"], true);
        register_pending(
            State(state.clone()),
            Json(RegisterPendingInput {
                token,
                email: "enterprise@example.com".into(),
                password: "EnterprisePass88".into(),
                verify_code: None,
            }),
        )
        .await
        .unwrap();
        let user_id: i64 =
            sqlx::query_scalar("SELECT id FROM users WHERE email = 'enterprise@example.com'")
                .fetch_one(&state.pool)
                .await
                .unwrap();
        let attributes: Vec<(String, String)> = sqlx::query_as(
            "SELECT attribute_key, value FROM user_external_attributes \
             WHERE user_id = ? ORDER BY attribute_key",
        )
        .bind(user_id)
        .fetch_all(&state.pool)
        .await
        .unwrap();
        assert_eq!(
            attributes,
            vec![
                ("corp_department".into(), "Research/Platform".into()),
                ("corp_email".into(), "enterprise@example.com".into()),
                ("corp_name".into(), "Enterprise Name".into()),
            ]
        );
        assert_eq!(app_token_calls.load(Ordering::SeqCst), 1);

        reject_member.store(true, Ordering::SeqCst);
        let rejected_url = create_flow(&state, "dingtalk", "login", None)
            .await
            .unwrap();
        let rejected = complete_callback(
            &state,
            "dingtalk",
            CallbackQuery {
                code: Some("enterprise-code".into()),
                state: Some(state_from_authorize_url(&rejected_url)),
                error: None,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(rejected.code, "DINGTALK_CORP_REJECTED");
        assert_eq!(member_calls.load(Ordering::SeqCst), 2);
        server.abort();
    }

    #[tokio::test]
    async fn google_requires_verified_email_and_linuxdo_does_not_trust_email() {
        let (_directory, state) = test_support::state().await;
        let mock = Router::new()
            .route(
                "/google-unverified",
                get(|| async {
                    Json(json!({
                        "sub": "google-subject", "email": "google@example.com",
                        "email_verified": false, "name": "Google User"
                    }))
                }),
            )
            .route(
                "/google-verified",
                get(|| async {
                    Json(json!({
                        "sub": "google-subject", "email": "google@example.com",
                        "email_verified": true, "name": "Google User"
                    }))
                }),
            )
            .route(
                "/linuxdo",
                get(|| async {
                    Json(json!({
                        "id": 7788, "email": "linuxdo@example.com", "name": "LinuxDo User"
                    }))
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, mock).await });

        for (provider, userinfo_url, scopes, subject_path, use_pkce) in [
            (
                "google",
                format!("{base_url}/google-unverified"),
                "openid email profile",
                "sub",
                true,
            ),
            (
                "linuxdo",
                format!("{base_url}/linuxdo"),
                "user",
                "id",
                false,
            ),
        ] {
            let _ = update_provider(
                State(state.clone()),
                Path(provider.into()),
                Json(ProviderInput {
                    enabled: true,
                    name: format!("{provider} test"),
                    client_id: format!("{provider}-client"),
                    client_secret: Some(format!("{provider}-secret")),
                    clear_client_secret: false,
                    authorize_url: format!("{base_url}/authorize"),
                    token_url: format!("{base_url}/token"),
                    userinfo_url,
                    emails_url: String::new(),
                    scopes: scopes.into(),
                    subject_path: subject_path.into(),
                    email_path: "email".into(),
                    display_name_path: "name".into(),
                    token_auth_method: "client_secret_post".into(),
                    use_pkce,
                    issuer_url: String::new(),
                    discovery_url: String::new(),
                    jwks_url: String::new(),
                    validate_id_token: false,
                    allowed_signing_algs: default_oidc_algs(),
                    clock_skew_seconds: 120,
                    require_email_verified: false,
                    ..ProviderInput::default()
                }),
            )
            .await
            .unwrap();
        }

        let google = load_provider(&state, "google", true).await.unwrap();
        let error = fetch_profile(&state, &google, "token").await.unwrap_err();
        assert_eq!(error.code, "OAUTH_EMAIL_UNVERIFIED");
        sqlx::query("UPDATE external_auth_providers SET userinfo_url=? WHERE provider='google'")
            .bind(format!("{base_url}/google-verified"))
            .execute(&state.pool)
            .await
            .unwrap();
        let google = load_provider(&state, "google", true).await.unwrap();
        let google_profile = fetch_profile(&state, &google, "token").await.unwrap();
        assert!(google_profile.email_verified);
        assert_eq!(google_profile.email.as_deref(), Some("google@example.com"));

        let linuxdo = load_provider(&state, "linuxdo", true).await.unwrap();
        let linuxdo_profile = fetch_profile(&state, &linuxdo, "token").await.unwrap();
        assert_eq!(linuxdo_profile.subject, "7788");
        assert_eq!(
            linuxdo_profile.email.as_deref(),
            Some("linuxdo@example.com")
        );
        assert!(!linuxdo_profile.email_verified);
        server.abort();
    }

    #[tokio::test]
    async fn pending_oidc_registration_is_atomic_and_one_time() {
        let (_directory, state) = test_support::state().await;
        configure_provider(&state, "http://127.0.0.1:9").await;
        sqlx::query("UPDATE app_settings SET value = 'true' WHERE key = 'registration_enabled'")
            .execute(&state.pool)
            .await
            .unwrap();
        let token = create_pending(
            &state,
            "oidc",
            PendingProfile {
                subject: "new-oidc-subject".into(),
                display_name: Some("New OIDC User".into()),
                email: Some("new-oidc@example.com".into()),
                email_verified: true,
                dingtalk: None,
            },
        )
        .await
        .unwrap();
        let stored: String = sqlx::query_scalar(
            "SELECT encrypted_profile FROM external_oauth_pending WHERE token_hash = ?",
        )
        .bind(token_hash(&token))
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert!(!stored.contains("new-oidc-subject"));
        assert!(!stored.contains("new-oidc@example.com"));

        let response = register_pending(
            State(state.clone()),
            Json(RegisterPendingInput {
                token: token.clone(),
                email: "new-oidc@example.com".into(),
                password: "new-password".into(),
                verify_code: None,
            }),
        )
        .await
        .unwrap();
        assert!(response.headers().contains_key(header::SET_COOKIE));
        let registered: (String, bool, String) = sqlx::query_as(
            "SELECT users.display_name, users.email_verified, external_auth_identities.subject \
             FROM users JOIN external_auth_identities ON external_auth_identities.user_id = users.id \
             WHERE users.email = 'new-oidc@example.com'",
        )
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(
            registered,
            ("New OIDC User".into(), true, "new-oidc-subject".into())
        );
        let reused = inspect_pending(State(state.clone()), Json(PendingTokenInput { token }))
            .await
            .unwrap_err();
        assert_eq!(reused.code, "OAUTH_PENDING_INVALID");

        let conflict_token = create_pending(
            &state,
            "oidc",
            PendingProfile {
                subject: "conflicting-oidc-subject".into(),
                display_name: Some("Conflict".into()),
                email: Some("new-oidc@example.com".into()),
                email_verified: true,
                dingtalk: None,
            },
        )
        .await
        .unwrap();
        let conflict = register_pending(
            State(state.clone()),
            Json(RegisterPendingInput {
                token: conflict_token.clone(),
                email: "new-oidc@example.com".into(),
                password: "new-password".into(),
                verify_code: None,
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(conflict.code, "EMAIL_EXISTS");
        let _ = inspect_pending(
            State(state.clone()),
            Json(PendingTokenInput {
                token: conflict_token,
            }),
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn oidc_discovery_and_signed_id_token_validate_nonce_and_jwks() {
        #[derive(Clone, Default)]
        struct OidcMock {
            base_url: Arc<StdMutex<String>>,
            nonce: Arc<StdMutex<String>>,
            jwks_calls: Arc<AtomicUsize>,
        }

        async fn discovery(Extension(mock): Extension<OidcMock>) -> Json<Value> {
            let base = mock.base_url.lock().unwrap().clone();
            Json(json!({
                "issuer":base,"authorization_endpoint":format!("{base}/authorize"),
                "token_endpoint":format!("{base}/token"),
                "userinfo_endpoint":format!("{base}/userinfo"),"jwks_uri":format!("{base}/jwks")
            }))
        }

        async fn token(Extension(mock): Extension<OidcMock>, body: Bytes) -> Json<Value> {
            let form = url::form_urlencoded::parse(&body)
                .into_owned()
                .collect::<HashMap<_, _>>();
            assert_eq!(form.get("code").map(String::as_str), Some("secure-code"));
            assert_eq!(
                form.get("client_id").map(String::as_str),
                Some("oidc-secure-client")
            );
            let issuer = mock.base_url.lock().unwrap().clone();
            let nonce = mock.nonce.lock().unwrap().clone();
            Json(json!({
                "access_token":"secure-access-token","token_type":"Bearer",
                "id_token":oidc_test_token(&issuer,&nonce,"secure-subject")
            }))
        }

        async fn userinfo(headers: HeaderMap) -> Json<Value> {
            assert_eq!(
                headers.get(header::AUTHORIZATION).unwrap(),
                "Bearer secure-access-token"
            );
            Json(json!({
                "sub":"secure-subject","email":"secure@example.com",
                "email_verified":true,"name":"Secure OIDC User"
            }))
        }

        async fn jwks(Extension(mock): Extension<OidcMock>) -> Json<Value> {
            mock.jwks_calls.fetch_add(1, Ordering::SeqCst);
            Json(json!({"keys":[{
                "kty":"RSA","kid":"oidc-test-key","use":"sig","alg":"RS256",
                "n":TEST_RSA_N,"e":"AQAB"
            }]}))
        }

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let mock_state = OidcMock::default();
        *mock_state.base_url.lock().unwrap() = base_url.clone();
        let mock = Router::new()
            .route("/.well-known/openid-configuration", get(discovery))
            .route("/token", post(token))
            .route("/userinfo", get(userinfo))
            .route("/jwks", get(jwks))
            .layer(Extension(mock_state.clone()));
        let server = tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
        let (_directory, state) = test_support::state().await;
        let Json(updated) = update_provider(
            State(state.clone()),
            Path("oidc".into()),
            Json(ProviderInput {
                enabled: true,
                name: "Secure OIDC".into(),
                client_id: "oidc-secure-client".into(),
                client_secret: Some("oidc-secure-secret".into()),
                clear_client_secret: false,
                authorize_url: String::new(),
                token_url: String::new(),
                userinfo_url: String::new(),
                emails_url: String::new(),
                scopes: "openid email profile".into(),
                subject_path: "sub".into(),
                email_path: "email".into(),
                display_name_path: "name".into(),
                token_auth_method: "client_secret_post".into(),
                use_pkce: true,
                issuer_url: String::new(),
                discovery_url: format!("{base_url}/.well-known/openid-configuration"),
                jwks_url: String::new(),
                validate_id_token: true,
                allowed_signing_algs: "RS256".into(),
                clock_skew_seconds: 30,
                require_email_verified: true,
                ..ProviderInput::default()
            }),
        )
        .await
        .unwrap();
        assert_eq!(updated["data"]["issuer_url"], base_url);
        assert_eq!(updated["data"]["jwks_url"], format!("{base_url}/jwks"));
        let authorization_url = create_flow(&state, "oidc", "login", None).await.unwrap();
        let parsed = url::Url::parse(&authorization_url).unwrap();
        let nonce = parsed
            .query_pairs()
            .find(|(key, _)| key == "nonce")
            .unwrap()
            .1
            .into_owned();
        let flow_state = parsed
            .query_pairs()
            .find(|(key, _)| key == "state")
            .unwrap()
            .1
            .into_owned();
        *mock_state.nonce.lock().unwrap() = nonce.clone();
        let encrypted_nonce: String = sqlx::query_scalar(
            "SELECT encrypted_nonce FROM external_oauth_flows WHERE state_hash=?",
        )
        .bind(token_hash(&flow_state))
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert!(!encrypted_nonce.contains(&nonce));
        let response = complete_callback(
            &state,
            "oidc",
            CallbackQuery {
                code: Some("secure-code".into()),
                state: Some(flow_state),
                error: None,
            },
        )
        .await
        .unwrap();
        assert!(
            response
                .headers()
                .get(header::LOCATION)
                .unwrap()
                .to_str()
                .unwrap()
                .contains("status=pending")
        );
        let config = load_provider(&state, "oidc", true).await.unwrap();
        let wrong_nonce = oidc_test_token(&base_url, "wrong-nonce", "secure-subject");
        let error = validate_oidc_id_token(&state, &config, &wrong_nonce, &nonce)
            .await
            .unwrap_err();
        assert_eq!(error.code, "OAUTH_ID_TOKEN_CLAIMS_INVALID");
        let valid = oidc_test_token(&base_url, &nonce, "secure-subject");
        let mut pieces = valid.split('.').map(str::to_string).collect::<Vec<_>>();
        let mut signature = URL_SAFE_NO_PAD.decode(&pieces[2]).unwrap();
        signature[0] ^= 1;
        pieces[2] = URL_SAFE_NO_PAD.encode(signature);
        let error = validate_oidc_id_token(&state, &config, &pieces.join("."), &nonce)
            .await
            .unwrap_err();
        assert_eq!(error.code, "OAUTH_ID_TOKEN_SIGNATURE_INVALID");
        assert_eq!(mock_state.jwks_calls.load(Ordering::SeqCst), 1);
        server.abort();
    }

    #[tokio::test]
    async fn pending_bind_requires_totp_without_consuming_token() {
        let (_directory, state) = test_support::state().await;
        configure_provider(&state, "http://127.0.0.1:9").await;
        let encrypted_secret = state.crypto.encrypt(b"JBSWY3DPEHPK3PXP").unwrap();
        sqlx::query("UPDATE users SET totp_secret = ? WHERE username = 'admin'")
            .bind(encrypted_secret)
            .execute(&state.pool)
            .await
            .unwrap();
        let token = create_pending(
            &state,
            "oidc",
            PendingProfile {
                subject: "totp-subject".into(),
                display_name: None,
                email: None,
                email_verified: false,
                dingtalk: None,
            },
        )
        .await
        .unwrap();
        let error = bind_pending(
            State(state.clone()),
            Json(BindPendingInput {
                token: token.clone(),
                identifier: "admin".into(),
                password: "test-password".into(),
                totp_code: None,
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "TOTP_REQUIRED");
        let _ = inspect_pending(State(state), Json(PendingTokenInput { token }))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn provider_validation_rejects_insecure_endpoints() {
        let (_directory, state) = test_support::state().await;
        let rejected = update_provider(
            State(state.clone()),
            Path("oidc".into()),
            Json(ProviderInput {
                enabled: true,
                name: "Unsafe".into(),
                client_id: "client".into(),
                client_secret: Some("secret".into()),
                clear_client_secret: false,
                authorize_url: "http://example.com/authorize".into(),
                token_url: "http://example.com/token".into(),
                userinfo_url: "http://example.com/userinfo".into(),
                emails_url: String::new(),
                scopes: "openid".into(),
                subject_path: "sub".into(),
                email_path: "email".into(),
                display_name_path: "name".into(),
                token_auth_method: "client_secret_post".into(),
                use_pkce: true,
                issuer_url: String::new(),
                discovery_url: String::new(),
                jwks_url: String::new(),
                validate_id_token: false,
                allowed_signing_algs: default_oidc_algs(),
                clock_skew_seconds: 120,
                require_email_verified: false,
                ..ProviderInput::default()
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(rejected.code, "INVALID_OIDC_URL");
        let providers = public_provider_summary(&state).await.unwrap();
        assert!(providers.is_empty());
    }
}
