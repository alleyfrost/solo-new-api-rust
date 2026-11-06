use axum::extract::{Extension, Query, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::Router;
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
    let mut sql = String::from(
        "SELECT created_at/86400*86400 AS day, COALESCE(model_name,'') AS model_name, COALESCE(username,'') AS username, \
         COALESCE(SUM(prompt_tokens),0)+COALESCE(SUM(completion_tokens),0) AS token_used, \
         COUNT(*) AS count, COALESCE(SUM(quota),0) AS quota FROM logs \
         WHERE type=2 AND created_at>=? AND created_at<=? \
         AND (?=0 OR user_id=?) AND (?='' OR username=?)",
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
