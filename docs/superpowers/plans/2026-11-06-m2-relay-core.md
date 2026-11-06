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

---

## Task 5: `/v1/chat/completions` 流式/非流式转发

**Files:**
- Create: `src/relay/mod.rs`、`src/relay/chat.rs`
- Modify: `src/lib.rs`、`src/main.rs`、`src/model.rs`、`src/state.rs`、`src/logging.rs`、`src/auth/token.rs`、`tests/routing.rs`、`tests/models_endpoint.rs`、`tests/log_writer.rs`、`Cargo.toml`
- Test: `tests/relay_helpers.rs`

**Interfaces:**
- Produces:
  - `relay::Usage { prompt_tokens: i64, completion_tokens: i64 }`、`relay::usage_from_json(&Value) -> Option<Usage>`
  - `relay::map_model(origin: &str, mapping: Option<&str>) -> String`（链式映射、循环保护）
  - `relay::SseTap::new(wants_usage: bool)`、`feed(&mut self, &[u8]) -> Vec<u8>`、`finish(&mut self) -> Vec<u8>`、`usage() -> Usage`
  - `relay::client_ip(&HeaderMap, trust_proxy: bool) -> String`
  - `relay::openai_error(StatusCode, message, type) -> Response`
  - `relay::chat::chat_completions(State, Extension<TokenContext>, HeaderMap, Bytes) -> Response`
  - `TokenContext` 新增 `token_name: String`
  - `ChannelRow` 新增 `model_mapping: Option<String>`、`header_override: Option<String>`
  - 日志 writer 在同一事务内按 token 去重更新 `tokens.accessed_time`

**核心约束：**
- 流式响应逐事件转发，不缓存完整响应；单事件缓冲上限 1 MiB。
- 流式请求默认向 OpenAI 兼容上游注入 `stream_options.include_usage=true`；若客户端未要求 usage，则丢弃“仅含 usage、choices 为空”的事件，保持客户端可见语义不变。
- 只有响应尚未开始时才允许重试；成功响应开始后不得重试或改写。
- 429/401/403 标记该 Key 不健康并立即切换；5xx/网络错误退避后重试；其它 4xx 直接返回。
- 不写 `logs.quota`（固定 0），不改 token/user 额度，只更新 `accessed_time`。

- [ ] **Step 1: 写失败测试 `tests/relay_helpers.rs`**

```rust
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use solo_new_api_rust::relay::{Usage, SseTap, client_ip, map_model, usage_from_json};

#[test]
fn model_mapping_follows_chain_and_stops_cycles() {
    assert_eq!(map_model("a", Some(r#"{"a":"b","b":"c"}"#)), "c");
    assert_eq!(map_model("x", Some(r#"{"a":"b"}"#)), "x");
    assert_eq!(map_model("a", Some(r#"{"a":"b","b":"a"}"#)), "a");
    assert_eq!(map_model("a", None), "a");
}

#[test]
fn usage_reads_openai_and_anthropic_names() {
    let v: serde_json::Value = serde_json::from_str(r#"{"usage":{"prompt_tokens":3,"completion_tokens":4}}"#).unwrap();
    assert_eq!(usage_from_json(&v), Some(Usage { prompt_tokens: 3, completion_tokens: 4 }));
    let v: serde_json::Value = serde_json::from_str(r#"{"usage":{"input_tokens":5,"output_tokens":6}}"#).unwrap();
    assert_eq!(usage_from_json(&v), Some(Usage { prompt_tokens: 5, completion_tokens: 6 }));
    assert_eq!(usage_from_json(&serde_json::json!({"id":"x"})), None);
}

#[test]
fn sse_tap_handles_split_chunks_and_drops_usage_only_event() {
    let mut tap = SseTap::new(false);
    let mut out = Vec::new();
    out.extend(tap.feed(b"data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n"));
    out.extend(tap.feed(b"data: {\"choices\":[],\"usa"));
    out.extend(tap.feed(b"ge\":{\"prompt_tokens\":2,\"completion_tokens\":3}}\n\ndata: [DONE]\n\n"));
    out.extend(tap.finish());
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("\"content\":\"hi\""));
    assert!(!text.contains("prompt_tokens"), "客户端未要求 usage 时必须丢弃 usage-only 事件");
    assert!(text.contains("[DONE]"));
    assert_eq!(tap.usage(), Usage { prompt_tokens: 2, completion_tokens: 3 });
}

#[test]
fn sse_tap_keeps_usage_when_requested() {
    let mut tap = SseTap::new(true);
    let out = tap.feed(b"data: {\"choices\":[],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":1}}\n\n");
    assert!(String::from_utf8(out).unwrap().contains("prompt_tokens"));
}

#[test]
fn client_ip_only_trusts_forwarded_headers_when_enabled() {
    let mut h = HeaderMap::new();
    h.insert("x-forwarded-for", HeaderValue::from_static("1.2.3.4, 5.6.7.8"));
    assert_eq!(client_ip(&h, true), "1.2.3.4");
    assert_eq!(client_ip(&h, false), "unknown");
    let _ = StatusCode::OK;
}
```

