# M2: 转发核心（选路 / 重试 / Chat 透传 / 日志）实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 M1 基础上实现渠道选择、多 Key 轮询、429/5xx 重试、OpenAI Chat 透传与用量日志，使 `/v1/chat/completions` 可对真实上游工作。

**Architecture:** 进程内 `RoutingState`（Key 游标 + 健康状态）+ 不可变缓存选路；reqwest 连接池转发；SSE 零缓冲；日志有界队列批量写 `logs`。

**Tech Stack:** axum 0.8, reqwest 0.13 (rustls/stream), tokio, sqlx, rand, wiremock (dev).

**Spec:** `docs/superpowers/specs/2026-11-06-solo-new-api-rust-design.md`

## Global Constraints

- 不引入 Redis；Key 轮询/健康状态全部进程内。
- 禁止缓冲完整流式响应统计 usage；断连立即取消上游。
- 只写 `logs`（quota=0）与 `tokens.accessed_time`；不改额度字段。
- 所有测试 hermetic（wiremock/临时库），真实上游冒烟需用户授权。
- 不 exec git commit；主代理审核后提交。

---

## Task 1: 渠道选择与多 Key 轮询

**Files:**
- Create: `src/routing.rs`
- Modify: `src/lib.rs`
- Test: `tests/routing.rs`

**Interfaces:**
- Produces:
  - `routing::SelectedChannel { channel_id: i64, channel_type: i64, channel_name: String, key: String, key_index: usize }`
  - `routing::RoutingState::new()`
  - `routing::select_channel(caches: &Caches, state: &RoutingState, group: &str, model: &str, retry: usize, used: &HashSet<(i64, usize)>) -> Option<SelectedChannel>`
  - `routing::mark_key_unhealthy(state: &RoutingState, channel_id: i64, key_index: usize)`
  - `routing::key_healthy(state: &RoutingState, channel_id: i64, key_index: usize) -> bool`

- [ ] **Step 1: 写失败测试 `tests/routing.rs`**

构造 caches：
- channel 1 priority 10 weight 0，two keys `k1\nk2`，abilities `vip:m1`；
- channel 2 priority 5 weight 0，key `k3`，abilities `vip:m1`；
- channel 3 priority 10 weight 0，status 2（禁用），abilities `vip:m1`；
- channel 4 priority 10，abilities `default:m1`。

断言：
1. `retry=0` 选 priority 10，不会选 channel 2/3/4；
2. `retry=1` 选 priority 5 → channel 2；
3. `retry=5` 仍选最低优先级 channel 2；
4. 多 Key：连续调用（used 为空）在 channel 1 的两个 key 间轮换；used 含 `(1,0)` 时选 `(1,1)`；
5. `mark_key_unhealthy(1,0)` 后不再选 key_index 0；
6. group `default` 只能选 channel 4。

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --test routing`
Expected: 编译失败（`routing` 模块不存在）。

- [ ] **Step 3: 实现 `src/routing.rs`**

```rust
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use rand::Rng;
use crate::model::{AbilityRow, ChannelRow};
use crate::state::Caches;

const KEY_DISABLED_TTL: Duration = Duration::from_secs(600);

#[derive(Debug, Clone)]
pub struct SelectedChannel {
    pub channel_id: i64,
    pub channel_type: i64,
    pub channel_name: String,
    pub key: String,
    pub key_index: usize,
}

#[derive(Default)]
pub struct RoutingState {
    cursors: std::sync::Mutex<HashMap<i64, usize>>,
    unhealthy: std::sync::Mutex<HashMap<(i64, usize), Instant>>,
}

impl RoutingState {
    pub fn new() -> Self { Self::default() }
}

pub fn key_healthy(state: &RoutingState, channel_id: i64, key_index: usize) -> bool {
    let map = state.unhealthy.lock().unwrap();
    match map.get(&(channel_id, key_index)) {
        Some(until) => *until > Instant::now(),
        None => true,
    }
}

