use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::{ApiError, json_ok};
use crate::model::ChannelRow;
use crate::state::AppState;

#[derive(Deserialize, Default)]
pub struct ChannelQuery {
    pub p: Option<i64>,
    pub page_size: Option<i64>,
    pub status: Option<String>,
    pub r#type: Option<i64>,
    pub group: Option<String>,
    pub id_sort: Option<bool>,
    pub keyword: Option<String>,
    pub model: Option<String>,
}

#[derive(Deserialize)]
pub struct AddChannelInput {
    #[serde(default)]
    pub mode: Option<String>,
    pub channel: ChannelInput,
}

#[derive(Deserialize, Default)]
pub struct ChannelInput {
    pub id: Option<i64>,
    pub r#type: Option<i64>,
    pub key: Option<String>,
    pub key_mode: Option<String>,
    pub status: Option<i64>,
    pub name: Option<String>,
    pub models: Option<String>,
    pub group: Option<String>,
    pub base_url: Option<String>,
    pub priority: Option<i64>,
    pub weight: Option<i64>,
    pub channel_info: Option<Value>,
    pub model_mapping: Option<Value>,
    pub header_override: Option<Value>,
    pub status_code_mapping: Option<Value>,
    pub param_override: Option<Value>,
    pub setting: Option<Value>,
    pub settings: Option<Value>,
    pub other: Option<Value>,
    pub other_info: Option<Value>,
    pub tag: Option<String>,
    pub remark: Option<String>,
    pub auto_ban: Option<i64>,
    pub open_ai_organization: Option<String>,
    pub test_model: Option<String>,
    pub test_time: Option<i64>,
    pub response_time: Option<i64>,
    pub balance: Option<f64>,
    pub balance_updated_time: Option<i64>,
    pub used_quota: Option<i64>,
    pub created_time: Option<i64>,
}

#[derive(Deserialize)]
pub struct StatusInput {
    pub status: i64,
}

#[derive(Deserialize)]
pub struct BatchStatusInput {
    pub ids: Vec<i64>,
    pub status: i64,
}

#[derive(Deserialize)]
pub struct IdsInput {
    pub ids: Vec<i64>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/channel", get(list).post(create))
        .route("/api/channel/", get(list).post(create).put(update))
        .route("/api/channel/search", get(search))
        .route("/api/channel/models", get(all_models))
        .route("/api/channel/models_enabled", get(enabled_models))
        .route("/api/channel/default_base_urls", get(default_base_urls))
        .route("/api/channel/ops", get(ops))
        .route("/api/channel/priority/order", get(priority_order))
        .route("/api/channel/priority/reorder", post(reorder_priority))
        .route("/api/channel/fix", post(fix_abilities))
        .route("/api/channel/batch", post(batch_delete))
        .route("/api/channel/status/batch", post(batch_status))
        .route("/api/channel/{id}", get(get_one).delete(delete_one))
        .route("/api/channel/{id}/status", post(update_status))
        .route("/api/channel/{id}/key", post(get_key))
}

/// `channels` 表的全部读取列，顺序与 `ChannelRow` 字段一致。
pub(crate) const SELECT_COLUMNS: &str = "id, type, `key`, status, name, models, `group`, base_url, priority, weight, channel_info, model_mapping, header_override, created_time, test_time, response_time, open_ai_organization, test_model, other, balance, balance_updated_time, used_quota, status_code_mapping, auto_ban, other_info, tag, setting, param_override, remark, settings";

pub(crate) async fn load_channel(state: &AppState, id: i64) -> Result<Option<ChannelRow>, ApiError> {
    let sql = format!("SELECT {SELECT_COLUMNS} FROM channels WHERE id=?");
    let row: Option<ChannelRow> = sqlx::query_as(&sql)
        .bind(id)
        .fetch_optional(&state.pool)
        .await
        .map_err(internal)?;
    Ok(row)
}

