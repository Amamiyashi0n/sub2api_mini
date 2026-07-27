use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use reqwest::Client;
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::FromRow;
use tokio_rustls::rustls::{self, RootCertStore};

use crate::{
    error::{ApiError, ApiResult},
    models::Account,
    state::{AppState, build_http_client},
};

pub fn admin_router() -> Router<AppState> {
    Router::new()
        .route("/tls-fingerprint-profiles", get(list).post(create))
        .route(
            "/tls-fingerprint-profiles/{id}",
            get(get_one).put(update).delete(delete_profile),
        )
}

#[derive(Clone, FromRow)]
struct ProfileRow {
    id: i64,
    name: String,
    description: Option<String>,
    enable_grease: bool,
    cipher_suites: String,
    curves: String,
    point_formats: String,
    signature_algorithms: String,
    alpn_protocols: String,
    supported_versions: String,
    key_share_groups: String,
    psk_modes: String,
    extensions: String,
    created_at: String,
    updated_at: String,
}

impl ProfileRow {
    fn value(&self) -> Value {
        json!({
            "id": self.id,
            "name": self.name,
            "description": self.description,
            "enable_grease": self.enable_grease,
            "cipher_suites": parse_array::<u16>(&self.cipher_suites),
            "curves": parse_array::<u16>(&self.curves),
            "point_formats": parse_array::<u16>(&self.point_formats),
            "signature_algorithms": parse_array::<u16>(&self.signature_algorithms),
            "alpn_protocols": parse_array::<String>(&self.alpn_protocols),
            "supported_versions": parse_array::<u16>(&self.supported_versions),
            "key_share_groups": parse_array::<u16>(&self.key_share_groups),
            "psk_modes": parse_array::<u16>(&self.psk_modes),
            "extensions": parse_array::<u16>(&self.extensions),
            "created_at": self.created_at,
            "updated_at": self.updated_at,
        })
    }
}

#[derive(Default, Deserialize)]
struct ProfileInput {
    name: Option<String>,
    #[serde(default, deserialize_with = "crate::models::deserialize_nullable")]
    description: Option<Option<String>>,
    enable_grease: Option<bool>,
    cipher_suites: Option<Vec<u16>>,
    curves: Option<Vec<u16>>,
    point_formats: Option<Vec<u16>>,
    signature_algorithms: Option<Vec<u16>>,
    alpn_protocols: Option<Vec<String>>,
    supported_versions: Option<Vec<u16>>,
    key_share_groups: Option<Vec<u16>>,
    psk_modes: Option<Vec<u16>>,
    extensions: Option<Vec<u16>>,
}

async fn list(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let rows = all_profiles(&state).await?;
    Ok(Json(
        json!({"data": rows.iter().map(ProfileRow::value).collect::<Vec<_>>() }),
    ))
}

async fn get_one(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    Ok(Json(
        json!({"data": find_profile(&state, id).await?.value()}),
    ))
}

async fn create(
    State(state): State<AppState>,
    Json(input): Json<ProfileInput>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let profile = normalized_profile(input, None)?;
    let result = insert_profile(&state, &profile)
        .await
        .map_err(unique_name_error)?;
    state.tls_clients.lock().await.clear();
    let created = find_profile(&state, result).await?;
    Ok((StatusCode::CREATED, Json(json!({"data": created.value()}))))
}

async fn update(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(input): Json<ProfileInput>,
) -> ApiResult<Json<Value>> {
    let current = find_profile(&state, id).await?;
    let profile = normalized_profile(input, Some(current))?;
    sqlx::query(
        "UPDATE tls_fingerprint_profiles SET name = ?, description = ?, enable_grease = ?, \
         cipher_suites = ?, curves = ?, point_formats = ?, signature_algorithms = ?, \
         alpn_protocols = ?, supported_versions = ?, key_share_groups = ?, psk_modes = ?, \
         extensions = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
    )
    .bind(&profile.name)
    .bind(&profile.description)
    .bind(profile.enable_grease)
    .bind(&profile.cipher_suites)
    .bind(&profile.curves)
    .bind(&profile.point_formats)
    .bind(&profile.signature_algorithms)
    .bind(&profile.alpn_protocols)
    .bind(&profile.supported_versions)
    .bind(&profile.key_share_groups)
    .bind(&profile.psk_modes)
    .bind(&profile.extensions)
    .bind(id)
    .execute(&state.pool)
    .await
    .map_err(unique_name_error)?;
    state.tls_clients.lock().await.clear();
    Ok(Json(
        json!({"data": find_profile(&state, id).await?.value()}),
    ))
}

