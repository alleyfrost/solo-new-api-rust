//! 渠道探测：连通性测试与上游模型列表获取。
//!
//! 只发一个最小探测请求，成功后写回 `channels.response_time/test_time`；
//! 失败不改渠道状态（手动测试不做自动禁用），也不写消费日志。

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Json;
use axum::Router;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio_stream::StreamExt;

use crate::error::{ApiError, json_ok};
use crate::model::ChannelRow;
use crate::relay::forward::{Wire, upstream_headers, upstream_wire};
use crate::routes::admin::channel::{self, split_list};
use crate::state::AppState;
use crate::upstream;

/// 单次探测最多尝试的 Key 数与整体时限，防止多 Key 渠道长时间占用请求。
const MAX_KEYS_PER_PROBE: usize = 8;
const PROBE_TIMEOUT: Duration = Duration::from_secs(30);
const PROBE_BUDGET: Duration = Duration::from_secs(60);
/// 探测响应体上限：模型列表可能较长，正文回复很短。
const MAX_REPLY_BYTES: usize = 2 * 1024 * 1024;
const MAX_MODEL_LIST_BYTES: usize = 8 * 1024 * 1024;
/// 流式探测只要拿到第一个事件就判定成功，随后立即断开上游读取。
const MAX_STREAM_PROBE_BYTES: usize = 256 * 1024;

const ENDPOINT_UNSUPPORTED: &str = "unsupported_endpoint_type";
const NO_TEST_MODEL: &str = "no_test_model";
const EMPTY_REPLY: &str = "empty_reply";
const UPSTREAM_ERROR: &str = "upstream_error";

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/channel/test/{id}", get(test_channel))
        .route("/api/channel/fetch_models/{id}", get(fetch_upstream_models))
        .route("/api/channel/fetch_models", post(fetch_models))
}

#[derive(Deserialize, Default)]
pub struct TestQuery {
    pub model: Option<String>,
    pub endpoint_type: Option<String>,
    pub stream: Option<bool>,
}

#[derive(Deserialize, Default)]
pub struct FetchModelsInput {
    pub base_url: Option<String>,
    pub r#type: Option<i64>,
    pub key: Option<String>,
    pub channel_id: Option<i64>,
    pub header_override: Option<Value>,
}

struct ProbeFailure {
    message: String,
    code: Option<String>,
}