pub(crate) fn json_text(value: Option<&Value>) -> Option<String> {
    match value {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(other) => Some(other.to_string()),
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn channel_info(value: Option<&Value>) -> String {
    match value {
        Some(v @ Value::Object(_)) => v.to_string(),
        Some(Value::String(s)) if !s.trim().is_empty() => s.clone(),
        _ => json!({
            "is_multi_key": false,
            "multi_key_size": 1,
            "multi_key_polling_index": 0,
            "multi_key_mode": "random"
        })
        .to_string(),
    }
}

fn channel_json(channel: &ChannelRow) -> Value {
    let info: Value = channel
        .channel_info
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_else(|| {
            json!({
                "is_multi_key": false,
                "multi_key_size": 1,
                "multi_key_polling_index": 0,
                "multi_key_mode": "random"
            })
        });
    json!({
        "id": channel.id,
        "type": channel.r#type,
        "key": "",
        "openai_organization": channel.open_ai_organization,
        "test_model": channel.test_model,
        "status": channel.status,
        "name": channel.name,
        "weight": channel.weight.unwrap_or(0),
        "created_time": channel.created_time.unwrap_or(0),
        "test_time": channel.test_time.unwrap_or(0),
        "response_time": channel.response_time.unwrap_or(0),
        "base_url": channel.base_url.clone().unwrap_or_default(),
        "other": channel.other.clone().unwrap_or_default(),
        "balance": channel.balance.unwrap_or(0.0),
        "balance_updated_time": channel.balance_updated_time.unwrap_or(0),
        "models": channel.models,
        "group": channel.group,
        "used_quota": channel.used_quota.unwrap_or(0),
        "model_mapping": channel.model_mapping,
        "status_code_mapping": channel.status_code_mapping.clone().unwrap_or_default(),
        "priority": channel.priority.unwrap_or(0),
        "auto_ban": channel.auto_ban.unwrap_or(1),
        "other_info": channel.other_info.clone().unwrap_or_default(),
        "tag": channel.tag,
        "setting": channel.setting,
        "param_override": channel.param_override,
        "header_override": channel.header_override,
        "remark": channel.remark.clone().unwrap_or_default(),
        "channel_info": info,
        "settings": channel.settings.clone().unwrap_or_else(|| "{}".to_string()),
    })
}

pub(crate) fn split_list(input: &str) -> Vec<String> {
    input
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

async fn rebuild_abilities(
    state: &AppState,
    channel_id: i64,
    models: &str,
    group: &str,
    status: i64,
    priority: i64,
    weight: i64,
    tag: Option<&str>,
) -> Result<(), ApiError> {
    sqlx::query("DELETE FROM abilities WHERE channel_id = ?")
        .bind(channel_id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    for model in split_list(models) {
        for g in split_list(group) {
            sqlx::query(
                "INSERT OR REPLACE INTO abilities (`group`, model, channel_id, enabled, priority, weight, tag) VALUES (?,?,?,?,?,?,?)",
            )
            .bind(&g)
            .bind(&model)
            .bind(channel_id)
            .bind(status == 1)
            .bind(priority)
            .bind(weight)
            .bind(tag)
            .execute(&state.pool)
            .await
            .map_err(internal)?;
        }
    }
    Ok(())
}

async fn insert_channel(
    state: &AppState,
    input: &ChannelInput,
    key: &str,
    name: &str,
    info: String,
) -> Result<i64, ApiError> {
    let now = now();
    let result = sqlx::query(
        "INSERT INTO channels (type, `key`, status, name, weight, created_time, test_time, response_time, base_url, other, balance, balance_updated_time, models, `group`, used_quota, model_mapping, status_code_mapping, priority, auto_ban, other_info, tag, setting, param_override, header_override, remark, channel_info, settings, open_ai_organization, test_model) \
         VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(input.r#type.unwrap_or(1))
    .bind(key)
    .bind(input.status.unwrap_or(1))
    .bind(name)
    .bind(input.weight.unwrap_or(0))
    .bind(input.created_time.unwrap_or(now))
    .bind(input.test_time.unwrap_or(0))
    .bind(input.response_time.unwrap_or(0))
    .bind(input.base_url.clone().unwrap_or_default())
    .bind(json_text(input.other.as_ref()).unwrap_or_default())
    .bind(input.balance.unwrap_or(0.0))
    .bind(input.balance_updated_time.unwrap_or(0))
    .bind(input.models.clone().unwrap_or_default())
    .bind(input.group.clone().unwrap_or_else(|| "default".to_string()))
    .bind(input.used_quota.unwrap_or(0))
    .bind(json_text(input.model_mapping.as_ref()))
    .bind(json_text(input.status_code_mapping.as_ref()).unwrap_or_default())
    .bind(input.priority.unwrap_or(0))
    .bind(input.auto_ban.unwrap_or(1))
    .bind(json_text(input.other_info.as_ref()).unwrap_or_default())
    .bind(input.tag.clone())
    .bind(json_text(input.setting.as_ref()).unwrap_or_default())
    .bind(json_text(input.param_override.as_ref()).unwrap_or_default())
    .bind(json_text(input.header_override.as_ref()))
    .bind(input.remark.clone().unwrap_or_default())
    .bind(info)
    .bind(json_text(input.settings.as_ref()).unwrap_or_else(|| "{}".to_string()))
    .bind(input.open_ai_organization.clone())
    .bind(input.test_model.clone())
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    Ok(result.last_insert_rowid())
}

fn internal(e: sqlx::Error) -> ApiError {
    ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

fn page(query: &ChannelQuery) -> (i64, i64, i64) {
    let p = query.p.unwrap_or(1).max(1);
    let size = query.page_size.unwrap_or(10).clamp(1, 200);
    (p, size, (p - 1) * size)
}

fn filters(query: &ChannelQuery) -> (String, String, Option<i64>, String) {
    let status = match query.status.as_deref() {
        Some("enabled") => "1".to_string(),
        Some("disabled") => "0".to_string(),
        _ => String::new(),
    };
    let keyword = query.keyword.clone().unwrap_or_default();
    let group = query.group.clone().unwrap_or_default();
    (status, keyword, query.r#type, group)
}

fn where_sql() -> &'static str {
    " (? = '' OR status = CAST(? AS INTEGER)) AND (? = '' OR name LIKE '%'||?||'%' OR models LIKE '%'||?||'%') AND (? = 0 OR type = ?) AND (? = '' OR `group` LIKE '%'||?||'%') "
}

async fn list_channels(
    state: &AppState,
    query: &ChannelQuery,
) -> Result<axum::response::Response, ApiError> {
    let (p, size, offset) = page(query);
    let (status, keyword, type_filter, group) = filters(query);
    let type_filter = type_filter.unwrap_or(0);
    let order = if query.id_sort.unwrap_or(false) {
        "ORDER BY id ASC"
    } else {
        "ORDER BY id DESC"
    };
    let count: (i64,) = sqlx::query_as(&format!(
        "SELECT COUNT(*) FROM channels WHERE {}",
        where_sql()
    ))
    .bind(&status)
    .bind(&status)
    .bind(&keyword)
    .bind(&keyword)
    .bind(&keyword)
    .bind(type_filter)
    .bind(type_filter)
    .bind(&group)
    .bind(&group)
    .fetch_one(&state.pool)
    .await
    .map_err(internal)?;
    let rows: Vec<ChannelRow> = sqlx::query_as(&format!(
        "SELECT {SELECT_COLUMNS} FROM channels WHERE {} {order} LIMIT ? OFFSET ?",
        where_sql()
    ))
    .bind(&status)
    .bind(&status)
    .bind(&keyword)
    .bind(&keyword)
    .bind(&keyword)
    .bind(type_filter)
    .bind(type_filter)
    .bind(&group)
    .bind(&group)
    .bind(size)
    .bind(offset)
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    let items: Vec<Value> = rows.iter().map(channel_json).collect();
    Ok(json_ok(&json!({
        "items": items,
        "total": count.0,
        "page": p,
        "page_size": size,
    })))
}

pub async fn list(
    State(state): State<AppState>,
    Query(query): Query<ChannelQuery>,
) -> Result<axum::response::Response, ApiError> {
    list_channels(&state, &query).await
}

pub async fn search(
    State(state): State<AppState>,
    Query(query): Query<ChannelQuery>,
) -> Result<axum::response::Response, ApiError> {
    list_channels(&state, &query).await
}

pub async fn get_one(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<axum::response::Response, ApiError> {
    let row = load_channel(&state, id)
        .await?
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "channel not found"))?;
    Ok(json_ok(&channel_json(&row)))
}

pub async fn create(
    State(state): State<AppState>,
    Json(input): Json<AddChannelInput>,
) -> Result<axum::response::Response, ApiError> {
    let channel = input.channel;
    let name = channel
        .name
        .clone()
        .filter(|n| !n.trim().is_empty())
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "name is required"))?;
    let raw_keys = channel.key.clone().unwrap_or_default();
    let mode = input.mode.unwrap_or_else(|| "single".to_string());
    let keys: Vec<String> = match mode.as_str() {
        "batch" => raw_keys
            .lines()
            .map(str::trim)
            .filter(|k| !k.is_empty())
            .map(str::to_string)
            .collect(),
        _ => vec![raw_keys.trim().to_string()],
    };
    if keys.is_empty() {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "key is required"));
    }
    let multi = mode == "multi_to_single";
    let key_count: usize = keys
        .iter()
        .map(|k| k.lines().filter(|l| !l.trim().is_empty()).count().max(1))
        .sum();
    let mut info = channel_info(channel.channel_info.as_ref());
    if multi {
        info = json!({
            "is_multi_key": true,
            "multi_key_size": key_count,
            "multi_key_polling_index": 0,
            "multi_key_mode": "random"
        })
        .to_string();
    }
    let mut created = Vec::new();
    for (index, key) in keys.iter().enumerate() {
        let channel_name = if mode == "batch" && keys.len() > 1 {
            format!("{name}-{}", index + 1)
        } else {
            name.clone()
        };
        let id = insert_channel(&state, &channel, key, &channel_name, info.clone()).await?;
        let row: (String, String, i64, i64, i64, Option<String>) =
            sqlx::query_as("SELECT models, `group`, status, priority, weight, tag FROM channels WHERE id=?")
                .bind(id)
                .fetch_one(&state.pool)
                .await
                .map_err(internal)?;
        rebuild_abilities(
            &state,
            id,
            &row.0,
            &row.1,
            row.2,
            row.3,
            row.4,
            row.5.as_deref(),
        )
        .await?;
        created.push(id);
    }
    state.reload_caches().await.map_err(internal)?;
    Ok(json_ok(&json!(created)))
}

pub async fn update(
    State(state): State<AppState>,
    Json(input): Json<ChannelInput>,
) -> Result<axum::response::Response, ApiError> {
    let id = input
        .id
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "id is required"))?;
    let current = load_channel(&state, id)
        .await?
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "channel not found"))?;

    let key = match input.key.as_deref().map(str::trim) {
        Some(k) if !k.is_empty() => {
            if input.key_mode.as_deref() == Some("append") && !current.key.is_empty() {
                format!("{}\n{}", current.key, k)
            } else {
                k.to_string()
            }
        }
        _ => current.key.clone(),
    };
    let name = input.name.clone().unwrap_or(current.name.clone());
    let models = input.models.clone().unwrap_or(current.models.clone());
    let group = input.group.clone().unwrap_or(current.group.clone());
    let status = input.status.unwrap_or(current.status);
    let priority = input.priority.unwrap_or(current.priority.unwrap_or(0));
    let weight = input.weight.unwrap_or(current.weight.unwrap_or(0));
    let model_mapping = match input.model_mapping.as_ref() {
        Some(_) => json_text(input.model_mapping.as_ref()),
        None => current.model_mapping.clone(),
    };
    let header_override = match input.header_override.as_ref() {
        Some(_) => json_text(input.header_override.as_ref()),
        None => current.header_override.clone(),
    };
    let info = match input.channel_info.as_ref() {
        Some(_) => channel_info(input.channel_info.as_ref()),
        None => current.channel_info.clone().unwrap_or_else(|| channel_info(None)),
    };
    sqlx::query(
        "UPDATE channels SET type=?, `key`=?, status=?, name=?, weight=?, base_url=?, other=?, models=?, `group`=?, model_mapping=?, status_code_mapping=?, priority=?, auto_ban=?, other_info=?, tag=?, setting=?, param_override=?, header_override=?, remark=?, channel_info=?, settings=?, open_ai_organization=?, test_model=? WHERE id=?",
    )
    .bind(input.r#type.unwrap_or(current.r#type))
    .bind(key)
    .bind(status)
    .bind(&name)
    .bind(weight)
    .bind(input.base_url.clone().or(current.base_url).unwrap_or_default())
    .bind(json_text(input.other.as_ref()).or(current.other).unwrap_or_default())
    .bind(&models)
    .bind(&group)
    .bind(model_mapping)
    .bind(json_text(input.status_code_mapping.as_ref()).or(current.status_code_mapping).unwrap_or_default())
    .bind(priority)
    .bind(input.auto_ban.or(current.auto_ban).unwrap_or(1))
    .bind(json_text(input.other_info.as_ref()).or(current.other_info).unwrap_or_default())
    .bind(input.tag.clone().or(current.tag))
    .bind(json_text(input.setting.as_ref()).or(current.setting).unwrap_or_default())
    .bind(json_text(input.param_override.as_ref()).or(current.param_override).unwrap_or_default())
    .bind(header_override)
    .bind(input.remark.clone().or(current.remark).unwrap_or_default())
    .bind(info)
    .bind(json_text(input.settings.as_ref()).or(current.settings).unwrap_or_else(|| "{}".to_string()))
    .bind(input.open_ai_organization.clone().or(current.open_ai_organization))
    .bind(input.test_model.clone().or(current.test_model))
    .bind(id)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    let tag: Option<String> = sqlx::query_scalar("SELECT tag FROM channels WHERE id=?")
        .bind(id)
        .fetch_one(&state.pool)
        .await
        .map_err(internal)?;
    rebuild_abilities(
        &state,
        id,
        &models,
        &group,
        status,
        priority,
        weight,
        tag.as_deref(),
    )
    .await?;
    state.reload_caches().await.map_err(internal)?;
    get_one(State(state), Path(id)).await
}

