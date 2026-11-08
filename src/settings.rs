use std::collections::HashMap;

use serde_json::Value;

use crate::state::AppState;

pub const KEY_SYSTEM_NAME: &str = "SystemName";
pub const KEY_LOGO: &str = "Logo";
pub const KEY_NOTICE: &str = "Notice";
pub const KEY_SERVER_ADDRESS: &str = "ServerAddress";
pub const KEY_LOG_CONSUME_ENABLED: &str = "LogConsumeEnabled";
pub const KEY_LOG_EXPORT_ENABLED: &str = "log_setting.user_export_enabled";
pub const KEY_LOG_MAX_RANGE_DAYS: &str = "log_setting.max_range_days";
pub const KEY_LOG_RETENTION_DAYS: &str = "LogRetentionDays";
/// 只会走 `/v1/images/generations` 的模型；列在这里的模型允许用 chat 请求触发生图。
pub const KEY_IMAGE_MODELS: &str = "ImageModels";
/// 大请求体（2–8 MiB / >8 MiB）的并发闸门，限制峰值内存。
pub const KEY_BODY_GATE_BIG: &str = "BodyGateBigConcurrency";
pub const KEY_BODY_GATE_HUGE: &str = "BodyGateHugeConcurrency";

pub const KEYS: [&str; 11] = [
    KEY_BODY_GATE_BIG,
    KEY_BODY_GATE_HUGE,
    KEY_IMAGE_MODELS,
    KEY_LOG_CONSUME_ENABLED,
    KEY_LOG_EXPORT_ENABLED,
    KEY_LOG_MAX_RANGE_DAYS,
    KEY_LOG_RETENTION_DAYS,
    KEY_LOGO,
    KEY_NOTICE,
    KEY_SERVER_ADDRESS,
    KEY_SYSTEM_NAME,
];

const MAX_SYSTEM_NAME_CHARS: usize = 64;
const MAX_LOGO_CHARS: usize = 512;
const MAX_NOTICE_CHARS: usize = 2000;
const MAX_ADDRESS_CHARS: usize = 256;
pub const MIN_RANGE_DAYS: i64 = 1;
pub const MAX_RANGE_DAYS: i64 = 3650;
pub const MIN_RETENTION_DAYS: i64 = 0;
pub const MAX_RETENTION_DAYS: i64 = 3650;
pub const DEFAULT_LOG_RETENTION_DAYS: i64 = 14;
pub const MAX_IMAGE_MODELS: usize = 64;
pub const MAX_MODEL_NAME_CHARS: usize = 128;
const MAX_IMAGE_MODELS_CHARS: usize = 4096;
pub const MIN_BODY_GATE: usize = 1;
pub const MAX_BODY_GATE: usize = 64;
pub const DEFAULT_BODY_GATE_BIG: usize = 8;
pub const DEFAULT_BODY_GATE_HUGE: usize = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub system_name: String,
    pub logo: String,
    pub notice: String,
    pub server_address: String,
    pub log_consume_enabled: bool,
    pub log_export_enabled: bool,
    pub log_max_range_days: i64,
    pub log_retention_days: i64,
    pub image_models: Vec<String>,
    pub body_gate_big: usize,
    pub body_gate_huge: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            system_name: crate::routes::status::DEFAULT_SYSTEM_NAME.to_string(),
            logo: String::new(),
            notice: String::new(),
            server_address: String::new(),
            log_consume_enabled: true,
            log_export_enabled: true,
            log_max_range_days: crate::routes::admin::log::MAX_EXPORT_DAYS,
            log_retention_days: DEFAULT_LOG_RETENTION_DAYS,
            image_models: Vec::new(),
            body_gate_big: DEFAULT_BODY_GATE_BIG,
            body_gate_huge: DEFAULT_BODY_GATE_HUGE,
        }
    }
}