- [ ] **Step 2: 数据模型与依赖**

- `Cargo.toml` 增加 `async-stream = "0.3"`、`tokio-stream = "0.1"`。
- `ChannelRow` 增加两个可空字段；`Caches::load` 的 SELECT 增加 `model_mapping, header_override`。
- 同步补 `tests/routing.rs`、`tests/models_endpoint.rs` 的 `ChannelRow` 字面量（字段填 `None`）。
- `TokenContext` 增加 `token_name: String`，`validate_token` 填 `token.name.clone()`。

- [ ] **Step 3: 实现 `src/relay/mod.rs`**

```rust
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
    let prompt = u.get("prompt_tokens").and_then(as_i64)
        .or_else(|| u.get("input_tokens").and_then(as_i64))?;
    let completion = u.get("completion_tokens").and_then(as_i64)
        .or_else(|| u.get("output_tokens").and_then(as_i64))
        .unwrap_or(0);
    Some(Usage { prompt_tokens: prompt, completion_tokens: completion })
}

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
    headers.get("x-forwarded-for").and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next()).map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .or_else(|| headers.get("x-real-ip").and_then(|v| v.to_str().ok()).map(|v| v.trim().to_string()))
        .unwrap_or_else(|| "unknown".to_string())
}

pub fn openai_error(status: StatusCode, message: impl Into<String>, kind: &str) -> Response {
    (status, axum::Json(json!({
        "error": { "message": message.into(), "type": kind, "param": null, "code": null }
    }))).into_response()
}

#[derive(Default)]
struct BlockInfo {
    usage: Option<Usage>,
    usage_only: bool,
}

pub struct SseTap {
    buf: Vec<u8>,
    usage: Usage,
    wants_usage: bool,
}

impl SseTap {
    pub fn new(wants_usage: bool) -> Self {
        Self { buf: Vec::new(), usage: Usage::default(), wants_usage }
    }

    pub fn usage(&self) -> Usage { self.usage }

    fn inspect(block: &[u8]) -> BlockInfo {
        let mut data: Vec<u8> = Vec::new();
        for line in block.split(|&b| b == b'\n') {
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            if let Some(rest) = line.strip_prefix(b"data:") {
                let rest = rest.strip_prefix(b" ").unwrap_or(rest);
                if !data.is_empty() { data.push(b'\n'); }
                data.extend_from_slice(rest);
            }
        }
        if data.is_empty() || data == b"[DONE]" { return BlockInfo::default(); }
        let Ok(value) = serde_json::from_slice::<Value>(&data) else { return BlockInfo::default(); };
        let usage = usage_from_json(&value);
        let usage_only = usage.is_some()
            && value.get("choices").and_then(Value::as_array).map(|a| a.is_empty()).unwrap_or(true);
        BlockInfo { usage, usage_only }
    }

    pub fn feed(&mut self, chunk: &[u8]) -> Vec<u8> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        loop {
            let Some((idx, sep)) = find_boundary(&self.buf) else {
                if self.buf.len() > MAX_EVENT { out.append(&mut self.buf); }
                break;
            };
            let block: Vec<u8> = self.buf.drain(..idx + sep).collect();
            let info = Self::inspect(&block);
            if let Some(u) = info.usage { self.usage = u; }
            if info.usage_only && !self.wants_usage { continue; }
            out.extend_from_slice(&block);
        }
        out
    }

    pub fn finish(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.buf)
    }
}

fn find_boundary(buf: &[u8]) -> Option<(usize, usize)> {
    let lf = buf.windows(2).position(|w| w == b"\n\n").map(|i| (i, 2));
    let crlf = buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| (i, 4));
    match (lf, crlf) {
        (Some(a), Some(b)) => Some(if a.0 <= b.0 { a } else { b }),
        (a, b) => a.or(b),
    }
}

pub mod chat;
```