pub fn mark_key_unhealthy(state: &RoutingState, channel_id: i64, key_index: usize) {
    let mut map = state.unhealthy.lock().unwrap();
    map.insert((channel_id, key_index), Instant::now() + KEY_DISABLED_TTL);
}

fn channel_keys(channel: &ChannelRow) -> Vec<String> {
    channel.key.lines().map(str::trim).filter(|k| !k.is_empty()).map(str::to_string).collect()
}

fn candidate_rows<'a>(caches: &'a Caches, group: &str, model: &str) -> Vec<(&'a AbilityRow, &'a ChannelRow)> {
    let Some(rows) = caches.abilities.get(&(group.to_string(), model.to_string())) else { return Vec::new() };
    rows.iter()
        .filter(|a| a.enabled)
        .filter_map(|a| caches.channels.get(&a.channel_id).map(|c| (a, c)))
        .filter(|(_, c)| c.status == 1)
        .collect()
}

fn priority_tiers(candidates: &[(&AbilityRow, &ChannelRow)]) -> Vec<i64> {
    let mut tiers: Vec<i64> = candidates.iter().map(|(a, _)| a.priority.unwrap_or(0)).collect();
    tiers.sort_unstable_by(|a, b| b.cmp(a));
    tiers.dedup();
    tiers
}

fn weighted_pick<'a, R: Rng>(items: &'a [(&'a AbilityRow, &'a ChannelRow)], rng: &mut R) -> &'a (&'a AbilityRow, &'a ChannelRow) {
    let total: u64 = items.iter().map(|(a, _)| a.weight.unwrap_or(0).max(0) as u64).sum();
    if total == 0 {
        return &items[rng.gen_range(0..items.len())];
    }
    let mut roll = rng.gen_range(0..total);
    for item in items {
        let w = item.0.weight.unwrap_or(0).max(0) as u64;
        if roll < w { return item; }
        roll -= w;
    }
    &items[items.len() - 1]
}

pub fn select_channel(
    caches: &Caches,
    state: &RoutingState,
    group: &str,
    model: &str,
    retry: usize,
    used: &HashSet<(i64, usize)>,
) -> Option<SelectedChannel> {
    let candidates = candidate_rows(caches, group, model);
    if candidates.is_empty() { return None; }
    let tiers = priority_tiers(&candidates);
    let tier = tiers[retry.min(tiers.len() - 1)];
    let tier_items: Vec<(&AbilityRow, &ChannelRow)> = candidates.into_iter()
        .filter(|(a, _)| a.priority.unwrap_or(0) == tier)
        .collect();
    if tier_items.is_empty() { return None; }

    let mut rng = rand::thread_rng();
    // 按权重无放回抽样，得到尝试顺序；权重全 0 时退化为均匀随机。
    let mut remaining = tier_items.clone();
    let mut order: Vec<(&AbilityRow, &ChannelRow)> = Vec::with_capacity(remaining.len());
    while !remaining.is_empty() {
        let picked = *weighted_pick(&remaining, &mut rng);
        remaining.retain(|(_, c)| c.id != picked.1.id);
        order.push(picked);
    }
    for (_, channel) in order {
        let keys = channel_keys(channel);
        if keys.is_empty() { continue; }
        let start = {
            let mut cursors = state.cursors.lock().unwrap();
            cursors.entry(channel.id).or_insert(0)
        };
        for offset in 0..keys.len() {
            let idx = (start + offset) % keys.len();
            if !key_healthy(state, channel.id, idx) { continue; }
            if used.contains(&(channel.id, idx)) { continue; }
            let mut cursors = state.cursors.lock().unwrap();
            cursors.insert(channel.id, (idx + 1) % keys.len());
            return Some(SelectedChannel {
                channel_id: channel.id,
                channel_type: channel.r#type,
                channel_name: channel.name.clone(),
                key: keys[idx].clone(),
                key_index: idx,
            });
        }
    }
    // 所有组合都用过：允许复用第一个健康 Key。
    for (_, channel) in tier_items {
        let keys = channel_keys(channel);
        for (idx, key) in keys.into_iter().enumerate() {
            if key_healthy(state, channel.id, idx) {
                return Some(SelectedChannel {
                    channel_id: channel.id,
                    channel_type: channel.r#type,
                    channel_name: channel.name.clone(),
                    key,
                    key_index: idx,
                });
            }
        }
    }
    None
}
```

修正 borrow：`cursors.entry(...).or_insert(0)` 返回 `&mut usize`，需 `*` 取值：

```rust
let start = {
    let mut cursors = state.cursors.lock().unwrap();
    *cursors.entry(channel.id).or_insert(0)
};
```

- [ ] **Step 4: `src/lib.rs` 增加 `pub mod routing;`**

- [ ] **Step 5: 运行测试**

Run: `cargo test --test routing`
Expected: PASS（6 个断言组）。

- [ ] **Step 6: 全量回归**

Run: `cargo test`
Expected: 全绿。

- [ ] **Step 7: 提交（主代理执行）**

```bash
git add src/routing.rs src/lib.rs tests/routing.rs
git commit -m "feat(m2): channel selection with priority and multi-key rotation"
```

---

## Task 2: 上游 URL、Header 改写与 HTTP 客户端

**Files:**
- Create: `src/upstream.rs`
- Modify: `Cargo.toml`（reqwest）、`src/lib.rs`
- Test: `tests/upstream.rs`

**Interfaces:**
- Produces:
  - `upstream::build_url(base_url: &str, path: &str, channel_type: i64) -> String`
  - `upstream::rewrite_headers(client: &HeaderMap, api_key: &str, overrides: Option<&str>) -> reqwest::header::HeaderMap`
  - `upstream::build_client() -> reqwest::Client`

- [ ] **Step 1: 加依赖**

```toml
reqwest = { version = "0.13", default-features = false, features = ["rustls-tls", "stream", "json"] }
```

（若 0.13 的 feature 名不同，按编译提示改为 `rustls`。）

- [ ] **Step 2: 写失败测试 `tests/upstream.rs`**

```rust
use axum::http::{HeaderMap, HeaderValue};

