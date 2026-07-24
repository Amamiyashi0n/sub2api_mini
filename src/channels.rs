use std::collections::{HashMap, HashSet};

use axum::{
    Json, Router,
    extract::{Extension, Path, State},
    http::StatusCode,
    routing::get,
};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::FromRow;

use crate::{
    auth::AuthSession,
    error::{ApiError, ApiResult},
    groups,
    state::AppState,
};

pub fn admin_router() -> Router<AppState> {
    Router::new()
        .route("/channels", get(list).post(create))
        .route(
            "/channels/{id}",
            get(get_channel).put(update).delete(delete),
        )
}

pub fn user_router() -> Router<AppState> {
    Router::new().route("/channels/available", get(available))
}

#[derive(Debug, Clone, Deserialize)]
struct PricingIntervalInput {
    #[serde(default)]
    min_tokens: i64,
    max_tokens: Option<i64>,
    input_price: Option<f64>,
    output_price: Option<f64>,
    cache_read_price: Option<f64>,
    cache_write_price: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
struct PricingInput {
    #[serde(default = "default_platform")]
    platform: String,
    models: Vec<String>,
    #[serde(default = "default_billing_mode")]
    billing_mode: String,
    #[serde(default)]
    input_price: f64,
    #[serde(default)]
    output_price: f64,
    #[serde(default)]
    per_request_price: f64,
    cache_read_price: Option<f64>,
    cache_write_price: Option<f64>,
    image_input_price: Option<f64>,
    image_output_price: Option<f64>,
    #[serde(default)]
    intervals: Vec<PricingIntervalInput>,
}

#[derive(Debug, Clone, Deserialize)]
struct AccountStatsPricingRuleInput {
    name: String,
    #[serde(default)]
    group_ids: Vec<i64>,
    #[serde(default)]
    account_ids: Vec<i64>,
    #[serde(default)]
    pricing: Vec<PricingInput>,
}

fn default_platform() -> String {
    "openai".into()
}
fn default_billing_mode() -> String {
    "tokens".into()
}

#[derive(Debug, Deserialize)]
struct ChannelInput {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default = "active_status")]
    status: String,
    #[serde(default)]
    restrict_models: bool,
    #[serde(default)]
    model_mapping: HashMap<String, HashMap<String, String>>,
    #[serde(default = "default_billing_model_source")]
    billing_model_source: String,
    #[serde(default)]
    group_ids: Vec<i64>,
    #[serde(default)]
    model_pricing: Vec<PricingInput>,
    #[serde(default)]
    apply_pricing_to_account_stats: bool,
    #[serde(default)]
    account_stats_pricing_rules: Vec<AccountStatsPricingRuleInput>,
}

fn active_status() -> String {
    "active".into()
}

fn default_billing_model_source() -> String {
    "channel_mapped".into()
}

async fn list(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM channels ORDER BY id DESC")
        .fetch_all(&state.pool)
        .await?;
    let mut data = Vec::with_capacity(ids.len());
    for id in ids {
        data.push(channel_view(&state, id).await?);
    }
    Ok(Json(json!({"data": data})))
}

async fn get_channel(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    Ok(Json(json!({"data": channel_view(&state, id).await?})))
}

async fn create(
    State(state): State<AppState>,
    Json(input): Json<ChannelInput>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    validate_channel(&input)?;
    let mut tx = state.pool.begin().await?;
    let result = sqlx::query(
        "INSERT INTO channels (name, description, status, restrict_models, model_mapping, \
         billing_model_source, apply_pricing_to_account_stats) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(input.name.trim())
    .bind(input.description.trim())
    .bind(&input.status)
    .bind(input.restrict_models)
    .bind(serde_json::to_string(&input.model_mapping).unwrap())
    .bind(&input.billing_model_source)
    .bind(input.apply_pricing_to_account_stats)
    .execute(&mut *tx)
    .await?;
    let id = result.last_insert_rowid();
    replace_children(
        &mut tx,
        id,
        &input.group_ids,
        &input.model_pricing,
        &input.account_stats_pricing_rules,
    )
    .await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"data": channel_view(&state, id).await?})),
    ))
}

async fn update(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(input): Json<ChannelInput>,
) -> ApiResult<Json<Value>> {
    validate_channel(&input)?;
    let mut tx = state.pool.begin().await?;
    let result = sqlx::query(
        "UPDATE channels SET name = ?, description = ?, status = ?, restrict_models = ?, \
         model_mapping = ?, billing_model_source = ?, apply_pricing_to_account_stats = ?, \
         updated_at = CURRENT_TIMESTAMP WHERE id = ?",
    )
    .bind(input.name.trim())
    .bind(input.description.trim())
    .bind(&input.status)
    .bind(input.restrict_models)
    .bind(serde_json::to_string(&input.model_mapping).unwrap())
    .bind(&input.billing_model_source)
    .bind(input.apply_pricing_to_account_stats)
    .bind(id)
    .execute(&mut *tx)
    .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found("channel not found"));
    }
    replace_children(
        &mut tx,
        id,
        &input.group_ids,
        &input.model_pricing,
        &input.account_stats_pricing_rules,
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"data": channel_view(&state, id).await?})))
}