enum Probe {
    Success(i64),
    Failure(ProbeFailure),
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// `endpoint_type` 决定探测协议；`auto` 时跟随渠道类型。
fn resolve_wire(endpoint_type: Option<&str>, channel_type: i64) -> Result<Wire, String> {
    let name = endpoint_type.unwrap_or("auto").trim().to_ascii_lowercase();
    match name.as_str() {
        "" | "auto" => Ok(upstream_wire(channel_type)),
        "openai" | "openai-chat" => Ok(Wire::Chat),
        "openai-response" | "openai-responses" => Ok(Wire::Responses),
        "anthropic" => Ok(Wire::Messages),
        other => Err(format!("endpoint_type '{other}' is not supported")),
    }
}

/// 空 base_url 时按探测协议取默认域名，而不是渠道类型。
fn probe_type(wire: Wire) -> i64 {
    if wire == Wire::Messages {
        14
    } else {
        1
    }
}

fn probe_payload(wire: Wire, model: &str, stream: bool) -> Value {
    let mut payload = match wire {
        Wire::Chat => json!({
            "model": model,
            "messages": [{ "role": "user", "content": "hi" }],
            "max_tokens": 64,
        }),
        Wire::Messages => json!({
            "model": model,
            "messages": [{ "role": "user", "content": "hi" }],
            "max_tokens": 64,
        }),
        Wire::Responses => json!({ "model": model, "input": "hi" }),
    };
    if stream {
        payload["stream"] = json!(true);
    }
    payload
}

fn resolve_model(query_model: Option<&str>, channel: &ChannelRow) -> Option<String> {
    [
        query_model,
        channel.test_model.as_deref(),
        split_list(&channel.models).first().map(String::as_str),
    ]
    .into_iter()
    .flatten()
    .map(str::trim)
    .find(|model| !model.is_empty())
    .map(str::to_string)
}

fn probe_keys(key: &str) -> Vec<String> {
    key.lines()
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .take(MAX_KEYS_PER_PROBE)
        .map(str::to_string)
        .collect()
}

fn test_envelope(success: bool, message: String, time_ms: i64, code: Option<String>) -> Response {
    // time 用秒（上游 new-api 口径），data.response_time 用毫秒（fork 前端优先读它）。
    let mut body = json!({
        "success": success,
        "message": message,
        "time": if success { time_ms as f64 / 1000.0 } else { 0.0 },
    });
    if success {
        body["data"] = json!({ "response_time": time_ms });
    }
    if let Some(code) = code.filter(|c| !c.is_empty()) {
        body["error_code"] = Value::String(code);
    }
    (StatusCode::OK, Json(body)).into_response()
}

fn error_text(error: &Value) -> String {
    match error {
        Value::String(text) => text.clone(),
        Value::Object(map) => map
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown upstream error")
            .to_string(),
        other => other.to_string(),
    }
}

fn error_code(error: &Value) -> Option<String> {
    let object = error.as_object()?;
    for key in ["code", "error_code", "type"] {
        if let Some(text) = object.get(key).and_then(Value::as_str)
            && !text.trim().is_empty()
        {
            return Some(text.to_string());
        }
    }
    None
}

/// 上游 2xx 回复里是否真的有内容；空回复按失败上报，避免"测试通过但不可用"。
/// 三种协议的回复形状不同，因此递归查找任何非空文本或工具调用。
fn reply_has_content(value: &Value) -> bool {
    has_reply_text(value) || generated_tokens(value) > 0
}

fn has_reply_text(value: &Value) -> bool {
    match value {
        Value::String(text) => !text.trim().is_empty(),
        Value::Array(items) => items.iter().any(has_reply_text),
        Value::Object(map) => [
            "content",
            "text",
            "thinking",
            "reasoning_content",
            "output_text",
            "choices",
            "message",
        ]
        .iter()
        .any(|key| map.get(*key).is_some_and(has_reply_text))
            || map
                .get("tool_calls")
                .is_some_and(|calls| calls.as_array().is_some_and(|calls| !calls.is_empty())),
        _ => false,
    }
}

fn generated_tokens(value: &Value) -> i64 {
    ["completion_tokens", "output_tokens"]
        .iter()
        .filter_map(|key| value.pointer(&format!("/usage/{key}")).and_then(Value::as_i64))
        .max()
        .unwrap_or(0)
}

async fn read_capped(response: reqwest::Response, cap: usize) -> Result<Bytes, ProbeFailure> {
    let mut response = response;
    let mut out: Vec<u8> = Vec::new();
    loop {
        let chunk = response
            .chunk()
            .await
            .map_err(|e| failure(format!("read upstream body failed: {e}")))?;
        let Some(chunk) = chunk else { break };
        out.extend_from_slice(&chunk);
        if out.len() > cap {
            return Err(failure(format!("upstream response exceeds {cap} bytes")));
        }
    }
    Ok(Bytes::from(out))
}

fn failure(message: String) -> ProbeFailure {
    ProbeFailure {
        message,
        code: Some(UPSTREAM_ERROR.to_string()),
    }
}

async fn probe_json(response: reqwest::Response) -> Result<(), ProbeFailure> {
    let bytes = read_capped(response, MAX_REPLY_BYTES).await?;
    if bytes.is_empty() {
        return Err(ProbeFailure {
            message: "upstream returned an empty body".to_string(),
            code: Some(EMPTY_REPLY.to_string()),
        });
    }
    let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
        return Err(ProbeFailure {
            message: format!("upstream returned non-JSON body: {}", preview(&bytes)),
            code: Some(EMPTY_REPLY.to_string()),
        });
    };
    if let Some(error) = value.get("error").filter(|e| !e.is_null()) {
        let code = error_code(error).or_else(|| Some(UPSTREAM_ERROR.to_string()));
        return Err(ProbeFailure {
            message: error_text(error),
            code,
        });
    }
    if !reply_has_content(&value) {
        return Err(ProbeFailure {
            message: "upstream returned a response without content".to_string(),
            code: Some(EMPTY_REPLY.to_string()),
        });
    }
    Ok(())
}