- [ ] **Step 4: 实现 `src/relay/chat.rs`**

核心流程（完整实现）：

```rust
use std::collections::HashSet;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::body::{Body, Bytes};
use axum::extract::{Extension, State};
use axum::http::header::{CONTENT_TYPE, HeaderValue};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use serde_json::{Value, json};
use tokio_stream::StreamExt;

use crate::auth::token::TokenContext;
use crate::logging::{self, LogEntry};
use crate::relay::{self, SseTap, Usage};
use crate::{retry, routing, upstream};
use crate::state::AppState;

const PATH: &str = "/v1/chat/completions";

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
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn model_allowed(ctx: &TokenContext, model: &str) -> bool {
    if !ctx.model_limits_enabled { return true; }
    ctx.model_limits.split(',').map(str::trim).any(|m| m == model)
}

fn upstream_request_id(headers: &HeaderMap) -> String {
    for name in ["x-request-id", "request-id", "x-oneapi-request-id", "x-upstream-request-id"] {
        if let Some(v) = headers.get(name).and_then(|v| v.to_str().ok()) {
            return v.to_string();
        }
    }
    String::new()
}

fn response_with_ct(status: StatusCode, ct: Option<HeaderValue>, body: Body) -> Response {
    let mut builder = Response::builder().status(status);
    if let Some(ct) = ct { builder = builder.header(CONTENT_TYPE, ct); }
    builder.body(body).unwrap_or_else(|_| {
        relay::openai_error(StatusCode::INTERNAL_SERVER_ERROR, "response build failed", "internal_error")
    })
}

fn upstream_error_response(status: StatusCode, bytes: &Bytes, ct: Option<HeaderValue>) -> Response {
    let is_json = ct.as_ref().and_then(|v| v.to_str().ok()).map(|v| v.contains("json")).unwrap_or(false);
    if is_json && !bytes.is_empty() {
        return response_with_ct(status, ct, Body::from(bytes.clone()));
    }
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(2048)]).to_string();
    relay::openai_error(status, format!("upstream error: {text}"), "upstream_error")
}

pub async fn chat_completions(
    State(state): State<AppState>,
    Extension(ctx): Extension<TokenContext>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let started = Instant::now();
    let Ok(payload) = serde_json::from_slice::<Value>(&body) else {
        return relay::openai_error(StatusCode::BAD_REQUEST, "invalid JSON body", "invalid_request_error");
    };
    if !payload.is_object() {
        return relay::openai_error(StatusCode::BAD_REQUEST, "body must be a JSON object", "invalid_request_error");
    }
    let Some(origin_model) = payload.get("model").and_then(Value::as_str).map(str::to_string) else {
        return relay::openai_error(StatusCode::BAD_REQUEST, "model is required", "invalid_request_error");
    };
    if !model_allowed(&ctx, &origin_model) {
        return relay::openai_error(StatusCode::FORBIDDEN, format!("token has no access to model {origin_model}"), "permission_error");
    }
    let stream = payload.get("stream").and_then(Value::as_bool).unwrap_or(false);
    let wants_usage = payload.pointer("/stream_options/include_usage").and_then(Value::as_bool).unwrap_or(false);

    let (max_retry, group, username) = {
        let caches = state.caches.read().expect("cache lock");
        let max_retry = retry::max_retry_from_options(&caches.options);
        let user = caches.users.get(&ctx.user_id).cloned();
        let username = user.as_ref().map(|u| u.username.clone()).unwrap_or_default();
        let group = if ctx.token_group.is_empty() || ctx.token_group == "auto" {
            user.map(|u| u.group).unwrap_or_else(|| ctx.token_group.clone())
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
            routing::select_channel(&caches, &state.routing, &group, &origin_model, attempt, &used)
        };
        let Some(selected) = selected else { break; };
        let channel = {
            let caches = state.caches.read().expect("cache lock");
            caches.channels.get(&selected.channel_id).cloned()
        };
        let Some(channel) = channel else { continue; };
        used.insert((selected.channel_id, selected.key_index));

        let mut attempt_payload = payload.clone();
        let mapped = relay::map_model(&origin_model, channel.model_mapping.as_deref());
        if mapped != origin_model {
            attempt_payload["model"] = Value::String(mapped);
        }
        if stream && channel.r#type != 14 {
            let obj = attempt_payload.as_object_mut().expect("object");
            let options = obj.entry("stream_options".to_string()).or_insert_with(|| json!({}));
            if !options.is_object() { *options = json!({}); }
            options.as_object_mut().expect("object").insert("include_usage".to_string(), Value::Bool(true));
        }
        let upstream_body = match serde_json::to_vec(&attempt_payload) {
            Ok(b) => b,
            Err(e) => return relay::openai_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string(), "internal_error"),
        };
        let url = upstream::build_url(channel.base_url.as_deref().unwrap_or(""), PATH, channel.r#type);
        let upstream_headers = upstream::rewrite_headers(&headers, &selected.key, channel.header_override.as_deref());

        let response = match state.client.post(&url).headers(upstream_headers).body(upstream_body).send().await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("upstream request failed: {e}");
                last_error = Some((StatusCode::BAD_GATEWAY,
                    Bytes::from(format!(r#"{{"error":{{"message":"upstream request failed","type":"upstream_error","code":null}}}}"#)),
                    Some(HeaderValue::from_static("application/json"))));
                if retry::decide(None, attempt, max_retry) == retry::RetryDecision::Retry {
                    tokio::time::sleep(retry::backoff(None, attempt)).await;
                    continue;
                }
                break;
            }
        };

        let status = response.status();
        if status.is_success() {
            let content_type = response.headers().get(CONTENT_TYPE).cloned()
                .or_else(|| Some(HeaderValue::from_static(if stream { "text/event-stream" } else { "application/json" })));
            let upstream_req = upstream_request_id(response.headers());
            let meta = LogMeta {
                origin_model: origin_model.clone(), stream,
                channel_id: selected.channel_id, channel_name: selected.channel_name.clone(),
                user_id: ctx.user_id, username: username.clone(),
                token_id: ctx.token_id, token_name: ctx.token_name.clone(),
                group: group.clone(), ip: ip.clone(), request_id: request_id.clone(),
                upstream_request_id: upstream_req, started,
            };
            if stream {
                let log_state = state.clone();
                let wants = wants_usage;
                let mut source = response.bytes_stream();
                let stream_body = async_stream::stream! {
                    let mut tap = SseTap::new(wants);
                    while let Some(item) = source.next().await {
                        match item {
                            Ok(chunk) => {
                                let out = tap.feed(&chunk);
                                if !out.is_empty() { yield Ok::<Bytes, std::io::Error>(Bytes::from(out)); }
                            }
                            Err(e) => { tracing::warn!("upstream stream error: {e}"); break; }
                        }
                    }
                    let tail = tap.finish();
                    if !tail.is_empty() { yield Ok::<Bytes, std::io::Error>(Bytes::from(tail)); }
                    logging::enqueue(&log_state, meta.entry(tap.usage())).await;
                };
                return response_with_ct(status, content_type, Body::from_stream(stream_body));
            } else {
                let bytes = match response.bytes().await {
                    Ok(b) => b,
                    Err(e) => {
                        logging::enqueue(&state, meta.entry(Usage::default())).await;
                        return relay::openai_error(StatusCode::BAD_GATEWAY, format!("read upstream body failed: {e}"), "upstream_error");
                    }
                };
                let usage = serde_json::from_slice::<Value>(&bytes).ok().as_ref().and_then(relay::usage_from_json).unwrap_or_default();
                logging::enqueue(&state, meta.entry(usage)).await;
                return response_with_ct(status, content_type, Body::from(bytes));
            }
        }

        let content_type = response.headers().get(CONTENT_TYPE).cloned();
        let bytes = response.bytes().await.unwrap_or_default();
        last_error = Some((status, bytes, content_type));
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
        None => relay::openai_error(StatusCode::SERVICE_UNAVAILABLE, "no available channel", "no_channel"),
    }
}
```

