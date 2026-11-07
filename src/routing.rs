use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
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

/// 进程内选路状态：每个渠道的 Key 游标 + 不健康 Key 的恢复时间点。
#[derive(Default)]
pub struct RoutingState {
    cursors: Mutex<HashMap<i64, usize>>,
    unhealthy: Mutex<HashMap<(i64, usize), Instant>>,
}

impl RoutingState {
    pub fn new() -> Self {
        Self::default()
    }
}

pub fn key_healthy(state: &RoutingState, channel_id: i64, key_index: usize) -> bool {
    let map = state.unhealthy.lock().unwrap();
    match map.get(&(channel_id, key_index)) {
        // 记录的是恢复时间点：已过期视为恢复健康。
        Some(until) => *until <= Instant::now(),
        None => true,
    }
}

pub fn mark_key_unhealthy(state: &RoutingState, channel_id: i64, key_index: usize) {
    let mut map = state.unhealthy.lock().unwrap();
    map.insert((channel_id, key_index), Instant::now() + KEY_DISABLED_TTL);
}

/// 日志里用到的 Key 数量；多 Key 渠道才展示 Key 序号。
pub fn key_count(channel: &ChannelRow) -> usize {
    channel_keys(channel).len()
}

fn channel_keys(channel: &ChannelRow) -> Vec<String> {
    channel
        .key
        .lines()
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .map(str::to_string)
        .collect()
}

fn candidate_rows<'a>(
    caches: &'a Caches,
    group: &str,
    model: &str,
) -> Vec<(&'a AbilityRow, &'a ChannelRow)> {
    let Some(rows) = caches.abilities.get(&(group.to_string(), model.to_string())) else {
        return Vec::new();
    };
    rows.iter()
        .filter(|a| a.enabled)
        .filter_map(|a| caches.channels.get(&a.channel_id).map(|c| (a, c)))
        .filter(|(_, c)| c.status == 1)
        .collect()
}

fn priority_tiers(candidates: &[(&AbilityRow, &ChannelRow)]) -> Vec<i64> {
    let mut tiers: Vec<i64> = candidates
        .iter()
        .map(|(a, _)| a.priority.unwrap_or(0))
        .collect();
    tiers.sort_unstable_by(|a, b| b.cmp(a));
    tiers.dedup();
    tiers
}

fn weighted_pick<'a, R: Rng>(
    items: &'a [(&'a AbilityRow, &'a ChannelRow)],
    rng: &mut R,
) -> &'a (&'a AbilityRow, &'a ChannelRow) {
    let total: u64 = items
        .iter()
        .map(|(a, _)| a.weight.unwrap_or(0).max(0) as u64)
        .sum();
    if total == 0 {
        return &items[rng.gen_range(0..items.len())];
    }
    let mut roll = rng.gen_range(0..total);
    for item in items {
        let w = item.0.weight.unwrap_or(0).max(0) as u64;
        if roll < w {
            return item;
        }
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
    if candidates.is_empty() {
        return None;
    }
    let tiers = priority_tiers(&candidates);
    let tier = tiers[retry.min(tiers.len() - 1)];
    let tier_items: Vec<(&AbilityRow, &ChannelRow)> = candidates
        .into_iter()
        .filter(|(a, _)| a.priority.unwrap_or(0) == tier)
        .collect();
    if tier_items.is_empty() {
        return None;
    }

    let mut rng = rand::thread_rng();
    // 同层内按权重无放回抽样，得到尝试顺序；权重全 0 时退化为均匀随机。
    let mut remaining = tier_items.clone();
    let mut order: Vec<(&AbilityRow, &ChannelRow)> = Vec::with_capacity(remaining.len());
    while !remaining.is_empty() {
        let picked_idx = {
            let picked = weighted_pick(&remaining, &mut rng);
            remaining
                .iter()
                .position(|r| r.1.id == picked.1.id)
                .expect("picked item must be in remaining")
        };
        order.push(remaining.remove(picked_idx));
    }

    for (_, channel) in order {
        let keys = channel_keys(channel);
        if keys.is_empty() {
            continue;
        }
        let start = {
            let mut cursors = state.cursors.lock().unwrap();
            *cursors.entry(channel.id).or_insert(0)
        };
        for offset in 0..keys.len() {
            let idx = (start + offset) % keys.len();
            if !key_healthy(state, channel.id, idx) {
                continue;
            }
            if used.contains(&(channel.id, idx)) {
                continue;
            }
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
