use axum::extract::{Extension, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::json;
use crate::auth::token::TokenContext;
use crate::error::ApiError;
use crate::state::{AppState, Caches};

pub fn available_models(caches: &Caches, ctx: &TokenContext) -> Vec<String> {
    let mut models: Vec<String> = caches.abilities.iter()
        .filter(|((group, _), _)| group == &ctx.token_group)
        .filter(|((_, _), rows)| rows.iter().any(|a| {
            a.enabled && caches.channels.get(&a.channel_id).map(|c| c.status == 1).unwrap_or(false)
        }))
        .map(|((_, model), _)| model.clone())
        .collect();
    models.sort();
    models.dedup();
    if ctx.model_limits_enabled {
        let allowed: std::collections::HashSet<&str> = ctx.model_limits.split(',').map(str::trim).collect();
        models.retain(|m| allowed.contains(m.as_str()));
    }
    models
}

fn wants_anthropic(headers: &HeaderMap) -> bool {
    headers.contains_key("anthropic-version") || headers.contains_key("x-api-key")
}

pub async fn list_models(State(state): State<AppState>, Extension(ctx): Extension<TokenContext>, headers: HeaderMap) -> Response {
    let caches = state.caches.read().expect("cache lock");
    let models = available_models(&caches, &ctx);
    if wants_anthropic(&headers) {
        let data: Vec<serde_json::Value> = models.iter().map(|m| json!({
            "type": "model", "id": m, "display_name": m, "created_at": "1970-01-01T00:00:00Z"
        })).collect();
        return (
            StatusCode::OK,
            axum::Json(json!({
                "data": data, "has_more": false,
                "first_id": models.first().map(String::as_str).unwrap_or(""),
                "last_id": models.last().map(String::as_str).unwrap_or("")
            })),
        )
            .into_response();
    }
    // new-api 将模型数组直接放在顶层 data；再包一层会让 OpenAI 客户端拿到字符串。
    let data: Vec<serde_json::Value> = models.iter().map(|m| json!({
        "id": m, "object": "model", "created": 0, "owned_by": "system"
    })).collect();
    (
        StatusCode::OK,
        axum::Json(json!({ "success": true, "data": data, "object": "list" })),
    )
        .into_response()
}

pub async fn retrieve_model(
    State(state): State<AppState>,
    Extension(ctx): Extension<TokenContext>,
    Path(model): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let caches = state.caches.read().expect("cache lock");
    let models = available_models(&caches, &ctx);
    if !models.iter().any(|m| m == &model) {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "model not found"));
    }
    let body = if wants_anthropic(&headers) {
        json!({
            "type": "model", "id": model, "display_name": model,
            "created_at": "1970-01-01T00:00:00Z"
        })
    } else {
        json!({ "id": model, "object": "model", "created": 0, "owned_by": "system" })
    };
    Ok((StatusCode::OK, axum::Json(body)).into_response())
}
