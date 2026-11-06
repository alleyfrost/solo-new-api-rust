use crate::auth::admin::AdminContext;
use crate::auth::session;
use crate::error::{ApiError, json_ok};
use crate::state::AppState;
use axum::extract::{Extension, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

fn client_ip(headers: &HeaderMap) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(|v| v.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn set_cookie(sid: &str, refresh_plain: &str) -> String {
    format!("new_api_refresh={sid}.{refresh_plain}; Path=/; HttpOnly; SameSite=Lax; Max-Age=604800")
}

pub async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::Json(req): axum::Json<LoginRequest>,
) -> Result<Response, ApiError> {
    let user = {
        let caches = state.caches.read().expect("cache lock");
        caches
            .users
            .values()
            .find(|u| u.username == req.username)
            .cloned()
    };
    let Some(user) = user else {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "Invalid username or password",
        ));
    };
    if user.status != 1 || !session::verify_password(&user.password, &req.password) {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "Invalid username or password",
        ));
    }
    let ip = client_ip(&headers);
    let ua = headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let (bundle, refresh_plain) =
        session::create_session(&state.pool, &user, &ip, &ua, &state.jwt_secret).await?;
    let mut response = json_ok(&bundle);
    response.headers_mut().insert(
        "set-cookie",
        set_cookie(&bundle.session.sid, &refresh_plain)
            .parse()
            .unwrap(),
    );
    Ok(response)
}

pub async fn refresh(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let (sid, secret) = session::parse_refresh_cookie(&headers)
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "Session expired!"))?;
    let (bundle, next_plain) = {
        let users = state.caches.read().expect("cache lock").users.clone();
        session::refresh_session(&state.pool, &users, &sid, &secret, &state.jwt_secret).await?
    };
    let mut response = json_ok(&bundle);
    response
        .headers_mut()
        .insert("set-cookie", set_cookie(&sid, &next_plain).parse().unwrap());
    Ok(response)
}

pub async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let sid = session::parse_refresh_cookie(&headers)
        .map(|(sid, _)| sid)
        .or_else(|| {
            headers
                .get("x-auth-session")
                .and_then(|v| v.to_str().ok())
                .map(|v| v.to_string())
        });
    if let Some(sid) = sid {
        let _ = sqlx::query("UPDATE user_sessions SET status='revoked', revoked_at=?, revoked_reason='logout' WHERE sid=?")
            .bind(session::now()).bind(&sid).execute(&state.pool).await;
    }
    let mut response = json_ok(&serde_json::json!({}));
    response.headers_mut().insert(
        "set-cookie",
        "new_api_refresh=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0"
            .parse()
            .unwrap(),
    );
    response
}

pub async fn self_info(
    State(state): State<AppState>,
    Extension(ctx): Extension<AdminContext>,
) -> Result<Response, ApiError> {
    let user = {
        let caches = state.caches.read().expect("cache lock");
        caches.users.get(&ctx.user_id).cloned()
    };
    let Some(user) = user else {
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "Session expired!"));
    };
    let extra: Option<(Option<String>, Option<i64>, String)> = sqlx::query_as(
        "SELECT setting, created_at, password FROM users WHERE id=? AND deleted_at IS NULL",
    )
    .bind(ctx.user_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?;
    // 只记用量不记金额：表头三个数字取自日志聚合，而不是 users.quota / used_quota。
    // 缺 logs 表时按零处理，资料页本身不应因此不可用。
    let usage: (i64, i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(SUM(prompt_tokens),0), COALESCE(SUM(completion_tokens),0) \
         FROM logs WHERE type=2 AND user_id=?",
    )
    .bind(ctx.user_id)
    .fetch_one(&state.pool)
    .await
    .unwrap_or_default();
    let mut value = session::user_json(&user);
    if let Some(map) = value.as_object_mut() {
        map.insert("request_count".into(), serde_json::json!(usage.0));
        map.insert("prompt_tokens".into(), serde_json::json!(usage.1));
        map.insert("completion_tokens".into(), serde_json::json!(usage.2));
        if let Some((setting, created_at, password)) = extra {
            map.insert("setting".into(), serde_json::json!(setting.unwrap_or_default()));
            map.insert("created_time".into(), serde_json::json!(created_at.unwrap_or(0)));
            map.insert("has_password".into(), serde_json::json!(!password.is_empty()));
        }
    }
    Ok(json_ok(&value))
}

