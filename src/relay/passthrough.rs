use std::collections::HashSet;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use axum::body::{Body, Bytes};
use axum::http::header::{ACCEPT, CONTENT_TYPE, HeaderValue};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use serde_json::{Value, json};
use tokio_stream::StreamExt;

use crate::auth::token::TokenContext;
use crate::logging::{self, LogEntry};
use crate::relay::{self, SseScanner, Usage};
use crate::state::AppState;
use crate::{retry, routing, upstream};

const RESPONSE_BUFFER_LIMIT: usize = 4 * 1024 * 1024;

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
    ctx.model_limits
        .split(',')
        .map(str::trim)
        .any(|m| m == model)
}

fn content_type(headers: &HeaderMap) -> Option<HeaderValue> {
    headers.get(CONTENT_TYPE).cloned()
}

fn is_json_content_type(ct: Option<&HeaderValue>) -> bool {
    ct.and_then(|v| v.to_str().ok())
        .map(|v| v.contains("json"))
        .unwrap_or(false)
}

fn extract_model(body: &Bytes, ct: Option<&HeaderValue>) -> Option<String> {
    let ct = ct.and_then(|v| v.to_str().ok()).unwrap_or("");
    if ct.contains("multipart/form-data") {
        return multipart_model(body, ct);
    }
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|v| v.get("model").and_then(Value::as_str).map(str::to_string))
}

/// 从 multipart 的 `model` 字段取路由模型，不解析文件内容。
fn multipart_model(body: &[u8], ct: &str) -> Option<String> {
    let boundary = ct
        .split("boundary=")
        .nth(1)?
        .trim()
        .trim_matches('"')
        .to_string();
    let marker = format!("--{boundary}").into_bytes();
    let mut offset = 0;
    while let Some(pos) = find(&body[offset..], &marker) {
        let start = offset + pos + marker.len();
        let Some(header_end) = find(&body[start..], b"\r\n\r\n") else {
            break;
        };
        let headers = &body[start..start + header_end];
        let value_start = start + header_end + 4;
        let Some(value_end) = find(&body[value_start..], &marker) else {
            break;
        };
        if contains(headers, b"name=\"model\"") {
            let value = &body[value_start..value_start + value_end];
            return Some(String::from_utf8_lossy(value).trim().to_string());
        }
        offset = value_start + value_end;
    }
    None
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    find(haystack, needle).is_some()
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
    let is_json = is_json_content_type(ct.as_ref());
    if is_json && !bytes.is_empty() {
        return response_with_ct(status, ct, Body::from(bytes.clone()));
    }
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(2048)]).to_string();
    relay::openai_error(status, format!("upstream error: {text}"), "upstream_error")
}

