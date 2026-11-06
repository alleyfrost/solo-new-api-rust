use std::collections::HashSet;
use std::sync::OnceLock;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use axum::body::{Body, Bytes};
use axum::http::header::{CONTENT_TYPE, HeaderValue};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use serde_json::{Value, json};
use switchyard_translation::{
    StreamTranslationState, TranslationEngine, TranslationPolicy, WireFormat,
};
use tokio_stream::StreamExt;

use crate::auth::token::TokenContext;
use crate::logging::{self, LogEntry};
use crate::relay::{self, SseScanner, SseTap, Usage};
use crate::state::AppState;
use crate::{retry, routing, upstream};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Wire {
    Chat,
    Messages,
    Responses,
}

impl Wire {
    pub(crate) fn sy(self) -> WireFormat {
        match self {
            Wire::Chat => WireFormat::OpenAiChat,
            Wire::Messages => WireFormat::AnthropicMessages,
            Wire::Responses => WireFormat::OpenAiResponses,
        }
    }

    pub(crate) fn path(self) -> &'static str {
        match self {
            Wire::Chat => "/v1/chat/completions",
            Wire::Messages => "/v1/messages",
            Wire::Responses => "/v1/responses",
        }
    }
}

pub(crate) fn upstream_wire(channel_type: i64) -> Wire {
    if channel_type == 14 {
        Wire::Messages
    } else {
        Wire::Chat
    }
}

static ENGINE: OnceLock<TranslationEngine> = OnceLock::new();

fn engine() -> &'static TranslationEngine {
    ENGINE.get_or_init(TranslationEngine::default)
}

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

