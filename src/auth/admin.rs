use crate::auth::session;
use crate::error::ApiError;
use crate::state::AppState;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::Response;

#[derive(Debug, Clone)]
pub struct AdminContext {
    pub user_id: i64,
    pub sid: String,
}

/// 桌面本地模式下挑一个启用的管理员（单用户库只有 root）；无管理员时拒绝请求。
pub fn desktop_admin(state: &AppState) -> Option<(i64, crate::model::UserRow)> {
    let caches = state.caches.read().expect("cache lock");
    caches
        .users
        .values()
        .filter(|user| user.role >= 10 && user.status == 1)
        .min_by_key(|user| user.id)
        .map(|user| (user.id, user.clone()))
}

pub async fn admin_auth(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    if state.disable_login {
        let Some((user_id, _)) = desktop_admin(&state) else {
            return Err(ApiError::new(
                StatusCode::UNAUTHORIZED,
                "No administrator account is available",
            ));
        };
        req.extensions_mut().insert(AdminContext {
            user_id,
            sid: "desktop".to_string(),
        });
        return Ok(next.run(req).await);
    }
    let header = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "Session expired!"))?;
    let token = header.strip_prefix("Bearer ").unwrap_or(header);
    let claims = session::decode_access(token, &state.jwt_secret)?;

    let row: Option<(i64, String, i64, i64, i64)> = sqlx::query_as(
        "SELECT s.user_id, s.status, s.version, s.user_auth_version, s.expires_at FROM user_sessions s WHERE s.sid=?"
    ).bind(&claims.sid).fetch_optional(&state.pool).await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let Some((session_user_id, status, version, user_auth_version, expires_at)) = row else {
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "Session expired!"));
    };
    if session_user_id != claims.sub
        || status != "active"
        || version != claims.ver
        || user_auth_version != claims.uver
        || expires_at <= session::now()
    {
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "Session expired!"));
    }
    {
        let caches = state.caches.read().expect("cache lock");
        let user = caches
            .users
            .get(&claims.sub)
            .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "Session expired!"))?;
        if user.status != 1 || user.auth_version != claims.uver {
            return Err(ApiError::new(StatusCode::UNAUTHORIZED, "Session expired!"));
        }
    }
    req.extensions_mut().insert(AdminContext {
        user_id: claims.sub,
        sid: claims.sid,
    });
    Ok(next.run(req).await)
}
