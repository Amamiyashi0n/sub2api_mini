use std::{net::IpAddr, sync::Arc, time::Duration};

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader},
    net::TcpStream,
    time::timeout,
};
use tokio_rustls::{
    TlsConnector,
    rustls::{ClientConfig, RootCertStore, pki_types::ServerName},
};

use crate::{
    auth,
    error::{ApiError, ApiResult},
    state::AppState,
};

const SMTP_TIMEOUT: Duration = Duration::from_secs(30);

pub fn admin_router() -> Router<AppState> {
    Router::new()
        .route("/mail-settings", get(get_settings).put(update_settings))
        .route("/mail-settings/test", post(test_saved_smtp))
        .route("/mail-settings/send-test", post(send_test_email))
}

#[derive(Clone, Debug)]
struct MailSettings {
    mode: String,
    host: String,
    port: u16,
    username: String,
    password: Option<String>,
    from_email: String,
    from_name: String,
    security: String,
}

impl Default for MailSettings {
    fn default() -> Self {
        Self {
            mode: "auto".into(),
            host: String::new(),
            port: 587,
            username: String::new(),
            password: None,
            from_email: String::new(),
            from_name: "Sub2API Mini".into(),
            security: "starttls".into(),
        }
    }
}

impl MailSettings {
    fn smtp_configured(&self) -> bool {
        !self.host.is_empty()
            && !self.from_email.is_empty()
            && (self.username.is_empty() || self.password.is_some())
    }

    fn validate(&self) -> ApiResult<()> {
        let loopback = self.host == "localhost"
            || self
                .host
                .parse::<IpAddr>()
                .is_ok_and(|address| address.is_loopback());
        if !matches!(self.mode.as_str(), "auto" | "webhook" | "smtp")
            || !matches!(
                self.security.as_str(),
                "starttls" | "implicit_tls" | "plain"
            )
            || self.host.len() > 253
            || (!self.host.is_empty() && self.port == 0)
            || self
                .host
                .bytes()
                .any(|byte| byte.is_ascii_whitespace() || matches!(byte, b'/' | b'\\' | b'@'))
            || self.username.chars().count() > 512
            || self.from_name.chars().count() > 80
            || [&self.username, &self.from_name]
                .iter()
                .any(|value| value.contains(['\r', '\n']))
            || self
                .password
                .as_deref()
                .is_some_and(|value| value.len() > 4096 || value.contains(['\r', '\n']))
            || (self.security == "plain" && !loopback)
        {
            return Err(ApiError::bad_request(
                "INVALID_SMTP_CONFIG",
                "SMTP settings are invalid or insecure",
            ));
        }
        if !self.from_email.is_empty() {
            auth::normalize_email(&self.from_email)?;
        }
        if !self.username.is_empty() && self.password.is_none() {
            return Err(ApiError::bad_request(
                "INVALID_SMTP_CONFIG",
                "SMTP password is required when a username is configured",
            ));
        }
        Ok(())
    }
}

async fn load_settings(state: &AppState) -> ApiResult<MailSettings> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT key, value FROM app_settings WHERE key IN \
         ('mail_delivery_mode','smtp_host','smtp_port','smtp_username', \
          'smtp_password_encrypted','smtp_from_email','smtp_from_name','smtp_security')",
    )
    .fetch_all(&state.pool)
    .await?;
    let value = |key: &str| {
        rows.iter()
            .find(|row| row.0 == key)
            .map(|row| row.1.trim().to_string())
    };
    let encrypted_password = value("smtp_password_encrypted");
    let password = encrypted_password
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(|encrypted| {
            state.crypto.decrypt(encrypted).and_then(|bytes| {
                String::from_utf8(bytes)
                    .map_err(|_| ApiError::internal("stored SMTP password is malformed"))
            })
        })
        .transpose()?;
    let mut settings = MailSettings {
        mode: value("mail_delivery_mode").unwrap_or_else(|| "auto".into()),
        host: value("smtp_host").unwrap_or_default(),
        port: value("smtp_port")
            .and_then(|value| value.parse::<u16>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(587),
        username: value("smtp_username").unwrap_or_default(),
        password,
        from_email: value("smtp_from_email").unwrap_or_default(),
        from_name: value("smtp_from_name").unwrap_or_else(|| "Sub2API Mini".into()),
        security: value("smtp_security").unwrap_or_else(|| "starttls".into()),
    };
    settings.mode.make_ascii_lowercase();
    settings.security.make_ascii_lowercase();
    Ok(settings)
}

