use std::collections::{BTreeSet, HashMap, HashSet};

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{delete, get};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::{ApiError, json_ok};
use crate::state::AppState;

/// 顺序与 Go 端 `model.NameRule*` 的 iota 一致：Exact、Prefix、Contains、Suffix。
const NAME_RULE_EXACT: i64 = 0;
const NAME_RULE_PREFIX: i64 = 1;
const NAME_RULE_CONTAINS: i64 = 2;
const NAME_RULE_SUFFIX: i64 = 3;
const CHANNEL_STATUS_ENABLED: i64 = 1;

#[derive(Deserialize, Default)]
pub struct ModelQuery {
    pub p: Option<i64>,
    pub size: Option<i64>,
    pub page_size: Option<i64>,
    pub keyword: Option<String>,
    pub vendor: Option<String>,
    pub status: Option<String>,
    pub sync_official: Option<String>,
    pub square_state: Option<String>,
    pub include_channel_models: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct ModelInput {
    pub id: Option<i64>,
    pub model_name: Option<String>,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub tags: Option<String>,
    pub vendor_id: Option<i64>,
    pub endpoints: Option<String>,
    pub status: Option<i64>,
    pub sync_official: Option<i64>,
    pub name_rule: Option<i64>,
}

#[derive(sqlx::FromRow)]
struct ModelRow {
    id: i64,
    model_name: String,
    description: Option<String>,
    icon: Option<String>,
    tags: Option<String>,
    vendor_id: Option<i64>,
    endpoints: Option<String>,
    status: i64,
    sync_official: i64,
    created_time: i64,
    updated_time: i64,
    name_rule: i64,
}

#[derive(sqlx::FromRow)]
struct VendorRow {
    id: i64,
    name: String,
    description: Option<String>,
    icon: Option<String>,
    status: i64,
    created_time: i64,
    updated_time: i64,
}

/// 一条可用路由：abilities.enabled 且所属渠道处于启用状态。
struct Connection {
    model: String,
    group: String,
    channel_id: i64,
    channel_name: String,
    channel_type: i64,
}

struct ChannelFacts {
    /// 渠道配置过的模型名 -> 渠道 id；与 Go 的 GetConfiguredModelChannels 一样包含禁用渠道。
    configured: HashMap<String, Vec<i64>>,
    connections: Vec<Connection>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/models", get(list).post(create).put(update))
        .route("/api/models/", get(list).post(create).put(update))
        .route("/api/models/search", get(search))
        .route("/api/models/enabled", get(enabled))
        .route("/api/vendors", get(vendors))
        .route("/api/vendors/", get(vendors))
        .route("/api/vendors/search", get(vendors))
        .route("/api/models/{id}", get(get_one).delete(delete_one))
        .route("/api/models/{id}/", delete(delete_one))
}

const SELECT: &str = "SELECT models.id, models.model_name, models.description, models.icon, models.tags, models.vendor_id, models.endpoints, models.status, models.sync_official, models.created_time, models.updated_time, models.name_rule FROM models";

fn model_json(row: &ModelRow) -> Value {
    json!({
        "id": row.id,
        "model_name": row.model_name,
        "description": row.description,
        "icon": row.icon,
        "tags": row.tags,
        "vendor_id": row.vendor_id,
        "endpoints": row.endpoints,
        "supported_endpoints": [],
        "status": row.status,
        "sync_official": row.sync_official,
        "created_time": row.created_time,
        "updated_time": row.updated_time,
        "name_rule": row.name_rule,
    })
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn page(query: &ModelQuery) -> (i64, i64, i64) {
    let p = query.p.unwrap_or(1).max(1);
    let size = query.size.or(query.page_size).unwrap_or(10).clamp(1, 200);
    (p, size, (p - 1) * size)
}

fn matches_name(rule: i64, pattern: &str, name: &str) -> bool {
    match rule {
        NAME_RULE_PREFIX => name.starts_with(pattern),
        NAME_RULE_CONTAINS => name.contains(pattern),
        NAME_RULE_SUFFIX => name.ends_with(pattern),
        _ => name == pattern,
    }
}

/// 与 Go 的 `Channel.GetModels` + `normalizeLookupValues` 一致：去空白、丢空项、渠道内去重。
fn split_names(raw: &str) -> Vec<String> {
    if raw.is_empty() {
        return Vec::new();
    }
    let mut seen = HashSet::new();
    let mut names = Vec::new();
    for part in raw.trim_matches(',').split(',') {
        let name = part.trim();
        if name.is_empty() {
            continue;
        }
        if seen.insert(name.to_owned()) {
            names.push(name.to_owned());
        }
    }
    names
}

fn channel_facts(state: &AppState) -> ChannelFacts {
    let caches = state.caches.read().expect("cache lock");
    let mut channel_ids: Vec<i64> = caches.channels.keys().copied().collect();
    channel_ids.sort_unstable();
    let mut configured: HashMap<String, Vec<i64>> = HashMap::new();
    for id in channel_ids {
        let Some(channel) = caches.channels.get(&id) else {
            continue;
        };
        for name in split_names(&channel.models) {
            configured.entry(name).or_default().push(id);
        }
    }

    let mut ability_keys: Vec<&(String, String)> = caches.abilities.keys().collect();
    ability_keys.sort();
    let mut connections = Vec::new();
    for key in ability_keys {
        let Some(rows) = caches.abilities.get(key) else {
            continue;
        };
        for ability in rows {
            if !ability.enabled {
                continue;
            }
            let enabled_channel = caches
                .channels
                .get(&ability.channel_id)
                .filter(|channel| channel.status == CHANNEL_STATUS_ENABLED);
            if let Some(channel) = enabled_channel {
                connections.push(Connection {
                    model: ability.model.clone(),
                    group: ability.group.clone(),
                    channel_id: channel.id,
                    channel_name: channel.name.clone(),
                    channel_type: channel.r#type,
                });
            }
        }
    }
    ChannelFacts {
        configured,
        connections,
    }
}

/// 元数据归属优先级：精确 > 前缀 > 后缀 > 包含，同规则内先到者胜。返回下标避免生命周期纠缠。
fn resolve_metadata(rows: &[ModelRow], names: &[String]) -> HashMap<String, usize> {
    let mut resolved: HashMap<String, usize> = HashMap::new();
    for (index, row) in rows.iter().enumerate() {
        if row.name_rule == NAME_RULE_EXACT {
            resolved.insert(row.model_name.clone(), index);
        }
    }
    for rule in [NAME_RULE_PREFIX, NAME_RULE_SUFFIX, NAME_RULE_CONTAINS] {
        for (index, row) in rows.iter().enumerate() {
            if row.name_rule != rule {
                continue;
            }
            for name in names {
                if !resolved.contains_key(name.as_str()) && matches_name(rule, &row.model_name, name)
                {
                    resolved.insert(name.clone(), index);
                }
            }
        }
    }
    resolved
}

/// 具体模型名的广场状态：命中的元数据被禁用即隐藏，否则有可用路由即展示。
fn square_states(meta: &[ModelRow], facts: &ChannelFacts) -> HashMap<String, &'static str> {
    let mut names: Vec<String> = facts.configured.keys().cloned().collect();
    for row in meta {
        if row.name_rule == NAME_RULE_EXACT && !names.contains(&row.model_name) {
            names.push(row.model_name.clone());
        }
    }
    let resolved = resolve_metadata(meta, &names);
    let available: HashSet<&str> = facts
        .connections
        .iter()
        .map(|connection| connection.model.as_str())
        .collect();
    names
        .into_iter()
        .map(|name| {
            let hidden = resolved
                .get(&name)
                .is_some_and(|index| meta[*index].status != 1);
            let state = if hidden {
                "hidden"
            } else if available.contains(name.as_str()) {
                "visible"
            } else {
                "unavailable"
            };
            (name, state)
        })
        .collect()
}

/// 规则行聚合它匹配到的具体模型：全隐藏即隐藏，全可见即可见，部分可见为部分展示。
fn rule_row_state(
    status: i64,
    rule: i64,
    pattern: &str,
    facts: &ChannelFacts,
    states: &HashMap<String, &'static str>,
) -> &'static str {
    let matched: Vec<&String> = facts
        .configured
        .keys()
        .filter(|name| matches_name(rule, pattern, name))
        .collect();
    let total = matched.len();
    if total == 0 {
        return if status != 1 { "hidden" } else { "unavailable" };
    }
    let visible = matched
        .iter()
        .filter(|name| states[name.as_str()] == "visible")
        .count();
    let hidden = matched
        .iter()
        .filter(|name| states[name.as_str()] == "hidden")
        .count();
    if hidden == total {
        "hidden"
    } else if visible == total {
        "visible"
    } else if visible > 0 {
        "partial"
    } else {
        "unavailable"
    }
}