pub async fn relay(
    state: AppState,
    ctx: TokenContext,
    headers: HeaderMap,
    body: Bytes,
    path: &'static str,
) -> Response {
    let started = Instant::now();
    let ct = content_type(&headers);
    let Some(origin_model) = extract_model(&body, ct.as_ref()) else {
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
            "invalid_request_error",
        );
    }
    let payload_stream = serde_json::from_slice::<Value>(&body)
        .ok()
        .and_then(|v| v.get("stream").and_then(Value::as_bool))
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

        let mapped = relay::map_model(&origin_model, channel.model_mapping.as_deref());
        let upstream_body = if mapped != origin_model {
            remap_body(&body, ct.as_ref(), &mapped)
        } else {
            body.clone()
        };
        let url = upstream::build_url(
            channel.base_url.as_deref().unwrap_or(""),
            path,
            channel.r#type,
        );
        let mut headers_out =
            upstream::rewrite_headers(&headers, &selected.key, channel.header_override.as_deref());
        if let Some(ct) = ct.clone() {
            headers_out.insert(CONTENT_TYPE, ct);
        }
        if let Some(accept) = headers.get(ACCEPT).cloned() {
            headers_out.insert(ACCEPT, accept);
        }

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
            let response_ct = response.headers().get(CONTENT_TYPE).cloned();
            let is_sse = response_ct
                .as_ref()
                .and_then(|v| v.to_str().ok())
                .map(|v| v.contains("text/event-stream"))
                .unwrap_or(false);
            let meta = LogMeta {
                origin_model: origin_model.clone(),
                stream: payload_stream || is_sse,
                channel_id: selected.channel_id,
                channel_name: selected.channel_name.clone(),
                user_id: ctx.user_id,
                username: username.clone(),
                token_id: ctx.token_id,
                token_name: ctx.token_name.clone(),
                group: group.clone(),
                ip: ip.clone(),
                request_id: request_id.clone(),
                upstream_request_id: upstream_request_id(response.headers()),
                started,
            };

            if is_sse {
                let log_state = state.clone();
                let mut source = response.bytes_stream();
                let stream_body = async_stream::stream! {
                    let mut scanner = SseScanner::new();
                    while let Some(item) = source.next().await {
                        match item {
                            Ok(chunk) => {
                                let _ = scanner.feed(&chunk);
                                yield Ok::<Bytes, std::io::Error>(chunk);
                            }
                            Err(e) => {
                                tracing::warn!("upstream stream error: {e}");
                                break;
                            }
                        }
                    }
                    let _ = scanner.finish();
                    logging::enqueue(&log_state, meta.entry(scanner.usage())).await;
                };
                return response_with_ct(status, response_ct, Body::from_stream(stream_body));
            }

            let fits_buffer = response
                .content_length()
                .map(|len| (len as usize) <= RESPONSE_BUFFER_LIMIT)
                .unwrap_or(false);
            if is_json_content_type(response_ct.as_ref()) && fits_buffer {
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
                    .and_then(relay::usage_from_event)
                    .unwrap_or_default();
                logging::enqueue(&state, meta.entry(usage)).await;
                return response_with_ct(status, response_ct, Body::from(bytes));
            }

            let log_state = state.clone();
            let mut source = response.bytes_stream();
            let stream_body = async_stream::stream! {
                while let Some(item) = source.next().await {
                    match item {
                        Ok(chunk) => yield Ok::<Bytes, std::io::Error>(chunk),
                        Err(e) => {
                            tracing::warn!("upstream stream error: {e}");
                            break;
                        }
                    }
                }
                logging::enqueue(&log_state, meta.entry(Usage::default())).await;
            };
            return response_with_ct(status, response_ct, Body::from_stream(stream_body));
        }

        let error_ct = response.headers().get(CONTENT_TYPE).cloned();
        let bytes = response.bytes().await.unwrap_or_default();
        last_error = Some((status, bytes, error_ct));
        if matches!(status.as_u16(), 401 | 403 | 429) {
            routing::mark_key_unhealthy(&state.routing, selected.channel_id, selected.key_index);
        }
        if retry::decide(Some(status.as_u16()), attempt, max_retry) == retry::RetryDecision::Retry {
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

fn remap_body(body: &Bytes, ct: Option<&HeaderValue>, mapped: &str) -> Bytes {
    if is_json_content_type(ct) {
        if let Ok(mut value) = serde_json::from_slice::<Value>(body) {
            value["model"] = Value::String(mapped.to_string());
            if let Ok(bytes) = serde_json::to_vec(&value) {
                return Bytes::from(bytes);
            }
        }
        return body.clone();
    }
    let ct_str = ct.and_then(|v| v.to_str().ok()).unwrap_or("");
    if !ct_str.contains("multipart/form-data") {
        return body.clone();
    }
    let Some(boundary) = ct_str
        .split("boundary=")
        .nth(1)
        .map(|b| b.trim().trim_matches('"').to_string())
    else {
        return body.clone();
    };
    let marker = format!("--{boundary}");
    let mut out = Vec::with_capacity(body.len());
    let mut offset = 0;
    while let Some(pos) = find(&body[offset..], marker.as_bytes()) {
        let end = offset + pos + marker.len();
        out.extend_from_slice(&body[offset..end]);
        let Some(header_end) = find(&body[end..], b"\r\n\r\n") else {
            out.extend_from_slice(&body[end..]);
            return Bytes::from(out);
        };
        let headers = &body[end..end + header_end];
        out.extend_from_slice(&body[end..end + header_end + 4]);
        let value_start = end + header_end + 4;
        let Some(next) = find(&body[value_start..], marker.as_bytes()) else {
            out.extend_from_slice(&body[value_start..]);
            return Bytes::from(out);
        };
        if contains(headers, b"name=\"model\"") {
            out.extend_from_slice(mapped.as_bytes());
            out.extend_from_slice(b"\r\n");
        } else {
            out.extend_from_slice(&body[value_start..value_start + next]);
        }
        offset = value_start + next;
    }
    out.extend_from_slice(&body[offset..]);
    Bytes::from(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_model_from_multipart() {
        let boundary = "----probe";
        let body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\nm1\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.wav\"\r\nContent-Type: audio/wav\r\n\r\nRIFFDATA\r\n--{boundary}--\r\n"
        );
        let ct = format!("multipart/form-data; boundary={boundary}");
        assert_eq!(
            multipart_model(body.as_bytes(), &ct),
            Some("m1".to_string())
        );
    }

    #[test]
    fn remaps_model_in_multipart() {
        let boundary = "----probe";
        let body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\nm1\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.wav\"\r\nContent-Type: audio/wav\r\n\r\nRIFFDATA\r\n--{boundary}--\r\n"
        );
        let ct = format!("multipart/form-data; boundary={boundary}");
        let out = remap_body(
            &Bytes::from(body.clone()),
            Some(&HeaderValue::from_str(&ct).unwrap()),
            "up-model",
        );
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains("\r\n\r\nup-model\r\n"), "{text}");
        assert!(text.contains("RIFFDATA"));
    }
}
