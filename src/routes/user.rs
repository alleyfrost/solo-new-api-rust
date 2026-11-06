use axum::extract::{Extension, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use crate::auth::admin::AdminContext;
use crate::auth::session;
use crate::error::{json_ok, ApiError};
use crate::state::AppState;

#[derive(Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

fn client_ip(headers: &HeaderMap) -> String {
    headers.get("x-forwarded-for").and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next()).map(|v| v.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn set_cookie(sid: &str, refresh_plain: &str) -> String {
    format!(
        "new_api_refresh={sid}.{refresh_plain}; Path=/; HttpOnly; SameSite=Lax; Max-Age=604800"
    )
}

pub async fn login(State(state): State<AppState>, headers: HeaderMap, axum::Json(req): axum::Json<LoginRequest>) -> Result<Response, ApiError> {
    let user = {
        let caches = state.caches.read().expect("cache lock");
        caches.users.values().find(|u| u.username == req.username).cloned()
    };
    let Some(user) = user else {
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "Invalid username or password"));
    };
    if user.status != 1 || !session::verify_password(&user.password, &req.password) {
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "Invalid username or password"));
    }
    let ip = client_ip(&headers);
    let ua = headers.get("user-agent").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let (bundle, refresh_plain) = session::create_session(&state.pool, &user, &ip, &ua, &state.jwt_secret).await?;
    let mut response = json_ok(&bundle);
    response.headers_mut().insert("set-cookie", set_cookie(&bundle.session.sid, &refresh_plain).parse().unwrap());
    Ok(response)
}

pub async fn refresh(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, ApiError> {
    let (sid, secret) = session::parse_refresh_cookie(&headers)
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "Session expired!"))?;
    let (bundle, next_plain) = {
        let users = state.caches.read().expect("cache lock").users.clone();
        session::refresh_session(&state.pool, &users, &sid, &secret, &state.jwt_secret).await?
    };
    let mut response = json_ok(&bundle);
    response.headers_mut().insert("set-cookie", set_cookie(&sid, &next_plain).parse().unwrap());
    Ok(response)
}

pub async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let sid = session::parse_refresh_cookie(&headers).map(|(sid, _)| sid)
        .or_else(|| headers.get("x-auth-session").and_then(|v| v.to_str().ok()).map(|v| v.to_string()));
    if let Some(sid) = sid {
        let _ = sqlx::query("UPDATE user_sessions SET status='revoked', revoked_at=?, revoked_reason='logout' WHERE sid=?")
            .bind(session::now()).bind(&sid).execute(&state.pool).await;
    }
    let mut response = json_ok(&serde_json::json!({}));
    response.headers_mut().insert("set-cookie",
        "new_api_refresh=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0".parse().unwrap());
    response
}

pub async fn self_info(State(state): State<AppState>, Extension(ctx): Extension<AdminContext>) -> Response {
    let caches = state.caches.read().expect("cache lock");
    match caches.users.get(&ctx.user_id) {
        Some(user) => json_ok(&session::user_json(user)),
        None => ApiError::new(StatusCode::UNAUTHORIZED, "Session expired!").into_response(),
    }
}
