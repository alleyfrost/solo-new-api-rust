use axum::extract::{Extension, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use serde_json::json;
use crate::auth::token::TokenContext;
use crate::error::{json_ok, ApiError};
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
        return json_ok(&json!({
            "data": data, "has_more": false,
            "first_id": models.first(), "last_id": models.last()
        }));
    }
    let data: Vec<serde_json::Value> = models.iter().map(|m| json!({
        "id": m, "object": "model", "created": 0, "owned_by": "system"
    })).collect();
    json_ok(&json!({ "object": "list", "data": data }))
}

pub async fn retrieve_model(State(state): State<AppState>, Extension(ctx): Extension<TokenContext>, Path(model): Path<String>) -> Result<Response, ApiError> {
    let caches = state.caches.read().expect("cache lock");
    let models = available_models(&caches, &ctx);
    if models.iter().any(|m| m == &model) {
        Ok(json_ok(&json!({ "id": model, "object": "model", "created": 0, "owned_by": "system" })))
    } else {
        Err(ApiError::new(StatusCode::NOT_FOUND, "model not found"))
    }
}
