use axum::extract::{Extension, Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use std::collections::HashMap;
use rand::Rng;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::auth::admin::AdminContext;
use crate::error::{ApiError, json_ok};
use crate::model::TokenRow;
use crate::state::AppState;

#[derive(Deserialize, Default)]
pub struct PageQuery {
    pub p: Option<i64>,
    pub size: Option<i64>,
    pub keyword: Option<String>,
    pub token: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct TokenInput {
    pub id: Option<i64>,
    pub name: Option<String>,
    pub key: Option<String>,
    pub remain_quota: Option<i64>,
    pub expired_time: Option<i64>,
    pub unlimited_quota: Option<bool>,
    pub model_limits_enabled: Option<bool>,
    pub model_limits: Option<String>,
    pub allow_ips: Option<String>,
    pub allow_user_quota_query: Option<bool>,
    pub group: Option<String>,
    pub auto_groups: Option<Vec<String>>,
    pub cross_group_retry: Option<bool>,
    pub status: Option<i64>,
}

#[derive(Deserialize)]
pub struct IdsInput {
    pub ids: Vec<i64>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/token", get(list).post(create).put(update))
        .route("/api/token/", get(list).post(create).put(update))
        .route("/api/token/search", get(search))
        .route("/api/token/auto-groups", get(auto_groups))
        .route("/api/token/batch", post(batch_delete))
        .route("/api/token/batch/keys", post(batch_keys))
        .route("/api/token/{id}", get(get_one))
        .route("/api/token/{id}/", axum::routing::delete(delete_one))
        .route("/api/token/{id}/key", post(get_key))
}

fn mask_key(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() <= 4 {
        return "*".repeat(chars.len());
    }
    if chars.len() <= 8 {
        let head: String = chars[..2].iter().collect();
        let tail: String = chars[chars.len() - 2..].iter().collect();
        return format!("{head}****{tail}");
    }
    let head: String = chars[..4].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}**********{tail}")
}

fn auto_groups_value(raw: &Option<String>) -> Value {
    match raw {
        Some(text) if !text.trim().is_empty() => serde_json::from_str::<Value>(text)
            .ok()
            .filter(Value::is_array)
            .unwrap_or(Value::Null),
        _ => Value::Null,
    }
}

type TokenUsage = (i64, i64, i64);

async fn token_usage(state: &AppState, user_id: i64) -> HashMap<i64, TokenUsage> {
    let rows: Result<Vec<(i64, i64, i64, i64)>, _> = sqlx::query_as(
        "SELECT q.token_id, COALESCE(SUM(q.count),0), COALESCE(SUM(q.token_used),0), \
         COALESCE(SUM(q.cache_read_tokens),0) FROM quota_data q \
         JOIN tokens t ON t.id=q.token_id \
         WHERE t.user_id=? AND t.deleted_at IS NULL GROUP BY q.token_id",
    )
    .bind(user_id)
    .fetch_all(&state.pool)
    .await;
    let rows = match rows {
        Ok(rows) if !rows.is_empty() => rows,
        _ => sqlx::query_as(
            "SELECT token_id, COUNT(*), \
             COALESCE(SUM(MAX(prompt_tokens,0)+MAX(completion_tokens,0)),0), \
             COALESCE(SUM(CASE WHEN json_valid(other) THEN COALESCE(json_extract(other,'$.cache_tokens'),0) ELSE 0 END),0) \
             FROM logs WHERE type=2 AND user_id=? AND token_id>0 GROUP BY token_id",
        )
        .bind(user_id)
        .fetch_all(&state.pool)
        .await
        .unwrap_or_default(),
    };
    rows.into_iter()
        .map(|(id, requests, tokens, cache)| (id, (requests, tokens, cache)))
        .collect()
}

fn token_json_with_usage(
    token: &TokenRow,
    full_key: bool,
    usage: Option<&TokenUsage>,
) -> Value {
    let mut value = token_json(token, full_key);
    let (requests, used_tokens, cache_read) = usage.copied().unwrap_or((0, 0, 0));
    value["request_count"] = json!(requests);
    value["used_tokens"] = json!(used_tokens);
    value["cache_read_tokens"] = json!(cache_read);
    value
}

fn token_json(token: &TokenRow, full_key: bool) -> Value {
    json!({
        "id": token.id,
        "user_id": token.user_id,
        "key": if full_key { token.key.clone() } else { mask_key(&token.key) },
        "status": token.status,
        "name": token.name,
        "created_time": token.created_time.unwrap_or(0),
        "accessed_time": token.accessed_time.unwrap_or(0),
        "expired_time": token.expired_time,
        "remain_quota": token.remain_quota,
        "used_quota": token.used_quota.unwrap_or(0),
        "unlimited_quota": token.unlimited_quota,
        "model_limits_enabled": token.model_limits_enabled,
        "model_limits": token.model_limits,
        "allow_ips": token.allow_ips.clone().unwrap_or_default(),
        "group": token.group,
        "auto_groups": auto_groups_value(&token.auto_groups),
        "cross_group_retry": token.cross_group_retry,
        "allow_user_quota_query": token.allow_user_quota_query.unwrap_or(true),
    })
}

fn page_of(p: Option<i64>, size: Option<i64>) -> (i64, i64, i64) {
    let page = p.unwrap_or(1).max(1);
    let size = size.unwrap_or(10).clamp(1, 200);
    (page, size, (page - 1) * size)
}

fn generate_key() -> String {
    const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut rng = rand::thread_rng();
    (0..48)
        .map(|_| CHARS[rng.gen_range(0..CHARS.len())] as char)
        .collect()
}

/// 自填密钥的长度边界：下限挡住一猜就中的短串，上限相对列宽 `varchar(128)`
/// 与随机生成的 48 位留余量。
const KEY_MIN_LEN: usize = 16;
const KEY_MAX_LEN: usize = 64;

/// 归一化自填密钥：容忍直接粘贴完整 `sk-...`（认证侧本来就会剥一层前缀），
/// 只接受字母、数字、下划线。`-` 必须禁用——`auth::token::extract_key` 与 Go 的
/// `strings.Split(key, "-")[0]` 都在第一个连字符处截断，含 `-` 的密钥会认错。
/// 返回空串表示“未提供”，由调用方回落随机生成。
fn normalize_key(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    let key = trimmed.strip_prefix("sk-").unwrap_or(trimmed);
    if key.is_empty() {
        return Ok(String::new());
    }
    if key.chars().count() < KEY_MIN_LEN || key.chars().count() > KEY_MAX_LEN {
        return Err(format!(
            "Token key must be {KEY_MIN_LEN}-{KEY_MAX_LEN} characters"
        ));
    }
    if !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err("Token key may only contain letters, digits or underscore".to_string());
    }
    Ok(key.to_string())
}