fn internal(e: sqlx::Error) -> ApiError {
    ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

pub async fn self_groups(State(state): State<AppState>) -> Response {
    let groups: Vec<(String,)> =
        sqlx::query_as("SELECT DISTINCT `group` FROM abilities ORDER BY `group`")
            .fetch_all(&state.pool)
            .await
            .unwrap_or_default();
    let mut data = serde_json::Map::new();
    // 只记用量不记金额：本部署不套用任何分组倍率，所以不返回 ratio，
    // 否则前端会把它渲染成一个并不存在的计价倍数。
    for (group,) in groups {
        data.insert(group, serde_json::json!({ "desc": "" }));
    }
    if data.is_empty() {
        data.insert("default".into(), serde_json::json!({ "desc": "" }));
    }
    json_ok(&serde_json::Value::Object(data))
}

/// Enabled models across abilities whose backing channel is enabled.
pub async fn user_models(State(state): State<AppState>) -> Response {
    let caches = state.caches.read().expect("cache lock");
    let mut models: Vec<String> = caches
        .abilities
        .iter()
        .filter(|(_, rows)| {
            rows.iter().any(|a| {
                a.enabled
                    && caches
                        .channels
                        .get(&a.channel_id)
                        .map(|c| c.status == 1)
                        .unwrap_or(false)
            })
        })
        .map(|((_, model), _)| model.clone())
        .collect();
    models.sort();
    models.dedup();
    json_ok(&models)
}

/// 只接受 Go 认识的三个字段：display_name、password（含 original_password）、
/// 以及写进 users.setting 的 language / sidebar_modules。
const SETTING_KEYS: [&str; 2] = ["language", "sidebar_modules"];

pub async fn update_self(
    State(state): State<AppState>,
    Extension(ctx): Extension<AdminContext>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> Result<Response, ApiError> {
    let new_password = body.get("password").and_then(serde_json::Value::as_str).unwrap_or("");
    if !new_password.is_empty() {
        let original = body.get("original_password").and_then(serde_json::Value::as_str).unwrap_or("");
        return change_password(&state, &ctx, new_password, original).await;
    }
    if let Some(display_name) = body.get("display_name").and_then(serde_json::Value::as_str) {
        let display_name = display_name.trim();
        if display_name.chars().count() > 20 {
            return Err(ApiError::new(
                StatusCode::OK,
                "Display name must be at most 20 characters",
            ));
        }
        sqlx::query("UPDATE users SET display_name=? WHERE id=? AND deleted_at IS NULL")
            .bind(display_name)
            .bind(ctx.user_id)
            .execute(&state.pool)
            .await
            .map_err(internal)?;
        if let Some(user) = state.caches.write().expect("cache lock").users.get_mut(&ctx.user_id) {
            user.display_name = Some(display_name.to_string());
        }
        return Ok(json_ok(&serde_json::Value::Null));
    }
    let mut patch = serde_json::Map::new();
    for key in SETTING_KEYS {
        if let Some(value) = body.get(key).and_then(serde_json::Value::as_str) {
            patch.insert(key.to_string(), serde_json::Value::String(value.to_string()));
        }
    }
    if patch.is_empty() {
        return Ok(json_ok(&serde_json::Value::Null));
    }
    write_setting_patch(&state, ctx.user_id, &patch).await?;
    Ok(json_ok(&serde_json::Value::Null))
}

pub async fn update_setting(
    State(state): State<AppState>,
    Extension(ctx): Extension<AdminContext>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> Result<Response, ApiError> {
    let Some(patch) = body.as_object() else {
        return Err(ApiError::new(StatusCode::OK, "Invalid parameters"));
    };
    write_setting_patch(&state, ctx.user_id, patch).await?;
    Ok(json_ok(&serde_json::Value::Null))
}

/// setting 是 Go 与本进程共享的 JSON 字段，按键合并而不是整体覆盖，
/// 这样 Go 侧写入的、本进程不认识的键不会被抹掉。
async fn write_setting_patch(
    state: &AppState,
    user_id: i64,
    patch: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), ApiError> {
    let current: Option<(Option<String>,)> = sqlx::query_as(
        "SELECT setting FROM users WHERE id=? AND deleted_at IS NULL",
    )
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?;
    let Some((raw,)) = current else {
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "Session expired!"));
    };
    let mut merged = raw
        .filter(|text| !text.trim().is_empty())
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    for (key, value) in patch {
        merged.insert(key.clone(), value.clone());
    }
    let encoded = serde_json::to_string(&serde_json::Value::Object(merged))
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    sqlx::query("UPDATE users SET setting=? WHERE id=?")
        .bind(&encoded)
        .bind(user_id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    Ok(())
}

async fn change_password(
    state: &AppState,
    ctx: &AdminContext,
    new_password: &str,
    original_password: &str,
) -> Result<Response, ApiError> {
    session::validate_new_password(new_password)?;
    let stored: Option<(String, i64)> = sqlx::query_as(
        "SELECT password, auth_version FROM users WHERE id=? AND deleted_at IS NULL",
    )
    .bind(ctx.user_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?;
    let Some((hash, auth_version)) = stored else {
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "Session expired!"));
    };
    // 账号原本没有密码时不设原密码门槛，与 Go 的 firstPassword 分支一致。
    if !hash.is_empty() && !session::verify_password(&hash, original_password) {
        return Err(ApiError::new(StatusCode::OK, "Original password is incorrect"));
    }
    let row: Option<(i64, String, String, String, String, i64, i64)> = sqlx::query_as(
        "SELECT version, status, login_method, ip, user_agent, created_at, expires_at FROM user_sessions WHERE sid=? AND user_id=?",
    )
    .bind(&ctx.sid)
    .bind(ctx.user_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?;
    let Some((version, status, method, ip, user_agent, created_at, expires_at)) = row else {
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "Session expired!"));
    };
    if status != "active" {
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "Session expired!"));
    }

    let new_hash = session::hash_password(new_password)?;
    let next_version = auth_version + 1;
    let now = session::now();
    sqlx::query("UPDATE users SET password=?, auth_version=? WHERE id=?")
        .bind(&new_hash)
        .bind(next_version)
        .bind(ctx.user_id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    // auth_version 抬高会让其他会话的 JWT 立即失效；当前会话跟着升到新版本。
    sqlx::query("UPDATE user_sessions SET user_auth_version=?, last_active_at=? WHERE sid=?")
        .bind(next_version)
        .bind(now)
        .bind(&ctx.sid)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    if let Some(user) = state.caches.write().expect("cache lock").users.get_mut(&ctx.user_id) {
        user.password = new_hash;
        user.auth_version = next_version;
    }

    let access_expires_at = now + 15 * 60;
    let claims = session::Claims {
        sub: ctx.user_id,
        sid: ctx.sid.clone(),
        ver: version,
        uver: next_version,
        iat: now,
        exp: access_expires_at,
    };
    let data = serde_json::json!({
        "access_token": session::issue_access(&claims, &state.jwt_secret)?,
        "token_type": "Bearer",
        "access_expires_at": access_expires_at,
        "has_password": true,
        "session": session::SessionView {
            sid: ctx.sid.clone(),
            current: true,
            login_method: method,
            ip,
            user_agent,
            created_at,
            last_active_at: now,
            expires_at,
        },
    });
    Ok(json_ok(&data))
}

