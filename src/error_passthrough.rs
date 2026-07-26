use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::FromRow;

use crate::{
    error::{ApiError, ApiResult},
    state::AppState,
};

pub fn admin_router() -> Router<AppState> {
    Router::new()
        .route("/error-passthrough-rules", get(list).post(create))
        .route(
            "/error-passthrough-rules/{id}",
            get(get_one).put(update).delete(delete_rule),
        )
}

#[derive(Clone, FromRow)]
struct RuleRow {
    id: i64,
    name: String,
    enabled: bool,
    priority: i32,
    error_codes: String,
    keywords: String,
    match_mode: String,
    platforms: String,
    passthrough_code: bool,
    response_code: Option<i32>,
    passthrough_body: bool,
    custom_message: Option<String>,
    skip_monitoring: bool,
    description: Option<String>,
    created_at: String,
    updated_at: String,
}

impl RuleRow {
    fn value(&self) -> Value {
        json!({
            "id": self.id,
            "name": self.name,
            "enabled": self.enabled,
            "priority": self.priority,
            "error_codes": parse_array::<i32>(&self.error_codes),
            "keywords": parse_array::<String>(&self.keywords),
            "match_mode": self.match_mode,
            "platforms": parse_array::<String>(&self.platforms),
            "passthrough_code": self.passthrough_code,
            "response_code": self.response_code,
            "passthrough_body": self.passthrough_body,
            "custom_message": self.custom_message,
            "skip_monitoring": self.skip_monitoring,
            "description": self.description,
            "created_at": self.created_at,
            "updated_at": self.updated_at,
        })
    }
}

#[derive(Default, Deserialize)]
struct RuleInput {
    name: Option<String>,
    enabled: Option<bool>,
    priority: Option<i32>,
    error_codes: Option<Vec<i32>>,
    keywords: Option<Vec<String>>,
    match_mode: Option<String>,
    platforms: Option<Vec<String>>,
    passthrough_code: Option<bool>,
    #[serde(default, deserialize_with = "crate::models::deserialize_nullable")]
    response_code: Option<Option<i32>>,
    passthrough_body: Option<bool>,
    #[serde(default, deserialize_with = "crate::models::deserialize_nullable")]
    custom_message: Option<Option<String>>,
    skip_monitoring: Option<bool>,
    #[serde(default, deserialize_with = "crate::models::deserialize_nullable")]
    description: Option<Option<String>>,
}

async fn list(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let rows = sqlx::query_as::<_, RuleRow>(
        "SELECT id, name, enabled, priority, error_codes, keywords, match_mode, platforms, \
         passthrough_code, response_code, passthrough_body, custom_message, skip_monitoring, \
         description, created_at, updated_at FROM error_passthrough_rules ORDER BY priority, id",
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(
        json!({"data": rows.iter().map(RuleRow::value).collect::<Vec<_>>() }),
    ))
}

async fn get_one(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    Ok(Json(json!({"data": find_rule(&state, id).await?.value()})))
}