pub async fn update_status(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(input): Json<StatusInput>,
) -> Result<axum::response::Response, ApiError> {
    let result = sqlx::query("UPDATE channels SET status=? WHERE id=?")
        .bind(input.status)
        .bind(id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    if result.rows_affected() == 0 {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "channel not found"));
    }
    sqlx::query("UPDATE abilities SET enabled=? WHERE channel_id=?")
        .bind(input.status == 1)
        .bind(id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    state.reload_caches().await.map_err(internal)?;
    Ok(json_ok(&json!(true)))
}

pub async fn batch_status(
    State(state): State<AppState>,
    Json(input): Json<BatchStatusInput>,
) -> Result<axum::response::Response, ApiError> {
    let mut affected = 0_u64;
    for id in input.ids {
        let result = sqlx::query("UPDATE channels SET status=? WHERE id=?")
            .bind(input.status)
            .bind(id)
            .execute(&state.pool)
            .await
            .map_err(internal)?;
        affected += result.rows_affected();
        sqlx::query("UPDATE abilities SET enabled=? WHERE channel_id=?")
            .bind(input.status == 1)
            .bind(id)
            .execute(&state.pool)
            .await
            .map_err(internal)?;
    }
    state.reload_caches().await.map_err(internal)?;
    Ok(json_ok(&affected))
}

