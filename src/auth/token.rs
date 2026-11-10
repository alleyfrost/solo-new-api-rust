use std::time::Instant;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::middleware::Next;
use axum::response::Response;
use tokio::sync::OwnedSemaphorePermit;
use tokio_stream::StreamExt;

use crate::error::ApiError;
use crate::state::AppState;

#[derive(Debug, Clone)]
pub struct TokenContext {
    pub user_id: i64,
    pub token_id: i64,
    pub token_name: String,
    pub token_group: String,
    pub model_limits_enabled: bool,
    pub model_limits: String,
}

pub fn extract_key(headers: &HeaderMap, uri: &Uri) -> Option<String> {
    let raw = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim())
        .or_else(|| {
            headers
                .get("x-api-key")
                .and_then(|v| v.to_str().ok())
                .map(|v| v.trim())
        })
        .or_else(|| {
            uri.query()
                .and_then(|q| q.split('&').find_map(|p| p.strip_prefix("key=")))
        })
        .filter(|v| !v.is_empty())?;
    let raw = raw
        .strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))
        .unwrap_or(raw);
    let raw = raw.strip_prefix("sk-").unwrap_or(raw);
    Some(raw.split('-').next().unwrap_or(raw).to_string())
}

pub fn validate_token(state: &AppState, key: &str) -> Result<TokenContext, ApiError> {
    let caches = state.caches.read().expect("cache lock");
    let token = caches
        .token_by_key(key)
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "Invalid token"))?;
    let user = caches
        .users
        .get(&token.user_id)
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
        token_name: token.name.clone(),
        token_group: token.group.clone(),
        model_limits_enabled: token.model_limits_enabled,
        model_limits: token.model_limits.clone(),
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
    req.extensions_mut().insert(ctx.clone());

    // 每-token 限流门：只对会产生上游调用的 POST 生效（GET /v1/models 不转发）。
    // 并发许可必须覆盖流式响应体消费全程，拿到响应后把它挂进 body 生命周期，
    // 流结束或客户端断连（body 被丢弃）时自动释放。
    let permit = if req.method() == Method::GET {
        None
    } else if let Some(limiter) = state.rate_limits.get(ctx.token_id) {
        let deadline = Instant::now() + state.rate_limits.max_wait();
        match limiter.gate(deadline).await {
            Ok(permit) => permit,
            Err(()) => return Ok(rate_limited_response(req.uri().path())),
        }
    } else {
        None
    };

    let mut response = next.run(req).await;
    if let Some(permit) = permit {
        response = hold_permit_until_body_end(response, permit);
    }
    Ok(response)
}

fn rate_limited_response(path: &str) -> Response {
    let message = "Token rate limit exceeded, please retry later";
    if path == "/v1/messages" {
        crate::relay::anthropic_error(StatusCode::TOO_MANY_REQUESTS, message, "rate_limit_error")
    } else {
        crate::relay::openai_error(StatusCode::TOO_MANY_REQUESTS, message, "rate_limit_error")
    }
}

/// 把并发许可放进响应体流里：`async_stream::stream!` 把 `permit` 捕获进生成器，
/// 流被完整消费或中途丢弃（客户端断开）时生成器析构，许可随之释放。
fn hold_permit_until_body_end(response: Response, permit: OwnedSemaphorePermit) -> Response {
    let (parts, body) = response.into_parts();
    let stream = async_stream::stream! {
        let _permit = permit;
        let mut inner = body.into_data_stream();
        while let Some(item) = inner.next().await {
            yield item;
        }
    };
    Response::from_parts(parts, Body::from_stream(stream))
}