网络错误伪造的错误体用 `axum::Json(json!({...}))` 的序列化结果或 `serde_json::to_vec(&json!({...}))`
生成，避免手写转义错误。

- [ ] **Step 5: 日志 writer 同事务更新 `accessed_time`**

`src/logging.rs::flush`：插入日志的同时，用 `HashMap<i64, i64>` 按 `token_id` 去重记录最大
`created_at`，事务提交前执行：

```rust
for (token_id, ts) in touched {
    if let Err(e) = sqlx::query("UPDATE tokens SET accessed_time = ? WHERE id = ?")
        .bind(ts).bind(token_id).execute(&mut *tx).await
    {
        tracing::warn!("update accessed_time failed: {e}");
    }
}
```

`tests/log_writer.rs` 增加 `CREATE TABLE tokens (...)`（至少 `id INTEGER PRIMARY KEY, accessed_time INTEGER`），
并断言 `SELECT accessed_time FROM tokens WHERE id=2` 等于最后一次日志的 `created_at`。

- [ ] **Step 6: 路由接线**

`src/lib.rs` 增加 `pub mod relay;`。
`src/main.rs` relay 路由增加：

```rust
.route("/v1/chat/completions", axum::routing::post(solo_new_api_rust::relay::chat::chat_completions))
.layer(axum::extract::DefaultBodyLimit::max(config.max_request_bytes))
```