async fn delete_channel(state: &AppState, id: i64) -> Result<bool, ApiError> {
    let result = sqlx::query("DELETE FROM channels WHERE id=?")
        .bind(id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    sqlx::query("DELETE FROM abilities WHERE channel_id=?")
        .bind(id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    Ok(result.rows_affected() > 0)
}

pub async fn delete_one(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<axum::response::Response, ApiError> {
    if !delete_channel(&state, id).await? {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "channel not found"));
    }
    state.reload_caches().await.map_err(internal)?;
    Ok(json_ok(&Value::Null))
}

pub async fn batch_delete(
    State(state): State<AppState>,
    Json(input): Json<IdsInput>,
) -> Result<axum::response::Response, ApiError> {
    let mut affected = 0_i64;
    for id in input.ids {
        if delete_channel(&state, id).await? {
            affected += 1;
        }
    }
    state.reload_caches().await.map_err(internal)?;
    Ok(json_ok(&affected))
}

pub async fn get_key(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<axum::response::Response, ApiError> {
    let key: Option<String> = sqlx::query_scalar("SELECT `key` FROM channels WHERE id=?")
        .bind(id)
        .fetch_optional(&state.pool)
        .await
        .map_err(internal)?;
    let key = key.ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "channel not found"))?;
    Ok(json_ok(&json!({ "key": key })))
}