/// 前端只用 `message_key` 查本地词条，所以错误里要同时带英文原文。
fn key_error(message: impl Into<String>) -> ApiError {
    let message = message.into();
    ApiError::new(StatusCode::BAD_REQUEST, message.clone()).with_message_key(message)
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

async fn fetch_token(state: &AppState, user_id: i64, id: i64) -> Result<TokenRow, ApiError> {
    let row: Option<TokenRow> = sqlx::query_as(
        "SELECT id, user_id, `key`, status, name, created_time, accessed_time, expired_time, remain_quota, unlimited_quota, model_limits_enabled, model_limits, allow_ips, `group`, auto_groups, cross_group_retry, used_quota, allow_user_quota_query FROM tokens WHERE id=? AND user_id=? AND deleted_at IS NULL",
    )
    .bind(id)
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    row.ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "token not found"))
}

async fn list_page(
    state: &AppState,
    user_id: i64,
    keyword: Option<String>,
    key_like: Option<String>,
    query: PageQuery,
) -> Result<axum::response::Response, ApiError> {
    let (page, size, offset) = page_of(query.p, query.size);
    let keyword = keyword.unwrap_or_default();
    let key_like = key_like.unwrap_or_default();
    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM tokens WHERE user_id=? AND deleted_at IS NULL \
         AND (? = '' OR name LIKE '%'||?||'%' OR `key` LIKE '%'||?||'%') \
         AND (? = '' OR `key` LIKE '%'||?||'%')",
    )
    .bind(user_id)
    .bind(&keyword)
    .bind(&keyword)
    .bind(&keyword)
    .bind(&key_like)
    .bind(&key_like)
    .fetch_one(&state.pool)
    .await
    .map_err(internal)?;
    let rows: Vec<TokenRow> = sqlx::query_as(
        "SELECT id, user_id, `key`, status, name, created_time, accessed_time, expired_time, remain_quota, unlimited_quota, model_limits_enabled, model_limits, allow_ips, `group`, auto_groups, cross_group_retry, used_quota, allow_user_quota_query FROM tokens WHERE user_id=? AND deleted_at IS NULL \
         AND (? = '' OR name LIKE '%'||?||'%' OR `key` LIKE '%'||?||'%') \
         AND (? = '' OR `key` LIKE '%'||?||'%') \
         ORDER BY id DESC LIMIT ? OFFSET ?",
    )
    .bind(user_id)
    .bind(&keyword)
    .bind(&keyword)
    .bind(&keyword)
    .bind(&key_like)
    .bind(&key_like)
    .bind(size)
    .bind(offset)
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    let usage = token_usage(state, user_id).await;
    let items: Vec<Value> = rows
        .iter()
        .map(|t| token_json_with_usage(t, false, usage.get(&t.id)))
        .collect();
    Ok(json_ok(&json!({
        "items": items,
        "total": count.0,
        "page": page,
        "page_size": size,
    })))
}