/// 列表里的一行：元数据行，或渠道派生的合成行。合成行不落库，只用于展示配置里的模型名。
fn row_json(
    meta: Option<&ModelRow>,
    name: &str,
    facts: &ChannelFacts,
    states: &HashMap<String, &'static str>,
) -> Value {
    let rule = meta.map(|row| row.name_rule).unwrap_or(NAME_RULE_EXACT);
    let status = meta.map(|row| row.status).unwrap_or(0);
    let id = meta.map(|row| row.id).unwrap_or(0);

    let mut out = match meta {
        Some(row) => model_json(row),
        None => json!({
            "id": 0,
            "model_name": name,
            "description": Value::Null,
            "icon": Value::Null,
            "tags": Value::Null,
            "vendor_id": Value::Null,
            "endpoints": Value::Null,
            "supported_endpoints": [],
            "status": 0,
            "sync_official": 0,
            "created_time": 0,
            "updated_time": 0,
            "name_rule": NAME_RULE_EXACT,
        }),
    };

    let configured_count: HashSet<i64> = facts
        .configured
        .iter()
        .filter(|(configured, _)| matches_name(rule, name, configured))
        .flat_map(|(_, ids)| ids.iter().copied())
        .collect();

    let mut bound: HashMap<i64, (String, i64)> = HashMap::new();
    let mut groups: BTreeSet<String> = BTreeSet::new();
    let mut matched: BTreeSet<String> = BTreeSet::new();
    for connection in &facts.connections {
        if !matches_name(rule, name, &connection.model) {
            continue;
        }
        matched.insert(connection.model.clone());
        groups.insert(connection.group.clone());
        bound.insert(
            connection.channel_id,
            (connection.channel_name.clone(), connection.channel_type),
        );
    }
    let mut bound_channels: Vec<Value> = bound
        .values()
        .map(|(channel_name, channel_type)| {
            json!({ "name": channel_name, "type": channel_type })
        })
        .collect();
    bound_channels.sort_by_key(|channel| {
        (
            channel["name"].as_str().unwrap_or_default().to_owned(),
            channel["type"].as_i64().unwrap_or_default(),
        )
    });

    let square = if rule == NAME_RULE_EXACT {
        states.get(name).copied().unwrap_or("unavailable")
    } else {
        rule_row_state(status, rule, name, facts, states)
    };

    out["has_metadata"] = json!(id > 0);
    out["configured_channel_count"] = json!(configured_count.len());
    out["bound_channels"] = json!(bound_channels);
    out["enable_groups"] = json!(groups.into_iter().collect::<Vec<_>>());
    out["square_state"] = json!(square);
    if rule != NAME_RULE_EXACT {
        let models: Vec<String> = matched.into_iter().collect();
        out["matched_count"] = json!(models.len());
        out["matched_models"] = json!(models);
    }
    out
}