async fn create(
    State(state): State<AppState>,
    Json(input): Json<RuleInput>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let rule = normalized_rule(input, None)?;
    let result = sqlx::query(
        "INSERT INTO error_passthrough_rules (name, enabled, priority, error_codes, keywords, \
         match_mode, platforms, passthrough_code, response_code, passthrough_body, custom_message, \
         skip_monitoring, description) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&rule.name)
    .bind(rule.enabled)
    .bind(rule.priority)
    .bind(&rule.error_codes)
    .bind(&rule.keywords)
    .bind(&rule.match_mode)
    .bind(&rule.platforms)
    .bind(rule.passthrough_code)
    .bind(rule.response_code)
    .bind(rule.passthrough_body)
    .bind(&rule.custom_message)
    .bind(rule.skip_monitoring)
    .bind(&rule.description)
    .execute(&state.pool)
    .await?;
    let created = find_rule(&state, result.last_insert_rowid()).await?;
    Ok((StatusCode::CREATED, Json(json!({"data": created.value()}))))
}

async fn update(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(input): Json<RuleInput>,
) -> ApiResult<Json<Value>> {
    let current = find_rule(&state, id).await?;
    let rule = normalized_rule(input, Some(current))?;
    sqlx::query(
        "UPDATE error_passthrough_rules SET name = ?, enabled = ?, priority = ?, error_codes = ?, \
         keywords = ?, match_mode = ?, platforms = ?, passthrough_code = ?, response_code = ?, \
         passthrough_body = ?, custom_message = ?, skip_monitoring = ?, description = ?, \
         updated_at = CURRENT_TIMESTAMP WHERE id = ?",
    )
    .bind(&rule.name)
    .bind(rule.enabled)
    .bind(rule.priority)
    .bind(&rule.error_codes)
    .bind(&rule.keywords)
    .bind(&rule.match_mode)
    .bind(&rule.platforms)
    .bind(rule.passthrough_code)
    .bind(rule.response_code)
    .bind(rule.passthrough_body)
    .bind(&rule.custom_message)
    .bind(rule.skip_monitoring)
    .bind(&rule.description)
    .bind(id)
    .execute(&state.pool)
    .await?;
    Ok(Json(json!({"data": find_rule(&state, id).await?.value()})))
}

async fn delete_rule(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    let result = sqlx::query("DELETE FROM error_passthrough_rules WHERE id = ?")
        .bind(id)
        .execute(&state.pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found("error passthrough rule not found"));
    }
    Ok(Json(json!({"data": {"id": id}})))
}