async fn replace_children(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    channel_id: i64,
    group_ids: &[i64],
    pricing: &[PricingInput],
    account_stats_rules: &[AccountStatsPricingRuleInput],
) -> ApiResult<()> {
    sqlx::query("DELETE FROM channel_groups WHERE channel_id = ?")
        .bind(channel_id)
        .execute(&mut **tx)
        .await?;
    for group_id in group_ids {
        let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM groups WHERE id = ?")
            .bind(group_id)
            .fetch_one(&mut **tx)
            .await?;
        if exists == 0 {
            return Err(ApiError::bad_request(
                "INVALID_GROUP",
                "channel group does not exist",
            ));
        }
        sqlx::query("INSERT INTO channel_groups (channel_id, group_id) VALUES (?, ?)")
            .bind(channel_id)
            .bind(group_id)
            .execute(&mut **tx)
            .await
            .map_err(|error| match error {
                sqlx::Error::Database(ref db) if db.is_unique_violation() => ApiError::bad_request(
                    "GROUP_ALREADY_ASSIGNED",
                    "group belongs to another channel",
                ),
                other => other.into(),
            })?;
    }
    sqlx::query("DELETE FROM channel_model_pricing WHERE channel_id = ?")
        .bind(channel_id)
        .execute(&mut **tx)
        .await?;
    for item in pricing {
        let models = normalize_models(&item.models);
        let result = sqlx::query(
            "INSERT INTO channel_model_pricing (channel_id, platform, models, billing_mode, \
             input_microusd_per_million, output_microusd_per_million, per_request_microusd, \
             cache_read_microusd_per_million, cache_write_microusd_per_million, \
             image_input_microusd_per_million, image_output_microusd_per_million) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(channel_id)
        .bind(item.platform.trim())
        .bind(
            serde_json::to_string(&models)
                .map_err(|_| ApiError::internal("pricing serialization failed"))?,
        )
        .bind(&item.billing_mode)
        .bind(price_to_microusd(item.input_price)?)
        .bind(price_to_microusd(item.output_price)?)
        .bind(price_to_microusd(item.per_request_price)?)
        .bind(optional_price_to_microusd(item.cache_read_price)?)
        .bind(optional_price_to_microusd(item.cache_write_price)?)
        .bind(optional_price_to_microusd(item.image_input_price)?)
        .bind(optional_price_to_microusd(item.image_output_price)?)
        .execute(&mut **tx)
        .await?;
        let pricing_id = result.last_insert_rowid();
        let mut intervals = item.intervals.clone();
        intervals.sort_by_key(|interval| interval.min_tokens);
        for (sort_order, interval) in intervals.iter().enumerate() {
            sqlx::query(
                "INSERT INTO channel_pricing_intervals (pricing_id, min_tokens, max_tokens, \
                 input_microusd_per_million, output_microusd_per_million, \
                 cache_read_microusd_per_million, cache_write_microusd_per_million, sort_order) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(pricing_id)
            .bind(interval.min_tokens)
            .bind(interval.max_tokens)
            .bind(optional_price_to_microusd(interval.input_price)?)
            .bind(optional_price_to_microusd(interval.output_price)?)
            .bind(optional_price_to_microusd(interval.cache_read_price)?)
            .bind(optional_price_to_microusd(interval.cache_write_price)?)
            .bind(sort_order as i64)
            .execute(&mut **tx)
            .await?;
        }
    }
    replace_account_stats_rules(tx, channel_id, group_ids, account_stats_rules).await?;
    Ok(())
}

async fn replace_account_stats_rules(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    channel_id: i64,
    channel_group_ids: &[i64],
    rules: &[AccountStatsPricingRuleInput],
) -> ApiResult<()> {
    sqlx::query("DELETE FROM channel_account_stats_rules WHERE channel_id = ?")
        .bind(channel_id)
        .execute(&mut **tx)
        .await?;
    for (sort_order, rule) in rules.iter().enumerate() {
        let rule_id = sqlx::query(
            "INSERT INTO channel_account_stats_rules (channel_id, name, sort_order) \
             VALUES (?, ?, ?)",
        )
        .bind(channel_id)
        .bind(rule.name.trim())
        .bind(sort_order as i64)
        .execute(&mut **tx)
        .await?
        .last_insert_rowid();
        for group_id in &rule.group_ids {
            if !channel_group_ids.contains(group_id) {
                return Err(ApiError::bad_request(
                    "ACCOUNT_STATS_GROUP_NOT_IN_CHANNEL",
                    "account stats pricing groups must belong to the channel",
                ));
            }
            sqlx::query(
                "INSERT INTO channel_account_stats_rule_groups (rule_id, group_id) VALUES (?, ?)",
            )
            .bind(rule_id)
            .bind(group_id)
            .execute(&mut **tx)
            .await?;
        }
        for account_id in &rule.account_ids {
            let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM accounts WHERE id = ?")
                .bind(account_id)
                .fetch_one(&mut **tx)
                .await?;
            if exists == 0 {
                return Err(ApiError::bad_request(
                    "INVALID_ACCOUNT_STATS_ACCOUNT",
                    "account stats pricing account does not exist",
                ));
            }
            sqlx::query(
                "INSERT INTO channel_account_stats_rule_accounts (rule_id, account_id) \
                 VALUES (?, ?)",
            )
            .bind(rule_id)
            .bind(account_id)
            .execute(&mut **tx)
            .await?;
        }
        for item in &rule.pricing {
            let pricing_id = sqlx::query(
                "INSERT INTO channel_account_stats_pricing \
                 (rule_id, platform, models, billing_mode, input_microusd_per_million, \
                  output_microusd_per_million, per_request_microusd, \
                  cache_read_microusd_per_million, cache_write_microusd_per_million, \
                  image_input_microusd_per_million, image_output_microusd_per_million) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(rule_id)
            .bind(item.platform.trim())
            .bind(
                serde_json::to_string(&normalize_models(&item.models))
                    .map_err(|_| ApiError::internal("pricing serialization failed"))?,
            )
            .bind(&item.billing_mode)
            .bind(price_to_microusd(item.input_price)?)
            .bind(price_to_microusd(item.output_price)?)
            .bind(price_to_microusd(item.per_request_price)?)
            .bind(optional_price_to_microusd(item.cache_read_price)?)
            .bind(optional_price_to_microusd(item.cache_write_price)?)
            .bind(optional_price_to_microusd(item.image_input_price)?)
            .bind(optional_price_to_microusd(item.image_output_price)?)
            .execute(&mut **tx)
            .await?
            .last_insert_rowid();
            let mut intervals = item.intervals.clone();
            intervals.sort_by_key(|interval| interval.min_tokens);
            for (interval_order, interval) in intervals.iter().enumerate() {
                sqlx::query(
                    "INSERT INTO channel_account_stats_intervals \
                     (pricing_id, min_tokens, max_tokens, input_microusd_per_million, \
                      output_microusd_per_million, cache_read_microusd_per_million, \
                      cache_write_microusd_per_million, sort_order) \
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(pricing_id)
                .bind(interval.min_tokens)
                .bind(interval.max_tokens)
                .bind(optional_price_to_microusd(interval.input_price)?)
                .bind(optional_price_to_microusd(interval.output_price)?)
                .bind(optional_price_to_microusd(interval.cache_read_price)?)
                .bind(optional_price_to_microusd(interval.cache_write_price)?)
                .bind(interval_order as i64)
                .execute(&mut **tx)
                .await?;
            }
        }
    }
    Ok(())
}

async fn delete(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    let result = sqlx::query("DELETE FROM channels WHERE id = ?")
        .bind(id)
        .execute(&state.pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found("channel not found"));
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn channel_view(state: &AppState, id: i64) -> ApiResult<Value> {
    let row: (
        i64,
        String,
        String,
        String,
        bool,
        String,
        String,
        bool,
        String,
        String,
    ) = sqlx::query_as(
        "SELECT id, name, description, status, restrict_models, model_mapping, \
         billing_model_source, apply_pricing_to_account_stats, created_at, updated_at \
         FROM channels WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| ApiError::not_found("channel not found"))?;
    let group_ids: Vec<i64> = sqlx::query_scalar(
        "SELECT group_id FROM channel_groups WHERE channel_id = ? ORDER BY group_id",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await?;
    let pricing = pricing_view(state, id).await?;
    let account_stats_rules = account_stats_rules_view(state, id).await?;
    let model_mapping = serde_json::from_str::<HashMap<String, HashMap<String, String>>>(&row.5)
        .map_err(|_| ApiError::internal("stored channel model mapping is malformed"))?;
    Ok(
        json!({"id": row.0, "name": row.1, "description": row.2, "status": row.3,
        "restrict_models": row.4, "model_mapping": model_mapping,
        "billing_model_source": row.6, "group_ids": group_ids, "model_pricing": pricing,
        "apply_pricing_to_account_stats": row.7,
        "account_stats_pricing_rules": account_stats_rules,
        "created_at": row.8, "updated_at": row.9}),
    )
}

async fn pricing_view(state: &AppState, channel_id: i64) -> ApiResult<Vec<Value>> {
    let rows: Vec<(
        i64,
        String,
        String,
        String,
        i64,
        i64,
        i64,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
    )> = sqlx::query_as(
        "SELECT id, platform, models, billing_mode, input_microusd_per_million, \
         output_microusd_per_million, per_request_microusd, \
         cache_read_microusd_per_million, cache_write_microusd_per_million, \
         image_input_microusd_per_million, image_output_microusd_per_million \
         FROM channel_model_pricing \
         WHERE channel_id = ? ORDER BY id",
    )
    .bind(channel_id)
    .fetch_all(&state.pool)
    .await?;
    let mut pricing = Vec::with_capacity(rows.len());
    for row in rows {
        let models: Vec<String> = serde_json::from_str(&row.2)
            .map_err(|_| ApiError::internal("stored pricing models are malformed"))?;
        let intervals: Vec<(
            i64,
            Option<i64>,
            Option<i64>,
            Option<i64>,
            Option<i64>,
            Option<i64>,
        )> = sqlx::query_as(
            "SELECT min_tokens, max_tokens, input_microusd_per_million, \
             output_microusd_per_million, cache_read_microusd_per_million, \
             cache_write_microusd_per_million FROM channel_pricing_intervals \
             WHERE pricing_id = ? ORDER BY min_tokens, sort_order, id",
        )
        .bind(row.0)
        .fetch_all(&state.pool)
        .await?;
        pricing.push(json!({
            "id": row.0, "platform": row.1, "models": models,
            "billing_mode": row.3, "input_price": microusd_to_price(row.4),
            "output_price": microusd_to_price(row.5), "per_request_price": microusd_to_price(row.6),
            "cache_read_price": row.7.map(microusd_to_price),
            "cache_write_price": row.8.map(microusd_to_price),
            "image_input_price": row.9.map(microusd_to_price),
            "image_output_price": row.10.map(microusd_to_price),
            "intervals": intervals.into_iter().map(|interval| json!({
                "min_tokens": interval.0, "max_tokens": interval.1,
                "input_price": interval.2.map(microusd_to_price),
                "output_price": interval.3.map(microusd_to_price),
                "cache_read_price": interval.4.map(microusd_to_price),
                "cache_write_price": interval.5.map(microusd_to_price)
            })).collect::<Vec<_>>()
        }));
    }
    Ok(pricing)
}

async fn account_stats_rules_view(state: &AppState, channel_id: i64) -> ApiResult<Vec<Value>> {
    let rules: Vec<(i64, String)> = sqlx::query_as(
        "SELECT id, name FROM channel_account_stats_rules \
         WHERE channel_id = ? ORDER BY sort_order, id",
    )
    .bind(channel_id)
    .fetch_all(&state.pool)
    .await?;
    let mut result = Vec::with_capacity(rules.len());
    for (rule_id, name) in rules {
        let group_ids: Vec<i64> = sqlx::query_scalar(
            "SELECT group_id FROM channel_account_stats_rule_groups \
             WHERE rule_id = ? ORDER BY group_id",
        )
        .bind(rule_id)
        .fetch_all(&state.pool)
        .await?;
        let account_ids: Vec<i64> = sqlx::query_scalar(
            "SELECT account_id FROM channel_account_stats_rule_accounts \
             WHERE rule_id = ? ORDER BY account_id",
        )
        .bind(rule_id)
        .fetch_all(&state.pool)
        .await?;
        let rows: Vec<(
            i64,
            String,
            String,
            String,
            i64,
            i64,
            i64,
            Option<i64>,
            Option<i64>,
            Option<i64>,
            Option<i64>,
        )> = sqlx::query_as(
            "SELECT id, platform, models, billing_mode, input_microusd_per_million, \
             output_microusd_per_million, per_request_microusd, \
             cache_read_microusd_per_million, cache_write_microusd_per_million, \
             image_input_microusd_per_million, image_output_microusd_per_million \
             FROM channel_account_stats_pricing WHERE rule_id = ? ORDER BY id",
        )
        .bind(rule_id)
        .fetch_all(&state.pool)
        .await?;
        let mut pricing = Vec::with_capacity(rows.len());
        for row in rows {
            let models: Vec<String> = serde_json::from_str(&row.2)
                .map_err(|_| ApiError::internal("stored pricing models are malformed"))?;
            let intervals: Vec<(
                i64,
                Option<i64>,
                Option<i64>,
                Option<i64>,
                Option<i64>,
                Option<i64>,
            )> = sqlx::query_as(
                "SELECT min_tokens, max_tokens, input_microusd_per_million, \
                 output_microusd_per_million, cache_read_microusd_per_million, \
                 cache_write_microusd_per_million FROM channel_account_stats_intervals \
                 WHERE pricing_id = ? ORDER BY min_tokens, sort_order, id",
            )
            .bind(row.0)
            .fetch_all(&state.pool)
            .await?;
            pricing.push(json!({
                "id": row.0, "platform": row.1, "models": models,
                "billing_mode": row.3, "input_price": microusd_to_price(row.4),
                "output_price": microusd_to_price(row.5),
                "per_request_price": microusd_to_price(row.6),
                "cache_read_price": row.7.map(microusd_to_price),
                "cache_write_price": row.8.map(microusd_to_price),
                "image_input_price": row.9.map(microusd_to_price),
                "image_output_price": row.10.map(microusd_to_price),
                "intervals": intervals.into_iter().map(|interval| json!({
                    "min_tokens": interval.0, "max_tokens": interval.1,
                    "input_price": interval.2.map(microusd_to_price),
                    "output_price": interval.3.map(microusd_to_price),
                    "cache_read_price": interval.4.map(microusd_to_price),
                    "cache_write_price": interval.5.map(microusd_to_price)
                })).collect::<Vec<_>>()
            }));
        }
        result.push(json!({
            "id": rule_id, "name": name, "group_ids": group_ids,
            "account_ids": account_ids, "pricing": pricing
        }));
    }
    Ok(result)
}

#[derive(FromRow)]
struct AvailableGroupRow {
    id: i64,
    name: String,
    description: String,
    allowed_models: String,
    platform: String,
    is_exclusive: bool,
    subscription_type: String,
    rate_multiplier_micros: i64,
    peak_rate_enabled: bool,
    peak_start: String,
    peak_end: String,
    peak_rate_multiplier_micros: i64,
    user_rate_multiplier_micros: Option<i64>,
}

async fn available(
    State(state): State<AppState>,
    Extension(session): Extension<AuthSession>,
) -> ApiResult<Json<Value>> {
    let ids: Vec<i64> =
        sqlx::query_scalar("SELECT id FROM channels WHERE status = 'active' ORDER BY id")
            .fetch_all(&state.pool)
            .await?;
    let mut channels = Vec::with_capacity(ids.len());
    let now = Utc::now();
    let (offset_minutes, offset_label) = groups::server_utc_offset();
    for id in ids {
        let channel = channel_view(&state, id).await?;
        let group_rows = sqlx::query_as::<_, AvailableGroupRow>(
            "SELECT groups.id, groups.name, groups.description, groups.allowed_models, \
             groups.platform, groups.is_exclusive, groups.subscription_type, \
             groups.rate_multiplier_micros, groups.peak_rate_enabled, groups.peak_start, \
             groups.peak_end, groups.peak_rate_multiplier_micros, \
             rates.rate_multiplier_micros AS user_rate_multiplier_micros FROM groups \
             JOIN channel_groups ON channel_groups.group_id = groups.id \
             JOIN users viewer ON viewer.id = ? \
             LEFT JOIN user_group_rate_multipliers rates ON rates.group_id = groups.id \
             AND rates.user_id = viewer.id WHERE channel_groups.channel_id = ? \
             AND groups.enabled = 1 AND (viewer.role = 'admin' OR \
               (groups.subscription_type = 'subscription' AND EXISTS (SELECT 1 FROM subscriptions \
                 WHERE subscriptions.user_id = viewer.id AND subscriptions.group_id = groups.id \
                 AND subscriptions.status = 'active' \
                 AND datetime(subscriptions.ends_at) > CURRENT_TIMESTAMP)) OR \
               (groups.subscription_type = 'standard' AND ( \
                 (groups.is_exclusive = 0 AND viewer.allow_all_standard_groups = 1) OR \
                 EXISTS (SELECT 1 FROM user_allowed_groups access \
                   WHERE access.user_id = viewer.id AND access.group_id = groups.id)))) \
             ORDER BY groups.sort_order, groups.id",
        )
        .bind(session.user_id)
        .bind(id)
        .fetch_all(&state.pool)
        .await?;
        let group_values = group_rows
            .into_iter()
            .map(|row| {
                let (applied_peak, effective) = groups::effective_rate_micros_at(
                    row.rate_multiplier_micros,
                    row.user_rate_multiplier_micros,
                    &row.subscription_type,
                    row.peak_rate_enabled,
                    &row.peak_start,
                    &row.peak_end,
                    row.peak_rate_multiplier_micros,
                    now,
                    offset_minutes,
                );
                json!({
                    "id": row.id, "name": row.name, "description": row.description,
                    "allowed_models": serde_json::from_str::<Vec<String>>(&row.allowed_models).unwrap_or_default(),
                    "platform": row.platform, "platform_label": groups::platform_label(&row.platform),
                    "platform_category": groups::platform_category(&row.platform),
                    "is_exclusive": row.is_exclusive, "subscription_type": row.subscription_type,
                    "rate_multiplier": groups::micros_to_multiplier(row.rate_multiplier_micros),
                    "user_rate_multiplier": row.user_rate_multiplier_micros.map(groups::micros_to_multiplier),
                    "resolved_rate_multiplier": groups::micros_to_multiplier(
                        row.user_rate_multiplier_micros.unwrap_or(row.rate_multiplier_micros)),
                    "peak_rate_enabled": row.peak_rate_enabled, "peak_start": row.peak_start,
                    "peak_end": row.peak_end,
                    "peak_rate_multiplier": groups::micros_to_multiplier(row.peak_rate_multiplier_micros),
                    "applied_peak_multiplier": groups::micros_to_multiplier(applied_peak),
                    "effective_rate_multiplier": groups::micros_to_multiplier(effective)
                })
            })
            .collect::<Vec<_>>();
        let pricing = channel["model_pricing"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let mut platform_ids = Vec::<String>::new();
        for rule in &pricing {
            if let Some(platform) = rule.get("platform").and_then(Value::as_str)
                && !platform_ids.iter().any(|item| item == platform)
            {
                platform_ids.push(platform.to_string());
            }
        }
        for group in &group_values {
            if let Some(platform) = group.get("platform").and_then(Value::as_str)
                && !platform_ids.iter().any(|item| item == platform)
            {
                platform_ids.push(platform.to_string());
            }
        }
        let platforms = platform_ids
            .into_iter()
            .map(|platform| {
                let platform_groups = group_values
                    .iter()
                    .filter(|group| {
                        group.get("platform").and_then(Value::as_str) == Some(&platform)
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                let mut supported_models = Vec::new();
                for rule in pricing
                    .iter()
                    .filter(|rule| rule.get("platform").and_then(Value::as_str) == Some(&platform))
                {
                    for model in rule
                        .get("models")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                    {
                        if !supported_models.iter().any(|item: &Value| {
                            item.get("name").and_then(Value::as_str) == Some(model)
                        }) {
                            supported_models.push(json!({
                                "name": model, "platform": platform, "pricing": rule
                            }));
                        }
                    }
                }
                json!({
                    "platform": platform,
                    "platform_label": groups::platform_label(&platform),
                    "platform_category": groups::platform_category(&platform),
                    "model_count": supported_models.len(),
                    "group_count": platform_groups.len(),
                    "groups": platform_groups,
                    "supported_models": supported_models
                })
            })
            .collect::<Vec<_>>();
        let platform_count = platforms.len();
        channels.push(json!({
            "id": id, "name": channel["name"], "description": channel["description"],
            "groups": group_values, "model_pricing": pricing, "platforms": platforms,
            "model_mapping": channel["model_mapping"],
            "billing_model_source": channel["billing_model_source"],
            "platform_count": platform_count, "server_utc_offset": offset_label,
            "observed_at": now.to_rfc3339()
        }));
    }
    Ok(Json(json!({"data": channels})))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ModelResolution {
    pub requested: String,
    pub mapped: String,
    pub billing: String,
    pub billing_source: String,
    pub mapping_chain: String,
}

pub(crate) async fn resolve_model(
    state: &AppState,
    group_id: Option<i64>,
    requested: &str,
) -> ApiResult<ModelResolution> {
    let Some(group_id) = group_id else {
        return Ok(unmapped_resolution(requested));
    };
    let row: Option<(String, String, String)> = sqlx::query_as(
        "SELECT channels.model_mapping, channels.billing_model_source, groups.platform \
         FROM channels JOIN channel_groups ON channel_groups.channel_id = channels.id \
         JOIN groups ON groups.id = channel_groups.group_id \
         WHERE channel_groups.group_id = ? AND channels.status = 'active'",
    )
    .bind(group_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some((stored, billing_source, platform)) = row else {
        return Ok(unmapped_resolution(requested));
    };
    let mapping = serde_json::from_str::<HashMap<String, HashMap<String, String>>>(&stored)
        .map_err(|_| ApiError::internal("stored channel model mapping is malformed"))?;
    let rules = mapping
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(&platform))
        .map(|(_, rules)| rules);
    let mapped = rules
        .and_then(|rules| resolve_mapping_rule(rules, requested))
        .unwrap_or(requested)
        .to_string();
    let billing = if billing_source == "requested" {
        requested.to_string()
    } else {
        mapped.clone()
    };
    Ok(ModelResolution {
        requested: requested.to_string(),
        mapping_chain: (mapped != requested)
            .then(|| format!("{requested}->{mapped}"))
            .unwrap_or_default(),
        mapped,
        billing,
        billing_source,
    })
}

fn unmapped_resolution(requested: &str) -> ModelResolution {
    ModelResolution {
        requested: requested.to_string(),
        mapped: requested.to_string(),
        billing: requested.to_string(),
        billing_source: "requested".into(),
        mapping_chain: String::new(),
    }
}

fn resolve_mapping_rule<'a>(
    rules: &'a HashMap<String, String>,
    requested: &str,
) -> Option<&'a str> {
    if let Some((_, target)) = rules
        .iter()
        .find(|(source, _)| source.eq_ignore_ascii_case(requested))
    {
        return Some(target.trim());
    }
    let requested = requested.to_ascii_lowercase();
    rules
        .iter()
        .filter_map(|(source, target)| {
            let prefix = source.trim().strip_suffix('*')?;
            requested
                .starts_with(&prefix.to_ascii_lowercase())
                .then_some((prefix.len(), target.trim()))
        })
        .max_by_key(|(length, _)| *length)
        .map(|(_, target)| target)
}

fn validate_channel(input: &ChannelInput) -> ApiResult<()> {
    if input.name.trim().is_empty()
        || input.name.chars().count() > 100
        || !matches!(input.status.as_str(), "active" | "inactive")
        || !matches!(
            input.billing_model_source.as_str(),
            "requested" | "upstream" | "channel_mapped"
        )
        || input.model_pricing.len() > 100
        || input.account_stats_pricing_rules.len() > 50
    {
        return Err(ApiError::bad_request(
            "INVALID_CHANNEL",
            "channel settings are invalid",
        ));
    }
    validate_pricing_entries(&input.model_pricing)?;
    for rule in &input.account_stats_pricing_rules {
        if rule.name.trim().is_empty()
            || rule.name.chars().count() > 100
            || (rule.group_ids.is_empty() && rule.account_ids.is_empty())
            || rule.group_ids.len() > 100
            || rule.account_ids.len() > 100
            || rule.pricing.is_empty()
            || rule.pricing.len() > 100
            || has_duplicate_ids(&rule.group_ids)
            || has_duplicate_ids(&rule.account_ids)
        {
            return Err(ApiError::bad_request(
                "INVALID_ACCOUNT_STATS_PRICING_RULE",
                "account stats pricing rules require a name, scope, and pricing",
            ));
        }
        validate_pricing_entries(&rule.pricing)?;
    }
    validate_model_mapping(&input.model_mapping)?;
    Ok(())
}

fn validate_pricing_entries(pricing: &[PricingInput]) -> ApiResult<()> {
    let mut model_patterns = Vec::<(String, String)>::new();
    for item in pricing {
        if item.platform.trim().is_empty()
            || !matches!(item.billing_mode.as_str(), "tokens" | "request")
            || normalize_models(&item.models).is_empty()
        {
            return Err(ApiError::bad_request(
                "INVALID_PRICING",
                "pricing rule is invalid",
            ));
        }
        price_to_microusd(item.input_price)?;
        price_to_microusd(item.output_price)?;
        price_to_microusd(item.per_request_price)?;
        optional_price_to_microusd(item.cache_read_price)?;
        optional_price_to_microusd(item.cache_write_price)?;
        optional_price_to_microusd(item.image_input_price)?;
        optional_price_to_microusd(item.image_output_price)?;
        validate_intervals(&item.intervals, &item.billing_mode)?;
        for model in normalize_models(&item.models) {
            let platform = item.platform.trim().to_ascii_lowercase();
            if model_patterns.iter().any(|(existing_platform, existing)| {
                *existing_platform == platform && model_patterns_conflict(existing, &model)
            }) {
                return Err(ApiError::bad_request(
                    "PRICING_MODEL_CONFLICT",
                    "pricing model patterns overlap within a platform",
                ));
            }
            model_patterns.push((platform, model));
        }
    }
    Ok(())
}

fn has_duplicate_ids(values: &[i64]) -> bool {
    let mut seen = HashSet::with_capacity(values.len());
    values
        .iter()
        .any(|value| *value <= 0 || !seen.insert(*value))
}

fn model_patterns_conflict(left: &str, right: &str) -> bool {
    let left = left.to_ascii_lowercase();
    let right = right.to_ascii_lowercase();
    let left_wildcard = left.ends_with('*');
    let right_wildcard = right.ends_with('*');
    let left = left.strip_suffix('*').unwrap_or(&left);
    let right = right.strip_suffix('*').unwrap_or(&right);
    match (left_wildcard, right_wildcard) {
        (false, false) => left == right,
        (true, false) => right.starts_with(left),
        (false, true) => left.starts_with(right),
        (true, true) => left.starts_with(right) || right.starts_with(left),
    }
}

fn validate_model_mapping(mapping: &HashMap<String, HashMap<String, String>>) -> ApiResult<()> {
    if mapping.len() > 16 || mapping.values().map(HashMap::len).sum::<usize>() > 200 {
        return Err(ApiError::bad_request(
            "INVALID_MODEL_MAPPING",
            "model mapping supports at most 16 platforms and 200 rules",
        ));
    }
    for (platform, rules) in mapping {
        if platform.trim().is_empty()
            || platform.chars().count() > 32
            || rules.iter().any(|(source, target)| {
                let source = source.trim();
                let target = target.trim();
                source.is_empty()
                    || target.is_empty()
                    || source.chars().count() > 128
                    || target.chars().count() > 128
                    || source.strip_suffix('*').unwrap_or(source).contains('*')
            })
        {
            return Err(ApiError::bad_request(
                "INVALID_MODEL_MAPPING",
                "model mapping contains an invalid platform, source, or target",
            ));
        }
    }
    Ok(())
}

fn normalize_models(values: &[String]) -> Vec<String> {
    let mut result = Vec::new();
    for value in values {
        let value = value.trim().to_string();
        if !value.is_empty() && !result.contains(&value) {
            result.push(value);
        }
    }
    result
}

fn price_to_microusd(value: f64) -> ApiResult<i64> {
    if !value.is_finite() || !(0.0..=1_000_000.0).contains(&value) {
        return Err(ApiError::bad_request(
            "INVALID_PRICE",
            "price is outside the supported range",
        ));
    }
    Ok((value * 1_000_000.0).round() as i64)
}

fn optional_price_to_microusd(value: Option<f64>) -> ApiResult<Option<i64>> {
    value.map(price_to_microusd).transpose()
}

fn validate_intervals(intervals: &[PricingIntervalInput], billing_mode: &str) -> ApiResult<()> {
    if intervals.is_empty() {
        return Ok(());
    }
    if billing_mode != "tokens" || intervals.len() > 100 {
        return Err(ApiError::bad_request(
            "INVALID_PRICING_INTERVALS",
            "token pricing supports at most 100 intervals",
        ));
    }
    let mut sorted = intervals.to_vec();
    sorted.sort_by_key(|interval| interval.min_tokens);
    for (index, interval) in sorted.iter().enumerate() {
        if interval.min_tokens < 0
            || interval
                .max_tokens
                .is_some_and(|maximum| maximum <= interval.min_tokens)
        {
            return Err(ApiError::bad_request(
                "INVALID_PRICING_INTERVALS",
                "interval bounds are invalid",
            ));
        }
        optional_price_to_microusd(interval.input_price)?;
        optional_price_to_microusd(interval.output_price)?;
        optional_price_to_microusd(interval.cache_read_price)?;
        optional_price_to_microusd(interval.cache_write_price)?;
        if interval.input_price.is_none()
            && interval.output_price.is_none()
            && interval.cache_read_price.is_none()
            && interval.cache_write_price.is_none()
        {
            return Err(ApiError::bad_request(
                "INVALID_PRICING_INTERVALS",
                "an interval must override at least one price",
            ));
        }
        if interval.max_tokens.is_none() && index + 1 != sorted.len() {
            return Err(ApiError::bad_request(
                "INVALID_PRICING_INTERVALS",
                "an unbounded interval must be last",
            ));
        }
        if index > 0
            && sorted[index - 1]
                .max_tokens
                .is_none_or(|maximum| maximum > interval.min_tokens)
        {
            return Err(ApiError::bad_request(
                "INVALID_PRICING_INTERVALS",
                "pricing intervals overlap",
            ));
        }
    }
    Ok(())
}

fn microusd_to_price(value: i64) -> f64 {
    value as f64 / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use axum::extract::{Extension, State};

    use super::*;
    use crate::{auth::AuthSession, test_support};

    #[test]
    fn prices_round_trip_through_integer_microusd() {
        let stored = price_to_microusd(2.5).unwrap();
        assert_eq!(stored, 2_500_000);
        assert_eq!(microusd_to_price(stored), 2.5);
    }

    #[test]
    fn resolves_exact_and_longest_wildcard_model_mappings() {
        let rules = HashMap::from([
            ("gpt-*".into(), "gpt-default".into()),
            ("gpt-5-*".into(), "gpt-5-latest".into()),
            ("gpt-exact".into(), "gpt-pinned".into()),
        ]);
        assert_eq!(
            resolve_mapping_rule(&rules, "GPT-EXACT"),
            Some("gpt-pinned")
        );
        assert_eq!(
            resolve_mapping_rule(&rules, "gpt-5-mini"),
            Some("gpt-5-latest")
        );
        assert_eq!(resolve_mapping_rule(&rules, "gpt-4.1"), Some("gpt-default"));
        assert_eq!(resolve_mapping_rule(&rules, "o3"), None);
        assert!(model_patterns_conflict("gpt-*", "gpt-5"));
        assert!(!model_patterns_conflict("gpt-4-*", "gpt-5-*"));
    }

    fn test_pricing(model: &str, input_price: f64) -> PricingInput {
        PricingInput {
            platform: "openai".into(),
            models: vec![model.into()],
            billing_mode: "tokens".into(),
            input_price,
            output_price: 2.0,
            per_request_price: 0.0,
            cache_read_price: Some(0.25),
            cache_write_price: Some(1.25),
            image_input_price: Some(3.0),
            image_output_price: Some(4.0),
            intervals: vec![PricingIntervalInput {
                min_tokens: 100,
                max_tokens: Some(200),
                input_price: Some(input_price + 0.5),
                output_price: None,
                cache_read_price: None,
                cache_write_price: Some(1.5),
            }],
        }
    }

    #[tokio::test]
    async fn account_stats_pricing_rules_round_trip_with_scoped_foreign_keys() {
        let (_directory, state) = test_support::state().await;
        let encrypted = state.crypto.encrypt(b"{}").unwrap();
        let account_id = sqlx::query(
            "INSERT INTO accounts (name, kind, base_url, encrypted_credentials) \
             VALUES ('priced account', 'api_key', 'https://example.com', ?)",
        )
        .bind(encrypted)
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        let group_id = sqlx::query("INSERT INTO groups (name) VALUES ('priced group')")
            .execute(&state.pool)
            .await
            .unwrap()
            .last_insert_rowid();
        let input = ChannelInput {
            name: "Account costs".into(),
            description: "separate upstream cost".into(),
            status: "active".into(),
            restrict_models: false,
            model_mapping: HashMap::new(),
            billing_model_source: "upstream".into(),
            group_ids: vec![group_id],
            model_pricing: vec![test_pricing("gpt-client", 1.0)],
            apply_pricing_to_account_stats: true,
            account_stats_pricing_rules: vec![AccountStatsPricingRuleInput {
                name: "OAuth actual".into(),
                group_ids: vec![group_id],
                account_ids: vec![account_id],
                pricing: vec![test_pricing("gpt-real*", 3.0)],
            }],
        };
        let (status, Json(created)) = create(State(state.clone()), Json(input)).await.unwrap();
        assert_eq!(status, StatusCode::CREATED);
        let channel = &created["data"];
        assert_eq!(channel["apply_pricing_to_account_stats"], true);
        assert_eq!(
            channel["account_stats_pricing_rules"][0]["name"],
            "OAuth actual"
        );
        assert_eq!(
            channel["account_stats_pricing_rules"][0]["account_ids"][0],
            account_id
        );
        assert_eq!(
            channel["account_stats_pricing_rules"][0]["pricing"][0]["cache_write_price"],
            1.25
        );
        assert_eq!(
            channel["account_stats_pricing_rules"][0]["pricing"][0]["intervals"][0]["input_price"],
            3.5
        );
        let channel_id = channel["id"].as_i64().unwrap();
        let counts: (i64, i64, i64, i64) = sqlx::query_as(
            "SELECT (SELECT COUNT(*) FROM channel_account_stats_rules WHERE channel_id = ?), \
             (SELECT COUNT(*) FROM channel_account_stats_rule_groups), \
             (SELECT COUNT(*) FROM channel_account_stats_rule_accounts), \
             (SELECT COUNT(*) FROM channel_account_stats_intervals)",
        )
        .bind(channel_id)
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(counts, (1, 1, 1, 1));

        sqlx::query("DELETE FROM accounts WHERE id = ?")
            .bind(account_id)
            .execute(&state.pool)
            .await
            .unwrap();
        let scoped_accounts: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM channel_account_stats_rule_accounts")
                .fetch_one(&state.pool)
                .await
                .unwrap();
        assert_eq!(scoped_accounts, 0);
        assert_eq!(
            channel_view(&state, channel_id).await.unwrap()["account_stats_pricing_rules"][0]["group_ids"]
                [0],
            group_id
        );
    }

    #[tokio::test]
    async fn available_channels_group_platforms_and_resolve_user_rates() {
        let (_directory, state) = test_support::state().await;
        let user_id: i64 =
            sqlx::query_scalar("SELECT id FROM users WHERE role = 'admin' ORDER BY id LIMIT 1")
                .fetch_one(&state.pool)
                .await
                .unwrap();
        let group_id = sqlx::query(
            "INSERT INTO groups (name, platform, rate_multiplier_micros) \
             VALUES ('OpenAI Users', 'openai', 800000)",
        )
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        sqlx::query(
            "INSERT INTO user_group_rate_multipliers \
             (user_id, group_id, rate_multiplier_micros) VALUES (?, ?, 600000)",
        )
        .bind(user_id)
        .bind(group_id)
        .execute(&state.pool)
        .await
        .unwrap();
        let channel_id = sqlx::query(
            "INSERT INTO channels (name, description) VALUES ('Primary', 'OpenAI models')",
        )
        .execute(&state.pool)
        .await
        .unwrap()
        .last_insert_rowid();
        sqlx::query("INSERT INTO channel_groups (channel_id, group_id) VALUES (?, ?)")
            .bind(channel_id)
            .bind(group_id)
            .execute(&state.pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO channel_model_pricing \
             (channel_id, platform, models, input_microusd_per_million) \
             VALUES (?, 'openai', '[\"gpt-5\"]', 1250000)",
        )
        .bind(channel_id)
        .execute(&state.pool)
        .await
        .unwrap();

        let Json(body) = available(
            State(state),
            Extension(AuthSession {
                id: 1,
                user_id,
                username: "admin".into(),
                display_name: "Admin".into(),
                role: "admin".into(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(body["data"][0]["platforms"][0]["platform_label"], "OpenAI");
        assert_eq!(body["data"][0]["platforms"][0]["model_count"], 1);
        assert_eq!(
            body["data"][0]["platforms"][0]["groups"][0]["user_rate_multiplier"],
            0.6
        );
        assert_eq!(
            body["data"][0]["platforms"][0]["groups"][0]["effective_rate_multiplier"],
            0.6
        );
    }
}
