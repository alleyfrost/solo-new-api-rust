//! 渠道优先级拖拽排序的纯计算部分。
//!
//! 前端一次拖拽只提交「把某个渠道移到目标渠道的前/后/同层」，具体要改动哪些
//! 渠道的 priority 由这里推导：优先取两层之间的中点，中点不够用时按“影响渠道
//! 数更少”的方向整体让位。与 Go 侧 model/channel_priority.go 保持同一套规则，
//! 这样同一份前端在两边都能算出一致的排序。

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// 优先级要能无损放进 JavaScript 的 number，超出范围直接拒绝而不是静默截断。
pub const MAX_SORTABLE_PRIORITY: i64 = (1i64 << 53) - 1;

pub const CONFLICT: &str = "Channel priorities changed. Refresh and try again.";
pub const INVALID: &str = "Invalid channel priority move";
pub const OUT_OF_RANGE: &str = "Channel priority is outside the sortable range";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    pub id: i64,
    pub priority: Option<i64>,
}

impl Item {
    /// NULL 优先级按 0 参与排序，与 Go 的取值方式一致。
    pub fn value(&self) -> i64 {
        self.priority.unwrap_or(0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    pub id: i64,
    pub previous: Option<i64>,
    pub priority: Option<i64>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Move {
    #[serde(default)]
    pub revision: String,
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub target_id: i64,
    #[serde(default)]
    pub position: String,
    #[serde(default)]
    pub preview: bool,
    #[serde(default)]
    pub restore: Vec<Change>,
}

/// 快照指纹：前端提交时带回，用于拒绝过期客户端与并发重排。
pub fn revision(items: &[Item]) -> String {
    let encoded = serde_json::to_vec(items).unwrap_or_default();
    hex::encode(Sha256::digest(encoded))
}

fn check_range(value: i64) -> Result<(), &'static str> {
    if value < -MAX_SORTABLE_PRIORITY || value > MAX_SORTABLE_PRIORITY {
        return Err(OUT_OF_RANGE);
    }
    Ok(())
}

/// 校验快照并返回需要落库的改动；`preview` 为真时调用方不得写库。
pub fn plan(items: &[Item], mv: &Move) -> Result<Vec<Change>, &'static str> {
    let mut changes: Vec<Change> = Vec::new();
    let mut by_id: HashMap<i64, Item> = HashMap::with_capacity(items.len());
    for item in items {
        check_range(item.value())?;
        by_id.insert(item.id, *item);
    }
    if mv.revision.is_empty() || mv.revision != revision(items) {
        return Err(CONFLICT);
    }

    if !mv.restore.is_empty() {
        if mv.id != 0 || mv.target_id != 0 || !mv.position.is_empty() {
            return Err(INVALID);
        }
        let mut seen: HashSet<i64> = HashSet::new();
        for change in &mv.restore {
            let Some(item) = by_id.get(&change.id) else {
                return Err(INVALID);
            };
            if !seen.insert(change.id) {
                return Err(INVALID);
            }
            if let Some(priority) = change.priority {
                check_range(priority)?;
            }
            if item.priority != change.previous {
                return Err(CONFLICT);
            }
            if item.priority != change.priority {
                changes.push(*change);
            }
        }
        return Ok(changes);
    }

    let (Some(item), Some(target)) = (by_id.get(&mv.id), by_id.get(&mv.target_id)) else {
        return Err(INVALID);
    };
    if mv.id == mv.target_id || !matches!(mv.position.as_str(), "before" | "after" | "same") {
        return Err(INVALID);
    }

    let mut values: HashMap<i64, i64> = items
        .iter()
        .map(|entry| (entry.id, entry.value()))
        .collect();
    if mv.position == "same" {
        values.insert(item.id, target.value());
    } else {
        let mut counts: HashMap<i64, usize> = HashMap::new();
        for entry in items {
            if entry.id != item.id {
                *counts.entry(entry.value()).or_default() += 1;
            }
        }
        let mut levels: Vec<i64> = counts.keys().copied().collect();
        levels.sort_unstable_by(|a, b| b.cmp(a));
        let len = levels.len();
        let target_index = levels
            .iter()
            .position(|level| *level == target.value())
            .unwrap_or(0);
        let index = if mv.position == "after" {
            target_index + 1
        } else {
            target_index
        };
        let mut hi = if index > 0 {
            levels[index - 1]
        } else {
            MAX_SORTABLE_PRIORITY + 1
        };
        let mut lo = if index < len {
            levels[index]
        } else {
            -MAX_SORTABLE_PRIORITY - 1
        };
        // 落在自己原有的区间里就不必重新编号。
        if item.value() < hi && item.value() > lo {
            return Ok(changes);
        }
        if hi - lo < 2 {
            let idx = index as i64;
            let mut up = idx - 1;
            let mut down = idx;
            let mut up_cost = 0usize;
            let mut down_cost = 0usize;
            while up >= 0 {
                up_cost += counts
                    .get(&levels[up as usize])
                    .copied()
                    .unwrap_or_default();
                let headroom = up == 0 && levels[0] < MAX_SORTABLE_PRIORITY;
                let gap = up > 0 && levels[(up - 1) as usize] - levels[up as usize] > 1;
                if headroom || gap {
                    break;
                }
                up -= 1;
            }
            while down < len as i64 {
                down_cost += counts
                    .get(&levels[down as usize])
                    .copied()
                    .unwrap_or_default();
                let headroom =
                    down == len as i64 - 1 && levels[down as usize] > -MAX_SORTABLE_PRIORITY;
                let gap = down + 1 < len as i64
                    && levels[down as usize] - levels[(down + 1) as usize] > 1;
                if headroom || gap {
                    break;
                }
                down += 1;
            }
            let can_up = up >= 0 && idx > 0;
            let can_down = down < len as i64 && idx < len as i64;
            if !can_up && !can_down {
                return Err(OUT_OF_RANGE);
            }
            if can_up && (!can_down || up_cost < down_cost) {
                let lower = levels[(idx - 1) as usize];
                let upper = levels[up as usize];
                for (id, value) in &mut values {
                    if *id != item.id && *value >= lower && *value <= upper {
                        *value += 1;
                    }
                }
                hi += 1;
            } else {
                let upper = levels[idx as usize];
                let lower = levels[down as usize];
                for (id, value) in &mut values {
                    if *id != item.id && *value <= upper && *value >= lower {
                        *value -= 1;
                    }
                }
                lo -= 1;
            }
        }
        let mut value = lo + (hi - lo) / 2;
        if index == 0 {
            value = lo.saturating_add(10).min(MAX_SORTABLE_PRIORITY);
        } else if index == len {
            value = hi.saturating_sub(10).max(-MAX_SORTABLE_PRIORITY);
        }
        values.insert(item.id, value);
    }

    for entry in items {
        let value = values.get(&entry.id).copied().unwrap_or_default();
        if value != entry.value() {
            changes.push(Change {
                id: entry.id,
                previous: entry.priority,
                priority: Some(value),
            });
        }
    }
    Ok(changes)
}

/// 把改动套回快照，得到提交后的 items；revision 由调用方重算。
pub fn apply(items: &[Item], changes: &[Change]) -> Vec<Item> {
    let mut updated = items.to_vec();
    for change in changes {
        if let Some(item) = updated.iter_mut().find(|item| item.id == change.id) {
            item.priority = change.priority;
        }
    }
    updated
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(values: &[i64]) -> Vec<Item> {
        values
            .iter()
            .map(|value| Item {
                id: 0,
                priority: Some(*value),
            })
            .enumerate()
            .map(|(index, mut item)| {
                item.id = (index + 1) as i64;
                item
            })
            .collect()
    }

    fn moved(values: &[i64], id: i64, target_id: i64, position: &str) -> Vec<Change> {
        let snapshot = items(values);
        let mv = Move {
            revision: revision(&snapshot),
            id,
            target_id,
            position: position.to_string(),
            ..Move::default()
        };
        plan(&snapshot, &mv).expect("plan")
    }

    fn result_map(changes: &[Change]) -> HashMap<i64, i64> {
        changes
            .iter()
            .map(|change| (change.id, change.priority.expect("priority")))
            .collect()
    }

    // 期望值逐条对齐 Go 的 TestChannelPriorityPlan。
    #[test]
    fn midpoint_opens_a_slot_without_renumbering() {
        assert_eq!(
            result_map(&moved(&[100, 80, 0], 3, 2, "before")),
            [(3, 90)].into()
        );
    }

    #[test]
    fn top_of_a_tier_keeps_peers() {
        assert_eq!(
            result_map(&moved(&[0, 0, 0], 3, 1, "before")),
            [(3, 10)].into()
        );
    }

    #[test]
    fn bottom_of_a_tier_goes_below() {
        assert_eq!(
            result_map(&moved(&[0, 0, 0], 1, 3, "after")),
            [(1, -10)].into()
        );
    }

    #[test]
    fn same_position_joins_the_target_tier() {
        assert_eq!(
            result_map(&moved(&[100, 80, 0], 3, 2, "same")),
            [(3, 80)].into()
        );
    }

    #[test]
    fn dropping_into_the_own_slot_changes_nothing() {
        assert!(moved(&[100, 80, 0], 2, 1, "after").is_empty());
        assert!(moved(&[0, 0], 1, 2, "same").is_empty());
    }

    #[test]
    fn a_full_slot_shifts_the_cheaper_direction() {
        assert_eq!(
            result_map(&moved(&[100, 99, 99, 0], 4, 2, "before")),
            [(1, 101), (4, 100)].into()
        );
        assert_eq!(
            result_map(&moved(&[100, 100, 99, 0], 4, 3, "before")),
            [(3, 98), (4, 99)].into()
        );
        assert_eq!(
            result_map(&moved(&[102, 101, 100, 99, 0], 5, 3, "after")),
            [(4, 98), (5, 99)].into()
        );
    }

    #[test]
    fn bounds_stay_inside_the_sortable_range() {
        assert_eq!(
            result_map(&moved(&[MAX_SORTABLE_PRIORITY, 0], 2, 1, "before")),
            [(1, MAX_SORTABLE_PRIORITY - 1), (2, MAX_SORTABLE_PRIORITY)].into()
        );
        assert_eq!(
            result_map(&moved(&[-MAX_SORTABLE_PRIORITY, 0], 2, 1, "after")),
            [(1, -MAX_SORTABLE_PRIORITY + 1), (2, -MAX_SORTABLE_PRIORITY)].into()
        );
    }

    #[test]
    fn rejects_bad_requests_and_stale_snapshots() {
        let snapshot = items(&[0, 0]);
        assert_eq!(
            plan(
                &snapshot,
                &Move {
                    revision: revision(&snapshot),
                    id: 1,
                    target_id: 2,
                    position: "sideways".to_string(),
                    ..Move::default()
                }
            ),
            Err(INVALID)
        );
        assert_eq!(
            plan(
                &snapshot,
                &Move {
                    revision: "0".to_string(),
                    id: 1,
                    target_id: 2,
                    position: "before".to_string(),
                    ..Move::default()
                }
            ),
            Err(CONFLICT)
        );
        assert_eq!(
            plan(
                &[Item {
                    id: 1,
                    priority: Some(MAX_SORTABLE_PRIORITY + 1)
                }],
                &Move::default()
            ),
            Err(OUT_OF_RANGE)
        );
    }

    #[test]
    fn undo_restores_the_previous_snapshot_exactly() {
        let before = items(&[100, 99, 99, 0]);
        let changes = moved(&[100, 99, 99, 0], 4, 2, "before");
        let after = apply(&before, &changes);
        let undo = Move {
            revision: revision(&after),
            restore: changes
                .iter()
                .map(|change| Change {
                    id: change.id,
                    previous: change.priority,
                    priority: change.previous,
                })
                .collect(),
            ..Move::default()
        };
        let restored_changes = plan(&after, &undo).expect("undo plan");
        assert_eq!(apply(&after, &restored_changes), before);
        assert_eq!(
            revision(&apply(&after, &restored_changes)),
            revision(&before)
        );
    }

    #[test]
    fn a_null_priority_undoes_back_to_null() {
        let snapshot = vec![
            Item {
                id: 1,
                priority: Some(100),
            },
            Item {
                id: 2,
                priority: None,
            },
        ];
        let mv = Move {
            revision: revision(&snapshot),
            id: 2,
            target_id: 1,
            position: "before".to_string(),
            ..Move::default()
        };
        let changes = plan(&snapshot, &mv).expect("plan");
        let after = apply(&snapshot, &changes);
        let undo = Move {
            revision: revision(&after),
            restore: changes
                .iter()
                .map(|change| Change {
                    id: change.id,
                    previous: change.priority,
                    priority: change.previous,
                })
                .collect(),
            ..Move::default()
        };
        let restored = apply(&after, &plan(&after, &undo).expect("undo"));
        assert_eq!(restored, snapshot);
        let null_change = restored
            .iter()
            .find(|item| item.id == 2)
            .expect("channel 2");
        assert_eq!(null_change.priority, None);
    }

    #[test]
    fn revision_is_stable_and_order_sensitive() {
        assert_eq!(revision(&items(&[1, 2])), revision(&items(&[1, 2])));
        assert_ne!(revision(&items(&[1, 2])), revision(&items(&[2, 1])));
    }
}
