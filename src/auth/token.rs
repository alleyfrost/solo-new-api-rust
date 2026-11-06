use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::middleware::Next;
use axum::response::Response;
use crate::error::ApiError;
use crate::state::AppState;

#[derive(Debug, Clone)]
pub struct TokenContext {
    pub user_id: i64,
    pub token_id: i64,
    pub token_group: String,
}

pub fn extract_key(headers: &HeaderMap, uri: &Uri) -> Option<String> {
    let raw = headers.get("authorization").and_then(|v| v.to_str().ok())
        .map(|v| v.trim())
        .or_else(|| headers.get("x-api-key").and_then(|v| v.to_str().ok()).map(|v| v.trim()))
        .or_else(|| uri.query().and_then(|q| q.split('&')
            .find_map(|p| p.strip_prefix("key="))))
        .filter(|v| !v.is_empty())?;
    let raw = raw.strip_prefix("Bearer ").or_else(|| raw.strip_prefix("bearer ")).unwrap_or(raw);
    let raw = raw.strip_prefix("sk-").unwrap_or(raw);
    Some(raw.split('-').next().unwrap_or(raw).to_string())
}

pub fn validate_token(state: &AppState, key: &str) -> Result<TokenContext, ApiError> {
    let caches = state.caches.read().expect("cache lock");
    let token = caches.token_by_key(key)
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "Invalid token"))?;
    let user = caches.users.get(&token.user_id)
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "Invalid token"))?;
    if user.status != 1 || token.status != 1 {
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "Invalid token"));
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    if token.expired_time != -1 && token.expired_time < now {
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "Token expired"));
    }
    Ok(TokenContext {
        user_id: token.user_id,
        token_id: token.id,
        token_group: token.group.clone(),
    })
}

pub async fn token_auth(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let key = extract_key(req.headers(), req.uri())
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "Token not provided"))?;
    let ctx = validate_token(&state, &key)?;
    req.extensions_mut().insert(ctx);
    Ok(next.run(req).await)
}
