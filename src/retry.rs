use std::collections::HashMap;
use std::time::Duration;

/// 重试决策：是否再试一次。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryDecision {
    Retry,
    Stop,
}

/// 根据上游状态码与已用重试次数决定是否继续重试。
///
/// - 网络类失败（无状态码）重试；
/// - 408/429（限流/超时）、401/403（密钥或权限问题，换渠道）、5xx 重试；
/// - 其余 4xx 客户端错误直接停止；
/// - 预算耗尽（`attempt >= max_retry`）停止。
pub fn decide(status: Option<u16>, attempt: usize, max_retry: usize) -> RetryDecision {
    if attempt >= max_retry {
        return RetryDecision::Stop;
    }
    match status {
        None => RetryDecision::Retry,
        Some(408) | Some(429) | Some(401) | Some(403) => RetryDecision::Retry,
        Some(status) if status >= 500 => RetryDecision::Retry,
        Some(_) => RetryDecision::Stop,
    }
}

/// 下一次重试前的等待时长。
///
/// 限流与鉴权失败立即切换渠道（零等待）；其余情况按 100ms 起步指数退避，上限 1s。
pub fn backoff(status: Option<u16>, attempt: usize) -> Duration {
    if matches!(status, Some(429) | Some(401) | Some(403)) {
        return Duration::ZERO;
    }
    let shift = attempt.min(4) as u32;
    let millis = 100u64.saturating_mul(1u64 << shift).min(1000);
    Duration::from_millis(millis)
}

/// 从渠道 `options` 读取 `RetryTimes`（默认 3，上限 10）。
pub fn max_retry_from_options(options: &HashMap<String, String>) -> usize {
    options
        .get("RetryTimes")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(3)
        .min(10)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_caps_and_zero_attempt_boundary() {
        assert_eq!(backoff(None, 0), Duration::from_millis(100));
        assert_eq!(backoff(Some(503), 4), Duration::from_millis(1000));
        assert_eq!(backoff(Some(503), 100), Duration::from_millis(1000));
    }

    #[test]
    fn max_retry_ignores_unparsable_values() {
        let mut options = HashMap::new();
        options.insert("RetryTimes".to_string(), "abc".to_string());
        assert_eq!(max_retry_from_options(&options), 3);
        options.insert("RetryTimes".to_string(), "-1".to_string());
        assert_eq!(max_retry_from_options(&options), 3);
    }
}
