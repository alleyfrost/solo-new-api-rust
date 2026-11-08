use axum::Router;
use axum::extract::{Extension, Query, State};
use axum::http::StatusCode;
use axum::routing::get;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::auth::admin::AdminContext;
use crate::error::{ApiError, json_ok};
use crate::state::AppState;

#[derive(Deserialize, Default)]
pub struct DataQuery {
    pub start_timestamp: Option<i64>,
    pub end_timestamp: Option<i64>,
    pub default_time: Option<String>,
    pub username: Option<String>,
    pub token_id: Option<i64>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/data", get(data))
        .route("/api/data/self", get(data_self))
        .route("/api/data/users", get(users))
        .route("/api/data/flow", get(flow))
        .route("/api/data/flow/self", get(flow_self))
        .route("/api/uptime/status", get(uptime))
}

fn range(query: &DataQuery) -> (i64, i64) {
    let now = now();
    let start = query.start_timestamp.unwrap_or(now - 7 * 86400);
    let end = query.end_timestamp.unwrap_or(now);
    (start, end)
}

async fn grouped(
    state: &AppState,
    query: &DataQuery,
    user_id: Option<i64>,
    by_user: bool,
) -> Result<axum::response::Response, ApiError> {
    let (start, end) = range(query);
    let user_filter = user_id.unwrap_or(0);
    let username = query.username.clone().unwrap_or_default();
    let token_filter = query.token_id.unwrap_or(0);
    // 聚合表命中就直接返回；缺失/为空时回退到 logs（兼容旧库与首轮回填前）。
    let mut quota_sql = String::from(
        "SELECT created_at/86400*86400 AS day, COALESCE(model_name,'') AS model_name, COALESCE(username,'') AS username, \
         COALESCE(SUM(token_used),0) AS token_used, COALESCE(SUM(cache_read_tokens),0) AS cache_read_tokens, \
         COALESCE(SUM(cache_creation_tokens),0) AS cache_creation_tokens, COALESCE(SUM(count),0) AS count, \
         COALESCE(SUM(quota),0) AS quota FROM quota_data \
         WHERE created_at>=? AND created_at<=? AND (?=0 OR user_id=?) AND (?='' OR username=?) AND (?=0 OR token_id=?)",
    );
    if by_user {
        quota_sql.push_str(" GROUP BY day, username ORDER BY day ASC");
    } else {
        quota_sql.push_str(" GROUP BY day, model_name, username ORDER BY day ASC");
    }
    if let Ok(rows) = sqlx::query(&quota_sql)
        .bind(start)
        .bind(end)
        .bind(user_filter)
        .bind(user_filter)
        .bind(&username)
        .bind(&username)
        .bind(token_filter)
        .bind(token_filter)
        .fetch_all(&state.pool)
        .await
        && !rows.is_empty()
    {
        let items: Vec<Value> = rows
            .iter()
            .map(|row| {
                use sqlx::Row;
                json!({
                    "created_at": row.get::<i64, _>("day"),
                    "model_name": row.get::<String, _>("model_name"),
                    "username": row.get::<String, _>("username"),
                    "token_used": row.get::<i64, _>("token_used"),
                    "cache_read_tokens": row.get::<i64, _>("cache_read_tokens"),
                    "cache_creation_tokens": row.get::<i64, _>("cache_creation_tokens"),
                    "count": row.get::<i64, _>("count"),
                    "quota": row.get::<i64, _>("quota"),
                })
            })
            .collect();
        return Ok(json_ok(&json!(items)));
    }
    let mut sql = String::from(
        "SELECT created_at/86400*86400 AS day, COALESCE(model_name,'') AS model_name, COALESCE(username,'') AS username, \
         COALESCE(SUM(prompt_tokens),0)+COALESCE(SUM(completion_tokens),0) AS token_used, \
         COALESCE(SUM(CASE WHEN json_valid(other) THEN json_extract(other,'$.cache_tokens') END),0) AS cache_read_tokens, \
         COALESCE(SUM(CASE WHEN json_valid(other) THEN json_extract(other,'$.cache_creation_tokens') END),0) AS cache_creation_tokens, \
         COUNT(*) AS count, COALESCE(SUM(quota),0) AS quota FROM logs \
         WHERE type=2 AND created_at>=? AND created_at<=? \
         AND (?=0 OR user_id=?) AND (?='' OR username=?) AND (?=0 OR token_id=?)",
    );
    if by_user {
        sql.push_str(" GROUP BY day, username ORDER BY day ASC");
    } else {
        sql.push_str(" GROUP BY day, model_name, username ORDER BY day ASC");
    }
    let rows = sqlx::query(&sql)
        .bind(start)
        .bind(end)
        .bind(user_filter)
        .bind(user_filter)
        .bind(&username)
        .bind(&username)
        .bind(token_filter)
        .bind(token_filter)
        .fetch_all(&state.pool)
        .await
        .map_err(internal)?;
    let items: Vec<Value> = rows
        .iter()
        .map(|row| {
            use sqlx::Row;
            json!({
                "created_at": row.get::<i64, _>("day"),
                "model_name": row.get::<String, _>("model_name"),
                "username": row.get::<String, _>("username"),
                "token_used": row.get::<i64, _>("token_used"),
                "cache_read_tokens": row.get::<i64, _>("cache_read_tokens"),
                "cache_creation_tokens": row.get::<i64, _>("cache_creation_tokens"),
                "count": row.get::<i64, _>("count"),
                "quota": row.get::<i64, _>("quota"),
            })
        })
        .collect();
    Ok(json_ok(&json!(items)))
}