#[test]
fn builds_url_with_and_without_v1_suffix() {
    assert_eq!(solo_new_api_rust::upstream::build_url("https://a.cn", "/v1/chat/completions", 1),
               "https://a.cn/v1/chat/completions");
    assert_eq!(solo_new_api_rust::upstream::build_url("https://a.cn/", "/v1/messages", 14),
               "https://a.cn/v1/messages");
    assert_eq!(solo_new_api_rust::upstream::build_url("https://a.cn/v1", "/v1/chat/completions", 1),
               "https://a.cn/v1/chat/completions");
}

#[test]
fn forwards_all_headers_and_sets_auth() {
    let mut client = HeaderMap::new();
    client.insert("x-trace-id", HeaderValue::from_static("t1"));
    client.insert("x-drop", HeaderValue::from_static("d"));
    let overrides = r#"{"*":true,"re:^X-Trace-.*$":true,"X-Drop":null,"X-Foo":"v-{client_header:x-trace-id}","Authorization":"Bearer {api_key}"}"#;
    let out = solo_new_api_rust::upstream::rewrite_headers(&client, "sk-up", Some(overrides));
    assert_eq!(out.get("x-trace-id").unwrap(), "t1");
    assert!(out.get("x-drop").is_none());
    assert_eq!(out.get("x-foo").unwrap(), "v-t1");
    assert_eq!(out.get("authorization").unwrap(), "Bearer sk-up");
    assert_eq!(out.get("accept-encoding").unwrap(), "identity");
}