fn parse_status_filter(raw: &str) -> Option<i64> {
    match raw.trim().to_lowercase().as_str() {
        "" | "all" => None,
        "enabled" | "1" => Some(1),
        "disabled" | "0" => Some(0),
        other => other.parse().ok(),
    }
}

fn parse_sync_filter(raw: &str) -> Option<i64> {
    match raw.trim().to_lowercase().as_str() {
        "" | "all" => None,
        "yes" | "1" => Some(1),
        "no" | "0" => Some(0),
        other => other.parse().ok(),
    }
}

enum VendorFilter {
    Id(i64),
    Name(String),
}

fn parse_vendor_filter(raw: &str) -> Option<VendorFilter> {
    if raw.is_empty() {
        return None;
    }
    match raw.parse::<i64>() {
        Ok(id) => Some(VendorFilter::Id(id)),
        Err(_) => Some(VendorFilter::Name(raw.to_owned())),
    }
}

async fn fetch_meta(
    state: &AppState,
    keyword: &str,
    vendor: Option<&VendorFilter>,
    status: Option<i64>,
    sync: Option<i64>,
) -> Result<Vec<ModelRow>, ApiError> {
    let mut sql = String::from(SELECT);
    if matches!(vendor, Some(VendorFilter::Name(_))) {
        sql.push_str(" JOIN vendors ON vendors.id = models.vendor_id");
    }
    sql.push_str(" WHERE models.deleted_at IS NULL");
    if !keyword.is_empty() {
        sql.push_str(" AND (models.model_name LIKE ? OR models.description LIKE ? OR models.tags LIKE ?)");
    }
    match vendor {
        Some(VendorFilter::Id(_)) => sql.push_str(" AND models.vendor_id = ?"),
        Some(VendorFilter::Name(_)) => sql.push_str(" AND vendors.name LIKE ?"),
        None => {}
    }
    if status.is_some() {
        sql.push_str(" AND models.status = ?");
    }
    if sync.is_some() {
        sql.push_str(" AND models.sync_official = ?");
    }
    sql.push_str(" ORDER BY models.id DESC");

    let mut query = sqlx::query_as::<_, ModelRow>(&sql);
    let like = format!("%{keyword}%");
    if !keyword.is_empty() {
        query = query.bind(&like).bind(&like).bind(&like);
    }
    if let Some(filter) = vendor {
        query = match filter {
            VendorFilter::Id(id) => query.bind(*id),
            VendorFilter::Name(name) => query.bind(format!("%{name}%")),
        };
    }
    if let Some(value) = status {
        query = query.bind(value);
    }
    if let Some(value) = sync {
        query = query.bind(value);
    }
    query.fetch_all(&state.pool).await.map_err(internal)
}

