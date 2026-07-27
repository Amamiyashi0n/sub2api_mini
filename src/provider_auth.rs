use std::time::{Duration, Instant};

use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use chrono::Utc;
use hmac::{Hmac, Mac};
use http::{HeaderMap, StatusCode, header};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use ring::{
    rand::SystemRandom,
    signature::{RSA_PKCS1_SHA256, RsaKeyPair},
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{
    crypto::token_hash,
    error::{ApiError, ApiResult},
    models::Account,
    state::{AppState, CachedVertexToken},
};

type HmacSha256 = Hmac<Sha256>;

#[derive(Deserialize)]
struct ServiceAccountKey {
    #[serde(rename = "type")]
    kind: String,
    project_id: String,
    #[serde(default)]
    private_key_id: String,
    private_key: String,
    client_email: String,
    #[serde(default = "default_google_token_url")]
    token_uri: String,
}

#[derive(Deserialize)]
struct GoogleTokenResponse {
    access_token: String,
    #[serde(default)]
    expires_in: i64,
}

fn default_google_token_url() -> String {
    "https://oauth2.googleapis.com/token".into()
}

fn parse_service_account(account: &Account) -> ApiResult<ServiceAccountKey> {
    let raw = account
        .credentials
        .provider_str("service_account_json")
        .ok_or_else(|| {
            ApiError::bad_request(
                "SERVICE_ACCOUNT_REQUIRED",
                "service account JSON is missing",
            )
        })?;
    if raw.len() > 128 * 1024 {
        return Err(ApiError::bad_request(
            "INVALID_SERVICE_ACCOUNT",
            "service account JSON is too large",
        ));
    }
    let mut key: ServiceAccountKey = serde_json::from_str(raw).map_err(|_| {
        ApiError::bad_request("INVALID_SERVICE_ACCOUNT", "service account JSON is invalid")
    })?;
    key.kind = key.kind.trim().to_string();
    key.project_id = key.project_id.trim().to_string();
    key.private_key_id = key.private_key_id.trim().to_string();
    key.client_email = key.client_email.trim().to_ascii_lowercase();
    if key.kind != "service_account"
        || key.project_id.is_empty()
        || key.private_key.is_empty()
        || !key.client_email.contains('@')
    {
        return Err(ApiError::bad_request(
            "INVALID_SERVICE_ACCOUNT",
            "service account JSON is missing required fields",
        ));
    }
    key.token_uri = default_google_token_url();
    Ok(key)
}

fn decode_pkcs8_private_key(pem: &str) -> ApiResult<Vec<u8>> {
    let mut inside = false;
    let mut ended = false;
    let mut encoded = String::new();
    for line in pem.lines().map(str::trim) {
        match line {
            "-----BEGIN PRIVATE KEY-----" => inside = true,
            "-----END PRIVATE KEY-----" if inside => {
                ended = true;
                break;
            }
            _ if inside => encoded.push_str(line),
            _ => {}
        }
    }
    if !ended || encoded.is_empty() {
        return Err(ApiError::bad_request(
            "INVALID_SERVICE_ACCOUNT",
            "service account private key must be PKCS#8 PEM",
        ));
    }
    STANDARD.decode(encoded).map_err(|_| {
        ApiError::bad_request(
            "INVALID_SERVICE_ACCOUNT",
            "service account private key is invalid",
        )
    })
}

fn service_account_assertion(key: &ServiceAccountKey, now: i64) -> ApiResult<String> {
    let header = if key.private_key_id.is_empty() {
        json!({"alg":"RS256","typ":"JWT"})
    } else {
        json!({"alg":"RS256","typ":"JWT","kid":key.private_key_id})
    };
    let claims = json!({
        "iss": key.client_email,
        "scope": "https://www.googleapis.com/auth/cloud-platform",
        "aud": key.token_uri,
        "iat": now,
        "exp": now.saturating_add(3600),
    });
    let header = URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&header)
            .map_err(|_| ApiError::internal("service account JWT serialization failed"))?,
    );
    let claims = URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&claims)
            .map_err(|_| ApiError::internal("service account JWT serialization failed"))?,
    );
    let signing_input = format!("{header}.{claims}");
    let der = decode_pkcs8_private_key(&key.private_key)?;
    let key_pair = RsaKeyPair::from_pkcs8(&der).map_err(|_| {
        ApiError::bad_request(
            "INVALID_SERVICE_ACCOUNT",
            "service account private key cannot be parsed",
        )
    })?;
    let mut signature = vec![0; key_pair.public().modulus_len()];
    key_pair
        .sign(
            &RSA_PKCS1_SHA256,
            &SystemRandom::new(),
            signing_input.as_bytes(),
            &mut signature,
        )
        .map_err(|_| ApiError::internal("service account JWT signing failed"))?;
    Ok(format!(
        "{signing_input}.{}",
        URL_SAFE_NO_PAD.encode(signature)
    ))
}