impl Settings {
    pub fn from_options(options: &HashMap<String, String>) -> Self {
        let defaults = Self::default();
        let boolean = |key: &str, fallback: bool| -> bool {
            options
                .get(key)
                .and_then(|v| parse_bool(v).ok())
                .unwrap_or(fallback)
        };
        Self {
            system_name: nonblank_or(options, KEY_SYSTEM_NAME, &defaults.system_name),
            logo: options
                .get(KEY_LOGO)
                .map(String::as_str)
                .unwrap_or("")
                .to_string(),
            notice: options
                .get(KEY_NOTICE)
                .map(String::as_str)
                .unwrap_or("")
                .to_string(),
            server_address: options
                .get(KEY_SERVER_ADDRESS)
                .map(String::as_str)
                .unwrap_or("")
                .to_string(),
            log_consume_enabled: boolean(KEY_LOG_CONSUME_ENABLED, defaults.log_consume_enabled),
            log_export_enabled: boolean(KEY_LOG_EXPORT_ENABLED, defaults.log_export_enabled),
            log_max_range_days: options
                .get(KEY_LOG_MAX_RANGE_DAYS)
                .and_then(|v| v.parse::<i64>().ok())
                .filter(|v| (MIN_RANGE_DAYS..=MAX_RANGE_DAYS).contains(v))
                .unwrap_or(defaults.log_max_range_days),
            log_retention_days: options
                .get(KEY_LOG_RETENTION_DAYS)
                .and_then(|v| v.parse::<i64>().ok())
                .filter(|v| (MIN_RETENTION_DAYS..=MAX_RETENTION_DAYS).contains(v))
                .unwrap_or(defaults.log_retention_days),
            image_models: split_models(
                options
                    .get(KEY_IMAGE_MODELS)
                    .map(String::as_str)
                    .unwrap_or(""),
            ),
            body_gate_big: gate_limit(options.get(KEY_BODY_GATE_BIG), defaults.body_gate_big),
            body_gate_huge: gate_limit(options.get(KEY_BODY_GATE_HUGE), defaults.body_gate_huge),
        }
    }

    pub async fn load(state: &AppState) -> Self {
        let caches = state.caches.read().expect("cache lock");
        Self::from_options(&caches.options)
    }

    /// 白名单键的当前生效值，供 `GET /api/option/` 返回；键名按字母序，前端可直接铺表单。
    pub fn entries(&self) -> Vec<(&'static str, String)> {
        vec![
            (KEY_BODY_GATE_BIG, self.body_gate_big.to_string()),
            (KEY_BODY_GATE_HUGE, self.body_gate_huge.to_string()),
            (KEY_IMAGE_MODELS, self.image_models.join(",")),
            (KEY_LOGO, self.logo.clone()),
            (
                KEY_LOG_CONSUME_ENABLED,
                self.log_consume_enabled.to_string(),
            ),
            (KEY_LOG_EXPORT_ENABLED, self.log_export_enabled.to_string()),
            (KEY_LOG_MAX_RANGE_DAYS, self.log_max_range_days.to_string()),
            (KEY_LOG_RETENTION_DAYS, self.log_retention_days.to_string()),
            (KEY_NOTICE, self.notice.clone()),
            (KEY_SERVER_ADDRESS, self.server_address.clone()),
            (KEY_SYSTEM_NAME, self.system_name.clone()),
        ]
    }

    pub fn is_image_model(&self, model: &str) -> bool {
        self.image_models.iter().any(|m| m == model)
    }
}