pub async fn is_configured(state: &AppState) -> ApiResult<bool> {
    let settings = load_settings(state).await?;
    let webhook = state.config.mail_webhook_url.is_some();
    Ok(match settings.mode.as_str() {
        "webhook" => webhook,
        "smtp" => settings.smtp_configured(),
        _ => webhook || settings.smtp_configured(),
    })
}

pub async fn deliver(
    state: &AppState,
    webhook_body: Value,
    recipient: &str,
    subject: &str,
    html: &str,
) -> ApiResult<()> {
    let recipient = auth::normalize_email(recipient)?;
    let settings = load_settings(state).await?;
    match settings.mode.as_str() {
        "webhook" => deliver_webhook(state, webhook_body).await,
        "smtp" => deliver_smtp(&settings, &recipient, subject, html).await,
        _ if state.config.mail_webhook_url.is_some() => deliver_webhook(state, webhook_body).await,
        _ => deliver_smtp(&settings, &recipient, subject, html).await,
    }
}

async fn deliver_webhook(state: &AppState, body: Value) -> ApiResult<()> {
    let endpoint = state
        .config
        .mail_webhook_url
        .as_deref()
        .ok_or_else(mail_not_configured)?;
    if !(endpoint.starts_with("https://")
        || endpoint.starts_with("http://127.0.0.1:")
        || endpoint.starts_with("http://localhost:"))
    {
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "MAIL_NOT_CONFIGURED",
            "mail webhook must use HTTPS or a loopback HTTP address",
        ));
    }
    let mut request = state
        .client
        .post(endpoint)
        .timeout(Duration::from_secs(10))
        .json(&body);
    if let Some(token) = state.config.mail_webhook_token.as_deref() {
        request = request.bearer_auth(token);
    }
    let response = request.send().await.map_err(|_| mail_delivery_failed())?;
    if !response.status().is_success() {
        return Err(mail_delivery_failed());
    }
    Ok(())
}

fn mail_not_configured() -> ApiError {
    ApiError::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "MAIL_NOT_CONFIGURED",
        "mail delivery is not configured",
    )
}

fn mail_delivery_failed() -> ApiError {
    ApiError::new(
        StatusCode::BAD_GATEWAY,
        "MAIL_DELIVERY_FAILED",
        "mail delivery failed",
    )
}

async fn deliver_smtp(
    settings: &MailSettings,
    recipient: &str,
    subject: &str,
    html: &str,
) -> ApiResult<()> {
    if !settings.smtp_configured() {
        return Err(mail_not_configured());
    }
    settings.validate()?;
    timeout(
        SMTP_TIMEOUT,
        smtp_session(settings, Some((recipient, subject, html))),
    )
    .await
    .map_err(|_| mail_delivery_failed())??;
    Ok(())
}

async fn test_smtp(settings: &MailSettings) -> ApiResult<()> {
    if !settings.smtp_configured() {
        return Err(mail_not_configured());
    }
    settings.validate()?;
    timeout(SMTP_TIMEOUT, smtp_session(settings, None))
        .await
        .map_err(|_| mail_delivery_failed())??;
    Ok(())
}