#[test]
fn regex_only_forwards_matching_when_no_wildcard() {
    let mut client = HeaderMap::new();
    client.insert("x-trace-id", HeaderValue::from_static("t1"));
    client.insert("x-other", HeaderValue::from_static("o"));
    let out = solo_new_api_rust::upstream::rewrite_headers(&client, "sk-up", Some(r#"{"re:^X-Trace-.*$":true}"#));
    assert!(out.get("x-trace-id").is_some());
    assert!(out.get("x-other").is_none());
    assert_eq!(out.get("authorization").unwrap(), "Bearer sk-up");
}
```

- [ ] **Step 3: 实现 `src/upstream.rs`**

```rust
use std::time::Duration;
use axum::http::{HeaderMap, HeaderName, HeaderValue};
use regex::Regex;

const HOP_BY_HOP: &[&str] = &[
    "connection", "keep-alive", "proxy-authenticate", "proxy-authorization",
    "te", "trailer", "transfer-encoding", "upgrade", "host", "content-length",
    "authorization", "x-api-key", "accept-encoding",
];

pub fn build_url(base_url: &str, path: &str, channel_type: i64) -> String {
    let base = if base_url.trim().is_empty() {
        if channel_type == 14 { "https://api.anthropic.com" } else { "https://api.openai.com" }
    } else {
        base_url.trim_end_matches('/')
    };
    let path = path.trim_start_matches('/');
    let path = path.strip_prefix("v1/").unwrap_or(path);
    if base.ends_with("/v1") {
        format!("{base}/{path}")
    } else {
        format!("{base}/v1/{path}")
    }
}

fn placeholder(value: &str, api_key: &str, client: &HeaderMap) -> String {
    let mut out = value.replace("{api_key}", api_key);
    let re = Regex::new(r"\{client_header:([^}]+)\}").unwrap();
    out = re.replace_all(&out, |caps: &regex::Captures| {
        let name = caps[1].to_ascii_lowercase();
        client.get(name.as_str()).and_then(|v| v.to_str().ok()).unwrap_or("").to_string()
    }).to_string();
    out
}

pub fn rewrite_headers(client: &HeaderMap, api_key: &str, overrides: Option<&str>) -> HeaderMap {
    let mut out = HeaderMap::new();
    let mut forward_all = false;
    let mut regexes: Vec<Regex> = Vec::new();
    let mut sets: Vec<(String, String)> = Vec::new();
    let mut deletes: Vec<String> = Vec::new();

    if let Some(overrides) = overrides {
        if let Ok(serde_json::Value::Object(map)) = serde_json::from_str(overrides) {
            for (key, value) in map {
                if key == "*" && value == serde_json::Value::Bool(true) {
                    forward_all = true;
                } else if let Some(pattern) = key.strip_prefix("re:") {
                    if value == serde_json::Value::Bool(true) {
                        // HeaderMap 的名称已小写化，匹配时忽略大小写。
                        if let Ok(re) = Regex::new(&format!("(?i){pattern}")) { regexes.push(re); }
                    }
                } else if value.is_null() {
                    deletes.push(key.to_ascii_lowercase());
                } else if let Some(text) = value.as_str() {
                    sets.push((key, text.to_string()));
                }
            }
        }
    }

    for (name, value) in client.iter() {
        let lname = name.as_str().to_ascii_lowercase();
        if HOP_BY_HOP.contains(&lname.as_str()) { continue; }
        if forward_all || regexes.iter().any(|re| re.is_match(name.as_str())) {
            out.insert(name.clone(), value.clone());
        }
    }
    for name in deletes {
        if let Ok(name) = HeaderName::from_bytes(name.as_bytes()) { out.remove(name); }
    }
    for (name, value) in sets {
        if let Ok(name) = HeaderName::from_bytes(name.as_bytes()) {
            let value = placeholder(&value, api_key, client);
            if let Ok(value) = HeaderValue::from_str(&value) { out.insert(name, value); }
        }
    }
    if !out.contains_key("authorization") {
        if let Ok(value) = HeaderValue::from_str(&format!("Bearer {api_key}")) {
            out.insert(HeaderName::from_static("authorization"), value);
        }
    }
    if !out.contains_key("accept-encoding") {
        out.insert(HeaderName::from_static("accept-encoding"), HeaderValue::from_static("identity"));
    }
    out
}

pub fn build_client() -> reqwest::Client {
    reqwest::Client::builder()
        .pool_max_idle_per_host(64)
        .tcp_nodelay(true)
        .connect_timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("build reqwest client")
}
```

- [ ] **Step 4: `src/lib.rs` 增加 `pub mod upstream;`**

- [ ] **Step 5: 运行测试**

Run: `cargo test --test upstream`
Expected: PASS。

- [ ] **Step 6: 全量回归并提交（主代理）**

Run: `cargo test`
Expected: 全绿。

```bash
git add Cargo.toml Cargo.lock src/upstream.rs src/lib.rs tests/upstream.rs
git commit -m "feat(m2): upstream url and header rewriting"
```

---

## Task 3: 重试决策与退避

**Files:**
- Create: `src/retry.rs`
- Modify: `src/lib.rs`
- Test: `tests/retry.rs`

**Interfaces:**
- Produces:
  - `retry::RetryDecision { Retry, Stop }`
  - `retry::decide(status: Option<u16>, attempt: usize, max_retry: usize) -> RetryDecision`
  - `retry::backoff(status: Option<u16>, attempt: usize) -> std::time::Duration`
  - `retry::max_retry_from_options(options: &std::collections::HashMap<String, String>) -> usize`

- [ ] **Step 1: 写失败测试 `tests/retry.rs`**

```rust
use std::collections::HashMap;
use std::time::Duration;
use solo_new_api_rust::retry::{backoff, decide, max_retry_from_options, RetryDecision};

#[test]
fn retries_rate_limits_server_errors_auth_and_network() {
    for status in [Some(408), Some(429), Some(401), Some(403), Some(500), Some(503), None] {
        assert_eq!(decide(status, 0, 10), RetryDecision::Retry, "status {status:?}");
    }
}

#[test]
fn stops_on_client_errors_and_budget_exhaustion() {
    for status in [Some(400), Some(404), Some(413), Some(422)] {
        assert_eq!(decide(status, 0, 10), RetryDecision::Stop, "status {status:?}");
    }
    assert_eq!(decide(Some(503), 10, 10), RetryDecision::Stop);
}

#[test]
fn rate_limit_rotates_immediately_and_server_errors_back_off() {
    assert_eq!(backoff(Some(429), 3), Duration::ZERO);
    assert_eq!(backoff(Some(401), 3), Duration::ZERO);
    assert_eq!(backoff(Some(500), 0), Duration::from_millis(100));
    assert_eq!(backoff(Some(500), 1), Duration::from_millis(200));
    assert_eq!(backoff(Some(500), 9), Duration::from_millis(1000));
}

#[test]
fn retry_budget_reads_option_and_clamps() {
    let mut options = HashMap::new();
    assert_eq!(max_retry_from_options(&options), 3);
    options.insert("RetryTimes".into(), "10".into());
    assert_eq!(max_retry_from_options(&options), 10);
    options.insert("RetryTimes".into(), "99".into());
    assert_eq!(max_retry_from_options(&options), 10);
    options.insert("RetryTimes".into(), "0".into());
    assert_eq!(max_retry_from_options(&options), 0);
}
```

- [ ] **Step 2: 实现 `src/retry.rs`**

```rust
use std::collections::HashMap;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryDecision {
    Retry,
    Stop,
}

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

pub fn backoff(status: Option<u16>, attempt: usize) -> Duration {
    if matches!(status, Some(429) | Some(401) | Some(403)) {
        return Duration::ZERO;
    }
    let shift = attempt.min(4) as u32;
    let millis = 100u64.saturating_mul(1u64 << shift).min(1000);
    Duration::from_millis(millis)
}

pub fn max_retry_from_options(options: &HashMap<String, String>) -> usize {
    options.get("RetryTimes")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(3)
        .min(10)
}
```

- [ ] **Step 3: `src/lib.rs` 增加 `pub mod retry;`**

- [ ] **Step 4: 运行测试**

Run: `cargo test --test retry`
Expected: PASS。

- [ ] **Step 5: 全量回归并提交（主代理）**

Run: `cargo test`
Expected: 全绿。

```bash
git add src/retry.rs src/lib.rs tests/retry.rs
git commit -m "feat(m2): retry decisions and backoff"
```

---

## Task 4: 用量日志批量写入与 AppState 扩展

**Files:**
- Create: `src/logging.rs`
- Modify: `src/state.rs`、`src/main.rs`、`src/lib.rs`、所有测试的 AppState 构造
- Test: `tests/log_writer.rs`

**Interfaces:**
- Produces:
  - `logging::LogEntry { created_at,user_id,username,token_name,model_name,prompt_tokens,completion_tokens,use_time_secs,is_stream,channel_id,channel_name,token_id,group,ip,request_id,upstream_request_id,other }`
  - `logging::channel(capacity) -> (mpsc::Sender<LogEntry>, mpsc::Receiver<LogEntry>)`
  - `logging::spawn_writer(pool, rx) -> JoinHandle<()>`
  - `logging::enqueue(state: &AppState, entry: LogEntry)`（无 sender 时忽略；`send` 超时 2s 后丢弃并 warn）
  - `AppState::new(config, pool, caches, jwt_secret, log_tx: Option<Sender<LogEntry>>) -> AppState`，内部构建 `client`、`routing`
  - `AppState` 新增 `client: reqwest::Client`、`routing: Arc<RoutingState>`、`log_tx: Option<mpsc::Sender<LogEntry>>`

- [ ] **Step 1: 写失败测试 `tests/log_writer.rs`**

```rust
#[tokio::test]
async fn batch_writer_inserts_consume_logs() {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::query("CREATE TABLE logs (id INTEGER PRIMARY KEY AUTOINCREMENT,user_id INTEGER,created_at INTEGER,type INTEGER,content TEXT,username TEXT,token_name TEXT,model_name TEXT,quota INTEGER,prompt_tokens INTEGER,completion_tokens INTEGER,use_time INTEGER,is_stream NUMERIC,channel_id INTEGER,channel_name TEXT,token_id INTEGER,`group` TEXT,ip TEXT,request_id TEXT,upstream_request_id TEXT,other TEXT)").execute(&pool).await.unwrap();
    let (tx, rx) = solo_new_api_rust::logging::channel(16);
    let writer = solo_new_api_rust::logging::spawn_writer(pool.clone(), rx);
    for i in 0..2 {
        tx.send(solo_new_api_rust::logging::LogEntry {
            created_at: 100 + i, user_id: 1, username: "root".into(), token_name: "t".into(),
            model_name: "m".into(), prompt_tokens: 10, completion_tokens: 5, use_time_secs: 1,
            is_stream: true, channel_id: 3, channel_name: "c".into(), token_id: 2,
            group: "default".into(), ip: "127.0.0.1".into(), request_id: format!("r{i}"),
            upstream_request_id: String::new(), other: "{}".into(),
        }).await.unwrap();
    }
    drop(tx);
    writer.await.unwrap();
    let (count, quota, pt): (i64, i64, i64) = sqlx::query_as("SELECT COUNT(*), SUM(quota), SUM(prompt_tokens) FROM logs").fetch_one(&pool).await.unwrap();
    assert_eq!((count, quota, pt), (2, 0, 20));
}
```

- [ ] **Step 2: 实现 `src/logging.rs`**

```rust
use std::time::Duration;
use sqlx::SqlitePool;
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub created_at: i64,
    pub user_id: i64,
    pub username: String,
    pub token_name: String,
    pub model_name: String,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub use_time_secs: i64,
    pub is_stream: bool,
    pub channel_id: i64,
    pub channel_name: String,
    pub token_id: i64,
    pub group: String,
    pub ip: String,
    pub request_id: String,
    pub upstream_request_id: String,
    pub other: String,
}

pub fn channel(capacity: usize) -> (mpsc::Sender<LogEntry>, mpsc::Receiver<LogEntry>) {
    mpsc::channel(capacity)
}

pub fn spawn_writer(pool: SqlitePool, mut rx: mpsc::Receiver<LogEntry>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut batch: Vec<LogEntry> = Vec::with_capacity(128);
        loop {
            match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
                Ok(Some(entry)) => {
                    batch.push(entry);
                    while batch.len() < 100 {
                        match rx.try_recv() {
                            Ok(entry) => batch.push(entry),
                            Err(_) => break,
                        }
                    }
                }
                Ok(None) => {
                    flush(&pool, &mut batch).await;
                    break;
                }
                Err(_) => {}
            }
            if !batch.is_empty() {
                flush(&pool, &mut batch).await;
            }
        }
    })
}