fn client_error(client: Wire, status: StatusCode, message: String) -> Response {
    match client {
        Wire::Messages => relay::anthropic_error(status, message, "invalid_request_error"),
        Wire::Chat | Wire::Responses => {
            relay::openai_error(status, message, "invalid_request_error")
        }
    }
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

fn upstream_error_response(client: Wire, status: StatusCode, bytes: &Bytes, ct: Option<HeaderValue>) -> Response {
    if client == Wire::Messages {
        let message = serde_json::from_slice::<Value>(bytes)
            .ok()
            .and_then(|v| {
                v.pointer("/error/message")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_else(|| String::from_utf8_lossy(&bytes[..bytes.len().min(2048)]).to_string());
        return relay::anthropic_error(status, message, "api_error");
    }
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

pub(crate) fn upstream_headers(
    headers: &HeaderMap,
    key: &str,
    overrides: Option<&str>,
    upstream_wire: Wire,
) -> HeaderMap {
    let mut out = upstream::rewrite_headers(headers, key, overrides);
    if upstream_wire == Wire::Messages {
        let explicit_auth = overrides
            .and_then(|o| serde_json::from_str::<Value>(o).ok())
            .and_then(|v| {
                v.as_object()
                    .map(|m| m.keys().any(|k| k.eq_ignore_ascii_case("authorization")))
            })
            .unwrap_or(false);
        if !explicit_auth {
            out.remove("authorization");
        }
        if !out.contains_key("x-api-key")
            && let Ok(value) = HeaderValue::from_str(key)
        {
            out.insert("x-api-key", value);
        }
        if !out.contains_key("anthropic-version") {
            out.insert(
                "anthropic-version",
                HeaderValue::from_static("2023-06-01"),
            );
        }
    }
    out
}

fn frame_event(client: Wire, value: &Value, out: &mut Vec<u8>) {
    let Ok(data) = serde_json::to_string(value) else {
        return;
    };
    match client {
        Wire::Chat => {
            out.extend_from_slice(b"data: ");
            out.extend_from_slice(data.as_bytes());
            out.extend_from_slice(b"\n\n");
        }
        Wire::Messages | Wire::Responses => {
            let ty = value
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("message");
            out.extend_from_slice(format!("event: {ty}\ndata: {data}\n\n").as_bytes());
        }
    }
}

fn translate_event(
    state: &mut StreamTranslationState,
    source: WireFormat,
    target: WireFormat,
    client: Wire,
    data: &Value,
) -> Vec<u8> {
    match engine().translate_event(state, source, target, data) {
        Ok(events) => {
            let mut out = Vec::new();
            for event in &events {
                frame_event(client, event, &mut out);
            }
            out
        }
        Err(e) => {
            tracing::warn!("stream translation failed: {e}");
            Vec::new()
        }
    }
}

pub async fn relay(
    state: AppState,
    ctx: TokenContext,
    headers: HeaderMap,
    body: Bytes,
    client: Wire,
) -> Response {
    let started = Instant::now();
    let Ok(payload) = serde_json::from_slice::<Value>(&body) else {
        return client_error(client, StatusCode::BAD_REQUEST, "invalid JSON body".to_string());
    };
    if !payload.is_object() {
        return client_error(
            client,
            StatusCode::BAD_REQUEST,
            "body must be a JSON object".to_string(),
        );
    }
    let Some(origin_model) = payload.get("model").and_then(Value::as_str).map(str::to_string)
    else {
        return client_error(client, StatusCode::BAD_REQUEST, "model is required".to_string());
    };
    if !model_allowed(&ctx, &origin_model) {
        return client_error(
            client,
            StatusCode::FORBIDDEN,
            format!("token has no access to model {origin_model}"),
        );
    }
    let stream = payload
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let wants_usage = if client == Wire::Chat {
        payload
            .pointer("/stream_options/include_usage")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    } else {
        true
    };

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
        let upstream_wire = upstream_wire(channel.r#type);

        let mut outbound = if client == upstream_wire {
            payload.clone()
        } else {
            match engine().translate_request(
                client.sy(),
                upstream_wire.sy(),
                &payload,
                &TranslationPolicy::default(),
            ) {
                Ok(out) => out.body,
                Err(e) => {
                    return client_error(
                        client,
                        StatusCode::BAD_REQUEST,
                        format!("request translation failed: {e}"),
                    );
                }
            }
        };
        let mapped = relay::map_model(&origin_model, channel.model_mapping.as_deref());
        if mapped != origin_model {
            outbound["model"] = Value::String(mapped);
        }
        if stream && upstream_wire == Wire::Chat {
            let obj = outbound.as_object_mut().expect("object");
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
        let upstream_body = match serde_json::to_vec(&outbound) {
            Ok(b) => b,
            Err(e) => {
                return client_error(client, StatusCode::INTERNAL_SERVER_ERROR, e.to_string());
            }
        };
        let url = upstream::build_url(
            channel.base_url.as_deref().unwrap_or(""),
            upstream_wire.path(),
            channel.r#type,
        );
        let headers_out = upstream_headers(
            &headers,
            &selected.key,
            channel.header_override.as_deref(),
            upstream_wire,
        );

        let response = match state
            .client
            .post(&url)
            .headers(headers_out)
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
            let same_format = client == upstream_wire;
            let default_ct = if stream {
                "text/event-stream"
            } else {
                "application/json"
            };
            let content_type = response
                .headers()
                .get(CONTENT_TYPE)
                .cloned()
                .or_else(|| Some(HeaderValue::from_static(default_ct)));
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
                let mut source = response.bytes_stream();
                if same_format {
                    let wants = wants_usage;
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
                }
                {
                    let source_wire = upstream_wire;
                    let target_wire = client;
                    let client_for_frame = client;
                    let stream_body = async_stream::stream! {
                        let mut scanner = SseScanner::new();
                        let mut tr_state = StreamTranslationState::new(
                            source_wire.sy(),
                            target_wire.sy(),
                        );
                        let mut clean = true;
                        while let Some(item) = source.next().await {
                            match item {
                                Ok(chunk) => {
                                    for event in scanner.feed(&chunk) {
                                        if let Some(data) = event.data.as_ref() {
                                            let out = translate_event(
                                                &mut tr_state,
                                                source_wire.sy(),
                                                target_wire.sy(),
                                                client_for_frame,
                                                data,
                                            );
                                            if !out.is_empty() {
                                                yield Ok::<Bytes, std::io::Error>(Bytes::from(out));
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!("upstream stream error: {e}");
                                    clean = false;
                                    break;
                                }
                            }
                        }
                        for event in scanner.finish() {
                            if let Some(data) = event.data.as_ref() {
                                let out = translate_event(
                                    &mut tr_state,
                                    source_wire.sy(),
                                    target_wire.sy(),
                                    client_for_frame,
                                    data,
                                );
                                if !out.is_empty() {
                                    yield Ok::<Bytes, std::io::Error>(Bytes::from(out));
                                }
                            }
                        }
                        if clean {
                            match engine().finish_stream(&mut tr_state, target_wire.sy()) {
                                Ok(events) => {
                                    let mut out = Vec::new();
                                    for event in &events {
                                        frame_event(client_for_frame, event, &mut out);
                                    }
                                    if client_for_frame == Wire::Chat {
                                        out.extend_from_slice(b"data: [DONE]\n\n");
                                    }
                                    if !out.is_empty() {
                                        yield Ok::<Bytes, std::io::Error>(Bytes::from(out));
                                    }
                                }
                                Err(e) => tracing::warn!("finish_stream failed: {e}"),
                            }
                        }
                        logging::enqueue(&log_state, meta.entry(scanner.usage())).await;
                    };
                    return response_with_ct(status, content_type, Body::from_stream(stream_body));
                }
            } else {
                let bytes = match response.bytes().await {
                    Ok(b) => b,
                    Err(e) => {
                        logging::enqueue(&state, meta.entry(Usage::default())).await;
                        return client_error(
                            client,
                            StatusCode::BAD_GATEWAY,
                            format!("read upstream body failed: {e}"),
                        );
                    }
                };
                let upstream_value = serde_json::from_slice::<Value>(&bytes).ok();
                let usage = upstream_value
                    .as_ref()
                    .and_then(relay::usage_from_event)
                    .unwrap_or_default();
                let out_bytes = if same_format {
                    bytes
                } else {
                    let Some(value) = upstream_value else {
                        logging::enqueue(&state, meta.entry(usage)).await;
                        return client_error(
                            client,
                            StatusCode::BAD_GATEWAY,
                            "upstream returned non-JSON body".to_string(),
                        );
                    };
                    match engine().translate_response(
                        upstream_wire.sy(),
                        client.sy(),
                        &value,
                        &TranslationPolicy::default(),
                    ) {
                        Ok(out) => Bytes::from(
                            serde_json::to_vec(&out.body).unwrap_or_default(),
                        ),
                        Err(e) => {
                            logging::enqueue(&state, meta.entry(usage)).await;
                            return client_error(
                                client,
                                StatusCode::BAD_GATEWAY,
                                format!("response translation failed: {e}"),
                            );
                        }
                    }
                };
                logging::enqueue(&state, meta.entry(usage)).await;
                let ct = if same_format {
                    content_type
                } else {
                    Some(HeaderValue::from_static("application/json"))
                };
                return response_with_ct(status, ct, Body::from(out_bytes));
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
        Some((status, bytes, ct)) => upstream_error_response(client, status, &bytes, ct),
        None => client_error(
            client,
            StatusCode::SERVICE_UNAVAILABLE,
            "no available channel".to_string(),
        ),
    }
}
