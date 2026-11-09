use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Bytes;
use axum::http::header::{CONTENT_TYPE, HeaderValue};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::auth::token::TokenContext;
use crate::relay::{openai_error, passthrough};
use crate::state::AppState;

/// 桥接要把图片塞进 chat 响应里，只能整包读取；超过上限直接报错，避免大图顶内存。
const MAX_IMAGE_BYTES: usize = 32 * 1024 * 1024;
/// 允许从 chat 请求透传给生图接口的可选字段（尺寸、张数、水印等）。
const PASSTHROUGH_FIELDS: [&str; 8] = [
    "size",
    "n",
    "watermark",
    "response_format",
    "quality",
    "style",
    "output_format",
    "prompt_extend",
];

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 取最后一条 user 消息的文本；兼容字符串与内容分片两种写法。
fn prompt_of(payload: &Value) -> Option<String> {
    let messages = payload.get("messages")?.as_array()?;
    for message in messages.iter().rev() {
        if message.get("role").and_then(Value::as_str) != Some("user") {
            continue;
        }
        let text = match message.get("content") {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Array(parts)) => parts
                .iter()
                .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n"),
            _ => String::new(),
        };
        let text = text.trim().to_string();
        if !text.is_empty() {
            return Some(text);
        }
    }
    None
}

/// 取最后一条 user 消息里的参考图；只放行公网 URL 与 data:image 前缀，
/// 纯 base64 字符串上游会直接拒绝。
fn images_of(payload: &Value) -> Vec<String> {
    let Some(messages) = payload.get("messages").and_then(Value::as_array) else {
        return Vec::new();
    };
    let Some(user) = messages
        .iter()
        .rev()
        .find(|message| message.get("role").and_then(Value::as_str) == Some("user"))
    else {
        return Vec::new();
    };
    let Some(parts) = user.get("content").and_then(Value::as_array) else {
        return Vec::new();
    };
    parts
        .iter()
        .filter(|part| part.get("type").and_then(Value::as_str) == Some("image_url"))
        .filter_map(|part| {
            let image = part.get("image_url")?;
            let url = image
                .as_str()
                .or_else(|| image.get("url").and_then(Value::as_str))?
                .trim();
            let supported = url.starts_with("data:image/")
                || url.starts_with("http://")
                || url.starts_with("https://");
            supported.then(|| url.to_string())
        })
        .collect()
}

fn mime_of(format: &str) -> &'static str {
    match format.to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        _ => "image/png",
    }
}

