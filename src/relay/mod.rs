use std::collections::HashMap;

use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

const MAX_EVENT: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Usage {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub cached_tokens: i64,
    pub cache_write_tokens: i64,
}

fn as_i64(v: &Value) -> Option<i64> {
    v.as_i64().or_else(|| v.as_u64().map(|n| n as i64))
}

pub fn usage_from_json(root: &Value) -> Option<Usage> {
    let u = root.get("usage")?;
    let prompt = u
        .get("prompt_tokens")
        .and_then(as_i64)
        .or_else(|| u.get("input_tokens").and_then(as_i64))
        .unwrap_or(0);
    let completion = u
        .get("completion_tokens")
        .and_then(as_i64)
        .or_else(|| u.get("output_tokens").and_then(as_i64))
        .unwrap_or(0);
    let cached = u
        .get("prompt_tokens_details")
        .and_then(|d| d.get("cached_tokens"))
        .and_then(as_i64)
        .or_else(|| {
            u.get("input_tokens_details")
                .and_then(|d| d.get("cached_tokens"))
                .and_then(as_i64)
        })
        .or_else(|| u.get("cache_read_input_tokens").and_then(as_i64))
        .unwrap_or(0);
    let cache_write = u
        .get("cache_creation_input_tokens")
        .and_then(as_i64)
        .or_else(|| {
            u.get("prompt_tokens_details")
                .and_then(|d| d.get("cache_write_tokens"))
                .and_then(as_i64)
        })
        .unwrap_or(0);
    Some(Usage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        cached_tokens: cached,
        cache_write_tokens: cache_write,
    })
}

/// 写入 logs.other：缓存命中字段 + 渠道列需要的 Key 序号与重试链。
pub fn log_other(
    usage: Usage,
    is_multi_key: bool,
    key_index: usize,
    use_channels: &[String],
) -> String {
    let mut obj = serde_json::Map::new();
    if usage.cached_tokens > 0 {
        obj.insert("cache_tokens".into(), usage.cached_tokens.into());
    }
    if usage.cache_write_tokens > 0 {
        obj.insert(
            "cache_creation_tokens".into(),
            usage.cache_write_tokens.into(),
        );
    }
    let mut admin = serde_json::Map::new();
    admin.insert("is_multi_key".into(), is_multi_key.into());
    admin.insert("multi_key_index".into(), (key_index as i64).into());
    admin.insert("use_channel".into(), json!(use_channels));
    obj.insert("admin_info".into(), Value::Object(admin));
    serde_json::to_string(&Value::Object(obj)).unwrap_or_else(|_| "{}".to_string())
}

pub fn upstream_error_message(bytes: &[u8]) -> String {
    if let Ok(value) = serde_json::from_slice::<Value>(bytes)
        && let Some(msg) = value
            .pointer("/error/message")
            .or_else(|| value.get("message"))
            .and_then(Value::as_str)
    {
        return msg.to_string();
    }
    String::from_utf8_lossy(bytes).chars().take(200).collect()
}

/// 兼容 Anthropic 的 `message.usage` 与 Responses 的 `response.usage` 包装。
pub fn usage_from_event(root: &Value) -> Option<Usage> {
    usage_from_json(root)
        .or_else(|| root.get("message").and_then(usage_from_json))
        .or_else(|| root.get("response").and_then(usage_from_json))
}

/// 渠道 `model_mapping` 是“请求模型 -> 上游模型”的链式映射；循环时退回原始模型。
pub fn map_model(origin: &str, mapping: Option<&str>) -> String {
    let Some(map) = mapping
        .filter(|m| !m.trim().is_empty())
        .and_then(|m| serde_json::from_str::<HashMap<String, String>>(m).ok())
    else {
        return origin.to_string();
    };
    let mut current = origin.to_string();
    let mut seen: Vec<String> = vec![current.clone()];
    while let Some(next) = map.get(&current).filter(|m| !m.is_empty()) {
        if seen.contains(next) {
            return origin.to_string();
        }
        seen.push(next.clone());
        current = next.clone();
    }
    current
}

pub fn client_ip(headers: &HeaderMap, trust_proxy: bool) -> String {
    if !trust_proxy {
        return "unknown".to_string();
    }
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .or_else(|| {
            headers
                .get("x-real-ip")
                .and_then(|v| v.to_str().ok())
                .map(|v| v.trim().to_string())
        })
        .unwrap_or_else(|| "unknown".to_string())
}

pub fn openai_error(status: StatusCode, message: impl Into<String>, kind: &str) -> Response {
    (
        status,
        axum::Json(json!({
            "error": { "message": message.into(), "type": kind, "param": null, "code": null }
        })),
    )
        .into_response()
}

pub fn anthropic_error(status: StatusCode, message: impl Into<String>, kind: &str) -> Response {
    (
        status,
        axum::Json(json!({
            "type": "error",
            "error": { "type": kind, "message": message.into() }
        })),
    )
        .into_response()
}