/// 流式探测：收到第一个 SSE 字段行即判定成功，随即丢弃响应流取消上游读取。
async fn probe_stream(response: reqwest::Response) -> Result<(), ProbeFailure> {
    let mut stream = response.bytes_stream();
    let mut seen = Vec::new();
    while let Some(item) = stream.next().await {
        match item {
            Ok(chunk) => {
                seen.extend_from_slice(&chunk);
                if contains_bytes(&seen, b"data:") || contains_bytes(&seen, b"event:") {
                    return Ok(());
                }
                if seen.len() > MAX_STREAM_PROBE_BYTES {
                    break;
                }
            }
            Err(e) => return Err(failure(format!("upstream stream error: {e}"))),
        }
    }
    Err(ProbeFailure {
        message: "upstream stream produced no event before ending".to_string(),
        code: Some(EMPTY_REPLY.to_string()),
    })
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

fn preview(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]);
    text.trim().to_string()
}

async fn upstream_failure(bytes: Bytes, status: StatusCode) -> ProbeFailure {
    let value = serde_json::from_slice::<Value>(&bytes).ok();
    let error = value
        .as_ref()
        .and_then(|v| v.get("error"))
        .filter(|e| !e.is_null());
    match error {
        Some(error) => ProbeFailure {
            message: format!("HTTP {}: {}", status.as_u16(), error_text(error)),
            code: error_code(error),
        },
        None => ProbeFailure {
            message: format!("HTTP {}: {}", status.as_u16(), preview(&bytes)),
            code: Some(UPSTREAM_ERROR.to_string()),
        },
    }
}

async fn run_probe(
    state: &AppState,
    client: &HeaderMap,
    channel: &ChannelRow,
    wire: Wire,
    model: &str,
    key: &str,
    stream: bool,
) -> Probe {
    let started = Instant::now();
    let mapped = crate::relay::map_model(model, channel.model_mapping.as_deref());
    let payload = probe_payload(wire, &mapped, stream);
    let url = upstream::build_url(
        channel.base_url.as_deref().unwrap_or(""),
        wire.path(),
        probe_type(wire),
    );
    let headers = upstream_headers(client, key, channel.header_override.as_deref(), wire);
    let response = match state
        .client
        .post(&url)
        .headers(headers)
        .json(&payload)
        .timeout(PROBE_TIMEOUT)
        .send()
        .await
    {
        Ok(response) => response,
        Err(e) => return Probe::Failure(failure(format!("upstream request failed: {e}"))),
    };
    let status = response.status();
    if !status.is_success() {
        let bytes = read_capped(response, MAX_REPLY_BYTES)
            .await
            .unwrap_or_default();
        return Probe::Failure(upstream_failure(bytes, status).await);
    }
    let outcome = if stream {
        probe_stream(response).await
    } else {
        probe_json(response).await
    };
    match outcome {
        Ok(()) => Probe::Success(started.elapsed().as_millis() as i64),
        Err(failure) => Probe::Failure(failure),
    }
}

async fn save_timing(state: &AppState, channel_id: i64, response_time_ms: i64) {
    let _ = sqlx::query("UPDATE channels SET response_time=?, test_time=? WHERE id=?")
        .bind(response_time_ms)
        .bind(now_secs())
        .bind(channel_id)
        .execute(&state.pool)
        .await
        .inspect_err(|e| tracing::warn!("save channel test timing failed: {e}"));
}

async fn test_channel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Query(query): Query<TestQuery>,
) -> Result<Response, ApiError> {
    let channel = channel::load_channel(&state, id)
        .await?
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "channel not found"))?;
    let wire = match resolve_wire(query.endpoint_type.as_deref(), channel.r#type) {
        Ok(wire) => wire,
        Err(message) => {
            return Ok(test_envelope(false, message, 0, Some(ENDPOINT_UNSUPPORTED.into())));
        }
    };
    let Some(model) = resolve_model(query.model.as_deref(), &channel) else {
        return Ok(test_envelope(
            false,
            "no test model: set 测试模型 or 渠道模型 first".to_string(),
            0,
            Some(NO_TEST_MODEL.to_string()),
        ));
    };
    let keys = probe_keys(&channel.key);
    if keys.is_empty() {
        return Ok(test_envelope(
            false,
            "channel has no key".to_string(),
            0,
            Some(UPSTREAM_ERROR.to_string()),
        ));
    }

    let total = keys.len();
    let started = Instant::now();
    let mut last: Option<ProbeFailure> = None;
    for (index, key) in keys.iter().enumerate() {
        if index > 0 && started.elapsed() >= PROBE_BUDGET {
            break;
        }
        let outcome = run_probe(
            &state,
            &headers,
            &channel,
            wire,
            &model,
            key,
            query.stream.unwrap_or(false),
        )
        .await;
        match outcome {
            Probe::Success(elapsed_ms) => {
                save_timing(&state, channel.id, elapsed_ms).await;
                return Ok(test_envelope(true, String::new(), elapsed_ms, None));
            }
            Probe::Failure(mut failure) => {
                if total > 1 {
                    failure.message = format!("[key {}/{total}] {}", index + 1, failure.message);
                }
                last = Some(failure);
            }
        }
    }

    let failure = last.unwrap_or_else(|| failure("no key tested".to_string()));
    Ok(test_envelope(
        false,
        failure.message,
        0,
        failure.code.or(Some(UPSTREAM_ERROR.to_string())),
    ))
}