pub async fn all_models(
    State(state): State<AppState>,
) -> Result<axum::response::Response, ApiError> {
    let rows: Vec<String> = sqlx::query_scalar("SELECT models FROM channels")
        .fetch_all(&state.pool)
        .await
        .map_err(internal)?;
    let mut seen = std::collections::BTreeSet::new();
    for row in rows {
        for model in split_list(&row) {
            seen.insert(model);
        }
    }
    let data: Vec<Value> = seen.into_iter().map(|m| json!({ "id": m })).collect();
    Ok(json_ok(&data))
}

pub async fn enabled_models(
    State(state): State<AppState>,
) -> Result<axum::response::Response, ApiError> {
    let rows: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT a.model FROM abilities a JOIN channels c ON c.id=a.channel_id WHERE a.enabled=1 AND c.status=1 ORDER BY a.model",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    Ok(json_ok(&rows))
}

pub async fn ops(
    State(state): State<AppState>,
) -> Result<axum::response::Response, ApiError> {
    let retry: Option<String> = sqlx::query_scalar("SELECT value FROM options WHERE `key`='RetryTimes'")
        .fetch_optional(&state.pool)
        .await
        .map_err(internal)?;
    Ok(json_ok(&json!({
        "retry_times": retry.and_then(|v| v.parse::<i64>().ok()).unwrap_or(3),
        "request_policy": {"automatic_disable": true, "source": "system"}
    })))
}