async fn smtp_session(
    settings: &MailSettings,
    message: Option<(&str, &str, &str)>,
) -> ApiResult<()> {
    let stream = TcpStream::connect((settings.host.as_str(), settings.port))
        .await
        .map_err(|_| mail_delivery_failed())?;
    stream
        .set_nodelay(true)
        .map_err(|_| mail_delivery_failed())?;
    match settings.security.as_str() {
        "implicit_tls" => {
            let tls = tls_connector(&settings.host)?.connect(tls_name(&settings.host)?, stream);
            let mut io = BufReader::new(tls.await.map_err(|_| mail_delivery_failed())?);
            smtp_greeting(&mut io).await?;
            smtp_ehlo(&mut io).await?;
            smtp_finish(&mut io, settings, message).await
        }
        "starttls" => {
            let mut io = BufReader::new(stream);
            smtp_greeting(&mut io).await?;
            let capabilities = smtp_ehlo(&mut io).await?;
            if !capabilities.lines().any(|line| {
                line.get(4..)
                    .and_then(|value| value.split_whitespace().next())
                    .is_some_and(|value| value.eq_ignore_ascii_case("STARTTLS"))
            }) {
                return Err(ApiError::new(
                    StatusCode::BAD_GATEWAY,
                    "SMTP_STARTTLS_REQUIRED",
                    "SMTP server does not advertise STARTTLS",
                ));
            }
            smtp_command(&mut io, "STARTTLS", &[220]).await?;
            let stream = io.into_inner();
            let tls = tls_connector(&settings.host)?.connect(tls_name(&settings.host)?, stream);
            let mut io = BufReader::new(tls.await.map_err(|_| mail_delivery_failed())?);
            smtp_ehlo(&mut io).await?;
            smtp_finish(&mut io, settings, message).await
        }
        "plain" => {
            let mut io = BufReader::new(stream);
            smtp_greeting(&mut io).await?;
            smtp_ehlo(&mut io).await?;
            smtp_finish(&mut io, settings, message).await
        }
        _ => Err(mail_not_configured()),
    }
}

fn tls_connector(host: &str) -> ApiResult<TlsConnector> {
    if host.is_empty() {
        return Err(mail_not_configured());
    }
    let roots = RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(TlsConnector::from(Arc::new(config)))
}

fn tls_name(host: &str) -> ApiResult<ServerName<'static>> {
    ServerName::try_from(host.to_string()).map_err(|_| {
        ApiError::bad_request("INVALID_SMTP_CONFIG", "SMTP TLS server name is invalid")
    })
}

async fn smtp_greeting<S>(io: &mut S) -> ApiResult<()>
where
    S: AsyncBufRead + AsyncWrite + Unpin,
{
    let (code, _) = smtp_read_response(io).await?;
    if code != 220 {
        return Err(mail_delivery_failed());
    }
    Ok(())
}

async fn smtp_ehlo<S>(io: &mut S) -> ApiResult<String>
where
    S: AsyncBufRead + AsyncWrite + Unpin,
{
    smtp_command(io, "EHLO sub2api-mini", &[250]).await
}

async fn smtp_finish<S>(
    io: &mut S,
    settings: &MailSettings,
    message: Option<(&str, &str, &str)>,
) -> ApiResult<()>
where
    S: AsyncBufRead + AsyncWrite + Unpin,
{
    if !settings.username.is_empty() {
        let password = settings
            .password
            .as_deref()
            .ok_or_else(mail_not_configured)?;
        let credentials = STANDARD.encode(format!("\0{}\0{}", settings.username, password));
        let response = smtp_command(io, &format!("AUTH PLAIN {credentials}"), &[235, 334]).await?;
        if response.starts_with("334") {
            smtp_command(io, &credentials, &[235]).await?;
        }
    }
    if let Some((recipient, subject, html)) = message {
        smtp_command(io, &format!("MAIL FROM:<{}>", settings.from_email), &[250]).await?;
        smtp_command(io, &format!("RCPT TO:<{recipient}>"), &[250, 251]).await?;
        smtp_command(io, "DATA", &[354]).await?;
        let message = smtp_message(settings, recipient, subject, html)?;
        io.write_all(message.as_bytes())
            .await
            .map_err(|_| mail_delivery_failed())?;
        io.flush().await.map_err(|_| mail_delivery_failed())?;
        let (code, _) = smtp_read_response(io).await?;
        if code != 250 {
            return Err(mail_delivery_failed());
        }
    }
    let _ = smtp_command(io, "QUIT", &[221]).await;
    Ok(())
}