async fn vendor_model_counts(state: &AppState) -> Result<Value, ApiError> {
    let rows: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT COALESCE(vendor_id, 0), COUNT(*) FROM models WHERE deleted_at IS NULL GROUP BY COALESCE(vendor_id, 0)",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    let mut counts = serde_json::Map::new();
    for (vendor_id, count) in rows {
        counts.insert(vendor_id.to_string(), json!(count));
    }
    Ok(Value::Object(counts))
}

async fn list_inner(
    state: &AppState,
    query: ModelQuery,
) -> Result<axum::response::Response, ApiError> {
    let (p, size, offset) = page(&query);
    let square_state = query.square_state.unwrap_or_default();
    if !matches!(
        square_state.as_str(),
        "" | "visible" | "unavailable" | "hidden" | "partial"
    ) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "Invalid model square state",
        ));
    }
    let keyword = query.keyword.unwrap_or_default();
    let vendor_raw = query.vendor.unwrap_or_default();
    let include_channel_models = query.include_channel_models.as_deref() == Some("true");
    let status = parse_status_filter(query.status.as_deref().unwrap_or(""));
    let sync = parse_sync_filter(query.sync_official.as_deref().unwrap_or(""));
    let vendor = parse_vendor_filter(&vendor_raw);

    let rows = fetch_meta(state, &keyword, vendor.as_ref(), status, sync).await?;
    // 规则归属与合成行去重看的是全量元数据，而不是当前筛选结果。
    let all_meta = fetch_meta(state, "", None, None, None).await?;
    let facts = channel_facts(state);
    let states = square_states(&all_meta, &facts);

    let mut items: Vec<Value> = rows
        .iter()
        .map(|row| row_json(Some(row), &row.model_name, &facts, &states))
        .collect();

    // status/sync/vendor 是元数据维度，渠道派生名无法表达，所以此时不做并集。
    let unionable =
        status.is_none() && sync.is_none() && (vendor_raw.is_empty() || vendor_raw == "0");
    if include_channel_models && unionable {
        let claimed: HashSet<&str> = all_meta
            .iter()
            .filter(|row| row.name_rule == NAME_RULE_EXACT)
            .map(|row| row.model_name.as_str())
            .collect();
        let needle = keyword.to_lowercase();
        let mut synthetic: Vec<&String> = facts
            .configured
            .keys()
            .filter(|name| !claimed.contains(name.as_str()))
            .filter(|name| needle.is_empty() || name.to_lowercase().contains(&needle))
            .collect();
        synthetic.sort();
        for name in synthetic {
            items.push(row_json(None, name, &facts, &states));
        }
    }

    let (total, items) = if square_state.is_empty() {
        let total = items.len();
        let page_items: Vec<Value> = items
            .into_iter()
            .skip(offset.max(0) as usize)
            .take(size as usize)
            .collect();
        (total, page_items)
    } else {
        // 展示状态依赖实时路由与规则，只能先全量富化再过滤、分页。
        let filtered: Vec<Value> = items
            .into_iter()
            .filter(|item| item["square_state"] == square_state)
            .collect();
        let total = filtered.len();
        let start = if p - 1 <= total as i64 / size {
            ((p - 1) * size) as usize
        } else {
            total
        };
        (total, filtered.into_iter().skip(start).take(size as usize).collect())
    };

    let counts = vendor_model_counts(state).await?;
    Ok(json_ok(&json!({
        "items": items,
        "total": total,
        "page": p,
        "page_size": size,
        "vendor_counts": counts,
    })))
}