async fn flush(pool: &SqlitePool, batch: &mut Vec<LogEntry>) {
    if batch.is_empty() { return; }
    let mut tx = match pool.begin().await { Ok(tx) => tx, Err(e) => { tracing::error!("log tx begin: {e}"); batch.clear(); return; } };
    for e in batch.drain(..) {
        if let Err(err) = sqlx::query(
            "INSERT INTO logs (user_id,created_at,type,content,username,token_name,model_name,quota,prompt_tokens,completion_tokens,use_time,is_stream,channel_id,channel_name,token_id,`group`,ip,request_id,upstream_request_id,other) \
             VALUES (?,?,2,NULL,?,?,?,0,?,?,?,?,?,?,?,?,?,?,?,?)"
        )
        .bind(e.user_id).bind(e.created_at).bind(e.username).bind(e.token_name).bind(e.model_name)
        .bind(e.prompt_tokens).bind(e.completion_tokens).bind(e.use_time_secs).bind(e.is_stream)
        .bind(e.channel_id).bind(e.channel_name).bind(e.token_id).bind(e.group).bind(e.ip)
        .bind(e.request_id).bind(e.upstream_request_id).bind(e.other)
        .execute(&mut *tx).await
        {
            tracing::error!("insert log failed: {err}");
        }
    }
    if let Err(e) = tx.commit().await { tracing::error!("log tx commit: {e}"); }
}