async fn vertex_access_token(state: &AppState, account: &Account) -> ApiResult<String> {
    let key = parse_service_account(account)?;
    let fingerprint = token_hash(&format!(
        "{}\0{}\0{}",
        key.client_email, key.private_key_id, key.private_key
    ));
    if let Some(cached) = state.vertex_tokens.lock().await.get(&account.row.id)
        && cached.credential_fingerprint == fingerprint
        && cached.expires_at > Instant::now()
    {
        return Ok(cached.token.clone());
    }
    let assertion = service_account_assertion(&key, Utc::now().timestamp())?;
    let client = state.client_for_account(account).await?;
    let response = client
        .post(&key.token_uri)
        .form(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
            ("assertion", assertion.as_str()),
        ])
        .send()
        .await?;
    if !response.status().is_success() {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "VERTEX_AUTH_FAILED",
            format!(
                "Google rejected the service account token exchange ({})",
                response.status()
            ),
        ));
    }
    let token: GoogleTokenResponse = response.json().await?;
    if token.access_token.trim().is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "VERTEX_AUTH_FAILED",
            "Google returned an empty access token",
        ));
    }
    let ttl = token.expires_in.clamp(360, 86_400).saturating_sub(300) as u64;
    state.vertex_tokens.lock().await.insert(
        account.row.id,
        CachedVertexToken {
            token: token.access_token.clone(),
            credential_fingerprint: fingerprint,
            expires_at: Instant::now() + Duration::from_secs(ttl),
        },
    );
    Ok(token.access_token)
}

fn provider_message_body(raw: &[u8], anthropic_version: &str) -> ApiResult<(String, Vec<u8>)> {
    let mut value: Value = serde_json::from_slice(raw)
        .map_err(|_| ApiError::bad_request("INVALID_JSON", "request body is not valid JSON"))?;
    let object = value.as_object_mut().ok_or_else(|| {
        ApiError::bad_request("INVALID_JSON", "request body must be a JSON object")
    })?;
    let model = object
        .remove("model")
        .and_then(|value| value.as_str().map(str::to_string))
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| ApiError::bad_request("MODEL_REQUIRED", "model is required"))?;
    object.remove("stream");
    object.insert(
        "anthropic_version".into(),
        Value::String(anthropic_version.into()),
    );
    let body = serde_json::to_vec(&value)
        .map_err(|_| ApiError::internal("provider request serialization failed"))?;
    Ok((model, body))
}

pub async fn send_vertex(
    state: &AppState,
    account: &Account,
    incoming_headers: &HeaderMap,
    raw: &[u8],
) -> ApiResult<reqwest::Response> {
    let (model, body) = provider_message_body(raw, "vertex-2023-10-16")?;
    let key = parse_service_account(account)?;
    let project = account
        .credentials
        .provider_str("project_id")
        .unwrap_or(&key.project_id);
    let location = account
        .credentials
        .provider_str("location")
        .unwrap_or("global");
    let base = account.row.base_url.trim_end_matches('/');
    let project = utf8_percent_encode(project, NON_ALPHANUMERIC);
    let location = utf8_percent_encode(location, NON_ALPHANUMERIC);
    let model = utf8_percent_encode(&model, NON_ALPHANUMERIC);
    let url = format!(
        "{base}/v1/projects/{project}/locations/{location}/publishers/anthropic/models/{model}:rawPredict"
    );
    let token = vertex_access_token(state, account).await?;
    let client = state.client_for_account(account).await?;
    let mut request = client
        .post(url)
        .bearer_auth(token)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(beta) = incoming_headers.get("anthropic-beta") {
        request = request.header("anthropic-beta", beta);
    }
    request.body(body).send().await.map_err(ApiError::from)
}

fn hmac(key: &[u8], value: &str) -> ApiResult<Vec<u8>> {
    let mut mac = HmacSha256::new_from_slice(key)
        .map_err(|_| ApiError::internal("AWS signing key initialization failed"))?;
    mac.update(value.as_bytes());
    Ok(mac.finalize().into_bytes().to_vec())
}

