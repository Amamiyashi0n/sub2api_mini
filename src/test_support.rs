use tempfile::TempDir;

use crate::{config::Config, crypto::Crypto, db, state::AppState};

pub async fn state() -> (TempDir, AppState) {
    let directory = tempfile::tempdir().unwrap();
    let config = Config {
        bind: "127.0.0.1:0".parse().unwrap(),
        callback_bind: "127.0.0.1:0".parse().unwrap(),
        database_path: directory.path().join("test.sqlite3"),
        admin_username: "admin".into(),
        admin_password: "test-password".into(),
        master_key: [9; 32],
        public_ui_url: "http://localhost:8080".into(),
        session_hours: 12,
        mail_webhook_url: None,
        mail_webhook_token: None,
        turnstile_verify_url: "http://127.0.0.1:0/turnstile".into(),
    };
    let pool = db::connect(
        &config.database_path,
        &config.admin_username,
        &config.admin_password,
    )
    .await
    .unwrap();
    let state = AppState::new(pool, Crypto::new(&config.master_key), config).unwrap();
    (directory, state)
}
