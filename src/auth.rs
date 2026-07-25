use std::time::{Duration, Instant};

use axum::{
    Json, Router,
    extract::{Extension, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::{Duration as ChronoDuration, Utc};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    crypto::{hash_password, random_token, token_hash, verify_password},
    error::{ApiError, ApiResult},
    state::AppState,
};

#[derive(Clone, Debug)]
pub struct AuthSession {
    pub id: i64,
    pub user_id: i64,
    pub username: String,
    pub display_name: String,
    pub role: String,
}

pub fn router(state: AppState) -> Router<AppState> {
    let protected = Router::new()
        .route("/me", get(me))
        .route("/logout", post(logout))
        .route_layer(middleware::from_fn_with_state(state, user_guard));
    Router::new()
        .route("/login", post(login))
        .route("/login/2fa", post(crate::totp::complete_login))
        .route("/register", post(register))
        .route("/send-verification-code", post(send_verification_code))
        .route("/forgot-password", post(forgot_password))
        .route("/reset-password", post(reset_password))
        .merge(protected)
}

pub async fn user_guard(
    State(state): State<AppState>,
    mut request: axum::extract::Request,
    next: Next,
) -> ApiResult<Response> {
    let session = authenticate(&state, request.headers(), request.method()).await?;
    request.extensions_mut().insert(session);
    Ok(next.run(request).await)
}

pub async fn admin_guard(
    State(state): State<AppState>,
    mut request: axum::extract::Request,
    next: Next,
) -> ApiResult<Response> {
    let session = authenticate(&state, request.headers(), request.method()).await?;
    if session.role != "admin" {
        return Err(ApiError::forbidden("administrator access is required"));
    }
    request.extensions_mut().insert(session);
    Ok(next.run(request).await)
}

async fn authenticate(
    state: &AppState,
    headers: &HeaderMap,
    method: &Method,
) -> ApiResult<AuthSession> {
    let token = cookie_value(headers, "mini_session")
        .ok_or_else(|| ApiError::unauthorized("session is required"))?;
    let row: Option<(i64, i64, String, String, String, String)> = sqlx::query_as(
        "SELECT auth_sessions.id, users.id, users.username, users.display_name, users.role, auth_sessions.csrf_hash \
         FROM auth_sessions JOIN users ON users.id = auth_sessions.user_id \
         WHERE auth_sessions.token_hash = ? AND datetime(auth_sessions.expires_at) > CURRENT_TIMESTAMP \
         AND users.enabled = 1",
    )
    .bind(token_hash(&token))
    .fetch_optional(&state.pool)
    .await?;
    let (id, user_id, username, display_name, role, csrf_hash) =
        row.ok_or_else(|| ApiError::unauthorized("session is invalid or expired"))?;

    if !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
        let csrf = headers
            .get("x-csrf-token")
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| ApiError::forbidden("CSRF token is required"))?;
        if token_hash(csrf) != csrf_hash {
            return Err(ApiError::forbidden("CSRF token is invalid"));
        }
    }

    Ok(AuthSession {
        id,
        user_id,
        username,
        display_name,
        role,
    })
}

#[derive(Deserialize)]
struct LoginInput {
    #[serde(alias = "email")]
    username: String,
    password: String,
    #[serde(default)]
    turnstile_token: String,
}

async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<LoginInput>,
) -> ApiResult<Response> {
    let source = headers
        .get("x-real-ip")
        .or_else(|| headers.get("x-forwarded-for"))
        .and_then(|value| value.to_str().ok())
        .unwrap_or("direct")
        .split(',')
        .next()
        .unwrap_or("direct")
        .trim()
        .to_string();
    verify_turnstile(&state, &input.turnstile_token, &source).await?;
    {
        let mut attempts = state.login_attempts.lock().await;
        let recent = attempts.entry(source).or_default();
        recent.retain(|time| time.elapsed() < Duration::from_secs(60));
        if recent.len() >= 10 {
            return Err(ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "LOGIN_RATE_LIMITED",
                "too many login attempts",
            ));
        }
        recent.push(Instant::now());
    }

    let identifier = input.username.trim();
    let row: Option<(i64, String, String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT id, username, password_hash, display_name, role, totp_secret FROM users \
         WHERE (username = ? COLLATE NOCASE OR email = ? COLLATE NOCASE) AND enabled = 1",
    )
    .bind(identifier)
    .bind(identifier)
    .fetch_optional(&state.pool)
    .await?;
    let (user_id, username, password_hash, display_name, role, totp_secret) =
        row.ok_or_else(|| ApiError::unauthorized("invalid username or password"))?;
    if !verify_password(&input.password, &password_hash) {
        return Err(ApiError::unauthorized("invalid username or password"));
    }
    if totp_secret.is_some() {
        return crate::totp::begin_login(&state, user_id, &username).await;
    }

    create_session_response(&state, user_id, &username, &display_name, &role).await
}