async fn find_rule(state: &AppState, id: i64) -> ApiResult<RuleRow> {
    sqlx::query_as::<_, RuleRow>(
        "SELECT id, name, enabled, priority, error_codes, keywords, match_mode, platforms, \
         passthrough_code, response_code, passthrough_body, custom_message, skip_monitoring, \
         description, created_at, updated_at FROM error_passthrough_rules WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| ApiError::not_found("error passthrough rule not found"))
}

fn normalized_rule(input: RuleInput, current: Option<RuleRow>) -> ApiResult<RuleRow> {
    let name = input
        .name
        .or_else(|| current.as_ref().map(|row| row.name.clone()))
        .unwrap_or_default()
        .trim()
        .to_string();
    let error_codes = input.error_codes.unwrap_or_else(|| {
        current
            .as_ref()
            .map(|row| parse_array(&row.error_codes))
            .unwrap_or_default()
    });
    let keywords = input
        .keywords
        .unwrap_or_else(|| {
            current
                .as_ref()
                .map(|row| parse_array(&row.keywords))
                .unwrap_or_default()
        })
        .into_iter()
        .map(|keyword| keyword.trim().to_string())
        .filter(|keyword| !keyword.is_empty())
        .collect::<Vec<_>>();
    let match_mode = input
        .match_mode
        .or_else(|| current.as_ref().map(|row| row.match_mode.clone()))
        .unwrap_or_else(|| "any".into());
    let passthrough_code = input
        .passthrough_code
        .or_else(|| current.as_ref().map(|row| row.passthrough_code))
        .unwrap_or(true);
    let response_code = input
        .response_code
        .unwrap_or_else(|| current.as_ref().and_then(|row| row.response_code));
    let passthrough_body = input
        .passthrough_body
        .or_else(|| current.as_ref().map(|row| row.passthrough_body))
        .unwrap_or(true);
    let custom_message = input
        .custom_message
        .unwrap_or_else(|| current.as_ref().and_then(|row| row.custom_message.clone()))
        .map(|message| message.trim().to_string())
        .filter(|message| !message.is_empty());
    if name.is_empty()
        || name.chars().count() > 100
        || !matches!(match_mode.as_str(), "any" | "all")
        || (error_codes.is_empty() && keywords.is_empty())
        || error_codes.iter().any(|code| !(100..=599).contains(code))
        || (!passthrough_code && response_code.is_none_or(|code| !(100..=599).contains(&code)))
        || (!passthrough_body && custom_message.is_none())
    {
        return Err(ApiError::bad_request(
            "INVALID_ERROR_PASSTHROUGH_RULE",
            "error passthrough rule is invalid",
        ));
    }
    let platforms = input
        .platforms
        .unwrap_or_else(|| {
            current
                .as_ref()
                .map(|row| parse_array(&row.platforms))
                .unwrap_or_else(|| vec!["openai".into()])
        })
        .into_iter()
        .filter(|platform| platform == "openai")
        .collect::<Vec<_>>();
    Ok(RuleRow {
        id: current.as_ref().map(|row| row.id).unwrap_or_default(),
        name,
        enabled: input
            .enabled
            .or_else(|| current.as_ref().map(|row| row.enabled))
            .unwrap_or(true),
        priority: input
            .priority
            .or_else(|| current.as_ref().map(|row| row.priority))
            .unwrap_or(0),
        error_codes: serde_json::to_string(&error_codes).expect("error codes serialize"),
        keywords: serde_json::to_string(&keywords).expect("keywords serialize"),
        match_mode,
        platforms: serde_json::to_string(&platforms).expect("platforms serialize"),
        passthrough_code,
        response_code,
        passthrough_body,
        custom_message,
        skip_monitoring: input
            .skip_monitoring
            .or_else(|| current.as_ref().map(|row| row.skip_monitoring))
            .unwrap_or(false),
        description: input
            .description
            .unwrap_or_else(|| current.as_ref().and_then(|row| row.description.clone()))
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
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

pub struct PassthroughDecision {
    pub status: StatusCode,
    pub body: Vec<u8>,
    pub skip_monitoring: bool,
}

pub async fn match_response(
    state: &AppState,
    status: StatusCode,
    body: &[u8],
) -> ApiResult<Option<PassthroughDecision>> {
    let rows = sqlx::query_as::<_, RuleRow>(
        "SELECT id, name, enabled, priority, error_codes, keywords, match_mode, platforms, \
         passthrough_code, response_code, passthrough_body, custom_message, skip_monitoring, \
         description, created_at, updated_at FROM error_passthrough_rules \
         WHERE enabled = 1 ORDER BY priority, id",
    )
    .fetch_all(&state.pool)
    .await?;
    let body_text = String::from_utf8_lossy(body).to_lowercase();
    for rule in rows {
        let platforms = parse_array::<String>(&rule.platforms);
        if !platforms.is_empty() && !platforms.iter().any(|platform| platform == "openai") {
            continue;
        }
        let codes = parse_array::<i32>(&rule.error_codes);
        let keywords = parse_array::<String>(&rule.keywords);
        let mut conditions = Vec::new();
        if !codes.is_empty() {
            conditions.push(codes.contains(&(status.as_u16() as i32)));
        }
        if !keywords.is_empty() {
            conditions.push(
                keywords
                    .iter()
                    .any(|keyword| body_text.contains(&keyword.to_lowercase())),
            );
        }
        let matched = if rule.match_mode == "all" {
            conditions.iter().all(|matched| *matched)
        } else {
            conditions.iter().any(|matched| *matched)
        };
        if !matched {
            continue;
        }
        let response_status = if rule.passthrough_code {
            status
        } else {
            StatusCode::from_u16(rule.response_code.unwrap_or(502) as u16)
                .unwrap_or(StatusCode::BAD_GATEWAY)
        };
        let response_body = if rule.passthrough_body {
            body.to_vec()
        } else {
            serde_json::to_vec(&json!({"error": {
                "message": rule.custom_message.unwrap_or_else(|| "upstream request failed".into()),
                "type": "upstream_error",
                "code": "ERROR_PASSTHROUGH_RULE"
            }}))
            .expect("error response serializes")
        };
        return Ok(Some(PassthroughDecision {
            status: response_status,
            body: response_body,
            skip_monitoring: rule.skip_monitoring,
        }));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn matches_status_and_keyword_rules() {
        let (_directory, state) = crate::test_support::state().await;
        sqlx::query(
            "INSERT INTO error_passthrough_rules (name, error_codes, keywords, match_mode, \
             passthrough_code, passthrough_body, custom_message) \
             VALUES ('context', '[400]', '[\"context limit\"]', 'all', 0, 0, 'context rejected')",
        )
        .execute(&state.pool)
        .await
        .unwrap();
        let decision = match_response(
            &state,
            StatusCode::BAD_REQUEST,
            br#"{"error":"Context limit exceeded"}"#,
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(decision.status, StatusCode::BAD_GATEWAY);
        assert!(
            String::from_utf8(decision.body)
                .unwrap()
                .contains("context rejected")
        );
    }
}
