use std::collections::{HashMap, HashSet};

use solo_new_api_rust::model::{AbilityRow, ChannelRow};
use solo_new_api_rust::routing::{
    RoutingState, key_healthy, mark_key_unhealthy, select_channel,
};
use solo_new_api_rust::state::Caches;

fn channel(
    id: i64,
    priority: i64,
    weight: i64,
    status: i64,
    key: &str,
    group: &str,
) -> ChannelRow {
    ChannelRow {
        id,
        r#type: 1,
        key: key.to_string(),
        status,
        name: format!("ch{id}"),
        models: String::new(),
        group: group.to_string(),
        base_url: None,
        priority: Some(priority),
        weight: Some(weight),
        channel_info: None,
        model_mapping: None,
        header_override: None,
        created_time: None,
        test_time: None,
        response_time: None,
        open_ai_organization: None,
        test_model: None,
        other: None,
        balance: None,
        balance_updated_time: None,
        used_quota: None,
        status_code_mapping: None,
        auto_ban: None,
        other_info: None,
        tag: None,
        setting: None,
        param_override: None,
        remark: None,
        settings: None,
    }
}

fn ability(group: &str, model: &str, channel_id: i64, priority: i64, weight: i64) -> AbilityRow {
    AbilityRow {
        group: group.to_string(),
        model: model.to_string(),
        channel_id,
        enabled: true,
        priority: Some(priority),
        weight: Some(weight),
    }
}

fn test_caches() -> Caches {
    let channels = vec![
        channel(1, 10, 0, 1, "k1\nk2", "vip"),
        channel(2, 5, 0, 1, "k3", "vip"),
        channel(3, 10, 0, 2, "k4", "vip"),
        channel(4, 10, 0, 1, "k5", "default"),
    ];
    let abilities = vec![
        ability("vip", "m1", 1, 10, 0),
        ability("vip", "m1", 2, 5, 0),
        ability("vip", "m1", 3, 10, 0),
        ability("default", "m1", 4, 10, 0),
    ];

    let mut abilities_map: HashMap<(String, String), Vec<AbilityRow>> = HashMap::new();
    for a in abilities {
        abilities_map
            .entry((a.group.clone(), a.model.clone()))
            .or_default()
            .push(a);
    }

    Caches {
        users: HashMap::new(),
        tokens: HashMap::new(),
        channels: channels.into_iter().map(|c| (c.id, c)).collect(),
        abilities: abilities_map,
        options: HashMap::new(),
    }
}

/// 1. retry=0 选最高优先级层（10），不会选 channel 2（低优先级）、3（禁用）、4（其它 group）。
#[test]
fn selects_highest_priority_tier_at_retry_zero() {
    let caches = test_caches();
    let state = RoutingState::new();
    let used = HashSet::new();
    for _ in 0..50 {
        let picked = select_channel(&caches, &state, "vip", "m1", 0, &used)
            .expect("should select a channel");
        assert_eq!(picked.channel_id, 1, "retry=0 必须选 priority 10 的 channel 1");
    }
}

/// 2. retry=1 落到次高层 priority 5 → channel 2。
#[test]
fn retry_one_falls_back_to_lower_priority() {
    let caches = test_caches();
    let state = RoutingState::new();
    let used = HashSet::new();
    for _ in 0..50 {
        let picked = select_channel(&caches, &state, "vip", "m1", 1, &used)
            .expect("should select a channel");
        assert_eq!(picked.channel_id, 2, "retry=1 必须选 priority 5 的 channel 2");
    }
}

/// 3. retry 超出层数时仍停在最低优先级层（channel 2），不越界 panic。
#[test]
fn retry_beyond_tiers_stays_at_lowest_priority() {
    let caches = test_caches();
    let state = RoutingState::new();
    let used = HashSet::new();
    for _ in 0..50 {
        let picked = select_channel(&caches, &state, "vip", "m1", 5, &used)
            .expect("should select a channel");
        assert_eq!(picked.channel_id, 2, "retry=5 必须兜底到最低优先级 channel 2");
    }
}