pub(crate) async fn create_session_response(
    state: &AppState,
    user_id: i64,
    username: &str,
    display_name: &str,
    role: &str,
) -> ApiResult<Response> {
    let session_token = random_token(32)?;
    let csrf_token = random_token(24)?;
    let expires_at = Utc::now() + ChronoDuration::hours(state.config.session_hours);
    sqlx::query(
        "INSERT INTO auth_sessions (user_id, token_hash, csrf_hash, expires_at) VALUES (?, ?, ?, ?)",
    )
    .bind(user_id)
    .bind(token_hash(&session_token))
    .bind(token_hash(&csrf_token))
    .bind(expires_at.to_rfc3339())
    .execute(&state.pool)
    .await?;

    let mut response = Json(json!({"data": {
        "username": username, "display_name": display_name,
        "role": role, "csrf_token": csrf_token
    }}))
    .into_response();
    let cookie = format!(
        "mini_session={session_token}; HttpOnly; SameSite=Lax; Path=/; Max-Age={}",
        state.config.session_hours * 3600
    );
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&cookie).map_err(|_| ApiError::internal("cookie creation failed"))?,
    );
    Ok(response)
}

#[derive(Deserialize)]
struct RegisterInput {
    email: String,
    password: String,
    verify_code: Option<String>,
    #[serde(default)]
    turnstile_token: String,
}

async fn register(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<RegisterInput>,
) -> ApiResult<Response> {
    if !bool_setting(&state, "registration_enabled", false).await? {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "REGISTRATION_DISABLED",
            "registration is disabled",
        ));
    }
    let verify_email = bool_setting(&state, "email_verification_enabled", false).await?;
    if !(verify_email
        && input
            .verify_code
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty()))
    {
        verify_turnstile(&state, &input.turnstile_token, client_ip(&headers)).await?;
    }
    let email = normalize_email(&input.email)?;
    validate_password(&input.password)?;
    let username = username_for_email(&email);
    let display_name = email.split('@').next().unwrap_or("user").to_string();
    let password_hash = hash_password(&input.password)?;
    let mut transaction = state.pool.begin().await?;

    if verify_email {
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

    let user_id = sqlx::query(
        "INSERT INTO users \
         (username, display_name, password_hash, role, email, email_verified) \
         VALUES (?, ?, ?, 'user', ?, ?)",
    )
    .bind(&username)
    .bind(&display_name)
    .bind(password_hash)
    .bind(&email)
    .bind(verify_email)
    .execute(&mut *transaction)
    .await
    .map_err(|error| match error {
        sqlx::Error::Database(ref database) if database.is_unique_violation() => {
            ApiError::bad_request("EMAIL_EXISTS", "email is already registered")
        }
        other => other.into(),
    })?
    .last_insert_rowid();
    transaction.commit().await?;
    create_session_response(&state, user_id, &username, &display_name, "user").await
}

#[derive(Deserialize)]
struct EmailInput {
    email: String,
    #[serde(default)]
    turnstile_token: String,
}

async fn send_verification_code(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<EmailInput>,
) -> ApiResult<Json<Value>> {
    if !bool_setting(&state, "registration_enabled", false).await?
        || !bool_setting(&state, "email_verification_enabled", false).await?
    {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "EMAIL_VERIFICATION_DISABLED",
            "email verification is disabled",
        ));
    }
    verify_turnstile(&state, &input.turnstile_token, client_ip(&headers)).await?;
    let email = normalize_email(&input.email)?;
    let exists: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE email = ? COLLATE NOCASE")
            .bind(&email)
            .fetch_one(&state.pool)
            .await?;
    if exists > 0 {
        return Err(ApiError::bad_request(
            "EMAIL_EXISTS",
            "email is already registered",
        ));
    }
    enforce_challenge_cooldown(&state, &email, "email_verification").await?;
    let code = verification_code()?;
    let challenge_hash = token_hash(&code);
    replace_challenge(
        &state,
        &email,
        "email_verification",
        &challenge_hash,
        ChronoDuration::minutes(10),
    )
    .await?;
    if let Err(error) = deliver_mail(&state, "email_verification", &email, Some(&code), None).await
    {
        sqlx::query("DELETE FROM auth_challenges WHERE token_hash = ?")
            .bind(challenge_hash)
            .execute(&state.pool)
            .await?;
        return Err(error);
    }
    Ok(Json(
        json!({"data": {"message": "verification code sent", "countdown": 60}}),
    ))
}