pub async fn list(
    State(state): State<AppState>,
    Query(query): Query<ModelQuery>,
) -> Result<axum::response::Response, ApiError> {
    list_inner(&state, query).await
}

pub async fn search(
    State(state): State<AppState>,
    Query(query): Query<ModelQuery>,
) -> Result<axum::response::Response, ApiError> {
    list_inner(&state, query).await
}

pub async fn get_one(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<axum::response::Response, ApiError> {
    let row: Option<ModelRow> = sqlx::query_as(&format!("{SELECT} WHERE id=? AND deleted_at IS NULL"))
        .bind(id)
        .fetch_optional(&state.pool)
        .await
        .map_err(internal)?;
    let row = row.ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "model not found"))?;
    let all_meta = fetch_meta(&state, "", None, None, None).await?;
    let facts = channel_facts(&state);
    let states = square_states(&all_meta, &facts);
    Ok(json_ok(&row_json(Some(&row), &row.model_name, &facts, &states)))
}

pub async fn create(
    State(state): State<AppState>,
    Json(input): Json<ModelInput>,
) -> Result<axum::response::Response, ApiError> {
    let model_name = input
        .model_name
        .clone()
        .filter(|m| !m.trim().is_empty())
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "model_name is required"))?;
    let ts = now();
    let result = sqlx::query(
        "INSERT INTO models (model_name, description, icon, tags, vendor_id, endpoints, status, sync_official, created_time, updated_time, name_rule) VALUES (?,?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(&model_name)
    .bind(input.description.clone())
    .bind(input.icon.clone())
    .bind(input.tags.clone())
    .bind(input.vendor_id)
    .bind(input.endpoints.clone())
    .bind(input.status.unwrap_or(1))
    .bind(input.sync_official.unwrap_or(1))
    .bind(ts)
    .bind(ts)
    .bind(input.name_rule.unwrap_or(0))
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    get_one(State(state), Path(result.last_insert_rowid())).await
}

