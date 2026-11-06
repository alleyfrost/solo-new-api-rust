use std::collections::HashSet;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use axum::body::{Body, Bytes};
use axum::extract::{Extension, State};
use axum::http::header::{CONTENT_TYPE, HeaderValue};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use serde_json::{Value, json};
use tokio_stream::StreamExt;

use crate::auth::token::TokenContext;
use crate::logging::{self, LogEntry};
use crate::relay::{self, SseTap, Usage};
use crate::state::AppState;
use crate::{retry, routing, upstream};

const PATH: &str = "/v1/chat/completions";

struct LogMeta {
    origin_model: String,
    stream: bool,
    channel_id: i64,
    channel_name: String,
    user_id: i64,
    username: String,
    token_id: i64,
    token_name: String,
    group: String,
    ip: String,
    request_id: String,
    upstream_request_id: String,
    started: Instant,
}

impl LogMeta {
    fn entry(&self, usage: Usage) -> LogEntry {
        LogEntry {
            created_at: now(),
            user_id: self.user_id,
            username: self.username.clone(),
            token_name: self.token_name.clone(),
            model_name: self.origin_model.clone(),
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
            use_time_secs: self.started.elapsed().as_secs() as i64,
            is_stream: self.stream,
            channel_id: self.channel_id,
            channel_name: self.channel_name.clone(),
            token_id: self.token_id,
            group: self.group.clone(),
            ip: self.ip.clone(),
            request_id: self.request_id.clone(),
            upstream_request_id: self.upstream_request_id.clone(),
            other: "{}".to_string(),
        }
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn model_allowed(ctx: &TokenContext, model: &str) -> bool {
    if !ctx.model_limits_enabled {
        return true;
    }
    ctx.model_limits.split(',').map(str::trim).any(|m| m == model)
}

fn upstream_request_id(headers: &HeaderMap) -> String {
    for name in [
        "x-request-id",
        "request-id",
        "x-oneapi-request-id",
        "x-upstream-request-id",
    ] {
        if let Some(v) = headers.get(name).and_then(|v| v.to_str().ok()) {
            return v.to_string();
        }
    }
    String::new()
}

fn response_with_ct(status: StatusCode, ct: Option<HeaderValue>, body: Body) -> Response {
    let mut builder = Response::builder().status(status);
    if let Some(ct) = ct {
        builder = builder.header(CONTENT_TYPE, ct);
    }
    builder.body(body).unwrap_or_else(|_| {
        relay::openai_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "response build failed",
            "internal_error",
        )
    })
}

fn upstream_error_response(status: StatusCode, bytes: &Bytes, ct: Option<HeaderValue>) -> Response {
    let is_json = ct
        .as_ref()
        .and_then(|v| v.to_str().ok())
        .map(|v| v.contains("json"))
        .unwrap_or(false);
    if is_json && !bytes.is_empty() {
        return response_with_ct(status, ct, Body::from(bytes.clone()));
    }
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(2048)]).to_string();
    relay::openai_error(status, format!("upstream error: {text}"), "upstream_error")
}