async fn forgot_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<EmailInput>,
) -> ApiResult<Json<Value>> {
    if !bool_setting(&state, "password_reset_enabled", true).await? {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "PASSWORD_RESET_DISABLED",
            "password reset is disabled",
        ));
    }
    if !crate::mail::is_configured(&state).await? {
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "MAIL_NOT_CONFIGURED",
            "mail delivery is not configured",
        ));
    }
    verify_turnstile(&state, &input.turnstile_token, client_ip(&headers)).await?;
    let email = normalize_email(&input.email)?;
    let user: Option<i64> =
        sqlx::query_scalar("SELECT id FROM users WHERE email = ? COLLATE NOCASE AND enabled = 1")
            .bind(&email)
            .fetch_optional(&state.pool)
            .await?;
    if user.is_none() {
        return Ok(generic_reset_response());
    }
    enforce_challenge_cooldown(&state, &email, "password_reset").await?;
    let token = random_token(32)?;
    let challenge_hash = token_hash(&token);
    replace_challenge(
        &state,
        &email,
        "password_reset",
        &challenge_hash,
        ChronoDuration::minutes(30),
    )
    .await?;
    let encoded_email: String = url::form_urlencoded::byte_serialize(email.as_bytes()).collect();
    let encoded_token: String = url::form_urlencoded::byte_serialize(token.as_bytes()).collect();
    let reset_url = format!(
        "{}/#/reset-password?email={encoded_email}&token={encoded_token}",
        state.config.public_ui_url.trim_end_matches('/')
    );
    if let Err(error) = deliver_mail(&state, "password_reset", &email, None, Some(&reset_url)).await
    {
        sqlx::query("DELETE FROM auth_challenges WHERE token_hash = ?")
            .bind(challenge_hash)
            .execute(&state.pool)
            .await?;
        tracing::warn!(%error, "password reset mail delivery failed");
    }
    Ok(generic_reset_response())
}

fn generic_reset_response() -> Json<Value> {
    Json(json!({"data": {"message":
        "if the email is registered, a password reset link will be sent"
    }}))
}

#[derive(Deserialize)]
struct ResetPasswordInput {
    email: String,
    token: String,
    new_password: String,
}

async fn reset_password(
    State(state): State<AppState>,
    Json(input): Json<ResetPasswordInput>,
) -> ApiResult<Json<Value>> {
    if !bool_setting(&state, "password_reset_enabled", true).await? {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "PASSWORD_RESET_DISABLED",
            "password reset is disabled",
        ));
    }
    let email = normalize_email(&input.email)?;
    validate_password(&input.new_password)?;
    let mut transaction = state.pool.begin().await?;
    let consumed = sqlx::query(
        "UPDATE auth_challenges SET consumed_at = CURRENT_TIMESTAMP \
         WHERE email = ? COLLATE NOCASE AND purpose = 'password_reset' \
         AND token_hash = ? AND consumed_at IS NULL \
         AND datetime(expires_at) > CURRENT_TIMESTAMP",
    )
    .bind(&email)
    .bind(token_hash(input.token.trim()))
    .execute(&mut *transaction)
    .await?;
    if consumed.rows_affected() != 1 {
        return Err(ApiError::bad_request(
            "INVALID_RESET_TOKEN",
            "password reset token is invalid or expired",
        ));
    }
    let user_id: Option<i64> =
        sqlx::query_scalar("SELECT id FROM users WHERE email = ? COLLATE NOCASE AND enabled = 1")
            .bind(&email)
            .fetch_optional(&mut *transaction)
            .await?;
    let user_id = user_id.ok_or_else(|| {
        ApiError::bad_request(
            "INVALID_RESET_TOKEN",
            "password reset token is invalid or expired",
        )
    })?;
    sqlx::query("UPDATE users SET password_hash = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?")
        .bind(hash_password(&input.new_password)?)
        .bind(user_id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query("DELETE FROM auth_sessions WHERE user_id = ?")
        .bind(user_id)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(Json(
        json!({"data": {"message": "password reset successfully"}}),
    ))
}