fn internal(e: sqlx::Error) -> ApiError {
    ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

pub async fn list(
    State(state): State<AppState>,
    Extension(ctx): Extension<AdminContext>,
    Query(query): Query<PageQuery>,
) -> Result<axum::response::Response, ApiError> {
    let page = PageQuery {
        p: query.p,
        size: query.size,
        keyword: None,
        token: None,
    };
    list_page(&state, ctx.user_id, None, None, page).await
}

pub async fn search(
    State(state): State<AppState>,
    Extension(ctx): Extension<AdminContext>,
    Query(query): Query<PageQuery>,
) -> Result<axum::response::Response, ApiError> {
    let keyword = query.keyword.clone();
    let token = query.token.clone();
    list_page(&state, ctx.user_id, keyword, token, query).await
}

pub async fn get_one(
    State(state): State<AppState>,
    Extension(ctx): Extension<AdminContext>,
    Path(id): Path<i64>,
) -> Result<axum::response::Response, ApiError> {
    let token = fetch_token(&state, ctx.user_id, id).await?;
    Ok(json_ok(&token_json(&token, false)))
}

pub async fn auto_groups(
    State(state): State<AppState>,
    Extension(ctx): Extension<AdminContext>,
) -> axum::response::Response {
    let group = {
        let caches = state.caches.read().expect("cache lock");
        caches
            .users
            .get(&ctx.user_id)
            .map(|u| u.group.clone())
            .unwrap_or_else(|| "default".to_string())
    };
    json_ok(&json!({ "groups": [group], "max_count": 1 }))
}

pub async fn create(
    State(state): State<AppState>,
    Extension(ctx): Extension<AdminContext>,
    Json(input): Json<TokenInput>,
) -> Result<axum::response::Response, ApiError> {
    let name = input
        .name
        .clone()
        .filter(|n| !n.trim().is_empty())
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "name is required"))?;
    if name.chars().count() > 50 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "Token name is too long",
        ));
    }
    let key = match input.key.as_deref() {
        Some(raw) => normalize_key(raw).map_err(key_error)?,
        None => String::new(),
    };
    let key = if key.is_empty() {
        generate_key()
    } else {
        // 唯一索引 `idx_tokens_key` 不区分软删行，所以这里也查全表：
        // 与已删除令牌重键同样会被库拒绝。
        let clash: Option<i64> =
            sqlx::query_scalar("SELECT id FROM tokens WHERE `key` = ? LIMIT 1")
                .bind(&key)
                .fetch_optional(&state.pool)
                .await
                .map_err(internal)?;
        if clash.is_some() {
            return Err(key_error("Token key is already in use"));
        }
        key
    };
    let ts = now();
    let auto_groups = input
        .auto_groups
        .as_ref()
        .filter(|g| !g.is_empty())
        .map(|g| serde_json::to_string(g).unwrap_or_default());
    let result = sqlx::query(
        "INSERT INTO tokens (user_id, `key`, status, name, created_time, accessed_time, expired_time, remain_quota, unlimited_quota, model_limits_enabled, model_limits, allow_ips, `group`, auto_groups, cross_group_retry, used_quota, allow_user_quota_query) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(ctx.user_id)
    .bind(&key)
    .bind(input.status.unwrap_or(1))
    .bind(&name)
    .bind(ts)
    .bind(ts)
    .bind(input.expired_time.unwrap_or(-1))
    .bind(input.remain_quota.unwrap_or(0))
    .bind(input.unlimited_quota.unwrap_or(true))
    .bind(input.model_limits_enabled.unwrap_or(false))
    .bind(input.model_limits.clone().unwrap_or_default())
    .bind(input.allow_ips.clone().unwrap_or_default())
    .bind(input.group.clone().unwrap_or_else(|| "default".to_string()))
    .bind(auto_groups)
    .bind(input.cross_group_retry.unwrap_or(false))
    .bind(0_i64)
    .bind(input.allow_user_quota_query.unwrap_or(true))
    .execute(&state.pool)
    .await
    .map_err(|e| match &e {
        // 上面已经查过重复，这里只兜并发写入撞 `idx_tokens_key` 的情况。
        sqlx::Error::Database(db) if db.is_unique_violation() => key_error("Token key is already in use"),
        _ => internal(e),
    })?;
    let id = result.last_insert_rowid();
    state.reload_caches().await.map_err(internal)?;
    let token = fetch_token(&state, ctx.user_id, id).await?;
    Ok(json_ok(&token_json(&token, false)))
}

