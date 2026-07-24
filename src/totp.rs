use axum::{
    Json, Router,
    extract::{Extension, State},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::{Duration, Utc};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use serde_json::{Value, json};
use sha1::Sha1;

use crate::{
    auth::{AuthSession, create_session_response},
    crypto::{random_token, token_hash, verify_password},
    error::{ApiError, ApiResult},
    state::AppState,
};

pub fn user_router() -> Router<AppState> {
    Router::new()
        .route("/totp/status", get(status))
        .route("/totp/setup", post(setup))
        .route("/totp/enable", post(enable))
        .route("/totp/disable", post(disable))
}

pub async fn begin_login(state: &AppState, user_id: i64, username: &str) -> ApiResult<Response> {
    let token = random_token(32)?;
    sqlx::query("DELETE FROM totp_login_challenges WHERE user_id = ? OR datetime(expires_at) <= CURRENT_TIMESTAMP")
        .bind(user_id)
        .execute(&state.pool)
        .await?;
    sqlx::query(
        "INSERT INTO totp_login_challenges (user_id, token_hash, expires_at) VALUES (?, ?, ?)",
    )
    .bind(user_id)
    .bind(token_hash(&token))
    .bind((Utc::now() + Duration::minutes(5)).to_rfc3339())
    .execute(&state.pool)
    .await?;
    Ok(
        Json(json!({"data": {"requires_2fa": true, "temp_token": token,
        "user_email_masked": mask_identifier(username)}}))
        .into_response(),
    )
}

#[derive(Deserialize)]
pub struct Login2faInput {
    temp_token: String,
    totp_code: String,
}

pub async fn complete_login(
    State(state): State<AppState>,
    Json(input): Json<Login2faInput>,
) -> ApiResult<Response> {
    let challenge: Option<(i64, i64)> = sqlx::query_as(
        "SELECT user_id, attempts FROM totp_login_challenges WHERE token_hash = ? \
         AND datetime(expires_at) > CURRENT_TIMESTAMP",
    )
    .bind(token_hash(input.temp_token.trim()))
    .fetch_optional(&state.pool)
    .await?;
    let (user_id, _attempts) = challenge
        .filter(|row| row.1 < 6)
        .ok_or_else(|| ApiError::unauthorized("2FA challenge is invalid or expired"))?;
    if !verify_user_code(&state, user_id, input.totp_code.trim(), true).await? {
        sqlx::query(
            "UPDATE totp_login_challenges SET attempts = attempts + 1 WHERE token_hash = ?",
        )
        .bind(token_hash(input.temp_token.trim()))
        .execute(&state.pool)
        .await?;
        return Err(ApiError::unauthorized("TOTP or recovery code is invalid"));
    }
    sqlx::query("DELETE FROM totp_login_challenges WHERE user_id = ?")
        .bind(user_id)
        .execute(&state.pool)
        .await?;
    let user: (String, String, String) = sqlx::query_as(
        "SELECT username, display_name, role FROM users WHERE id = ? AND enabled = 1",
    )
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| ApiError::unauthorized("user is unavailable"))?;
    create_session_response(&state, user_id, &user.0, &user.1, &user.2).await
}

async fn status(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
) -> ApiResult<Json<Value>> {
    let row: (Option<String>, Option<String>) =
        sqlx::query_as("SELECT totp_secret, totp_enabled_at FROM users WHERE id = ?")
            .bind(session.user_id)
            .fetch_one(&state.pool)
            .await?;
    Ok(Json(
        json!({"data": {"enabled": row.0.is_some(), "enabled_at": row.1}}),
    ))
}

#[derive(Deserialize)]
struct PasswordInput {
    password: String,
}

async fn setup(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
    Json(input): Json<PasswordInput>,
) -> ApiResult<Json<Value>> {
    verify_current_password(&state, session.user_id, &input.password).await?;
    let mut secret = [0u8; 20];
    getrandom::fill(&mut secret).map_err(|_| ApiError::internal("random generation failed"))?;
    let secret = base32_encode(&secret);
    let encrypted = state.crypto.encrypt(secret.as_bytes())?;
    sqlx::query(
        "UPDATE users SET totp_pending_secret = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
    )
    .bind(encrypted)
    .bind(session.user_id)
    .execute(&state.pool)
    .await?;
    let issuer = percent_encode("Sub2API Mini");
    let label = percent_encode(&session.username);
    let uri = format!(
        "otpauth://totp/{issuer}:{label}?secret={secret}&issuer={issuer}&digits=6&period=30"
    );
    Ok(Json(
        json!({"data": {"secret": secret, "otpauth_uri": uri}}),
    ))
}