pub(crate) async fn deliver_mail(
    state: &AppState,
    kind: &str,
    email: &str,
    code: Option<&str>,
    reset_url: Option<&str>,
) -> ApiResult<()> {
    let site_name: Option<String> =
        sqlx::query_scalar("SELECT value FROM app_settings WHERE key = 'site_name'")
            .fetch_optional(&state.pool)
            .await?;
    let site_name = site_name.unwrap_or_else(|| "Sub2API Mini".into());
    let (subject, content) = match kind {
        "password_reset" => (
            format!("[{site_name}] Password reset"),
            format!(
                "<h2>{}</h2><p>Use the following link to reset your password:</p><p><a href=\"{}\">Reset password</a></p><p>This link expires in 30 minutes.</p>",
                crate::mail::escape_html(&site_name),
                crate::mail::escape_html(reset_url.unwrap_or_default())
            ),
        ),
        "profile_email_verification" => (
            format!("[{site_name}] Confirm your email"),
            format!(
                "<h2>{}</h2><p>Your email verification code is:</p><p><strong>{}</strong></p><p>The code expires in 10 minutes.</p>",
                crate::mail::escape_html(&site_name),
                crate::mail::escape_html(code.unwrap_or_default())
            ),
        ),
        _ => (
            format!("[{site_name}] Email verification code"),
            format!(
                "<h2>{}</h2><p>Your verification code is:</p><p><strong>{}</strong></p><p>The code expires in 10 minutes.</p>",
                crate::mail::escape_html(&site_name),
                crate::mail::escape_html(code.unwrap_or_default())
            ),
        ),
    };
    crate::mail::deliver(
        state,
        json!({
            "kind": kind, "to": email, "site_name": site_name,
            "code": code, "reset_url": reset_url
        }),
        email,
        &subject,
        &content,
    )
    .await
}

async fn replace_challenge(
    state: &AppState,
    email: &str,
    purpose: &str,
    challenge_hash: &str,
    lifetime: ChronoDuration,
) -> ApiResult<()> {
    let mut transaction = state.pool.begin().await?;
    sqlx::query(
        "DELETE FROM auth_challenges WHERE email = ? COLLATE NOCASE AND purpose = ? \
         AND consumed_at IS NULL",
    )
    .bind(email)
    .bind(purpose)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "INSERT INTO auth_challenges (email, purpose, token_hash, expires_at) VALUES (?, ?, ?, ?)",
    )
    .bind(email)
    .bind(purpose)
    .bind(challenge_hash)
    .bind((Utc::now() + lifetime).to_rfc3339())
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(())
}

async fn enforce_challenge_cooldown(state: &AppState, email: &str, purpose: &str) -> ApiResult<()> {
    let recent: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM auth_challenges WHERE email = ? COLLATE NOCASE AND purpose = ? \
         AND datetime(created_at) > datetime('now', '-60 seconds')",
    )
    .bind(email)
    .bind(purpose)
    .fetch_one(&state.pool)
    .await?;
    if recent > 0 {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "VERIFICATION_RATE_LIMITED",
            "please wait before requesting another message",
        ));
    }
    Ok(())
}

#[derive(Deserialize)]
struct TurnstileResponse {
    success: bool,
    #[serde(default, rename = "error-codes")]
    error_codes: Vec<String>,
}

fn client_ip(headers: &HeaderMap) -> &str {
    headers
        .get("x-real-ip")
        .or_else(|| headers.get("x-forwarded-for"))
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("")
}