#[derive(Debug)]
pub struct ScannedEvent {
    pub raw: Vec<u8>,
    pub data: Option<Value>,
    pub usage_only: bool,
}

/// 按 SSE 事件边界切分字节流；只缓存一个事件，逐个产出，避免整段响应滞留内存。
pub struct SseScanner {
    buf: Vec<u8>,
    usage: Usage,
}

impl Default for SseScanner {
    fn default() -> Self {
        Self::new()
    }
}

impl SseScanner {
    pub fn new() -> Self {
        Self {
            buf: Vec::new(),
            usage: Usage::default(),
        }
    }

    pub fn usage(&self) -> Usage {
        self.usage
    }

    fn emit_usage(&mut self, usage: Option<Usage>) {
        if let Some(u) = usage {
            self.usage.prompt_tokens = self.usage.prompt_tokens.max(u.prompt_tokens);
            self.usage.completion_tokens = self.usage.completion_tokens.max(u.completion_tokens);
            self.usage.cached_tokens = self.usage.cached_tokens.max(u.cached_tokens);
            self.usage.cache_write_tokens =
                self.usage.cache_write_tokens.max(u.cache_write_tokens);
        }
    }

    fn scan_block(mut raw: Vec<u8>) -> ScannedEvent {
        let mut data: Vec<u8> = Vec::new();
        for line in raw.split(|&b| b == b'\n') {
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            if let Some(rest) = line.strip_prefix(b"data:") {
                let rest = rest.strip_prefix(b" ").unwrap_or(rest);
                if !data.is_empty() {
                    data.push(b'\n');
                }
                data.extend_from_slice(rest);
            }
        }
        if data.is_empty() || data == b"[DONE]" {
            return ScannedEvent {
                raw,
                data: None,
                usage_only: false,
            };
        }
        let Ok(value) = serde_json::from_slice::<Value>(&data) else {
            return ScannedEvent {
                raw,
                data: None,
                usage_only: false,
            };
        };
        let usage = usage_from_event(&value);
        let usage_only = usage.is_some()
            && value
                .get("choices")
                .and_then(Value::as_array)
                .map(|a| a.is_empty())
                .unwrap_or(true);
        ScannedEvent {
            raw: std::mem::take(&mut raw),
            data: Some(value),
            usage_only,
        }
    }

    pub fn feed(&mut self, chunk: &[u8]) -> Vec<ScannedEvent> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        loop {
            let Some((idx, sep)) = find_boundary(&self.buf) else {
                if self.buf.len() > MAX_EVENT {
                    out.push(ScannedEvent {
                        raw: std::mem::take(&mut self.buf),
                        data: None,
                        usage_only: false,
                    });
                }
                break;
            };
            let block: Vec<u8> = self.buf.drain(..idx + sep).collect();
            let event = Self::scan_block(block);
            if event.data.is_some() {
                let usage = event.data.as_ref().and_then(usage_from_event);
                self.emit_usage(usage);
            }
            out.push(event);
        }
        out
    }

    pub fn finish(&mut self) -> Vec<ScannedEvent> {
        let tail = std::mem::take(&mut self.buf);
        if tail.is_empty() {
            return Vec::new();
        }
        let event = Self::scan_block(tail);
        if event.data.is_some() {
            let usage = event.data.as_ref().and_then(usage_from_event);
            self.emit_usage(usage);
        }
        vec![event]
    }
}

/// Chat 同格式转发用：按事件转发原始字节，并按需隐藏 usage-only 事件。
pub struct SseTap {
    scanner: SseScanner,
    wants_usage: bool,
}

impl SseTap {
    pub fn new(wants_usage: bool) -> Self {
        Self {
            scanner: SseScanner::new(),
            wants_usage,
        }
    }

    pub fn usage(&self) -> Usage {
        self.scanner.usage()
    }

    pub fn feed(&mut self, chunk: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        for event in self.scanner.feed(chunk) {
            if event.usage_only && !self.wants_usage {
                continue;
            }
            out.extend_from_slice(&event.raw);
        }
        out
    }

    pub fn finish(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        for event in self.scanner.finish() {
            if event.usage_only && !self.wants_usage {
                continue;
            }
            out.extend_from_slice(&event.raw);
        }
        out
    }
}

fn find_boundary(buf: &[u8]) -> Option<(usize, usize)> {
    let lf = buf.windows(2).position(|w| w == b"\n\n").map(|i| (i, 2));
    let crlf = buf
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| (i, 4));
    match (lf, crlf) {
        (Some(a), Some(b)) => Some(if a.0 <= b.0 { a } else { b }),
        (a, b) => a.or(b),
    }
}

pub mod chat;
pub mod extra;
pub mod forward;
pub mod image_bridge;
pub mod messages;
pub mod passthrough;
pub mod playground;
pub mod responses;
