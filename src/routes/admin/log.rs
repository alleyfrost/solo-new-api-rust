use axum::extract::{Extension, Query, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::Router;
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::QueryBuilder;
use sqlx::Sqlite;

use crate::auth::admin::AdminContext;
use crate::error::{ApiError, json_ok};
use crate::state::AppState;

#[derive(Deserialize, Default)]
pub struct LogQuery {
    pub p: Option<i64>,
    pub page_size: Option<i64>,
    pub r#type: Option<i64>,
    pub username: Option<String>,
    pub token_name: Option<String>,
    pub model_name: Option<String>,
    pub start_timestamp: Option<i64>,
    pub end_timestamp: Option<i64>,
    pub channel: Option<i64>,
    pub group: Option<String>,
    pub request_id: Option<String>,
    pub upstream_request_id: Option<String>,
}

#[derive(sqlx::FromRow)]
struct LogRow {
    id: i64,
    user_id: i64,
    created_at: i64,
    log_type: i64,
    content: String,
    username: String,
    token_name: String,
    model_name: String,
    quota: i64,
    prompt_tokens: i64,
    completion_tokens: i64,
    use_time: i64,
    is_stream: bool,
    channel_id: i64,
    channel_name: String,
    token_id: i64,
    group_name: String,
    ip: String,
    request_id: String,
    upstream_request_id: String,
    other: String,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/log", get(list))
        .route("/api/log/", get(list))
        .route("/api/log/self", get(list_self))
        .route("/api/log/stat", get(stat))
        .route("/api/log/self/stat", get(stat_self))
}

fn page(query: &LogQuery) -> (i64, i64, i64) {
    let p = query.p.unwrap_or(1).max(1);
    let size = query.page_size.unwrap_or(20).clamp(1, 200);
    (p, size, (p - 1) * size)
}

fn push_filters(qb: &mut QueryBuilder<'_, Sqlite>, query: &LogQuery, user_id: Option<i64>) {
    qb.push(" WHERE 1=1");
    if let Some(user_id) = user_id {
        qb.push(" AND user_id = ").push_bind(user_id);
    }
    if let Some(t) = query.r#type.filter(|t| *t > 0) {
        qb.push(" AND type = ").push_bind(t);
    }
    if let Some(v) = query.username.as_deref().filter(|v| !v.is_empty()) {
        qb.push(" AND username = ").push_bind(v.to_string());
    }
    if let Some(v) = query.token_name.as_deref().filter(|v| !v.is_empty()) {
        qb.push(" AND token_name = ").push_bind(v.to_string());
    }
    if let Some(v) = query.model_name.as_deref().filter(|v| !v.is_empty()) {
        qb.push(" AND model_name = ").push_bind(v.to_string());
    }
    if let Some(v) = query.start_timestamp.filter(|v| *v > 0) {
        qb.push(" AND created_at >= ").push_bind(v);
    }
    if let Some(v) = query.end_timestamp.filter(|v| *v > 0) {
        qb.push(" AND created_at <= ").push_bind(v);
    }
    if let Some(v) = query.channel.filter(|v| *v > 0) {
        qb.push(" AND channel_id = ").push_bind(v);
    }
    if let Some(v) = query.group.as_deref().filter(|v| !v.is_empty()) {
        qb.push(" AND `group` = ").push_bind(v.to_string());
    }
    if let Some(v) = query.request_id.as_deref().filter(|v| !v.is_empty()) {
        qb.push(" AND request_id = ").push_bind(v.to_string());
    }
    if let Some(v) = query
        .upstream_request_id
        .as_deref()
        .filter(|v| !v.is_empty())
    {
        qb.push(" AND upstream_request_id = ").push_bind(v.to_string());
    }
}