async fn verify_turnstile(state: &AppState, token: &str, remote_ip: &str) -> ApiResult<()> {
    if !bool_setting(state, "turnstile_enabled", false).await? {
        return Ok(());
    }
    let encrypted: Option<String> = sqlx::query_scalar(
        "SELECT value FROM app_settings WHERE key = 'turnstile_secret_key_encrypted'",
    )
    .fetch_optional(&state.pool)
    .await?;
    let encrypted = encrypted.filter(|value| !value.is_empty()).ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "TURNSTILE_NOT_CONFIGURED",
            "turnstile is enabled but its secret key is not configured",
        )
    })?;
    let secret = String::from_utf8(state.crypto.decrypt(&encrypted)?)
        .map_err(|_| ApiError::internal("stored turnstile secret key is malformed"))?;
    let token = token.trim();
    if token.is_empty() {
        return Err(ApiError::bad_request(
            "TURNSTILE_VERIFICATION_FAILED",
            "turnstile verification is required",
        ));
    }
    let mut form = vec![("secret", secret.as_str()), ("response", token)];
    if !remote_ip.is_empty() {
        form.push(("remoteip", remote_ip));
    }
    let response = state
        .client
        .post(&state.config.turnstile_verify_url)
        .timeout(Duration::from_secs(10))
        .form(&form)
        .send()
        .await
        .map_err(|_| {
            ApiError::new(
                StatusCode::BAD_GATEWAY,
                "TURNSTILE_UNAVAILABLE",
                "turnstile verification service is unavailable",
            )
        })?;
    if !response.status().is_success() {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "TURNSTILE_UNAVAILABLE",
            "turnstile verification service is unavailable",
        ));
    }
    let result: TurnstileResponse = response.json().await.map_err(|_| {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            "TURNSTILE_UNAVAILABLE",
            "turnstile verification service returned an invalid response",
        )
    })?;
    if !result.success {
        tracing::warn!(error_codes = ?result.error_codes, "turnstile verification rejected");
        return Err(ApiError::bad_request(
            "TURNSTILE_VERIFICATION_FAILED",
            "turnstile verification failed",
        ));
    }
    Ok(())
}

pub(crate) async fn bool_setting(state: &AppState, key: &str, default: bool) -> ApiResult<bool> {
    let value: Option<String> = sqlx::query_scalar("SELECT value FROM app_settings WHERE key = ?")
        .bind(key)
        .fetch_optional(&state.pool)
        .await?;
    Ok(value
        .as_deref()
        .and_then(|value| value.parse::<bool>().ok())
        .unwrap_or(default))
}

pub(crate) fn normalize_email(value: &str) -> ApiResult<String> {
    let email = value.trim().to_ascii_lowercase();
    let Some((local, domain)) = email.split_once('@') else {
        return Err(ApiError::bad_request("INVALID_EMAIL", "email is invalid"));
    };
    if email.len() > 254
        || local.is_empty()
        || local.len() > 64
        || domain.is_empty()
        || !domain.contains('.')
        || local.contains('@')
        || domain.contains('@')
        || !email.is_ascii()
        || email
            .chars()
            .any(|character| character.is_ascii_whitespace())
    {
        return Err(ApiError::bad_request("INVALID_EMAIL", "email is invalid"));
    }
    Ok(email)
}

pub(crate) fn validate_password(password: &str) -> ApiResult<()> {
    if !(8..=128).contains(&password.len()) {
        return Err(ApiError::bad_request(
            "INVALID_PASSWORD",
            "password must be 8-128 characters",
        ));
    }
    Ok(())
}

pub(crate) fn username_for_email(email: &str) -> String {
    let local = email.split('@').next().unwrap_or("user");
    let mut base = local
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || "._-".contains(*character))
        .take(48)
        .collect::<String>();
    if base.len() < 3 {
        base = "user".into();
    }
    format!("{base}_{}", &token_hash(email)[..8])
}

pub(crate) fn verification_code() -> ApiResult<String> {
    let entropy = random_token(16)?;
    Ok(token_hash(&entropy)[..8].to_ascii_uppercase())
}

async fn me(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
) -> ApiResult<Json<Value>> {
    let csrf = random_token(24)?;
    sqlx::query("UPDATE auth_sessions SET csrf_hash = ? WHERE id = ?")
        .bind(token_hash(&csrf))
        .bind(session.id)
        .execute(&state.pool)
        .await?;
    Ok(Json(json!({"data": {
        "id": session.user_id, "username": session.username,
        "display_name": session.display_name, "role": session.role, "csrf_token": csrf
    }})))
}