fn sign_bedrock_headers(
    account: &Account,
    url: &url::Url,
    body: &[u8],
) -> ApiResult<Vec<(String, String)>> {
    let access_key = account
        .credentials
        .provider_str("aws_access_key_id")
        .ok_or_else(|| {
            ApiError::bad_request(
                "BEDROCK_AWS_CREDENTIALS_REQUIRED",
                "AWS access key id is missing",
            )
        })?;
    let secret_key = account
        .credentials
        .provider_str("aws_secret_access_key")
        .ok_or_else(|| {
            ApiError::bad_request(
                "BEDROCK_AWS_CREDENTIALS_REQUIRED",
                "AWS secret access key is missing",
            )
        })?;
    let region = account
        .credentials
        .provider_str("region")
        .unwrap_or("us-east-1");
    let now = Utc::now();
    let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
    let date = now.format("%Y%m%d").to_string();
    let host = url
        .host_str()
        .ok_or_else(|| ApiError::internal("Bedrock URL has no host"))?;
    let payload_hash = hex::encode(Sha256::digest(body));
    let session_token = account.credentials.provider_str("aws_session_token");
    let mut canonical_headers =
        format!("content-type:application/json\nhost:{host}\nx-amz-date:{amz_date}\n");
    let mut signed_headers = "content-type;host;x-amz-date".to_string();
    if let Some(token) = session_token {
        canonical_headers.push_str(&format!("x-amz-security-token:{token}\n"));
        signed_headers.push_str(";x-amz-security-token");
    }
    let canonical_request = format!(
        "POST\n{}\n\n{canonical_headers}\n{signed_headers}\n{payload_hash}",
        url.path()
    );
    let scope = format!("{date}/{region}/bedrock/aws4_request");
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
        hex::encode(Sha256::digest(canonical_request.as_bytes()))
    );
    let date_key = hmac(format!("AWS4{secret_key}").as_bytes(), &date)?;
    let region_key = hmac(&date_key, region)?;
    let service_key = hmac(&region_key, "bedrock")?;
    let signing_key = hmac(&service_key, "aws4_request")?;
    let signature = hex::encode(hmac(&signing_key, &string_to_sign)?);
    let authorization = format!(
        "AWS4-HMAC-SHA256 Credential={access_key}/{scope}, SignedHeaders={signed_headers}, Signature={signature}"
    );
    let mut headers = vec![
        ("x-amz-date".into(), amz_date),
        ("authorization".into(), authorization),
    ];
    if let Some(token) = session_token {
        headers.push(("x-amz-security-token".into(), token.to_string()));
    }
    Ok(headers)
}

pub async fn send_bedrock(
    state: &AppState,
    account: &Account,
    raw: &[u8],
) -> ApiResult<reqwest::Response> {
    let (model, body) = provider_message_body(raw, "bedrock-2023-05-31")?;
    let region = account
        .credentials
        .provider_str("region")
        .unwrap_or("us-east-1");
    let model = utf8_percent_encode(&model, NON_ALPHANUMERIC);
    let url = url::Url::parse(&format!(
        "https://bedrock-runtime.{region}.amazonaws.com/model/{model}/invoke"
    ))
    .map_err(|_| ApiError::internal("Bedrock URL is invalid"))?;
    let client = state.client_for_account(account).await?;
    let mut request = client
        .post(url.clone())
        .header(header::CONTENT_TYPE, "application/json");
    if account.credentials.provider_str("auth_mode") == Some("api_key") {
        let token = account.credentials.api_key.as_deref().ok_or_else(|| {
            ApiError::bad_request("BEDROCK_API_KEY_REQUIRED", "Bedrock API key is missing")
        })?;
        request = request.bearer_auth(token);
    } else {
        for (name, value) in sign_bedrock_headers(account, &url, &body)? {
            request = request.header(name, value);
        }
    }
    request.body(body).send().await.map_err(ApiError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Map;

    #[test]
    fn bedrock_sigv4_contains_scoped_credential_and_signed_headers() {
        let mut provider = Map::new();
        provider.insert("region".into(), Value::String("us-east-1".into()));
        provider.insert(
            "aws_access_key_id".into(),
            Value::String("AKIDEXAMPLE".into()),
        );
        provider.insert(
            "aws_secret_access_key".into(),
            Value::String("secret".into()),
        );
        let account = crate::models::Account {
            row: crate::models::AccountRow {
                id: 1,
                name: "bedrock".into(),
                kind: "bedrock".into(),
                platform: "anthropic".into(),
                account_type: "bedrock".into(),
                base_url: "https://bedrock-runtime.us-east-1.amazonaws.com".into(),
                encrypted_credentials: String::new(),
                priority: 50,
                concurrency: 3,
                enabled: true,
                cooldown_until: None,
                last_used_at: None,
                last_error: None,
                proxy_id: None,
                proxy_name: None,
                proxy_active: None,
                encrypted_proxy_url: None,
                parent_account_id: None,
                quota_dimension: "global".into(),
                notes: String::new(),
                crs_account_id: None,
                tls_fingerprint_profile_id: None,
                created_at: String::new(),
                updated_at: String::new(),
            },
            credentials: crate::models::Credentials {
                provider,
                ..Default::default()
            },
            proxy_url: None,
        };
        let url =
            url::Url::parse("https://bedrock-runtime.us-east-1.amazonaws.com/model/test/invoke")
                .unwrap();
        let headers = sign_bedrock_headers(&account, &url, br#"{}"#).unwrap();
        let authorization = headers
            .iter()
            .find(|(name, _)| name == "authorization")
            .unwrap()
            .1
            .as_str();
        assert!(authorization.contains("AKIDEXAMPLE/"));
        assert!(authorization.contains("/us-east-1/bedrock/aws4_request"));
        assert!(authorization.contains("SignedHeaders=content-type;host;x-amz-date"));
    }
}
