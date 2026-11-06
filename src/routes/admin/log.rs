use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::{Extension, Query, State};
use axum::http::StatusCode;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::{QueryBuilder, Sqlite, SqlitePool};

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

const LOG_SELECT: &str = "SELECT id, user_id, created_at, type AS log_type, COALESCE(content,'') AS content, \
     COALESCE(username,'') AS username, COALESCE(token_name,'') AS token_name, \
     COALESCE(model_name,'') AS model_name, COALESCE(quota,0) AS quota, \
     COALESCE(prompt_tokens,0) AS prompt_tokens, COALESCE(completion_tokens,0) AS completion_tokens, \
     COALESCE(use_time,0) AS use_time, COALESCE(is_stream,0) AS is_stream, \
     COALESCE(channel_id,0) AS channel_id, COALESCE(channel_name,'') AS channel_name, \
     COALESCE(token_id,0) AS token_id, COALESCE(`group`,'') AS group_name, \
     COALESCE(ip,'') AS ip, COALESCE(request_id,'') AS request_id, \
     COALESCE(upstream_request_id,'') AS upstream_request_id, COALESCE(other,'') AS other FROM logs";

fn log_json(r: &LogRow) -> Value {
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
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/log", get(list))
        .route("/api/log/", get(list))
        .route("/api/log/self", get(list_self))
        .route("/api/log/export", get(export))
        .route("/api/log/self/export", get(export_self))
        .route("/api/log/stat", get(stat))
        .route("/api/log/self/stat", get(stat_self))
        .route("/api/log/cleanup", axum::routing::delete(cleanup))
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

    let mut qb = QueryBuilder::<Sqlite>::new(LOG_SELECT);
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
    let items: Vec<Value> = rows.iter().map(log_json).collect();
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

/// `log_setting.max_range_days` 未设置时的一次导出最长时间跨度，与前端 `DEFAULT_MAX_RANGE_DAYS` 一致。
pub const MAX_EXPORT_DAYS: i64 = 365;

/// 导出分页大小：单页约千条日志，处理完即释放，避免整表驻留内存。
const EXPORT_PAGE_SIZE: i64 = 1_000;

/// 同一用户同时只允许一个导出；句柄离开作用域（含客户端断连丢流）即释放。
struct ExportSlot {
    busy: Arc<Mutex<HashSet<i64>>>,
    user_id: i64,
}

impl Drop for ExportSlot {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.busy.lock() {
            guard.remove(&self.user_id);
        }
    }
}

fn claim_export_slot(busy: &Arc<Mutex<HashSet<i64>>>, user_id: i64) -> Option<ExportSlot> {
    let mut guard = busy.lock().expect("export slots lock");
    // 不能写成 `insert(id).then_some(ExportSlot { .. })`：临时句柄会在 guard 仍持锁时析构，
    // Drop 再锁同一个非重入 Mutex，被拒绝的并发请求会当场死锁。
    if guard.insert(user_id) {
        Some(ExportSlot { busy: Arc::clone(busy), user_id })
    } else {
        None
    }
}

fn export_range(query: &LogQuery, max_days: i64) -> Result<(i64, i64), ApiError> {
    let max_secs = max_days * 24 * 60 * 60;
    let end = query
        .end_timestamp
        .filter(|v| *v > 0)
        .map_or_else(now, |v| v.min(now()));
    match query.start_timestamp.filter(|v| *v > 0) {
        Some(start) if end - start > max_secs => Err(ApiError::new(
            StatusCode::OK,
            format!("The time range cannot exceed {max_days} days"),
        )),
        Some(start) => Ok((start, end)),
        None => Ok((end - max_secs, end)),
    }
}

fn ndjson_line(value: &Value) -> axum::body::Bytes {
    let mut buf = serde_json::to_vec(value).unwrap_or_default();
    buf.push(b'\n');
    axum::body::Bytes::from(buf)
}

/// 按 id 游标分页取日志；每页独立持有连接，导出全程不缓存整张结果集。
async fn export_page(
    pool: &SqlitePool,
    query: &LogQuery,
    scoped_user: Option<i64>,
    before: Option<i64>,
) -> Result<Vec<LogRow>, sqlx::Error> {
    let mut qb = QueryBuilder::<Sqlite>::new(LOG_SELECT);
    push_filters(&mut qb, query, scoped_user);
    if let Some(id) = before {
        qb.push(" AND id < ").push_bind(id);
    }
    qb.push(" ORDER BY id DESC LIMIT ")
        .push_bind(EXPORT_PAGE_SIZE);
    qb.build_query_as::<LogRow>().fetch_all(pool).await
}