async fn logout(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
) -> ApiResult<Response> {
    sqlx::query("DELETE FROM auth_sessions WHERE id = ?")
        .bind(session.id)
        .execute(&state.pool)
        .await?;
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_static("mini_session=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0"),
    );
    Ok(response)
}

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|cookies| {
            cookies.split(';').find_map(|cookie| {
                let (key, value) = cookie.trim().split_once('=')?;
                (key == name).then(|| value.to_string())
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;
    use axum::{Router, extract::State as AxumState, routing::post};
    use std::sync::Arc;
    use tokio::sync::Mutex;

    async fn set_flag(state: &AppState, key: &str, value: bool) {
        sqlx::query(
            "INSERT INTO app_settings (key, value) VALUES (?, ?) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(key)
        .bind(value.to_string())
        .execute(&state.pool)
        .await
        .unwrap();
    }

    #[test]
    fn normalizes_email_and_derives_private_username() {
        let email = normalize_email(" Alice.Example@Example.COM ").unwrap();
        assert_eq!(email, "alice.example@example.com");
        let username = username_for_email(&email);
        assert!(username.starts_with("alice.example_"));
        assert!(!username.contains("@example.com"));
        assert!(normalize_email("invalid").is_err());
    }

    #[tokio::test]
    async fn turnstile_uses_encrypted_secret_and_verifies_form_tokens() {
        async fn verify(
            AxumState(captured): AxumState<Arc<Mutex<Vec<String>>>>,
            body: String,
        ) -> Json<Value> {
            captured.lock().await.push(body.clone());
            Json(json!({"success": body.contains("response=valid-token"),
                "error-codes": if body.contains("response=valid-token") { vec![] } else { vec!["invalid-input-response"] }}))
        }

        let captured = Arc::new(Mutex::new(Vec::new()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server_state = captured.clone();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new()
                    .route("/turnstile", post(verify))
                    .with_state(server_state),
            )
            .await
            .unwrap();
        });
        let (_directory, mut state) = test_support::state().await;
        state.config.turnstile_verify_url = format!("http://{address}/turnstile");
        set_flag(&state, "turnstile_enabled", true).await;
        let encrypted = state.crypto.encrypt(b"turnstile-secret-canary").unwrap();
        sqlx::query(
            "INSERT INTO app_settings (key,value) VALUES ('turnstile_secret_key_encrypted',?)",
        )
        .bind(&encrypted)
        .execute(&state.pool)
        .await
        .unwrap();
        assert!(!encrypted.contains("turnstile-secret-canary"));
        verify_turnstile(&state, "valid-token", "192.0.2.10")
            .await
            .unwrap();
        let error = verify_turnstile(&state, "invalid-token", "192.0.2.11")
            .await
            .unwrap_err();
        assert_eq!(error.code, "TURNSTILE_VERIFICATION_FAILED");
        let requests = captured.lock().await;
        assert!(requests[0].contains("secret=turnstile-secret-canary"));
        assert!(requests[0].contains("response=valid-token"));
        assert!(requests[0].contains("remoteip=192.0.2.10"));
        drop(requests);
        server.abort();
    }

    #[tokio::test]
    async fn reset_token_is_one_time_and_revokes_sessions() {
        let (_directory, state) = test_support::state().await;
        set_flag(&state, "registration_enabled", true).await;
        register(
            State(state.clone()),
            HeaderMap::new(),
            Json(RegisterInput {
                email: "reset@example.com".into(),
                password: "old-password".into(),
                verify_code: None,
                turnstile_token: String::new(),
            }),
        )
        .await
        .unwrap();
        let reset_token = "one-time-reset-token";
        sqlx::query(
            "INSERT INTO auth_challenges (email, purpose, token_hash, expires_at) \
             VALUES ('reset@example.com', 'password_reset', ?, datetime('now', '+10 minutes'))",
        )
        .bind(token_hash(reset_token))
        .execute(&state.pool)
        .await
        .unwrap();
        let _ = reset_password(
            State(state.clone()),
            Json(ResetPasswordInput {
                email: "reset@example.com".into(),
                token: reset_token.into(),
                new_password: "new-password".into(),
            }),
        )
        .await
        .unwrap();
        let password_hash: String =
            sqlx::query_scalar("SELECT password_hash FROM users WHERE email = 'reset@example.com'")
                .fetch_one(&state.pool)
                .await
                .unwrap();
        assert!(verify_password("new-password", &password_hash));
        let sessions: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM auth_sessions JOIN users ON users.id = auth_sessions.user_id \
             WHERE users.email = 'reset@example.com'",
        )
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(sessions, 0);
        assert!(
            reset_password(
                State(state),
                Json(ResetPasswordInput {
                    email: "reset@example.com".into(),
                    token: reset_token.into(),
                    new_password: "another-password".into(),
                }),
            )
            .await
            .is_err()
        );
    }
}