pub async fn default_base_urls() -> axum::response::Response {
    json_ok(&json!({}))
}

/// 快照按 id 升序，revision 是它的指纹；前端提交时原样带回用于并发校验。
async fn priority_items(
    conn: &mut sqlx::sqlite::SqliteConnection,
) -> Result<Vec<crate::priority::Item>, sqlx::Error> {
    let rows: Vec<(i64, Option<i64>)> =
        sqlx::query_as("SELECT id, priority FROM channels ORDER BY id")
            .fetch_all(&mut *conn)
            .await?;
    Ok(rows
        .into_iter()
        .map(|(id, priority)| crate::priority::Item { id, priority })
        .collect())
}

fn order_json(
    items: &[crate::priority::Item],
    changes: &[crate::priority::Change],
) -> Value {
    json!({
        "revision": crate::priority::revision(items),
        "items": items,
        "changes": changes,
    })
}

pub async fn priority_order(
    State(state): State<AppState>,
) -> Result<axum::response::Response, ApiError> {
    let mut conn = state.pool.acquire().await.map_err(internal)?;
    let items = priority_items(&mut conn).await.map_err(internal)?;
    Ok(json_ok(&order_json(&items, &[])))
}

/// 拖拽排序：preview 只算不写；正式提交在一个事务里同时改 channels 与 abilities，
/// 否则路由缓存会读到与渠道表不一致的优先级。
pub async fn reorder_priority(
    State(state): State<AppState>,
    Json(mv): Json<crate::priority::Move>,
) -> Result<axum::response::Response, ApiError> {
    let business = |message: &'static str| ApiError::new(StatusCode::OK, message);
    if mv.preview {
        let mut conn = state.pool.acquire().await.map_err(internal)?;
        let items = priority_items(&mut conn).await.map_err(internal)?;
        let changes = crate::priority::plan(&items, &mv).map_err(business)?;
        let items = crate::priority::apply(&items, &changes);
        return Ok(json_ok(&order_json(&items, &changes)));
    }

    let mut tx = state.pool.begin().await.map_err(internal)?;
    let items = priority_items(&mut *tx).await.map_err(internal)?;
    let changes = crate::priority::plan(&items, &mv).map_err(business)?;
    for change in &changes {
        sqlx::query("UPDATE channels SET priority=? WHERE id=?")
            .bind(change.priority)
            .bind(change.id)
            .execute(&mut *tx)
            .await
            .map_err(internal)?;
        sqlx::query("UPDATE abilities SET priority=? WHERE channel_id=?")
            .bind(change.priority)
            .bind(change.id)
            .execute(&mut *tx)
            .await
            .map_err(internal)?;
    }
    let items = crate::priority::apply(&items, &changes);
    tx.commit().await.map_err(internal)?;
    if !changes.is_empty() {
        state.reload_caches().await.map_err(internal)?;
    }
    Ok(json_ok(&order_json(&items, &changes)))
}

/// 按 channels 全量重建 abilities，修复人工改库或与 Go 交替写入造成的路由漂移。
pub async fn fix_abilities(
    State(state): State<AppState>,
) -> Result<axum::response::Response, ApiError> {
    sqlx::query("DELETE FROM abilities")
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    let rows: Vec<ChannelRow> =
        sqlx::query_as(&format!("SELECT {SELECT_COLUMNS} FROM channels ORDER BY id"))
            .fetch_all(&state.pool)
            .await
            .map_err(internal)?;
    let mut success = 0i64;
    let mut fails = 0i64;
    for row in &rows {
        let result = rebuild_abilities(
            &state,
            row.id,
            &row.models,
            &row.group,
            row.status,
            row.priority.unwrap_or(0),
            row.weight.unwrap_or(0),
            row.tag.as_deref(),
        )
        .await;
        match result {
            Ok(()) => success += 1,
            Err(error) => {
                fails += 1;
                tracing::warn!("rebuild abilities for channel {} failed: {error:?}", row.id);
            }
        }
    }
    state.reload_caches().await.map_err(internal)?;
    Ok(json_ok(&json!({ "success": success, "fails": fails })))
}