pub async fn enqueue(state: &crate::state::AppState, entry: LogEntry) {
    let Some(tx) = &state.log_tx else { return; };
    match tokio::time::timeout(Duration::from_secs(2), tx.send(entry)).await {
        Ok(Ok(())) => {}
        Ok(Err(_)) => tracing::warn!("log queue closed; entry dropped"),
        Err(_) => tracing::warn!("log queue full; entry dropped"),
    }
}
```

- [ ] **Step 3: 扩展 `AppState`**

```rust
#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub pool: SqlitePool,
    pub caches: Arc<RwLock<Caches>>,
    pub jwt_secret: Arc<Vec<u8>>,
    pub client: reqwest::Client,
    pub routing: Arc<crate::routing::RoutingState>,
    pub log_tx: Option<tokio::sync::mpsc::Sender<crate::logging::LogEntry>>,
}

impl AppState {
    pub fn new(config: Config, pool: SqlitePool, caches: Caches, jwt_secret: Vec<u8>,
               log_tx: Option<tokio::sync::mpsc::Sender<crate::logging::LogEntry>>) -> Self {
        Self {
            config, pool,
            caches: Arc::new(RwLock::new(caches)),
            jwt_secret: Arc::new(jwt_secret),
            client: crate::upstream::build_client(),
            routing: Arc::new(crate::routing::RoutingState::new()),
            log_tx,
        }
    }
}
```

同步把所有测试里的 `AppState { ... }` 改为 `AppState::new(config, pool, caches, secret, None)`（integration_m1、token_auth、admin_login、admin_refresh、models_endpoint、real_db）。

- [ ] **Step 4: `main.rs` 接线**

```rust
let (log_tx, log_rx) = logging::channel(20_000);
let writer = logging::spawn_writer(pool.clone(), log_rx);
let state = state::AppState::new(config.clone(), pool, caches, jwt_secret, Some(log_tx));
// ... after axum::serve(...).await?;
drop(app);
let _ = tokio::time::timeout(std::time::Duration::from_secs(5), writer).await;
```

`src/lib.rs` 增加 `pub mod logging;`。

- [ ] **Step 5: 运行测试**

Run: `cargo test --test log_writer`
Expected: PASS。

- [ ] **Step 6: 全量回归并提交（主代理）**

Run: `cargo test`
Expected: 全绿。

```bash
git add src tests
git commit -m "feat(m2): async batch log writer"
```