async fn fetch_upstream_models(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Response, ApiError> {
    let channel = channel::load_channel(&state, id)
        .await?
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "channel not found"))?;
    let base_url = channel.base_url.as_deref().unwrap_or("");
    let overrides = channel.header_override.as_deref();
    Ok(fetch_with_keys(
        &state,
        &headers,
        base_url,
        channel.r#type,
        &probe_keys(&channel.key),
        overrides,
    )
    .await)
}

async fn fetch_models(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<FetchModelsInput>,
) -> Result<Response, ApiError> {
    if let Some(id) = input.channel_id.filter(|id| *id > 0) {
        let channel = channel::load_channel(&state, id)
            .await?
            .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "channel not found"))?;
        let base_url = input
            .base_url
            .as_deref()
            .map(str::trim)
            .filter(|url| !url.is_empty())
            .unwrap_or(channel.base_url.as_deref().unwrap_or(""));
        let keys = match input.key.as_deref().map(str::trim) {
            Some(key) if !key.is_empty() => probe_keys(key),
            _ => probe_keys(&channel.key),
        };
        let overrides = channel
            .header_override
            .as_deref()
            .or(input.header_override.as_ref().and_then(|v| {
                v.as_str().filter(|s| !s.trim().is_empty())
            }));
        return Ok(fetch_with_keys(
            &state,
            &headers,
            base_url,
            channel.r#type,
            &keys,
            overrides,
        )
        .await);
    }

    let channel_type = input
        .r#type
        .filter(|ty| *ty > 0)
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "type is required"))?;
    let base_url = input.base_url.as_deref().unwrap_or("");
    let keys = probe_keys(input.key.as_deref().unwrap_or(""));
    let overrides = input
        .header_override
        .as_ref()
        .and_then(|v| v.as_str().filter(|s| !s.is_empty()))
        .map(str::to_string);
    let response = fetch_with_keys(
        &state,
        &headers,
        base_url,
        channel_type,
        &keys,
        overrides.as_deref(),
    )
    .await;
    Ok(response)
}

async fn fetch_with_keys(
    state: &AppState,
    headers: &HeaderMap,
    base_url: &str,
    channel_type: i64,
    keys: &[String],
    overrides: Option<&str>,
) -> Response {
    if keys.is_empty() {
        return models_failure("channel has no key".to_string());
    }
    let started = Instant::now();
    let mut last = String::new();
    for (index, key) in keys.iter().enumerate() {
        if index > 0 && started.elapsed() >= PROBE_BUDGET {
            break;
        }
        match fetch_model_ids(state, headers, base_url, channel_type, key, overrides).await {
            Ok(models) => return json_ok(&models),
            Err(message) => last = message,
        }
    }
    models_failure(last)
}

fn models_failure(message: String) -> Response {
    (
        StatusCode::OK,
        Json(json!({
            "success": false,
            "message": format!("Failed to fetch models: {message}"),
        })),
    )
        .into_response()
}

async fn fetch_model_ids(
    state: &AppState,
    client: &HeaderMap,
    base_url: &str,
    channel_type: i64,
    key: &str,
    overrides: Option<&str>,
) -> Result<Vec<String>, String> {
    let wire = upstream_wire(channel_type);
    let url = upstream::build_url(base_url, "/v1/models", probe_type(wire));
    let headers = upstream_headers(client, key, overrides, wire);
    let response = state
        .client
        .get(&url)
        .headers(headers)
        .timeout(PROBE_TIMEOUT)
        .send()
        .await
        .map_err(|e| format!("upstream request failed: {e}"))?;
    let status = response.status();
    let bytes = read_capped(response, MAX_MODEL_LIST_BYTES)
        .await
        .map_err(|e| e.message)?;
    if !status.is_success() {
        return Err(match serde_json::from_slice::<Value>(&bytes).ok() {
            Some(value) => match value.get("error").filter(|e| !e.is_null()) {
                Some(error) => format!("HTTP {}: {}", status.as_u16(), error_text(error)),
                None => format!("HTTP {}: {}", status.as_u16(), preview(&bytes)),
            },
            None => format!("HTTP {}: {}", status.as_u16(), preview(&bytes)),
        });
    }
    let value = serde_json::from_slice::<Value>(&bytes)
        .map_err(|_| format!("upstream model list is not JSON: {}", preview(&bytes)))?;
    Ok(model_ids(&value))
}

