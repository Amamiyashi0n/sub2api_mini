mod account_data;
mod account_tools;
mod admin;
mod admin_users;
mod audit;
mod auth;
mod batch_images;
mod channel_monitor;
mod channels;
mod config;
mod content;
mod crypto;
mod dashboard;
mod db;
mod error;
mod gateway;
mod groups;
mod key_policy;
mod mail;
mod models;
mod oauth;
mod ops;
mod orders;
mod prompt_audit;
mod proxies;
mod public;
mod redeem;
mod risk_control;
mod runtime_log;
mod scheduled_tests;
mod setup;
mod state;
mod subscriptions;
#[cfg(test)]
mod test_support;
mod totp;
mod usage;
mod user;
mod web;

use crate::{config::Config, crypto::Crypto, error::ApiError, state::AppState};
use axum::{
    Json, Router,
    extract::{Query, State},
    response::{IntoResponse, Redirect, Response},
    routing::get,
};
use serde::Deserialize;
use serde_json::json;
use tower_http::{
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    trace::TraceLayer,
};

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let runtime_log_receiver = runtime_log::init();

    let config = Config::from_env()?;
    let pool = db::connect(
        &config.database_path,
        &config.admin_username,
        &config.admin_password,
    )
    .await?;
    let state = AppState::new(pool, Crypto::new(&config.master_key), config.clone())?;
    runtime_log::configure_from_database(&state.pool).await?;
    runtime_log::start_sink(state.pool.clone(), runtime_log_receiver);
    state.load_runtime_settings().await?;
    risk_control::initialize(&state).await?;
    prompt_audit::initialize(&state).await?;
    channel_monitor::start_scheduler(state.clone());
    batch_images::start_scheduler(state.clone());
    ops::start_scheduler(state.clone());
    scheduled_tests::start_scheduler(state.clone());
    subscriptions::start_scheduler(state.clone());

    let app = Router::new()
        .merge(web::router())
        .merge(setup::router())
        .route("/health", get(health))
        .nest("/v1", gateway::router(state.clone()))
        .nest("/api/auth", auth::router(state.clone()))
        .nest("/api/public", public::router(state.clone()))
        .nest("/api/admin", admin::router(state.clone()))
        .nest("/api/user", user::router(state.clone()))
        .fallback(not_found)
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
        .layer(TraceLayer::new_for_http())
        .with_state(state.clone());

    let callback_app = Router::new()
        .route("/auth/callback", get(oauth_callback))
        .route("/health", get(health))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(config.bind).await?;
    let callback_listener = tokio::net::TcpListener::bind(config.callback_bind).await?;
    tracing::info!(bind = %config.bind, "UI and API server listening");
    tracing::info!(bind = %config.callback_bind, "OAuth callback listening");

    let server = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    );
    let callback_server = axum::serve(callback_listener, callback_app);
    tokio::select! {
        result = server => result?,
        result = callback_server => result?,
        _ = shutdown_signal() => tracing::info!("shutdown requested"),
    }
    Ok(())
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({"status":"ok", "service":"sub2api_mini", "version":env!("CARGO_PKG_VERSION")}))
}

async fn not_found() -> ApiError {
    ApiError::not_found("route not found")
}

#[derive(Deserialize)]
struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

async fn oauth_callback(
    State(state): State<AppState>,
    Query(query): Query<CallbackQuery>,
) -> Response {
    let target = if let Some(error) = query.error {
        tracing::warn!(%error, description = ?query.error_description, "OAuth callback rejected");
        format!(
            "{}/#/accounts?oauth=error",
            state.config.public_ui_url.trim_end_matches('/')
        )
    } else {
        match (query.code, query.state) {
            (Some(code), Some(flow_state)) => {
                match oauth::complete_flow(&state, &code, &flow_state).await {
                    Ok(completed) => format!(
                        "{}/#/accounts?oauth={}&account_id={}",
                        state.config.public_ui_url.trim_end_matches('/'),
                        if completed.reauthorized {
                            "reauthorized"
                        } else {
                            "success"
                        },
                        completed.account_id
                    ),
                    Err(error) => {
                        tracing::warn!(%error, "OAuth callback failed");
                        format!(
                            "{}/#/accounts?oauth=error",
                            state.config.public_ui_url.trim_end_matches('/')
                        )
                    }
                }
            }
            _ => format!(
                "{}/#/accounts?oauth=error",
                state.config.public_ui_url.trim_end_matches('/')
            ),
        }
    };
    Redirect::temporary(&target).into_response()
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("install Ctrl+C handler");
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

#[cfg(test)]
mod tests {
    use axum::{
        Router,
        body::{Body, to_bytes},
        http::{Request, StatusCode, header},
    };
    use tower::ServiceExt;

    use super::{not_found, web};

    #[tokio::test]
    async fn unknown_static_path_returns_json_not_found() {
        let app = Router::<()>::new().merge(web::router()).fallback(not_found);
        for path in ["/missing.txt", "/identity.js"] {
            let response = app
                .clone()
                .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
                .await
                .unwrap();

            assert_eq!(response.status(), StatusCode::NOT_FOUND);
            assert_eq!(
                response.headers().get(header::CONTENT_TYPE).unwrap(),
                "application/json"
            );
            let body = to_bytes(response.into_body(), 4096).await.unwrap();
            assert!(String::from_utf8_lossy(&body).contains("NOT_FOUND"));
        }
    }
}