pub async fn update(
    State(state): State<AppState>,
    Extension(ctx): Extension<AdminContext>,
    Query(query): Query<std::collections::HashMap<String, String>>,
    Json(input): Json<TokenInput>,
) -> Result<axum::response::Response, ApiError> {
    let id = input
        .id
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "id is required"))?;
    let current = fetch_token(&state, ctx.user_id, id).await?;
    if query.contains_key("status_only") {
        sqlx::query("UPDATE tokens SET status=? WHERE id=? AND user_id=?")
            .bind(input.status.unwrap_or(current.status))
            .bind(id)
            .bind(ctx.user_id)
            .execute(&state.pool)
            .await
            .map_err(internal)?;
        state.reload_caches().await.map_err(internal)?;
        let token = fetch_token(&state, ctx.user_id, id).await?;
        return Ok(json_ok(&token_json(&token, false)));
    }
    // 密钥只在创建时可指定，更新一律忽略 `input.key`：换密钥会让已下发的客户端
    // 配置静默失效，要改就新建一个再删旧的。
    let name = input.name.clone().unwrap_or(current.name);
    if name.chars().count() > 50 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "Token name is too long",
        ));
    }
    let auto_groups = match input.auto_groups.as_ref() {
        Some(groups) if groups.is_empty() => None,
        Some(groups) => Some(serde_json::to_string(groups).unwrap_or_default()),
        None => current.auto_groups.clone(),
    };
    sqlx::query(
        "UPDATE tokens SET name=?, expired_time=?, remain_quota=?, unlimited_quota=?, model_limits_enabled=?, model_limits=?, allow_ips=?, `group`=?, auto_groups=?, cross_group_retry=?, allow_user_quota_query=? WHERE id=? AND user_id=?",
    )
    .bind(&name)
    .bind(input.expired_time.unwrap_or(current.expired_time))
    .bind(input.remain_quota.unwrap_or(current.remain_quota))
    .bind(input.unlimited_quota.unwrap_or(current.unlimited_quota))
    .bind(input.model_limits_enabled.unwrap_or(current.model_limits_enabled))
    .bind(input.model_limits.clone().unwrap_or(current.model_limits))
    .bind(input.allow_ips.clone().or(current.allow_ips).unwrap_or_default())
    .bind(input.group.clone().unwrap_or(current.group))
    .bind(auto_groups)
    .bind(input.cross_group_retry.unwrap_or(current.cross_group_retry))
    .bind(input.allow_user_quota_query.or(current.allow_user_quota_query).unwrap_or(true))
    .bind(id)
    .bind(ctx.user_id)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    state.reload_caches().await.map_err(internal)?;
    let token = fetch_token(&state, ctx.user_id, id).await?;
    Ok(json_ok(&token_json(&token, false)))
}

pub async fn delete_one(
    State(state): State<AppState>,
    Extension(ctx): Extension<AdminContext>,
    Path(id): Path<i64>,
) -> Result<axum::response::Response, ApiError> {
    let _ = fetch_token(&state, ctx.user_id, id).await?;
    sqlx::query("UPDATE tokens SET deleted_at=datetime('now') WHERE id=? AND user_id=?")
        .bind(id)
        .bind(ctx.user_id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    state.reload_caches().await.map_err(internal)?;
    Ok(json_ok(&Value::Null))
}

pub async fn batch_delete(
    State(state): State<AppState>,
    Extension(ctx): Extension<AdminContext>,
    Json(input): Json<IdsInput>,
) -> Result<axum::response::Response, ApiError> {
    let mut affected = 0_u64;
    for id in input.ids {
        let result = sqlx::query(
            "UPDATE tokens SET deleted_at=datetime('now') WHERE id=? AND user_id=? AND deleted_at IS NULL",
        )
        .bind(id)
        .bind(ctx.user_id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
        affected += result.rows_affected();
    }
    state.reload_caches().await.map_err(internal)?;
    Ok(json_ok(&affected))
}

pub async fn get_key(
    State(state): State<AppState>,
    Extension(ctx): Extension<AdminContext>,
    Path(id): Path<i64>,
) -> Result<axum::response::Response, ApiError> {
    let token = fetch_token(&state, ctx.user_id, id).await?;
    Ok(json_ok(&json!({ "key": token.key })))
}

pub async fn batch_keys(
    State(state): State<AppState>,
    Extension(ctx): Extension<AdminContext>,
    Json(input): Json<IdsInput>,
) -> Result<axum::response::Response, ApiError> {
    let mut keys = serde_json::Map::new();
    for id in input.ids {
        if let Ok(token) = fetch_token(&state, ctx.user_id, id).await {
            keys.insert(id.to_string(), Value::String(token.key));
        }
    }
    Ok(json_ok(&json!({ "keys": keys })))
}
