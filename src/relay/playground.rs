use axum::body::Bytes;
use axum::extract::{Extension, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use serde_json::Value;

use crate::auth::admin::AdminContext;
use crate::auth::token::TokenContext;
use crate::relay::forward::{self, Wire};
use crate::relay::{openai_error, passthrough};
use crate::state::AppState;

/// 会话凭证不能出现在上游请求里；`cookie` 不在 hop-by-hop 名单内，需在此显式剥离。
const STRIP_HEADERS: [&str; 3] = ["cookie", "authorization", "x-api-key"];

fn bad_request(message: impl Into<String>) -> Response {
    openai_error(StatusCode::BAD_REQUEST, message, "invalid_request_error")
}

fn forbidden(message: impl Into<String>) -> Response {
    openai_error(StatusCode::FORBIDDEN, message, "permission_error")
}

fn group_known(state: &AppState, group: &str) -> bool {
    let caches = state.caches.read().expect("cache lock");
    caches.abilities.keys().any(|(g, _)| g == group)
}

/// 把游乐场请求整理成可直接转发的三件套：临时令牌上下文、剥掉凭证的请求头、去掉 `group` 的请求体。
struct Prepared {
    ctx: TokenContext,
    headers: HeaderMap,
    body: Bytes,
}

fn prepare(
    state: &AppState,
    admin: &AdminContext,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Prepared, Box<Response>> {
    let mut payload = match serde_json::from_slice::<Value>(&body) {
        Ok(value) => value,
        Err(_) => return Err(Box::new(bad_request("invalid JSON body"))),
    };
    let Some(obj) = payload.as_object_mut() else {
        return Err(Box::new(bad_request("body must be a JSON object")));
    };
    // `group` 只用于选路，不属于上游协议字段，必须剥掉再转发。
    let requested = obj
        .remove("group")
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default();
    let requested = requested.trim().to_string();

    let user_group = {
        let caches = state.caches.read().expect("cache lock");
        caches
            .users
            .get(&admin.user_id)
            .map(|u| u.group.clone())
            .unwrap_or_default()
    };
    let group = if requested.is_empty() || requested == "auto" {
        user_group
    } else if requested == user_group || group_known(state, &requested) {
        requested
    } else {
        return Err(Box::new(forbidden(format!(
            "no access to group {requested}"
        ))));
    };

    let mut sanitized = headers.clone();
    for name in STRIP_HEADERS {
        sanitized.remove(name);
    }
    let Ok(outbound) = serde_json::to_vec(&payload) else {
        return Err(Box::new(openai_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "request re-encode failed",
            "internal_error",
        )));
    };
    // 与 Go 侧 controller.Playground 一致：临时令牌名 playground-<group>，日志按它归类。
    let ctx = TokenContext {
        user_id: admin.user_id,
        token_id: 0,
        token_name: format!("playground-{group}"),
        token_group: group,
        model_limits_enabled: false,
        model_limits: String::new(),
    };
    Ok(Prepared {
        ctx,
        headers: sanitized,
        body: Bytes::from(outbound),
    })
}

pub async fn chat_completions(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminContext>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let prepared = match prepare(&state, &admin, headers, body) {
        Ok(prepared) => prepared,
        Err(response) => return *response,
    };
    forward::relay(
        state,
        prepared.ctx,
        prepared.headers,
        prepared.body,
        Wire::Chat,
    )
    .await
}

/// 游乐场生图：选中的是生图模型时，前端直接发 `/v1/images/generations` 形状的请求。
pub async fn image_generations(
    State(state): State<AppState>,
    Extension(admin): Extension<AdminContext>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let prepared = match prepare(&state, &admin, headers, body) {
        Ok(prepared) => prepared,
        Err(response) => return *response,
    };
    passthrough::relay(
        state,
        prepared.ctx,
        prepared.headers,
        prepared.body,
        "/v1/images/generations",
    )
    .await
}