#[derive(Deserialize)]
struct CodeInput {
    totp_code: String,
}

async fn enable(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
    Json(input): Json<CodeInput>,
) -> ApiResult<Json<Value>> {
    let pending: String = sqlx::query_scalar(
        "SELECT totp_pending_secret FROM users WHERE id = ? AND totp_pending_secret IS NOT NULL",
    )
    .bind(session.user_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| ApiError::bad_request("TOTP_SETUP_REQUIRED", "start TOTP setup first"))?;
    let secret = decrypt_secret(&state, &pending)?;
    if !verify_totp(&secret, input.totp_code.trim(), Utc::now().timestamp()) {
        return Err(ApiError::bad_request(
            "INVALID_TOTP",
            "TOTP code is invalid",
        ));
    }
    let mut recovery_codes = Vec::with_capacity(10);
    for _ in 0..10 {
        recovery_codes.push(random_token(9)?);
    }
    let hashes = recovery_codes
        .iter()
        .map(|code| token_hash(code))
        .collect::<Vec<_>>();
    sqlx::query(
        "UPDATE users SET totp_secret = totp_pending_secret, totp_pending_secret = NULL, \
         totp_recovery_hashes = ?, totp_enabled_at = CURRENT_TIMESTAMP, updated_at = CURRENT_TIMESTAMP \
         WHERE id = ?",
    )
    .bind(serde_json::to_string(&hashes).map_err(|_| ApiError::internal("recovery code serialization failed"))?)
    .bind(session.user_id)
    .execute(&state.pool)
    .await?;
    Ok(Json(json!({"data": {"recovery_codes": recovery_codes}})))
}

#[derive(Deserialize)]
struct DisableInput {
    password: String,
    totp_code: String,
}

async fn disable(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
    Json(input): Json<DisableInput>,
) -> ApiResult<Json<Value>> {
    verify_current_password(&state, session.user_id, &input.password).await?;
    if !verify_user_code(&state, session.user_id, input.totp_code.trim(), false).await? {
        return Err(ApiError::bad_request(
            "INVALID_TOTP",
            "TOTP code is invalid",
        ));
    }
    sqlx::query(
        "UPDATE users SET totp_secret = NULL, totp_pending_secret = NULL, \
         totp_recovery_hashes = '[]', totp_enabled_at = NULL, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
    )
    .bind(session.user_id)
    .execute(&state.pool)
    .await?;
    sqlx::query("DELETE FROM totp_login_challenges WHERE user_id = ?")
        .bind(session.user_id)
        .execute(&state.pool)
        .await?;
    Ok(Json(json!({"data": {"enabled": false}})))
}

async fn verify_current_password(state: &AppState, user_id: i64, password: &str) -> ApiResult<()> {
    let hash: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id = ?")
        .bind(user_id)
        .fetch_one(&state.pool)
        .await?;
    if !verify_password(password, &hash) {
        return Err(ApiError::bad_request(
            "CURRENT_PASSWORD_INVALID",
            "current password is invalid",
        ));
    }
    Ok(())
}

pub(crate) async fn verify_user_code(
    state: &AppState,
    user_id: i64,
    code: &str,
    allow_recovery: bool,
) -> ApiResult<bool> {
    let row: Option<(String, String)> = sqlx::query_as(
        "SELECT totp_secret, totp_recovery_hashes FROM users WHERE id = ? AND totp_secret IS NOT NULL",
    )
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some((encrypted, recovery_json)) = row else {
        return Ok(false);
    };
    let secret = decrypt_secret(state, &encrypted)?;
    if verify_totp(&secret, code, Utc::now().timestamp()) {
        return Ok(true);
    }
    if allow_recovery {
        let mut hashes: Vec<String> = serde_json::from_str(&recovery_json)
            .map_err(|_| ApiError::internal("stored recovery codes are malformed"))?;
        let hash = token_hash(code);
        if let Some(index) = hashes.iter().position(|value| value == &hash) {
            hashes.remove(index);
            sqlx::query("UPDATE users SET totp_recovery_hashes = ? WHERE id = ?")
                .bind(
                    serde_json::to_string(&hashes)
                        .map_err(|_| ApiError::internal("recovery code serialization failed"))?,
                )
                .bind(user_id)
                .execute(&state.pool)
                .await?;
            return Ok(true);
        }
    }
    Ok(false)
}