fn usage_of(parsed: &Value) -> Value {
    let Some(usage) = parsed.get("usage") else {
        return json!({"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0});
    };
    let int = |keys: [&str; 2]| -> i64 {
        keys.iter()
            .find_map(|key| usage.get(*key).and_then(Value::as_i64))
            .unwrap_or(0)
    };
    let prompt = int(["input_tokens", "prompt_tokens"]);
    let completion = int(["output_tokens", "completion_tokens"]);
    let total = usage
        .get("total_tokens")
        .and_then(Value::as_i64)
        .unwrap_or(prompt + completion);
    json!({
        "prompt_tokens": prompt,
        "completion_tokens": completion,
        "total_tokens": total
    })
}

fn json_body(status: StatusCode, value: Value) -> Response {
    let body = serde_json::to_vec(&value).unwrap_or_else(|_| b"{}".to_vec());
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
        .body(axum::body::Body::from(body))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// 调用方用 chat 接口请求一个生图模型时，带参考图转成 `/v1/images/edits`、
/// 否则转成 `/v1/images/generations`，再把图片包成一条 chat completion 返回
/// （流式请求退化为单块）。
pub async fn relay(
    state: AppState,
    ctx: TokenContext,
    headers: HeaderMap,
    origin_model: String,
    payload: &Value,
    stream: bool,
) -> Response {
    let Some(prompt) = prompt_of(payload) else {
        return openai_error(
            StatusCode::BAD_REQUEST,
            "messages must contain a user prompt",
            "invalid_request_error",
        );
    };
    let reference_images = images_of(payload);
    let mut images = Map::new();
    images.insert("model".to_string(), json!(origin_model));
    images.insert("prompt".to_string(), json!(prompt));
    for key in PASSTHROUGH_FIELDS {
        if let Some(value) = payload.get(key) {
            images.insert(key.to_string(), value.clone());
        }
    }
    let endpoint = if reference_images.is_empty() {
        "/v1/images/generations"
    } else {
        images.insert(
            "images".to_string(),
            Value::Array(
                reference_images
                    .iter()
                    .map(|url| json!({ "image_url": url }))
                    .collect(),
            ),
        );
        // 编辑接口只支持单张输出，n>1 会被上游拒绝。
        images.insert("n".to_string(), json!(1));
        "/v1/images/edits"
    };
    let Ok(body) = serde_json::to_vec(&Value::Object(images)) else {
        return openai_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "request re-encode failed",
            "internal_error",
        );
    };

    let upstream = passthrough::relay(state, ctx, headers, Bytes::from(body), endpoint).await;
    if !upstream.status().is_success() {
        // 上游错误原样返回，保持调用方看到的报错文本与直连一致。
        return upstream;
    }
    let bytes = match axum::body::to_bytes(upstream.into_body(), MAX_IMAGE_BYTES).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return openai_error(
                StatusCode::BAD_GATEWAY,
                format!("image response exceeds {MAX_IMAGE_BYTES} bytes"),
                "upstream_error",
            );
        }
    };
    let parsed: Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => {
            return openai_error(
                StatusCode::BAD_GATEWAY,
                "upstream returned a non-JSON image response",
                "upstream_error",
            );
        }
    };
    let item = parsed
        .get("data")
        .and_then(Value::as_array)
        .and_then(|items| items.first());
    let content = match item {
        Some(item) if item.get("b64_json").and_then(Value::as_str).is_some() => {
            let format = parsed
                .get("output_format")
                .and_then(Value::as_str)
                .unwrap_or("png");
            let b64 = item["b64_json"].as_str().unwrap_or_default();
            format!("![image](data:{};base64,{b64})", mime_of(format))
        }
        Some(item) if item.get("url").and_then(Value::as_str).is_some() => {
            format!("![image]({})", item["url"].as_str().unwrap_or_default())
        }
        _ => {
            return openai_error(
                StatusCode::BAD_GATEWAY,
                "upstream image response has no data",
                "upstream_error",
            );
        }
    };

    let created = now();
    let id = format!("chatcmpl-{}", Uuid::new_v4().simple());
    let usage = usage_of(&parsed);
    if !stream {
        return json_body(
            StatusCode::OK,
            json!({
                "id": id,
                "object": "chat.completion",
                "created": created,
                "model": origin_model,
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": content},
                    "finish_reason": "stop"
                }],
                "usage": usage
            }),
        );
    }

    let first = json!({
        "id": id,
        "object": "chat.completion.chunk",
        "created": created,
        "model": origin_model,
        "choices": [{
            "index": 0,
            "delta": {"role": "assistant", "content": content},
            "finish_reason": null
        }]
    });
    let last = json!({
        "id": id,
        "object": "chat.completion.chunk",
        "created": created,
        "model": origin_model,
        "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}],
        "usage": usage
    });
    let sse = format!("data: {first}\n\ndata: {last}\n\ndata: [DONE]\n\n");
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, HeaderValue::from_static("text/event-stream"))
        .body(axum::body::Body::from(sse))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_prefers_last_user_message_and_joins_text_parts() {
        let payload = json!({
            "messages": [
                {"role": "system", "content": "系统提示"},
                {"role": "user", "content": "第一句"},
                {"role": "assistant", "content": "回复"},
                {"role": "user", "content": [
                    {"type": "text", "text": "画一只猫"},
                    {"type": "image_url", "image_url": {"url": "https://x/y.png"}},
                    {"type": "text", "text": "蓝紫色"}
                ]}
            ]
        });
        assert_eq!(prompt_of(&payload).as_deref(), Some("画一只猫\n蓝紫色"));
        assert_eq!(prompt_of(&json!({"messages": []})), None);
        assert_eq!(
            prompt_of(&json!({"messages": [{"role": "user", "content": "  "}]})),
            None
        );
    }

    #[test]
    fn images_come_from_the_last_user_message_and_require_a_supported_url() {
        let payload = json!({
            "messages": [
                {"role": "user", "content": [
                    {"type": "image_url", "image_url": {"url": "https://old/x.png"}}
                ]},
                {"role": "assistant", "content": "ok"},
                {"role": "user", "content": [
                    {"type": "text", "text": "把背景换成雪山"},
                    {"type": "image_url", "image_url": {"url": "data:image/png;base64,QUJD"}},
                    {"type": "image_url", "image_url": "http://ref/y.jpg"},
                    {"type": "image_url", "image_url": {"url": "QUJD"}}
                ]}
            ]
        });
        assert_eq!(
            images_of(&payload),
            vec![
                "data:image/png;base64,QUJD".to_string(),
                "http://ref/y.jpg".to_string()
            ]
        );
        assert!(images_of(&json!({"messages": [{"role": "user", "content": "hi"}]})).is_empty());
    }

    #[test]
    fn usage_maps_image_counters() {
        let parsed =
            json!({"usage": {"input_tokens": 12, "output_tokens": 34, "total_tokens": 46}});
        assert_eq!(
            usage_of(&parsed),
            json!({"prompt_tokens": 12, "completion_tokens": 34, "total_tokens": 46})
        );
        assert_eq!(
            usage_of(&json!({})),
            json!({"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0})
        );
    }
}