/// 校验并归一化成落库的字符串；返回 `Err(message)` 时调用方按业务错误回给前端。
pub fn normalize(key: &str, raw: &Value) -> Result<String, String> {
    let text = match raw {
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        _ => return Err("Invalid parameters".to_string()),
    };
    match key {
        KEY_SYSTEM_NAME => {
            let trimmed = text.trim();
            if trimmed.is_empty() || trimmed.chars().count() > MAX_SYSTEM_NAME_CHARS {
                return Err(format!(
                    "System name must be 1-{MAX_SYSTEM_NAME_CHARS} characters"
                ));
            }
            Ok(trimmed.to_string())
        }
        KEY_LOGO => validate_urlish(
            &text,
            MAX_LOGO_CHARS,
            &["http://", "https://", "/", "data:image/"],
        ),
        KEY_NOTICE => {
            if text.chars().count() > MAX_NOTICE_CHARS {
                return Err(format!(
                    "Notice must be at most {MAX_NOTICE_CHARS} characters"
                ));
            }
            Ok(text.clone())
        }
        KEY_SERVER_ADDRESS => {
            let trimmed = text.trim().trim_end_matches('/');
            if trimmed.is_empty() {
                return Ok(String::new());
            }
            validate_urlish(trimmed, MAX_ADDRESS_CHARS, &["http://", "https://"])
        }
        KEY_LOG_CONSUME_ENABLED | KEY_LOG_EXPORT_ENABLED => {
            parse_bool(&text).map(|b| b.to_string())
        }
        KEY_LOG_MAX_RANGE_DAYS => {
            let days = text.trim().parse::<i64>().map_err(|_| {
                format!("Expected an integer between {MIN_RANGE_DAYS} and {MAX_RANGE_DAYS}")
            })?;
            if !(MIN_RANGE_DAYS..=MAX_RANGE_DAYS).contains(&days) {
                return Err(format!(
                    "Expected an integer between {MIN_RANGE_DAYS} and {MAX_RANGE_DAYS}"
                ));
            }
            Ok(days.to_string())
        }
        KEY_BODY_GATE_BIG | KEY_BODY_GATE_HUGE => {
            let limit = text.trim().parse::<usize>().map_err(|_| {
                format!("Expected an integer between {MIN_BODY_GATE} and {MAX_BODY_GATE}")
            })?;
            if !(MIN_BODY_GATE..=MAX_BODY_GATE).contains(&limit) {
                return Err(format!(
                    "Expected an integer between {MIN_BODY_GATE} and {MAX_BODY_GATE}"
                ));
            }
            Ok(limit.to_string())
        }
        KEY_LOG_RETENTION_DAYS => {
            let days = text.trim().parse::<i64>().map_err(|_| {
                format!("Expected an integer between {MIN_RETENTION_DAYS} and {MAX_RETENTION_DAYS}")
            })?;
            if !(MIN_RETENTION_DAYS..=MAX_RETENTION_DAYS).contains(&days) {
                return Err(format!(
                    "Expected an integer between {MIN_RETENTION_DAYS} and {MAX_RETENTION_DAYS}"
                ));
            }
            Ok(days.to_string())
        }
        KEY_IMAGE_MODELS => {
            let models = split_models(&text);
            if models.len() > MAX_IMAGE_MODELS {
                return Err(format!("At most {MAX_IMAGE_MODELS} image models"));
            }
            for model in &models {
                if model.chars().count() > MAX_MODEL_NAME_CHARS {
                    return Err(format!(
                        "Each model name must be at most {MAX_MODEL_NAME_CHARS} characters"
                    ));
                }
                if !model
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-._:/@+".contains(c))
                {
                    return Err(format!("Invalid model name: {model}"));
                }
            }
            let joined = models.join(",");
            if joined.chars().count() > MAX_IMAGE_MODELS_CHARS {
                return Err(format!(
                    "Image model list must be at most {MAX_IMAGE_MODELS_CHARS} characters"
                ));
            }
            Ok(joined)
        }
        other => Err(format!("Option {other} is not supported")),
    }
}

fn split_models(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for item in text.split(',') {
        let name = item.trim();
        if name.is_empty() || out.iter().any(|existing| existing == name) {
            continue;
        }
        out.push(name.to_string());
    }
    out
}

fn gate_limit(raw: Option<&String>, fallback: usize) -> usize {
    raw.and_then(|v| v.parse::<usize>().ok())
        .filter(|v| (MIN_BODY_GATE..=MAX_BODY_GATE).contains(v))
        .unwrap_or(fallback)
}

fn parse_bool(text: &str) -> Result<bool, String> {
    match text.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "on" => Ok(true),
        "false" | "0" | "off" => Ok(false),
        _ => Err("Expected true or false".to_string()),
    }
}

fn validate_urlish(text: &str, max_chars: usize, prefixes: &[&str]) -> Result<String, String> {
    if text.is_empty() {
        return Ok(String::new());
    }
    if text.chars().count() > max_chars {
        return Err(format!("Must be at most {max_chars} characters"));
    }
    if text.chars().any(char::is_whitespace) {
        return Err("Must not contain whitespace".to_string());
    }
    if !prefixes.iter().any(|p| text.starts_with(p)) {
        return Err(format!("Must start with {}", prefixes.join(" / ")));
    }
    Ok(text.to_string())
}

fn nonblank_or(options: &HashMap<String, String>, key: &str, fallback: &str) -> String {
    options
        .get(key)
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map_or_else(|| fallback.to_string(), ToString::to_string)
}