pub(crate) async fn verify_code(state: &AppState, user_id: i64, code: &str) -> ApiResult<bool> {
    verify_user_code(state, user_id, code, false).await
}

fn decrypt_secret(state: &AppState, encrypted: &str) -> ApiResult<String> {
    String::from_utf8(state.crypto.decrypt(encrypted)?)
        .map_err(|_| ApiError::internal("stored TOTP secret is malformed"))
}

fn verify_totp(secret: &str, code: &str, timestamp: i64) -> bool {
    let Some(expected) = code.parse::<u32>().ok().filter(|_| code.len() == 6) else {
        return false;
    };
    let Ok(secret) = base32_decode(secret) else {
        return false;
    };
    (-1..=1).any(|offset| totp_at(&secret, timestamp / 30 + offset) == expected)
}

fn totp_at(secret: &[u8], counter: i64) -> u32 {
    let mut mac = Hmac::<Sha1>::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(&(counter as u64).to_be_bytes());
    let digest = mac.finalize().into_bytes();
    let offset = (digest[19] & 0x0f) as usize;
    let value = ((digest[offset] as u32 & 0x7f) << 24)
        | (digest[offset + 1] as u32) << 16
        | (digest[offset + 2] as u32) << 8
        | digest[offset + 3] as u32;
    value % 1_000_000
}

const BASE32: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

fn base32_encode(bytes: &[u8]) -> String {
    let mut output = String::new();
    let mut buffer = 0u32;
    let mut bits = 0;
    for byte in bytes {
        buffer = (buffer << 8) | *byte as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            output.push(BASE32[((buffer >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        output.push(BASE32[((buffer << (5 - bits)) & 31) as usize] as char);
    }
    output
}

fn base32_decode(value: &str) -> Result<Vec<u8>, ()> {
    let mut output = Vec::new();
    let mut buffer = 0u32;
    let mut bits = 0;
    for byte in value.bytes() {
        let index = BASE32.iter().position(|item| *item == byte).ok_or(())? as u32;
        buffer = (buffer << 5) | index;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            output.push((buffer >> bits) as u8);
        }
    }
    Ok(output)
}

fn percent_encode(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

fn mask_identifier(value: &str) -> String {
    if value.len() <= 3 {
        "***".into()
    } else {
        format!("{}***", &value[..2])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;

    #[test]
    fn matches_rfc_totp_vector_and_base32_round_trip() {
        let secret = b"12345678901234567890";
        let encoded = base32_encode(secret);
        assert_eq!(base32_decode(&encoded).unwrap(), secret);
        assert_eq!(totp_at(secret, 59 / 30), 287082);
        assert!(verify_totp(&encoded, "287082", 59));
    }

    #[tokio::test]
    async fn setup_enables_totp_and_consumes_recovery_code_once() {
        let (_directory, state) = test_support::state().await;
        let session = AuthSession {
            id: 1,
            user_id: 1,
            username: "admin".into(),
            display_name: "admin".into(),
            role: "admin".into(),
        };
        let Json(setup_data) = setup(
            State(state.clone()),
            Extension(session.clone()),
            Json(PasswordInput {
                password: "test-password".into(),
            }),
        )
        .await
        .unwrap();
        let secret = setup_data["data"]["secret"].as_str().unwrap();
        let secret_bytes = base32_decode(secret).unwrap();
        let code = format!("{:06}", totp_at(&secret_bytes, Utc::now().timestamp() / 30));
        let Json(enabled) = enable(
            State(state.clone()),
            Extension(session),
            Json(CodeInput { totp_code: code }),
        )
        .await
        .unwrap();
        let recovery = enabled["data"]["recovery_codes"][0]
            .as_str()
            .unwrap()
            .to_string();
        assert!(verify_user_code(&state, 1, &recovery, true).await.unwrap());
        assert!(!verify_user_code(&state, 1, &recovery, true).await.unwrap());
        let encrypted: String = sqlx::query_scalar("SELECT totp_secret FROM users WHERE id = 1")
            .fetch_one(&state.pool)
            .await
            .unwrap();
        assert!(!encrypted.contains(secret));
    }
}