async fn flow_data(
    state: &AppState,
    query: &DataQuery,
    user_id: Option<i64>,
) -> Result<axum::response::Response, ApiError> {
    let (start, end) = range(query);
    let user_filter = user_id.unwrap_or(0);
    let username = query.username.clone().unwrap_or_default();
    if let Ok(rows) = sqlx::query(
        "SELECT q.user_id, COALESCE(q.username,'') AS username, COALESCE(q.token_id,0) AS token_id, \
         COALESCE(t.name,'') AS token_name, COALESCE(q.channel_id,0) AS channel_id, \
         COALESCE(c.name,'') AS channel_name, COALESCE(q.model_name,'') AS model_name, \
         COALESCE(q.use_group,'') AS group_name, COALESCE(SUM(q.token_used),0) AS token_used, \
         COALESCE(SUM(q.count),0) AS count, COALESCE(SUM(q.quota),0) AS quota \
         FROM quota_data q LEFT JOIN tokens t ON t.id=q.token_id LEFT JOIN channels c ON c.id=q.channel_id \
         WHERE q.created_at>=? AND q.created_at<=? AND (?=0 OR q.user_id=?) AND (?='' OR q.username=?) \
         GROUP BY q.user_id, q.username, q.token_id, q.use_group, q.channel_id, q.model_name \
         ORDER BY token_used DESC",
    )
    .bind(start)
    .bind(end)
    .bind(user_filter)
    .bind(user_filter)
    .bind(&username)
    .bind(&username)
    .fetch_all(&state.pool)
    .await
    && !rows.is_empty()
    {
        let items: Vec<Value> = rows
                .iter()
                .map(|row| {
                    use sqlx::Row;
                    json!({
                        "user_id": row.get::<i64, _>("user_id"),
                        "username": row.get::<String, _>("username"),
                        "token_id": row.get::<i64, _>("token_id"),
                        "token_name": row.get::<String, _>("token_name"),
                        "channel_id": row.get::<i64, _>("channel_id"),
                        "channel_name": row.get::<String, _>("channel_name"),
                        "model_name": row.get::<String, _>("model_name"),
                        "use_group": row.get::<String, _>("group_name"),
                        "token_used": row.get::<i64, _>("token_used"),
                        "count": row.get::<i64, _>("count"),
                        "quota": row.get::<i64, _>("quota"),
                    })
                })
                .collect();
            return Ok(json_ok(&json!(items)));
    }
    let rows = sqlx::query(
        "SELECT user_id, COALESCE(username,'') AS username, token_id, COALESCE(token_name,'') AS token_name, channel_id, COALESCE(channel_name,'') AS channel_name, COALESCE(model_name,'') AS model_name, \
         COALESCE(`group`,'') AS group_name, \
         COALESCE(SUM(prompt_tokens),0)+COALESCE(SUM(completion_tokens),0) AS token_used, \
         COUNT(*) AS count, COALESCE(SUM(quota),0) AS quota FROM logs \
         WHERE type=2 AND created_at>=? AND created_at<=? \
         AND (?=0 OR user_id=?) AND (?='' OR username=?) \
         GROUP BY user_id, username, token_id, token_name, channel_id, channel_name, model_name, `group` \
         ORDER BY token_used DESC",
    )
    .bind(start)
    .bind(end)
    .bind(user_filter)
    .bind(user_filter)
    .bind(&username)
    .bind(&username)
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    let items: Vec<Value> = rows
        .iter()
        .map(|row| {
            use sqlx::Row;
            json!({
                "user_id": row.get::<i64, _>("user_id"),
                "username": row.get::<String, _>("username"),
                "token_id": row.get::<i64, _>("token_id"),
                "token_name": row.get::<String, _>("token_name"),
                "channel_id": row.get::<i64, _>("channel_id"),
                "channel_name": row.get::<String, _>("channel_name"),
                "model_name": row.get::<String, _>("model_name"),
                "use_group": row.get::<String, _>("group_name"),
                "token_used": row.get::<i64, _>("token_used"),
                "count": row.get::<i64, _>("count"),
                "quota": row.get::<i64, _>("quota"),
            })
        })
        .collect();
    Ok(json_ok(&json!(items)))
}

pub async fn data(
    State(state): State<AppState>,
    Query(query): Query<DataQuery>,
) -> Result<axum::response::Response, ApiError> {
    grouped(&state, &query, None, false).await
}

pub async fn data_self(
    State(state): State<AppState>,
    Extension(ctx): Extension<AdminContext>,
    Query(query): Query<DataQuery>,
) -> Result<axum::response::Response, ApiError> {
    grouped(&state, &query, Some(ctx.user_id), false).await
}

pub async fn users(
    State(state): State<AppState>,
    Query(query): Query<DataQuery>,
) -> Result<axum::response::Response, ApiError> {
    grouped(&state, &query, None, true).await
}

pub async fn flow(
    State(state): State<AppState>,
    Query(query): Query<DataQuery>,
) -> Result<axum::response::Response, ApiError> {
    flow_data(&state, &query, None).await
}

pub async fn flow_self(
    State(state): State<AppState>,
    Extension(ctx): Extension<AdminContext>,
    Query(query): Query<DataQuery>,
) -> Result<axum::response::Response, ApiError> {
    flow_data(&state, &query, Some(ctx.user_id)).await
}

pub async fn uptime() -> axum::response::Response {
    json_ok(&json!([]))
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