pub async fn list_sessions(
    State(state): State<AppState>,
    Extension(ctx): Extension<AdminContext>,
) -> Result<Response, ApiError> {
    let now = session::now();
    let rows: Vec<(String, String, String, String, i64, i64, i64)> = sqlx::query_as(
        "SELECT sid, login_method, ip, user_agent, created_at, last_active_at, expires_at FROM user_sessions \
         WHERE user_id=? AND status='active' AND expires_at>? \
           AND user_auth_version=(SELECT auth_version FROM users WHERE id=?) \
         ORDER BY last_active_at DESC, created_at DESC LIMIT 100",
    )
    .bind(ctx.user_id)
    .bind(now)
    .bind(ctx.user_id)
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    let mut views: Vec<session::SessionView> = rows
        .into_iter()
        .map(|(sid, login_method, ip, user_agent, created_at, last_active_at, expires_at)| {
            session::SessionView {
                current: sid == ctx.sid,
                sid,
                login_method,
                ip,
                user_agent,
                created_at,
                last_active_at,
                expires_at,
            }
        })
        .collect();
    // Go 把当前会话排在列表首位。
    views.sort_by_key(|view| !view.current);
    Ok(json_ok(&views))
}

pub async fn revoke_session(
    State(state): State<AppState>,
    Extension(ctx): Extension<AdminContext>,
    Path(sid): Path<String>,
) -> Result<Response, ApiError> {
    let sid = sid.trim().to_string();
    if sid.is_empty() {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "session id is required")
            .with_code("AUTH_SESSION_ID_REQUIRED"));
    }
    let now = session::now();
    let result = sqlx::query(
        "UPDATE user_sessions SET status='revoked', revoked_at=?, revoked_reason='user_revoked' \
         WHERE sid=? AND user_id=? AND status='active' AND expires_at>?",
    )
    .bind(now)
    .bind(&sid)
    .bind(ctx.user_id)
    .bind(now)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    if result.rows_affected() != 1 {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "session not found")
            .with_code("AUTH_SESSION_NOT_FOUND"));
    }
    let current = sid == ctx.sid;
    let mut response = json_ok(&serde_json::json!({ "revoked_sid": sid, "current": current }));
    if current {
        response.headers_mut().insert(
            "set-cookie",
            "new_api_refresh=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0".parse().unwrap(),
        );
    }
    Ok(response)
}

pub async fn revoke_other_sessions(
    State(state): State<AppState>,
    Extension(ctx): Extension<AdminContext>,
) -> Result<Response, ApiError> {
    let result = sqlx::query(
        "UPDATE user_sessions SET status='revoked', revoked_at=?, revoked_reason='user_revoked_others' \
         WHERE user_id=? AND sid<>? AND status='active'",
    )
    .bind(session::now())
    .bind(ctx.user_id)
    .bind(&ctx.sid)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    Ok(json_ok(&serde_json::json!({ "revoked_count": result.rows_affected() })))
}