pub async fn update(
    State(state): State<AppState>,
    Query(query): Query<std::collections::HashMap<String, String>>,
    Json(input): Json<ModelInput>,
) -> Result<axum::response::Response, ApiError> {
    let id = input
        .id
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "id is required"))?;
    let current: Option<ModelRow> = sqlx::query_as(&format!("{SELECT} WHERE id=? AND deleted_at IS NULL"))
        .bind(id)
        .fetch_optional(&state.pool)
        .await
        .map_err(internal)?;
    let current = current.ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "model not found"))?;
    if query.contains_key("status_only") {
        sqlx::query("UPDATE models SET status=?, updated_time=? WHERE id=?")
            .bind(input.status.unwrap_or(current.status))
            .bind(now())
            .bind(id)
            .execute(&state.pool)
            .await
            .map_err(internal)?;
        return get_one(State(state), Path(id)).await;
    }
    sqlx::query(
        "UPDATE models SET model_name=?, description=?, icon=?, tags=?, vendor_id=?, endpoints=?, status=?, sync_official=?, name_rule=?, updated_time=? WHERE id=?",
    )
    .bind(input.model_name.unwrap_or(current.model_name))
    .bind(input.description.or(current.description))
    .bind(input.icon.or(current.icon))
    .bind(input.tags.or(current.tags))
    .bind(input.vendor_id.or(current.vendor_id))
    .bind(input.endpoints.or(current.endpoints))
    .bind(input.status.unwrap_or(current.status))
    .bind(input.sync_official.unwrap_or(current.sync_official))
    .bind(input.name_rule.unwrap_or(current.name_rule))
    .bind(now())
    .bind(id)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    get_one(State(state), Path(id)).await
}

pub async fn delete_one(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<axum::response::Response, ApiError> {
    let result = sqlx::query("UPDATE models SET deleted_at=datetime('now') WHERE id=? AND deleted_at IS NULL")
        .bind(id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    if result.rows_affected() == 0 {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "model not found"));
    }
    Ok(json_ok(&json!({"deleted": 1, "unbound": 0})))
}

pub async fn vendors(
    State(state): State<AppState>,
    Query(query): Query<ModelQuery>,
) -> Result<axum::response::Response, ApiError> {
    let (p, size, offset) = page(&query);
    let keyword = query.keyword.unwrap_or_default();
    let like = format!("%{keyword}%");

    let count_sql = if keyword.is_empty() {
        "SELECT COUNT(*) FROM vendors WHERE deleted_at IS NULL"
    } else {
        "SELECT COUNT(*) FROM vendors WHERE deleted_at IS NULL AND name LIKE ?"
    };
    let mut count_query = sqlx::query_as::<_, (i64,)>(count_sql);
    if !keyword.is_empty() {
        count_query = count_query.bind(&like);
    }
    let count: (i64,) = count_query
        .fetch_one(&state.pool)
        .await
        .map_err(internal)?;

    let select_sql = if keyword.is_empty() {
        "SELECT id, name, description, icon, status, created_time, updated_time FROM vendors WHERE deleted_at IS NULL ORDER BY id LIMIT ? OFFSET ?"
    } else {
        "SELECT id, name, description, icon, status, created_time, updated_time FROM vendors WHERE deleted_at IS NULL AND name LIKE ? ORDER BY id LIMIT ? OFFSET ?"
    };
    let mut rows_query = sqlx::query_as::<_, VendorRow>(select_sql);
    if !keyword.is_empty() {
        rows_query = rows_query.bind(&like);
    }
    let rows: Vec<VendorRow> = rows_query
        .bind(size)
        .bind(offset)
        .fetch_all(&state.pool)
        .await
        .map_err(internal)?;
    let items: Vec<Value> = rows
        .iter()
        .map(|row| {
            json!({
                "id": row.id,
                "name": row.name,
                "description": row.description,
                "icon": row.icon,
                "status": row.status,
                "created_time": row.created_time,
                "updated_time": row.updated_time,
            })
        })
        .collect();
    Ok(json_ok(&json!({
        "items": items,
        "total": count.0,
        "page": p,
        "page_size": size,
    })))
}

pub async fn enabled(
    State(state): State<AppState>,
) -> Result<axum::response::Response, ApiError> {
    let rows: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT model_name FROM models WHERE deleted_at IS NULL AND status=1 ORDER BY model_name",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    Ok(json_ok(&rows))
}

fn internal(e: sqlx::Error) -> ApiError {
    ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}
