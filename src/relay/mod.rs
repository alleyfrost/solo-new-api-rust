use std::collections::HashMap;

use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

const MAX_EVENT: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Usage {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
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
    Some(Usage {
        prompt_tokens: prompt,
        completion_tokens: completion,
    })
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

#[derive(Default)]
struct BlockInfo {
    usage: Option<Usage>,
    usage_only: bool,
}

/// 逐事件扫描 SSE：只缓存一个事件，把 usage 增量抽取出来，不缓存完整响应。
pub struct SseTap {
    buf: Vec<u8>,
    usage: Usage,
    wants_usage: bool,
}

impl SseTap {
    pub fn new(wants_usage: bool) -> Self {
        Self {
            buf: Vec::new(),
            usage: Usage::default(),
            wants_usage,
        }
    }

    pub fn usage(&self) -> Usage {
        self.usage
    }

    fn inspect(block: &[u8]) -> BlockInfo {
        let mut data: Vec<u8> = Vec::new();
        for line in block.split(|&b| b == b'\n') {
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
            return BlockInfo::default();
        }
        let Ok(value) = serde_json::from_slice::<Value>(&data) else {
            return BlockInfo::default();
        };
        let usage = usage_from_json(&value);
        let usage_only = usage.is_some()
            && value
                .get("choices")
                .and_then(Value::as_array)
                .map(|a| a.is_empty())
                .unwrap_or(true);
        BlockInfo {
            usage,
            usage_only,
        }
    }

    pub fn feed(&mut self, chunk: &[u8]) -> Vec<u8> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        loop {
            let Some((idx, sep)) = find_boundary(&self.buf) else {
                if self.buf.len() > MAX_EVENT {
                    out.append(&mut self.buf);
                }
                break;
            };
            let block: Vec<u8> = self.buf.drain(..idx + sep).collect();
            let info = Self::inspect(&block);
            if let Some(u) = info.usage {
                self.usage = u;
            }
            if info.usage_only && !self.wants_usage {
                continue;
            }
            out.extend_from_slice(&block);
        }
        out
    }

    pub fn finish(&mut self) -> Vec<u8> {
        let tail = std::mem::take(&mut self.buf);
        if tail.is_empty() {
            return tail;
        }
        let info = Self::inspect(&tail);
        if let Some(u) = info.usage {
            self.usage = u;
        }
        if info.usage_only && !self.wants_usage {
            return Vec::new();
        }
        tail
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