async fn smtp_command<S>(io: &mut S, command: &str, expected: &[u16]) -> ApiResult<String>
where
    S: AsyncBufRead + AsyncWrite + Unpin,
{
    if command.contains(['\r', '\n']) || command.len() > 16 * 1024 {
        return Err(mail_delivery_failed());
    }
    io.write_all(command.as_bytes())
        .await
        .map_err(|_| mail_delivery_failed())?;
    io.write_all(b"\r\n")
        .await
        .map_err(|_| mail_delivery_failed())?;
    io.flush().await.map_err(|_| mail_delivery_failed())?;
    let (code, response) = smtp_read_response(io).await?;
    if !expected.contains(&code) {
        return Err(mail_delivery_failed());
    }
    Ok(response)
}

async fn smtp_read_response<S>(io: &mut S) -> ApiResult<(u16, String)>
where
    S: AsyncBufRead + Unpin,
{
    let mut response = String::new();
    let mut response_code = None;
    for _ in 0..100 {
        let mut line = Vec::new();
        let read = io
            .read_until(b'\n', &mut line)
            .await
            .map_err(|_| mail_delivery_failed())?;
        if read == 0 || line.len() > 8192 || response.len() + line.len() > 64 * 1024 {
            return Err(mail_delivery_failed());
        }
        let text = std::str::from_utf8(&line).map_err(|_| mail_delivery_failed())?;
        if text.len() < 4 || !text.as_bytes()[..3].iter().all(u8::is_ascii_digit) {
            return Err(mail_delivery_failed());
        }
        let code = text[..3]
            .parse::<u16>()
            .map_err(|_| mail_delivery_failed())?;
        if response_code.is_some_and(|expected| expected != code) {
            return Err(mail_delivery_failed());
        }
        response_code = Some(code);
        response.push_str(text);
        if text.as_bytes()[3] == b' ' {
            return Ok((code, response));
        }
        if text.as_bytes()[3] != b'-' {
            return Err(mail_delivery_failed());
        }
    }
    Err(mail_delivery_failed())
}

fn smtp_message(
    settings: &MailSettings,
    recipient: &str,
    subject: &str,
    html: &str,
) -> ApiResult<String> {
    let recipient = auth::normalize_email(recipient)?;
    let from = auth::normalize_email(&settings.from_email)?;
    let subject = encode_header(subject)?;
    let from_header = if settings.from_name.trim().is_empty() {
        from.clone()
    } else {
        format!("{} <{}>", encode_header(settings.from_name.trim())?, from)
    };
    let body = html.replace("\r\n", "\n").replace('\r', "\n");
    let mut stuffed = String::with_capacity(body.len() + 512);
    for line in body.split('\n') {
        if line.starts_with('.') {
            stuffed.push('.');
        }
        stuffed.push_str(line);
        stuffed.push_str("\r\n");
    }
    Ok(format!(
        "From: {from_header}\r\nTo: {recipient}\r\nSubject: {subject}\r\nMIME-Version: 1.0\r\nContent-Type: text/html; charset=UTF-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\n{stuffed}.\r\n"
    ))
}

fn encode_header(value: &str) -> ApiResult<String> {
    let value = value.trim();
    if value.is_empty() || value.contains(['\r', '\n']) || value.chars().count() > 200 {
        return Err(mail_delivery_failed());
    }
    if value.is_ascii() {
        Ok(value.to_string())
    } else {
        Ok(format!("=?UTF-8?B?{}?=", STANDARD.encode(value)))
    }
}

#[derive(Deserialize)]
struct MailSettingsInput {
    #[serde(default = "default_mode")]
    mode: String,
    #[serde(default)]
    host: String,
    #[serde(default = "default_port")]
    port: u16,
    #[serde(default)]
    username: String,
    password: Option<String>,
    #[serde(default)]
    clear_password: bool,
    #[serde(default)]
    from_email: String,
    #[serde(default)]
    from_name: String,
    #[serde(default = "default_security")]
    security: String,
}

fn default_mode() -> String {
    "auto".into()
}

fn default_port() -> u16 {
    587
}

fn default_security() -> String {
    "starttls".into()
}

async fn get_settings(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    settings_json(&state).await
}

