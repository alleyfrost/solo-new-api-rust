use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{delete, get};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::{ApiError, json_ok};
use crate::state::AppState;

#[derive(Deserialize, Default)]
pub struct ModelQuery {
    pub p: Option<i64>,
    pub size: Option<i64>,
    pub page_size: Option<i64>,
    pub keyword: Option<String>,
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

const SELECT: &str = "SELECT id, model_name, description, icon, tags, vendor_id, endpoints, status, sync_official, created_time, updated_time, name_rule FROM models";

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
        "configured_channel_count": 0,
        "enable_groups": [],
        "bound_channels": [],
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

async fn list_inner(
    state: &AppState,
    query: ModelQuery,
) -> Result<axum::response::Response, ApiError> {
    let (p, size, offset) = page(&query);
    let keyword = query.keyword.unwrap_or_default();
    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM models WHERE deleted_at IS NULL AND (?='' OR model_name LIKE '%'||?||'%')",
    )
    .bind(&keyword)
    .bind(&keyword)
    .fetch_one(&state.pool)
    .await
    .map_err(internal)?;
    let rows: Vec<ModelRow> = sqlx::query_as(&format!(
        "{SELECT} WHERE deleted_at IS NULL AND (?='' OR model_name LIKE '%'||?||'%') ORDER BY id DESC LIMIT ? OFFSET ?",
    ))
    .bind(&keyword)
    .bind(&keyword)
    .bind(size)
    .bind(offset)
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    let items: Vec<Value> = rows.iter().map(model_json).collect();
    Ok(json_ok(&json!({
        "items": items,
        "total": count.0,
        "page": p,
        "page_size": size,
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
    Ok(json_ok(&model_json(&row)))
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

pub async fn vendors(Query(query): Query<ModelQuery>) -> axum::response::Response {
    let p = query.p.unwrap_or(1);
    let size = query.size.or(query.page_size).unwrap_or(10);
    json_ok(&json!({"items": [], "total": 0, "page": p, "page_size": size}))
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
