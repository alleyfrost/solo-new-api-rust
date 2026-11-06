use std::sync::OnceLock;
use std::time::Duration;

use axum::http::{HeaderMap, HeaderName, HeaderValue};
use regex::Regex;

/// 不应转发到上游的头部：hop-by-hop 头、由本服务重建的头（host/content-length/
/// authorization/accept-encoding），以及客户端可能自带的上游凭证头。
const HOP_BY_HOP: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "host",
    "content-length",
    "authorization",
    "x-api-key",
    "accept-encoding",
];

const ANTHROPIC_BASE: &str = "https://api.anthropic.com";
const OPENAI_BASE: &str = "https://api.openai.com";

/// 渠道类型 14（Anthropic）的默认域名与 OpenAI 不同。
fn default_base(channel_type: i64) -> &'static str {
    if channel_type == 14 {
        ANTHROPIC_BASE
    } else {
        OPENAI_BASE
    }
}

/// 拼接上游 URL：base 为空时按渠道类型取默认域名；base 已带 `/v1` 时不重复追加。
pub fn build_url(base_url: &str, path: &str, channel_type: i64) -> String {
    let base = if base_url.trim().is_empty() {
        default_base(channel_type).to_string()
    } else {
        base_url.trim().trim_end_matches('/').to_string()
    };
    let path = path.trim_start_matches('/');
    let path = path.strip_prefix("v1/").unwrap_or(path);
    if base.ends_with("/v1") {
        format!("{base}/{path}")
    } else {
        format!("{base}/v1/{path}")
    }
}

fn client_header_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\{client_header:([^}]+)\}").expect("client_header regex"))
}

/// 替换 `{api_key}` 与 `{client_header:X}` 占位符；客户端头名大小写不敏感。
fn placeholder(value: &str, api_key: &str, client: &HeaderMap) -> String {
    let with_key = value.replace("{api_key}", api_key);
    client_header_re()
        .replace_all(&with_key, |caps: &regex::Captures<'_>| {
            let name = caps[1].to_ascii_lowercase();
            client
                .get(name.as_str())
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string()
        })
        .to_string()
}

/// 按 `header_override` 规则把客户端头改写成上游头。
///
/// 规则（JSON 对象）：
/// - `"*": true`：转发全部客户端头；
/// - `"re:<pattern>": true`：转发名称匹配正则的头（大小写不敏感）；
/// - `"Name": null`：删除该头；
/// - `"Name": "value"`：设置该头，值支持 `{api_key}` / `{client_header:X}` 占位符。
///
/// 未显式设置 Authorization 时默认使用 `Bearer {api_key}`；Accept-Encoding 固定
/// 为 identity，避免上游压得 SSE 无法逐块下发。
pub fn rewrite_headers(client: &HeaderMap, api_key: &str, overrides: Option<&str>) -> HeaderMap {
    let mut out = HeaderMap::new();
    let mut forward_all = false;
    let mut regexes: Vec<Regex> = Vec::new();
    let mut sets: Vec<(String, String)> = Vec::new();
    let mut deletes: Vec<String> = Vec::new();

    if let Some(overrides) = overrides
        && let Ok(serde_json::Value::Object(map)) = serde_json::from_str(overrides)
    {
        for (key, value) in &map {
            if key == "*" {
                forward_all = value == &serde_json::Value::Bool(true);
            } else if let Some(pattern) = key.strip_prefix("re:") {
                if value == &serde_json::Value::Bool(true) {
                    // HeaderMap 的名称已小写化，匹配时忽略大小写。
                    if let Ok(re) = Regex::new(&format!("(?i){pattern}")) {
                        regexes.push(re);
                    }
                }
            } else if value.is_null() {
                deletes.push(key.to_ascii_lowercase());
            } else if let Some(text) = value.as_str() {
                sets.push((key.clone(), text.to_string()));
            }
        }
    }

    for (name, value) in client.iter() {
        let lname = name.as_str();
        if HOP_BY_HOP.contains(&lname) {
            continue;
        }
        if forward_all || regexes.iter().any(|re| re.is_match(lname)) {
            out.insert(name.clone(), value.clone());
        }
    }
    for name in deletes {
        if let Ok(name) = HeaderName::from_bytes(name.as_bytes()) {
            out.remove(name);
        }
    }
    for (name, value) in sets {
        if let Ok(name) = HeaderName::from_bytes(name.as_bytes()) {
            let value = placeholder(&value, api_key, client);
            if let Ok(value) = HeaderValue::from_str(&value) {
                out.insert(name, value);
            }
        }
    }
    if !out.contains_key("authorization")
        && let Ok(value) = HeaderValue::from_str(&format!("Bearer {api_key}"))
    {
        out.insert(HeaderName::from_static("authorization"), value);
    }
    if !out.contains_key("accept-encoding") {
        out.insert(
            HeaderName::from_static("accept-encoding"),
            HeaderValue::from_static("identity"),
        );
    }
    out
}

/// 上游共享客户端：大连接池、禁用重定向（避免把 Key 带去第三方域名）。
pub fn build_client() -> reqwest::Client {
    reqwest::Client::builder()
        .pool_max_idle_per_host(64)
        .tcp_nodelay(true)
        .connect_timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("build reqwest client")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_base_uses_default_domain() {
        assert_eq!(
            build_url("", "/v1/messages", 14),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(
            build_url("  ", "/v1/chat/completions", 1),
            "https://api.openai.com/v1/chat/completions"
        );
    }
}