async fn settings_json(state: &AppState) -> ApiResult<Json<Value>> {
    let settings = load_settings(state).await?;
    let webhook_configured = state.config.mail_webhook_url.is_some();
    Ok(Json(json!({"data": {
        "mode": settings.mode, "host": settings.host, "port": settings.port,
        "username": settings.username, "has_password": settings.password.is_some(),
        "from_email": settings.from_email, "from_name": settings.from_name,
        "security": settings.security, "smtp_configured": settings.smtp_configured(),
        "webhook_configured": webhook_configured,
        "mail_configured": match settings.mode.as_str() {
            "webhook" => webhook_configured,
            "smtp" => settings.smtp_configured(),
            _ => webhook_configured || settings.smtp_configured()
        }
    }})))
}

async fn update_settings(
    State(state): State<AppState>,
    Json(input): Json<MailSettingsInput>,
) -> ApiResult<Json<Value>> {
    let existing = load_settings(&state).await?;
    let supplied_password = input
        .password
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let password = if input.clear_password {
        None
    } else {
        supplied_password.clone().or(existing.password)
    };
    let settings = MailSettings {
        mode: input.mode.trim().to_ascii_lowercase(),
        host: input.host.trim().to_string(),
        port: input.port,
        username: input.username.trim().to_string(),
        password,
        from_email: input.from_email.trim().to_ascii_lowercase(),
        from_name: input.from_name.trim().to_string(),
        security: input.security.trim().to_ascii_lowercase(),
    };
    settings.validate()?;
    if settings.mode == "smtp" && !settings.smtp_configured() {
        return Err(mail_not_configured());
    }
    if settings.mode == "webhook" && state.config.mail_webhook_url.is_none() {
        return Err(mail_not_configured());
    }
    let values = [
        ("mail_delivery_mode", settings.mode.as_str()),
        ("smtp_host", settings.host.as_str()),
        ("smtp_username", settings.username.as_str()),
        ("smtp_from_email", settings.from_email.as_str()),
        ("smtp_from_name", settings.from_name.as_str()),
        ("smtp_security", settings.security.as_str()),
    ];
    let mut transaction = state.pool.begin().await?;
    for (key, value) in values {
        sqlx::query(
            "INSERT INTO app_settings (key, value) VALUES (?, ?) \
             ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=CURRENT_TIMESTAMP",
        )
        .bind(key)
        .bind(value)
        .execute(&mut *transaction)
        .await?;
    }
    sqlx::query(
        "INSERT INTO app_settings (key, value) VALUES ('smtp_port', ?) \
         ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=CURRENT_TIMESTAMP",
    )
    .bind(settings.port.to_string())
    .execute(&mut *transaction)
    .await?;
    if input.clear_password {
        sqlx::query("DELETE FROM app_settings WHERE key='smtp_password_encrypted'")
            .execute(&mut *transaction)
            .await?;
    } else if let Some(password) = supplied_password {
        sqlx::query(
            "INSERT INTO app_settings (key, value) VALUES ('smtp_password_encrypted', ?) \
             ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=CURRENT_TIMESTAMP",
        )
        .bind(state.crypto.encrypt(password.as_bytes())?)
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;
    settings_json(&state).await
}

async fn test_saved_smtp(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    test_smtp(&load_settings(&state).await?).await?;
    Ok(Json(
        json!({"data": {"message": "SMTP connection succeeded"}}),
    ))
}

#[derive(Deserialize)]
struct TestEmailInput {
    email: String,
}

async fn send_test_email(
    State(state): State<AppState>,
    Json(input): Json<TestEmailInput>,
) -> ApiResult<Json<Value>> {
    let recipient = auth::normalize_email(&input.email)?;
    let settings = load_settings(&state).await?;
    let site_name: Option<String> =
        sqlx::query_scalar("SELECT value FROM app_settings WHERE key='site_name'")
            .fetch_optional(&state.pool)
            .await?;
    let site_name = site_name.unwrap_or_else(|| "Sub2API Mini".into());
    let subject = format!("[{site_name}] SMTP test");
    let html = format!(
        "<h2>{}</h2><p>SMTP delivery is configured correctly.</p>",
        escape_html(&site_name)
    );
    deliver_smtp(&settings, &recipient, &subject, &html).await?;
    Ok(Json(json!({"data": {"message": "test email sent"}})))
}

pub fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;

    async fn read_client_line<R: AsyncBufRead + Unpin>(reader: &mut R) -> String {
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        line
    }

    #[tokio::test]
    async fn smtp_settings_encrypt_password_and_deliver_complete_message() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (read, mut write) = stream.into_split();
            let mut read = BufReader::new(read);
            write.write_all(b"220 local.test ESMTP\r\n").await.unwrap();

            let ehlo = read_client_line(&mut read).await;
            assert_eq!(ehlo, "EHLO sub2api-mini\r\n");
            write
                .write_all(b"250-local.test\r\n250 AUTH PLAIN\r\n")
                .await
                .unwrap();
            let auth = read_client_line(&mut read).await;
            assert!(auth.starts_with("AUTH PLAIN "));
            let encoded = auth
                .trim()
                .split_once(' ')
                .unwrap()
                .1
                .split_once(' ')
                .unwrap()
                .1;
            assert_eq!(STANDARD.decode(encoded).unwrap(), b"\0mailer\0mail-secret");
            write.write_all(b"235 authenticated\r\n").await.unwrap();
            assert_eq!(
                read_client_line(&mut read).await,
                "MAIL FROM:<sender@example.com>\r\n"
            );
            write.write_all(b"250 sender ok\r\n").await.unwrap();
            assert_eq!(
                read_client_line(&mut read).await,
                "RCPT TO:<recipient@example.com>\r\n"
            );
            write.write_all(b"250 recipient ok\r\n").await.unwrap();
            assert_eq!(read_client_line(&mut read).await, "DATA\r\n");
            write.write_all(b"354 send message\r\n").await.unwrap();
            let mut message = String::new();
            loop {
                let line = read_client_line(&mut read).await;
                if line == ".\r\n" {
                    break;
                }
                message.push_str(&line);
            }
            write.write_all(b"250 queued\r\n").await.unwrap();
            assert_eq!(read_client_line(&mut read).await, "QUIT\r\n");
            write.write_all(b"221 bye\r\n").await.unwrap();
            message
        });

        let (_directory, state) = test_support::state().await;
        let Json(saved) = update_settings(
            State(state.clone()),
            Json(MailSettingsInput {
                mode: "smtp".into(),
                host: "127.0.0.1".into(),
                port,
                username: "mailer".into(),
                password: Some("mail-secret".into()),
                clear_password: false,
                from_email: "sender@example.com".into(),
                from_name: "示例网关".into(),
                security: "plain".into(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(saved["data"]["smtp_configured"], true);
        assert_eq!(saved["data"]["has_password"], true);
        let encrypted: String = sqlx::query_scalar(
            "SELECT value FROM app_settings WHERE key='smtp_password_encrypted'",
        )
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert!(!encrypted.contains("mail-secret"));
        assert_eq!(state.crypto.decrypt(&encrypted).unwrap(), b"mail-secret");
        assert!(is_configured(&state).await.unwrap());

        deliver(
            &state,
            json!({"kind":"test","to":"recipient@example.com"}),
            "recipient@example.com",
            "欢迎使用",
            "<p>Hello</p>\n.leading dot",
        )
        .await
        .unwrap();
        let message = server.await.unwrap();
        assert!(message.contains("Subject: =?UTF-8?B?"));
        assert!(message.contains("From: =?UTF-8?B?"));
        assert!(message.contains("\r\n..leading dot\r\n"));
        assert!(!message.contains("mail-secret"));

        let Json(preserved) = update_settings(
            State(state.clone()),
            Json(MailSettingsInput {
                mode: "smtp".into(),
                host: "127.0.0.1".into(),
                port,
                username: "mailer".into(),
                password: None,
                clear_password: false,
                from_email: "sender@example.com".into(),
                from_name: "Gateway".into(),
                security: "plain".into(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(preserved["data"]["has_password"], true);
    }

    #[test]
    fn plaintext_smtp_is_limited_to_loopback() {
        let settings = MailSettings {
            mode: "smtp".into(),
            host: "smtp.example.com".into(),
            port: 25,
            username: String::new(),
            password: None,
            from_email: "sender@example.com".into(),
            from_name: "Gateway".into(),
            security: "plain".into(),
        };
        assert_eq!(settings.validate().unwrap_err().code, "INVALID_SMTP_CONFIG");
    }
}