async fn list_inner(
    state: &AppState,
    query: LogQuery,
    user_id: Option<i64>,
) -> Result<axum::response::Response, ApiError> {
    let (p, size, offset) = page(&query);
    let mut count_qb = QueryBuilder::<Sqlite>::new("SELECT COUNT(*) FROM logs");
    push_filters(&mut count_qb, &query, user_id);
    let total: (i64,) = count_qb
        .build_query_as()
        .fetch_one(&state.pool)
        .await
        .map_err(internal)?;

    let mut qb = QueryBuilder::<Sqlite>::new(
        "SELECT id, user_id, created_at, type AS log_type, COALESCE(content,'') AS content, \
         COALESCE(username,'') AS username, COALESCE(token_name,'') AS token_name, \
         COALESCE(model_name,'') AS model_name, COALESCE(quota,0) AS quota, \
         COALESCE(prompt_tokens,0) AS prompt_tokens, COALESCE(completion_tokens,0) AS completion_tokens, \
         COALESCE(use_time,0) AS use_time, COALESCE(is_stream,0) AS is_stream, \
         COALESCE(channel_id,0) AS channel_id, COALESCE(channel_name,'') AS channel_name, \
         COALESCE(token_id,0) AS token_id, COALESCE(`group`,'') AS group_name, \
         COALESCE(ip,'') AS ip, COALESCE(request_id,'') AS request_id, \
         COALESCE(upstream_request_id,'') AS upstream_request_id, COALESCE(other,'') AS other FROM logs",
    );
    push_filters(&mut qb, &query, user_id);
    qb.push(" ORDER BY id DESC LIMIT ")
        .push_bind(size)
        .push(" OFFSET ")
        .push_bind(offset);
    let rows: Vec<LogRow> = qb
        .build_query_as()
        .fetch_all(&state.pool)
        .await
        .map_err(internal)?;
    let items: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.id,
                "user_id": r.user_id,
                "created_at": r.created_at,
                "type": r.log_type,
                "content": r.content,
                "username": r.username,
                "token_name": r.token_name,
                "model_name": r.model_name,
                "quota": r.quota,
                "prompt_tokens": r.prompt_tokens,
                "completion_tokens": r.completion_tokens,
                "use_time": r.use_time,
                "is_stream": r.is_stream,
                "channel": r.channel_id,
                "channel_name": r.channel_name,
                "token_id": r.token_id,
                "group": r.group_name,
                "ip": r.ip,
                "request_id": r.request_id,
                "upstream_request_id": r.upstream_request_id,
                "other": r.other,
            })
        })
        .collect();
    Ok(json_ok(&json!({
        "items": items,
        "total": total.0,
        "page": p,
        "page_size": size,
    })))
}

async fn stat_inner(
    state: &AppState,
    query: LogQuery,
    user_id: Option<i64>,
) -> Result<axum::response::Response, ApiError> {
    let mut quota_qb =
        QueryBuilder::<Sqlite>::new("SELECT COALESCE(SUM(quota),0) FROM logs");
    push_filters(&mut quota_qb, &query, user_id);
    let quota: (i64,) = quota_qb
        .build_query_as()
        .fetch_one(&state.pool)
        .await
        .map_err(internal)?;

    let mut rate_qb = QueryBuilder::<Sqlite>::new(
        "SELECT COUNT(*), COALESCE(SUM(prompt_tokens),0) + COALESCE(SUM(completion_tokens),0) FROM logs",
    );
    push_filters(&mut rate_qb, &query, user_id);
    let sixty = now() - 60;
    rate_qb
        .push(" AND created_at >= ")
        .push_bind(sixty)
        .push(" AND type = 2");
    let (rpm, tpm): (i64, i64) = rate_qb
        .build_query_as()
        .fetch_one(&state.pool)
        .await
        .map_err(internal)?;
    Ok(json_ok(&json!({
        "quota": quota.0,
        "rpm": rpm,
        "tpm": tpm,
    })))
}

pub async fn list(
    State(state): State<AppState>,
    Query(query): Query<LogQuery>,
) -> Result<axum::response::Response, ApiError> {
    list_inner(&state, query, None).await
}

pub async fn list_self(
    State(state): State<AppState>,
    Extension(ctx): Extension<AdminContext>,
    Query(query): Query<LogQuery>,
) -> Result<axum::response::Response, ApiError> {
    list_inner(&state, query, Some(ctx.user_id)).await
}

pub async fn stat(
    State(state): State<AppState>,
    Query(query): Query<LogQuery>,
) -> Result<axum::response::Response, ApiError> {
    stat_inner(&state, query, None).await
}

pub async fn stat_self(
    State(state): State<AppState>,
    Extension(ctx): Extension<AdminContext>,
    Query(query): Query<LogQuery>,
) -> Result<axum::response::Response, ApiError> {
    stat_inner(&state, query, Some(ctx.user_id)).await
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn internal(e: sqlx::Error) -> ApiError {
    ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}