- [ ] **Step 7: 运行测试**

Run: `cargo test --test relay_helpers`、`cargo test`
Expected: 全绿。

```bash
git add Cargo.toml Cargo.lock src tests
git commit -m "feat(m2): chat completions relay with streaming usage tap"
```

---

## Task 6: 转发端到端测试（wiremock）

**Files:**
- Create: `tests/chat_relay.rs`
- Modify: `Cargo.toml`（dev-dependency `wiremock = "0.6"`）

**覆盖场景：**
1. 非流式成功：上游鉴权头、请求模型透传、响应体原样返回；日志 `quota=0`、usage 正确、`is_stream=0`。
2. 模型映射 + `header_override`：上游收到映射后的模型与自定义头。
3. 429 重试：多 Key 渠道先 `k1` 429、再 `k2` 200，且只请求两次。
4. 400 不重试：4xx 原样透传，上游只被调用一次。
5. 流式：客户端未要求 usage 时隐藏 usage-only 事件，日志仍记录 usage；上游收到 `stream_options.include_usage=true`。
6. 流式且客户端要求 usage：usage 事件原样转发。

**验证：**

```bash
cargo test --test chat_relay
cargo test
```

Expected: 全绿。

```bash
git add Cargo.toml Cargo.lock src tests
git commit -m "feat(m2): chat completions relay with streaming usage tap"
```

---

## Task 7: 跨协议转发（Anthropic Messages / OpenAI Responses）

**Files:**
- Create: `vendor/switchyard-protocol`、`vendor/switchyard-translation`（Switchyard 0.3.0，源码 commit `c365efe832822541024704aadf6a0d5c81cc0528`，Apache-2.0）
- Create: `src/relay/forward.rs`、`src/relay/messages.rs`、`src/relay/responses.rs`
- Modify: `src/relay/mod.rs`、`src/relay/chat.rs`、`src/main.rs`、`Cargo.toml`
- Test: `tests/translation_probe.rs`、扩充 `tests/chat_relay.rs`

**设计：**
- 客户端协议由入口决定：`Wire::Chat` / `Messages` / `Responses`；上游协议由渠道类型决定：14 → Anthropic Messages，其余 → OpenAI Chat。
- 同协议：请求/响应/SSE 原样透传（Chat 仍做 usage-only 事件隐藏与 usage 抽取）。
- 跨协议：请求用 `TranslationEngine::translate_request`，响应用 `translate_response`，流式用 `translate_event` + `finish_stream` 逐事件转换并重新加 SSE 帧。
- Anthropic 上游：无显式 Authorization override 时移除 Bearer，改用 `x-api-key`，并补 `anthropic-version: 2023-06-01`。
- usage 抽取兼容 `usage`、`message.usage`、`response.usage`，流式按字段取最大值合并。
- 转换失败不重试：请求转换失败返回 400，响应转换失败返回 502。

**验收：**
1. Chat / Responses 客户端 → Chat 上游请求与响应转换正确。
2. Anthropic 客户端 → Chat 上游（非流式 + 流式）转换正确，流式为 `event:` + `data:` 帧。
3. Chat 客户端 → Anthropic 上游转换正确。
4. Anthropic 同协议透传，使用 `x-api-key` 且不带 Authorization。
5. 上述路径日志 usage 正确。

```bash
cargo test --test translation_probe
cargo test --test chat_relay
cargo test
```

```bash
git add Cargo.toml Cargo.lock vendor src tests
git commit -m "feat(m2): cross-protocol relay via vendored switchyard"
```
