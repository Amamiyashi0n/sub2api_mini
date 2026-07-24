use std::{
    env,
    net::SocketAddr,
    path::{Path, PathBuf},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};

use crate::error::{ApiError, ApiResult};

#[derive(Clone, Debug)]
pub struct Config {
    pub bind: SocketAddr,
    pub callback_bind: SocketAddr,
    pub database_path: PathBuf,
    pub admin_username: String,
    pub admin_password: String,
    pub master_key: [u8; 32],
    pub public_ui_url: String,
    pub session_hours: i64,
    pub mail_webhook_url: Option<String>,
    pub mail_webhook_token: Option<String>,
    pub turnstile_verify_url: String,
}

impl Config {
    pub fn from_env() -> ApiResult<Self> {
        load_env_file()?;

        let key = env::var("SUB2API_MINI_MASTER_KEY")
            .map_err(|_| ApiError::config("SUB2API_MINI_MASTER_KEY is required"))?;
        let decoded = STANDARD
            .decode(key.trim())
            .map_err(|_| ApiError::config("SUB2API_MINI_MASTER_KEY must be base64"))?;
        let master_key: [u8; 32] = decoded.try_into().map_err(|_| {
            ApiError::config("SUB2API_MINI_MASTER_KEY must decode to exactly 32 bytes")
        })?;

        Ok(Self {
            bind: env::var("SUB2API_MINI_BIND")
                .or_else(|_| env::var("SUB2API_MINI_API_BIND"))
                .unwrap_or_else(|_| "0.0.0.0:8080".into())
                .parse()
                .map_err(|_| ApiError::config("invalid SUB2API_MINI_BIND"))?,
            callback_bind: env::var("SUB2API_MINI_CALLBACK_BIND")
                .unwrap_or_else(|_| "0.0.0.0:1455".into())
                .parse()
                .map_err(|_| ApiError::config("invalid SUB2API_MINI_CALLBACK_BIND"))?,
            database_path: env::var("SUB2API_MINI_DATABASE_PATH")
                .unwrap_or_else(|_| "/data/sub2api_mini/sub2api_mini.sqlite3".into())
                .into(),
            admin_username: env::var("SUB2API_MINI_ADMIN_USERNAME")
                .unwrap_or_else(|_| "admin".into()),
            admin_password: env::var("SUB2API_MINI_ADMIN_PASSWORD")
                .map_err(|_| ApiError::config("SUB2API_MINI_ADMIN_PASSWORD is required"))?,
            master_key,
            public_ui_url: env::var("SUB2API_MINI_PUBLIC_UI_URL")
                .unwrap_or_else(|_| "http://192.168.30.180:8080".into()),
            session_hours: env::var("SUB2API_MINI_SESSION_HOURS")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(12),
            mail_webhook_url: optional_env("SUB2API_MINI_MAIL_WEBHOOK_URL"),
            mail_webhook_token: optional_env("SUB2API_MINI_MAIL_WEBHOOK_TOKEN"),
            turnstile_verify_url: env::var("SUB2API_MINI_TURNSTILE_VERIFY_URL").unwrap_or_else(
                |_| "https://challenges.cloudflare.com/turnstile/v0/siteverify".into(),
            ),
        })
    }
}

fn optional_env(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn load_env_file() -> ApiResult<()> {
    const RUNTIME_ENV_FILE: &str = "/data/sub2api_mini/.env";

    if let Some(path) = env::var_os("SUB2API_MINI_ENV_FILE") {
        return dotenvy::from_path(&path)
            .map(|_| ())
            .map_err(|_| ApiError::config("cannot load SUB2API_MINI_ENV_FILE"));
    }
    if Path::new(RUNTIME_ENV_FILE).is_file() {
        dotenvy::from_path(RUNTIME_ENV_FILE)
            .map(|_| ())
            .map_err(|_| ApiError::config("cannot load runtime environment file"))?;
    } else {
        let _ = dotenvy::dotenv();
    }
    Ok(())
}