/// 上游模型列表兼容 `{"data":[...]}`、`{"models":[...]}` 与裸数组三种形状。
fn model_ids(value: &Value) -> Vec<String> {
    let items = value
        .get("data")
        .or_else(|| value.get("models"))
        .and_then(Value::as_array)
        .or_else(|| value.as_array());
    let Some(items) = items else {
        return Vec::new();
    };
    let mut seen = std::collections::BTreeSet::new();
    for item in items {
        let name = match item {
            Value::String(text) => Some(text.as_str()),
            Value::Object(_) => item
                .get("id")
                .and_then(Value::as_str)
                .or_else(|| item.get("name").and_then(Value::as_str)),
            _ => None,
        };
        if let Some(name) = name.map(str::trim).filter(|name| !name.is_empty()) {
            seen.insert(name.to_string());
        }
    }
    seen.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_type_maps_to_wire() {
        assert_eq!(resolve_wire(None, 14).unwrap(), Wire::Messages);
        assert_eq!(resolve_wire(Some("auto"), 1).unwrap(), Wire::Chat);
        assert_eq!(resolve_wire(Some("Anthropic"), 1).unwrap(), Wire::Messages);
        assert_eq!(
            resolve_wire(Some("openai-response"), 1).unwrap(),
            Wire::Responses
        );
        assert!(resolve_wire(Some("gemini"), 1).is_err());
    }

    #[test]
    fn model_falls_back_to_channel_model_list() {
        let channel = ChannelRow {
            id: 1,
            r#type: 1,
            key: "k".into(),
            status: 1,
            name: "c".into(),
            models: " m1 , m2 ".into(),
            group: "default".into(),
            base_url: None,
            priority: None,
            weight: None,
            channel_info: None,
            model_mapping: None,
            header_override: None,
            created_time: None,
            test_time: None,
            response_time: None,
            open_ai_organization: None,
            test_model: None,
            other: None,
            balance: None,
            balance_updated_time: None,
            used_quota: None,
            status_code_mapping: None,
            auto_ban: None,
            other_info: None,
            tag: None,
            setting: None,
            param_override: None,
            remark: None,
            settings: None,
        };
        assert_eq!(resolve_model(None, &channel).unwrap(), "m1");
        assert_eq!(resolve_model(Some("zz"), &channel).unwrap(), "zz");
    }

    #[test]
    fn content_detection_covers_all_wires() {
        assert!(reply_has_content(&json!({ "choices": [{ "message": { "content": "Hi" } }] })));
        assert!(!reply_has_content(
            &json!({ "choices": [{ "message": { "content": "" } }] })
        ));
        assert!(reply_has_content(
            &json!({ "choices": [{ "message": { "tool_calls": [{ "id": "1" }] } }] })
        ));
        assert!(reply_has_content(
            &json!({ "content": [{ "type": "thinking" }, { "type": "text", "text": "Hi" }] })
        ));
        assert!(reply_has_content(
            &json!({ "output_text": "Hi", "output": [] })
        ));
        assert!(reply_has_content(
            &json!({ "choices": [], "usage": { "completion_tokens": 3 } })
        ));
        assert!(!reply_has_content(
            &json!({ "id": "x", "created": 1, "usage": { "completion_tokens": 0 } })
        ));
    }

    #[test]
    fn model_ids_dedups_and_sorts_across_shapes() {
        assert_eq!(
            model_ids(&json!({ "data": [{ "id": "b" }, { "id": "a" }, { "id": "b" }] })),
            vec!["a".to_string(), "b".to_string()]
        );
        assert_eq!(
            model_ids(&json!({ "models": [{ "name": "x" }, "y"] })),
            vec!["x".to_string(), "y".to_string()]
        );
        assert_eq!(model_ids(&json!([])), Vec::<String>::new());
    }

}