pub async fn chat_completions(
    State(state): State<AppState>,
    Extension(ctx): Extension<TokenContext>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let started = Instant::now();
    let Ok(payload) = serde_json::from_slice::<Value>(&body) else {
        return relay::openai_error(
            StatusCode::BAD_REQUEST,
            "invalid JSON body",
            "invalid_request_error",
        );
    };
    if !payload.is_object() {
        return relay::openai_error(
            StatusCode::BAD_REQUEST,
            "body must be a JSON object",
            "invalid_request_error",
        );
    }
    let Some(origin_model) = payload.get("model").and_then(Value::as_str).map(str::to_string)
    else {
        return relay::openai_error(
            StatusCode::BAD_REQUEST,
            "model is required",
            "invalid_request_error",
        );
    };
    if !model_allowed(&ctx, &origin_model) {
        return relay::openai_error(
            StatusCode::FORBIDDEN,
            format!("token has no access to model {origin_model}"),
            "permission_error",
        );
    }
    let stream = payload
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let wants_usage = payload
        .pointer("/stream_options/include_usage")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let (max_retry, group, username) = {
        let caches = state.caches.read().expect("cache lock");
        let max_retry = retry::max_retry_from_options(&caches.options);
        let user = caches.users.get(&ctx.user_id).cloned();
        let username = user
            .as_ref()
            .map(|u| u.username.clone())
            .unwrap_or_default();
        let group = if ctx.token_group.is_empty() || ctx.token_group == "auto" {
            user.map(|u| u.group)
                .unwrap_or_else(|| ctx.token_group.clone())
        } else {
            ctx.token_group.clone()
        };
        (max_retry, group, username)
    };
    let ip = relay::client_ip(&headers, state.config.trust_proxy_headers);
    let request_id = format!("req_{}", uuid::Uuid::new_v4().simple());
    let mut used: HashSet<(i64, usize)> = HashSet::new();
    let mut last_error: Option<(StatusCode, Bytes, Option<HeaderValue>)> = None;

    for attempt in 0..=max_retry {
        let selected = {
            let caches = state.caches.read().expect("cache lock");
            routing::select_channel(
                &caches,
                &state.routing,
                &group,
                &origin_model,
                attempt,
                &used,
            )
        };
        let Some(selected) = selected else {
            break;
        };
        let channel = {
            let caches = state.caches.read().expect("cache lock");
            caches.channels.get(&selected.channel_id).cloned()
        };
        let Some(channel) = channel else {
            continue;
        };
        used.insert((selected.channel_id, selected.key_index));

        let mut attempt_payload = payload.clone();
        let mapped = relay::map_model(&origin_model, channel.model_mapping.as_deref());
        if mapped != origin_model {
            attempt_payload["model"] = Value::String(mapped);
        }
        if stream && channel.r#type != 14 {
            let obj = attempt_payload.as_object_mut().expect("object");
            let options = obj
                .entry("stream_options".to_string())
                .or_insert_with(|| json!({}));
            if !options.is_object() {
                *options = json!({});
            }
            options
                .as_object_mut()
                .expect("object")
                .insert("include_usage".to_string(), Value::Bool(true));
        }
        let upstream_body = match serde_json::to_vec(&attempt_payload) {
            Ok(b) => b,
            Err(e) => {
                return relay::openai_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    e.to_string(),
                    "internal_error",
                );
            }
        };
        let url = upstream::build_url(
            channel.base_url.as_deref().unwrap_or(""),
            PATH,
            channel.r#type,
        );
        let upstream_headers =
            upstream::rewrite_headers(&headers, &selected.key, channel.header_override.as_deref());

        let response = match state
            .client
            .post(&url)
            .headers(upstream_headers)
            .body(upstream_body)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("upstream request failed: {e}");
                let error_body = serde_json::to_vec(&json!({
                    "error": {
                        "message": "upstream request failed",
                        "type": "upstream_error",
                        "param": null,
                        "code": null
                    }
                }))
                .unwrap_or_default();
                last_error = Some((
                    StatusCode::BAD_GATEWAY,
                    Bytes::from(error_body),
                    Some(HeaderValue::from_static("application/json")),
                ));
                if retry::decide(None, attempt, max_retry) == retry::RetryDecision::Retry {
                    tokio::time::sleep(retry::backoff(None, attempt)).await;
                    continue;
                }
                break;
            }
        };

        let status = response.status();
        if status.is_success() {
            let content_type = response
                .headers()
                .get(CONTENT_TYPE)
                .cloned()
                .or_else(|| {
                    Some(HeaderValue::from_static(if stream {
                        "text/event-stream"
                    } else {
                        "application/json"
                    }))
                });
            let upstream_req = upstream_request_id(response.headers());
            let meta = LogMeta {
                origin_model: origin_model.clone(),
                stream,
                channel_id: selected.channel_id,
                channel_name: selected.channel_name.clone(),
                user_id: ctx.user_id,
                username: username.clone(),
                token_id: ctx.token_id,
                token_name: ctx.token_name.clone(),
                group: group.clone(),
                ip: ip.clone(),
                request_id: request_id.clone(),
                upstream_request_id: upstream_req,
                started,
            };
            if stream {
                let log_state = state.clone();
                let wants = wants_usage;
                let mut source = response.bytes_stream();
                let stream_body = async_stream::stream! {
                    let mut tap = SseTap::new(wants);
                    while let Some(item) = source.next().await {
                        match item {
                            Ok(chunk) => {
                                let out = tap.feed(&chunk);
                                if !out.is_empty() {
                                    yield Ok::<Bytes, std::io::Error>(Bytes::from(out));
                                }
                            }
                            Err(e) => {
                                tracing::warn!("upstream stream error: {e}");
                                break;
                            }
                        }
                    }
                    let tail = tap.finish();
                    if !tail.is_empty() {
                        yield Ok::<Bytes, std::io::Error>(Bytes::from(tail));
                    }
                    logging::enqueue(&log_state, meta.entry(tap.usage())).await;
                };
                return response_with_ct(status, content_type, Body::from_stream(stream_body));
            } else {
                let bytes = match response.bytes().await {
                    Ok(b) => b,
                    Err(e) => {
                        logging::enqueue(&state, meta.entry(Usage::default())).await;
                        return relay::openai_error(
                            StatusCode::BAD_GATEWAY,
                            format!("read upstream body failed: {e}"),
                            "upstream_error",
                        );
                    }
                };
                let usage = serde_json::from_slice::<Value>(&bytes)
                    .ok()
                    .as_ref()
                    .and_then(relay::usage_from_json)
                    .unwrap_or_default();
                logging::enqueue(&state, meta.entry(usage)).await;
                return response_with_ct(status, content_type, Body::from(bytes));
            }
        }

        let content_type = response.headers().get(CONTENT_TYPE).cloned();
        let bytes = response.bytes().await.unwrap_or_default();
        last_error = Some((status, bytes, content_type));
        if matches!(status.as_u16(), 401 | 403 | 429) {
            routing::mark_key_unhealthy(&state.routing, selected.channel_id, selected.key_index);
        }
        if retry::decide(Some(status.as_u16()), attempt, max_retry)
            == retry::RetryDecision::Retry
        {
            tokio::time::sleep(retry::backoff(Some(status.as_u16()), attempt)).await;
            continue;
        }
        break;
    }

    match last_error {
        Some((status, bytes, ct)) => upstream_error_response(status, &bytes, ct),
        None => relay::openai_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "no available channel",
            "no_channel",
        ),
    }
}