async fn export_inner(
    state: &AppState,
    caller: i64,
    query: LogQuery,
    scoped_user: Option<i64>,
) -> Result<Response, ApiError> {
    let settings = crate::settings::Settings::load(state).await;
    if scoped_user.is_some() && !settings.log_export_enabled {
        return Err(ApiError::new(
            StatusCode::OK,
            "Log export is disabled in system settings",
        ));
    }
    let (start, end) = export_range(&query, settings.log_max_range_days)?;
    let mut query = query;
    query.start_timestamp = Some(start);
    query.end_timestamp = Some(end);

    let slot = claim_export_slot(&state.export_slots, caller).ok_or_else(|| {
        ApiError::new(
            StatusCode::OK,
            "Another log export is running. Try again after it finishes.",
        )
    })?;

    let pool = state.pool.clone();
    let stream = async_stream::stream! {
        let mut count: i64 = 0;
        let mut failed = false;
        let mut before: Option<i64> = None;
        loop {
            match export_page(&pool, &query, scoped_user, before).await {
                Ok(rows) => {
                    let Some(last) = rows.last().map(|r| r.id) else { break };
                    before = Some(last);
                    let full = rows.len() as i64 == EXPORT_PAGE_SIZE;
                    for row in rows {
                        count += 1;
                        yield Ok::<_, std::io::Error>(ndjson_line(&log_json(&row)));
                    }
                    if !full { break; }
                }
                Err(e) => {
                    tracing::warn!("log export page failed: {e}");
                    failed = true;
                    break;
                }
            }
        }
        let result = if failed {
            json!({ "success": false, "message": "Failed to export logs" })
        } else {
            json!({ "success": true, "message": "", "data": { "count": count } })
        };
        yield Ok::<_, std::io::Error>(ndjson_line(&result));
        drop(slot);
    };

    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "application/x-ndjson; charset=utf-8")
        .header(CACHE_CONTROL, "no-store")
        .header("x-accel-buffering", "no")
        .body(Body::from_stream(stream))
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

pub async fn export(
    State(state): State<AppState>,
    Extension(ctx): Extension<AdminContext>,
    Query(query): Query<LogQuery>,
) -> Result<Response, ApiError> {
    export_inner(&state, ctx.user_id, query, None).await
}

pub async fn export_self(
    State(state): State<AppState>,
    Extension(ctx): Extension<AdminContext>,
    Query(query): Query<LogQuery>,
) -> Result<Response, ApiError> {
    let user_id = ctx.user_id;
    export_inner(&state, user_id, query, Some(user_id)).await
}

/// 一次清理最多删除的日志行数。分批而不是一条语句删空，
/// 是为了让长事务不长时间占住 WAL 写锁。
const CLEAN_DEFAULT_LIMIT: i64 = 50_000;
const CLEAN_MAX_LIMIT: i64 = 100_000;

#[derive(Deserialize, Default)]
struct CleanQuery {
    before: Option<i64>,
    limit: Option<i64>,
}

async fn cleanup(
    State(state): State<AppState>,
    Query(query): Query<CleanQuery>,
) -> Result<Response, ApiError> {
    let Some(before) = query.before.filter(|v| *v > 0) else {
        return Err(ApiError::new(
            StatusCode::OK,
            "A valid before timestamp is required",
        ));
    };
    if before >= now() {
        return Err(ApiError::new(
            StatusCode::OK,
            "The cutoff time must be in the past",
        ));
    }
    let limit = query
        .limit
        .filter(|v| *v > 0)
        .unwrap_or(CLEAN_DEFAULT_LIMIT)
        .min(CLEAN_MAX_LIMIT);
    let ids: Vec<i64> =
        sqlx::query_scalar("SELECT id FROM logs WHERE created_at < ? ORDER BY id LIMIT ?")
            .bind(before)
            .bind(limit)
            .fetch_all(&state.pool)
            .await
            .map_err(internal)?;

    let mut deleted = 0;
    if !ids.is_empty() {
        let mut qb: QueryBuilder<'_, Sqlite> = QueryBuilder::new("DELETE FROM logs WHERE id IN (");
        for (index, id) in ids.iter().enumerate() {
            if index > 0 {
                qb.push(",");
            }
            qb.push_bind(*id);
        }
        qb.push(")");
        deleted = qb
            .build()
            .execute(&state.pool)
            .await
            .map_err(internal)?
            .rows_affected() as i64;
    }

    let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM logs WHERE created_at < ?")
        .bind(before)
        .fetch_one(&state.pool)
        .await
        .map_err(internal)?;
    Ok(json_ok(&json!({
        "deleted": deleted,
        "remaining": remaining,
        "has_more": remaining > 0,
    })))
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
