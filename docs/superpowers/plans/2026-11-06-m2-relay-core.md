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