async fn delete_profile(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<Value>> {
    let result = sqlx::query("DELETE FROM tls_fingerprint_profiles WHERE id = ?")
        .bind(id)
        .execute(&state.pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found("TLS fingerprint profile not found"));
    }
    state.tls_clients.lock().await.clear();
    Ok(Json(json!({"data": {"id": id}})))
}

async fn all_profiles(state: &AppState) -> ApiResult<Vec<ProfileRow>> {
    Ok(sqlx::query_as::<_, ProfileRow>(
        "SELECT id, name, description, enable_grease, cipher_suites, curves, point_formats, \
         signature_algorithms, alpn_protocols, supported_versions, key_share_groups, psk_modes, \
         extensions, created_at, updated_at FROM tls_fingerprint_profiles ORDER BY name, id",
    )
    .fetch_all(&state.pool)
    .await?)
}

async fn find_profile(state: &AppState, id: i64) -> ApiResult<ProfileRow> {
    sqlx::query_as::<_, ProfileRow>(
        "SELECT id, name, description, enable_grease, cipher_suites, curves, point_formats, \
         signature_algorithms, alpn_protocols, supported_versions, key_share_groups, psk_modes, \
         extensions, created_at, updated_at FROM tls_fingerprint_profiles WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| ApiError::not_found("TLS fingerprint profile not found"))
}

async fn insert_profile(state: &AppState, profile: &ProfileRow) -> Result<i64, sqlx::Error> {
    let result = sqlx::query(
        "INSERT INTO tls_fingerprint_profiles (name, description, enable_grease, cipher_suites, \
         curves, point_formats, signature_algorithms, alpn_protocols, supported_versions, \
         key_share_groups, psk_modes, extensions) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&profile.name)
    .bind(&profile.description)
    .bind(profile.enable_grease)
    .bind(&profile.cipher_suites)
    .bind(&profile.curves)
    .bind(&profile.point_formats)
    .bind(&profile.signature_algorithms)
    .bind(&profile.alpn_protocols)
    .bind(&profile.supported_versions)
    .bind(&profile.key_share_groups)
    .bind(&profile.psk_modes)
    .bind(&profile.extensions)
    .execute(&state.pool)
    .await?;
    Ok(result.last_insert_rowid())
}

fn normalized_profile(input: ProfileInput, current: Option<ProfileRow>) -> ApiResult<ProfileRow> {
    let name = input
        .name
        .or_else(|| current.as_ref().map(|row| row.name.clone()))
        .unwrap_or_default()
        .trim()
        .to_string();
    let description = input
        .description
        .unwrap_or_else(|| current.as_ref().and_then(|row| row.description.clone()))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let cipher_suites = input.cipher_suites.unwrap_or_else(|| {
        current
            .as_ref()
            .map(|row| parse_array(&row.cipher_suites))
            .unwrap_or_default()
    });
    let curves = input.curves.unwrap_or_else(|| {
        current
            .as_ref()
            .map(|row| parse_array(&row.curves))
            .unwrap_or_default()
    });
    let point_formats = input.point_formats.unwrap_or_else(|| {
        current
            .as_ref()
            .map(|row| parse_array(&row.point_formats))
            .unwrap_or_default()
    });
    let signature_algorithms = input.signature_algorithms.unwrap_or_else(|| {
        current
            .as_ref()
            .map(|row| parse_array(&row.signature_algorithms))
            .unwrap_or_default()
    });
    let alpn_protocols = input.alpn_protocols.unwrap_or_else(|| {
        current
            .as_ref()
            .map(|row| parse_array(&row.alpn_protocols))
            .unwrap_or_default()
    });
    let supported_versions = input.supported_versions.unwrap_or_else(|| {
        current
            .as_ref()
            .map(|row| parse_array(&row.supported_versions))
            .unwrap_or_default()
    });
    let key_share_groups = input.key_share_groups.unwrap_or_else(|| {
        current
            .as_ref()
            .map(|row| parse_array(&row.key_share_groups))
            .unwrap_or_default()
    });
    let psk_modes = input.psk_modes.unwrap_or_else(|| {
        current
            .as_ref()
            .map(|row| parse_array(&row.psk_modes))
            .unwrap_or_default()
    });
    let extensions = input.extensions.unwrap_or_else(|| {
        current
            .as_ref()
            .map(|row| parse_array(&row.extensions))
            .unwrap_or_default()
    });
    let lengths = [
        cipher_suites.len(),
        curves.len(),
        point_formats.len(),
        signature_algorithms.len(),
        alpn_protocols.len(),
        supported_versions.len(),
        key_share_groups.len(),
        psk_modes.len(),
        extensions.len(),
    ];
    if name.is_empty()
        || name.chars().count() > 100
        || description
            .as_ref()
            .is_some_and(|value| value.chars().count() > 1000)
        || lengths.into_iter().any(|length| length > 128)
        || alpn_protocols
            .iter()
            .any(|protocol| protocol.is_empty() || protocol.len() > 32)
    {
        return Err(ApiError::bad_request(
            "INVALID_TLS_FINGERPRINT_PROFILE",
            "TLS fingerprint profile is invalid",
        ));
    }
    let json_array = |value: &dyn std::fmt::Debug| format!("{value:?}");
    Ok(ProfileRow {
        id: current.as_ref().map(|row| row.id).unwrap_or_default(),
        name,
        description,
        enable_grease: input
            .enable_grease
            .or_else(|| current.as_ref().map(|row| row.enable_grease))
            .unwrap_or(false),
        cipher_suites: serde_json::to_string(&cipher_suites)
            .unwrap_or_else(|_| json_array(&cipher_suites)),
        curves: serde_json::to_string(&curves).unwrap_or_else(|_| json_array(&curves)),
        point_formats: serde_json::to_string(&point_formats)
            .unwrap_or_else(|_| json_array(&point_formats)),
        signature_algorithms: serde_json::to_string(&signature_algorithms)
            .unwrap_or_else(|_| json_array(&signature_algorithms)),
        alpn_protocols: serde_json::to_string(&alpn_protocols).expect("ALPN serializes"),
        supported_versions: serde_json::to_string(&supported_versions)
            .unwrap_or_else(|_| json_array(&supported_versions)),
        key_share_groups: serde_json::to_string(&key_share_groups)
            .unwrap_or_else(|_| json_array(&key_share_groups)),
        psk_modes: serde_json::to_string(&psk_modes).unwrap_or_else(|_| json_array(&psk_modes)),
        extensions: serde_json::to_string(&extensions).unwrap_or_else(|_| json_array(&extensions)),
        created_at: current
            .as_ref()
            .map(|row| row.created_at.clone())
            .unwrap_or_default(),
        updated_at: current
            .as_ref()
            .map(|row| row.updated_at.clone())
            .unwrap_or_default(),
    })
}

fn parse_array<T: serde::de::DeserializeOwned>(value: &str) -> Vec<T> {
    serde_json::from_str(value).unwrap_or_default()
}

fn unique_name_error(error: sqlx::Error) -> ApiError {
    match error {
        sqlx::Error::Database(ref database) if database.is_unique_violation() => {
            ApiError::bad_request(
                "TLS_FINGERPRINT_NAME_EXISTS",
                "TLS fingerprint profile name already exists",
            )
        }
        other => other.into(),
    }
}

pub async fn client_for_account(state: &AppState, account: &Account) -> ApiResult<Client> {
    client_for_settings(
        state,
        account.row.tls_fingerprint_profile_id,
        account.proxy_url.as_deref(),
    )
    .await
}

pub(crate) async fn client_for_settings(
    state: &AppState,
    profile_id: Option<i64>,
    proxy_url: Option<&str>,
) -> ApiResult<Client> {
    let Some(profile_id) = profile_id else {
        return match proxy_url {
            Some(proxy) => build_http_client(Some(proxy)),
            None => Ok(state.client.clone()),
        };
    };
    let profile = find_profile(state, profile_id).await?;
    let cache_key = format!(
        "{}:{}:{}",
        profile.id,
        profile.updated_at,
        proxy_url.unwrap_or("")
    );
    if let Some(client) = state.tls_clients.lock().await.get(&cache_key).cloned() {
        return Ok(client);
    }
    let client = build_profile_client(&profile, proxy_url)?;
    let mut cache = state.tls_clients.lock().await;
    if cache.len() >= 32 {
        cache.clear();
    }
    cache.insert(cache_key, client.clone());
    Ok(client)
}

fn build_profile_client(profile: &ProfileRow, proxy_url: Option<&str>) -> ApiResult<Client> {
    let mut provider = rustls::crypto::ring::default_provider();
    let cipher_ids = parse_array::<u16>(&profile.cipher_suites);
    if !cipher_ids.is_empty() {
        provider.cipher_suites.sort_by_key(|suite| {
            cipher_ids
                .iter()
                .position(|id| *id == u16::from(suite.suite()))
                .unwrap_or(usize::MAX)
        });
        provider
            .cipher_suites
            .retain(|suite| cipher_ids.contains(&u16::from(suite.suite())));
    }
    let group_ids = parse_array::<u16>(&profile.key_share_groups);
    let curve_ids = parse_array::<u16>(&profile.curves);
    let requested_groups = if group_ids.is_empty() {
        curve_ids
    } else {
        group_ids
    };
    if !requested_groups.is_empty() {
        provider.kx_groups.sort_by_key(|group| {
            requested_groups
                .iter()
                .position(|id| *id == u16::from(group.name()))
                .unwrap_or(usize::MAX)
        });
        provider
            .kx_groups
            .retain(|group| requested_groups.contains(&u16::from(group.name())));
    }
    if provider.cipher_suites.is_empty() || provider.kx_groups.is_empty() {
        return Err(ApiError::bad_request(
            "UNSUPPORTED_TLS_FINGERPRINT",
            "TLS profile does not contain cipher suites and groups supported by rustls",
        ));
    }
    let version_ids = parse_array::<u16>(&profile.supported_versions);
    let mut versions = Vec::new();
    if version_ids.is_empty() || version_ids.contains(&0x0304) {
        versions.push(&rustls::version::TLS13);
    }
    if version_ids.is_empty() || version_ids.contains(&0x0303) {
        versions.push(&rustls::version::TLS12);
    }
    let roots = RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let mut tls = rustls::ClientConfig::builder_with_provider(Arc::new(provider))
        .with_protocol_versions(&versions)
        .map_err(|_| {
            ApiError::bad_request(
                "UNSUPPORTED_TLS_FINGERPRINT",
                "TLS versions are incompatible with cipher suites",
            )
        })?
        .with_root_certificates(roots)
        .with_no_client_auth();
    let alpn = parse_array::<String>(&profile.alpn_protocols);
    if !alpn.is_empty() {
        tls.alpn_protocols = alpn.into_iter().map(String::into_bytes).collect();
    }
    let mut builder = Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(120))
        .pool_idle_timeout(std::time::Duration::from_secs(90))
        .redirect(reqwest::redirect::Policy::none())
        .use_preconfigured_tls(tls);
    if let Some(proxy_url) = proxy_url {
        builder = builder.proxy(
            reqwest::Proxy::all(proxy_url)
                .map_err(|_| ApiError::bad_request("INVALID_PROXY_URL", "proxy URL is invalid"))?,
        );
    }
    builder
        .build()
        .map_err(|_| ApiError::config("failed to initialize TLS fingerprint HTTP client"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn profile_crud_round_trips_arrays() {
        let (_directory, state) = crate::test_support::state().await;
        let profile = normalized_profile(
            ProfileInput {
                name: Some("node profile".into()),
                alpn_protocols: Some(vec!["http/1.1".into()]),
                supported_versions: Some(vec![0x0304, 0x0303]),
                ..Default::default()
            },
            None,
        )
        .unwrap();
        let id = insert_profile(&state, &profile).await.unwrap();
        let stored = find_profile(&state, id).await.unwrap();
        assert_eq!(
            parse_array::<String>(&stored.alpn_protocols),
            vec!["http/1.1"]
        );
        assert!(build_profile_client(&stored, None).is_ok());
    }
}