/// 4. 多 Key 轮换：连续调用在 key 0/1 间交替；used 含 (1,0) 时选 key 1。
#[test]
fn multi_key_rotates_and_skips_used_key() {
    let caches = test_caches();
    let state = RoutingState::new();
    let used = HashSet::new();

    let picks: Vec<usize> = (0..4)
        .map(|_| {
            select_channel(&caches, &state, "vip", "m1", 0, &used)
                .expect("should select")
                .key_index
        })
        .collect();
    assert_eq!(picks, vec![0, 1, 0, 1], "同一渠道的两个 Key 必须轮换");

    let mut used = HashSet::new();
    used.insert((1_i64, 0_usize));
    let picked = select_channel(&caches, &state, "vip", "m1", 0, &used).expect("should select");
    assert_eq!(picked.channel_id, 1);
    assert_eq!(picked.key_index, 1, "used 中的 Key 必须被跳过");
    assert_eq!(picked.key, "k2");
}

/// 5. 标记不健康后不再选该 Key；健康标记可查询。
#[test]
fn unhealthy_key_is_skipped() {
    let caches = test_caches();
    let state = RoutingState::new();
    let used = HashSet::new();

    assert!(key_healthy(&state, 1, 0));
    mark_key_unhealthy(&state, 1, 0);
    assert!(!key_healthy(&state, 1, 0));
    assert!(key_healthy(&state, 1, 1));

    for _ in 0..10 {
        let picked = select_channel(&caches, &state, "vip", "m1", 0, &used).expect("should select");
        assert_eq!(picked.channel_id, 1);
        assert_eq!(picked.key_index, 1, "不健康的 key 0 不能被选中");
        assert_eq!(picked.key, "k2");
    }
}

/// 6. group 隔离：default 组只能选到 channel 4。
#[test]
fn group_isolation() {
    let caches = test_caches();
    let state = RoutingState::new();
    let used = HashSet::new();
    for retry in 0..3 {
        for _ in 0..20 {
            let picked = select_channel(&caches, &state, "default", "m1", retry, &used)
                .expect("should select");
            assert_eq!(picked.channel_id, 4, "default 组只能选 channel 4");
        }
    }
    // 不存在的 model / group 返回 None。
    assert!(select_channel(&caches, &state, "vip", "no-such-model", 0, &used).is_none());
    assert!(select_channel(&caches, &state, "no-such-group", "m1", 0, &used).is_none());
}

/// 轮询策略按游标顺序轮换 Key（进程内游标，不依赖 Redis）。
#[test]
fn polling_mode_rotates_keys_in_cursor_order() {
    let mut caches = test_caches();
    {
        let ch = caches.channels.get_mut(&1).unwrap();
        ch.key = "k1\nk2\nk3".into();
        ch.channel_info = Some(r#"{"multi_key_mode":"polling"}"#.into());
    }
    let state = RoutingState::new();
    let mut picked = Vec::new();
    for _ in 0..3 {
        let used = HashSet::new();
        picked.push(
            select_channel(&caches, &state, "vip", "m1", 0, &used)
                .unwrap()
                .key,
        );
    }
    assert_eq!(picked, vec!["k1", "k2", "k3"]);
}

/// 随机策略每次从健康且未使用的 Key 中随机选一个，不维护游标。
#[test]
fn random_mode_selects_across_keys() {
    let mut caches = test_caches();
    {
        let ch = caches.channels.get_mut(&1).unwrap();
        ch.key = "k1\nk2\nk3\nk4".into();
        ch.channel_info = Some(r#"{"multi_key_mode":"random"}"#.into());
    }
    let state = RoutingState::new();
    let mut seen = HashSet::new();
    for _ in 0..40 {
        let used = HashSet::new();
        seen.insert(
            select_channel(&caches, &state, "vip", "m1", 0, &used)
                .unwrap()
                .key,
        );
    }
    assert!(seen.len() > 1, "random mode should not stick to one key");
}

/// 随机策略同样跳过冷却中的 Key。
#[test]
fn random_mode_skips_unhealthy_keys() {
    let mut caches = test_caches();
    {
        let ch = caches.channels.get_mut(&1).unwrap();
        ch.key = "k1\nk2".into();
        ch.channel_info = Some(r#"{"multi_key_mode":"random"}"#.into());
    }
    let state = RoutingState::new();
    mark_key_unhealthy(&state, 1, 0);
    for _ in 0..20 {
        let used = HashSet::new();
        let picked = select_channel(&caches, &state, "vip", "m1", 0, &used).unwrap();
        assert_eq!(picked.key, "k2");
        assert_eq!(picked.key_index, 1);
    }
}
