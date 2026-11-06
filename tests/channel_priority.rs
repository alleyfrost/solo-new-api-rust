use axum::Router;
use axum::body::Body;
use axum::http::Request;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::SqlitePool;
use tower::ServiceExt;

use solo_new_api_rust::state::{AppState, Caches};

const PRIORITIES: [i64; 4] = [100, 99, 99, 0];

async fn setup() -> (Router, SqlitePool, AppState) {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    for ddl in [
        "CREATE TABLE users (id INTEGER PRIMARY KEY, username TEXT, password TEXT, display_name TEXT, role INTEGER, status INTEGER, `group` TEXT, auth_version INTEGER, email TEXT, deleted_at TEXT)",
        "CREATE TABLE tokens (id INTEGER PRIMARY KEY, user_id INTEGER, `key` TEXT, status INTEGER, name TEXT, created_time INTEGER, accessed_time INTEGER, expired_time INTEGER, remain_quota INTEGER, unlimited_quota INTEGER, model_limits_enabled INTEGER, model_limits TEXT, allow_ips TEXT, `group` TEXT, auto_groups TEXT, cross_group_retry INTEGER, used_quota INTEGER, allow_user_quota_query INTEGER, deleted_at TEXT)",
        "CREATE TABLE channels (id INTEGER PRIMARY KEY AUTOINCREMENT, type INTEGER, `key` TEXT, status INTEGER, name TEXT, models TEXT, `group` TEXT, base_url TEXT, priority INTEGER, weight INTEGER, channel_info TEXT, model_mapping TEXT, header_override TEXT, created_time INTEGER, test_time INTEGER, response_time INTEGER, open_ai_organization TEXT, test_model TEXT, other TEXT, balance REAL, balance_updated_time INTEGER, used_quota INTEGER, status_code_mapping TEXT, auto_ban INTEGER, other_info TEXT, tag TEXT, setting TEXT, param_override TEXT, remark TEXT, settings TEXT)",
        "CREATE TABLE abilities (`group` TEXT, model TEXT, channel_id INTEGER, enabled INTEGER, priority INTEGER, weight INTEGER, tag TEXT, PRIMARY KEY (`group`, model, channel_id))",
        "CREATE TABLE options (`key` TEXT PRIMARY KEY, value TEXT)",
    ] {
        sqlx::query(ddl).execute(&pool).await.unwrap();
    }
    sqlx::query("INSERT INTO users VALUES (1,'root','x',NULL,100,1,'default',1,NULL,NULL)")
        .execute(&pool)
        .await
        .unwrap();
    for (index, priority) in PRIORITIES.iter().enumerate() {
        let id = index as i64 + 1;
        sqlx::query(
            "INSERT INTO channels (id, type, `key`, status, name, models, `group`, priority, weight, tag) \
             VALUES (?,1,'k',1,?,?, 'default,vip', ?,?, 'a')",
        )
        .bind(id)
        .bind(format!("c{id}"))
        .bind("m1,m2")
        .bind(priority)
        .bind(10 + id)
        .execute(&pool)
        .await
        .unwrap();
        for group in ["default", "vip"] {
            for model in ["m1", "m2"] {
                sqlx::query(
                    "INSERT INTO abilities (`group`, model, channel_id, enabled, priority, weight, tag) \
                     VALUES (?,?,?,1,?,?, 'a')",
                )
                .bind(group)
                .bind(model)
                .bind(id)
                .bind(priority)
                .bind(10 + id)
                .execute(&pool)
                .await
                .unwrap();
            }
        }
    }

    let cfg = solo_new_api_rust::config::Config {
        port: 0,
        db_path: ":memory:".into(),
        web_dist: "./web/dist".into(),
        jwt_secret_path: "/tmp/.jwt-test".into(),
        trust_proxy_headers: true,
        max_request_bytes: 64 * 1024 * 1024,
    };
    let caches = Caches::load(&pool).await.unwrap();
    let state = AppState::new(cfg, pool.clone(), caches, b"test-secret".to_vec(), None);
    let app = solo_new_api_rust::routes::admin::channel::router().with_state(state.clone());
    (app, pool, state)
}

async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (u16, Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(value) => {
            builder = builder.header("content-type", "application/json");
            Body::from(serde_json::to_vec(&value).unwrap())
        }
        None => Body::empty(),
    };
    let res = app
        .clone()
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    let status = res.status().as_u16();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn order(app: &Router) -> Value {
    let (status, body) = call(app, "GET", "/api/channel/priority/order", None).await;
    assert_eq!(status, 200);
    assert_eq!(body["success"], json!(true));
    body["data"].clone()
}

async fn priorities(pool: &SqlitePool, table: &str) -> Vec<(i64, Option<i64>)> {
    let sql = if table == "channels" {
        "SELECT id, priority FROM channels ORDER BY id"
    } else {
        "SELECT channel_id, MIN(priority) FROM abilities GROUP BY channel_id ORDER BY channel_id"
    };
    sqlx::query_as(sql).fetch_all(pool).await.unwrap()
}

#[tokio::test]
async fn preview_shifts_the_cheaper_tier_without_writing() {
    let (app, pool, _) = setup().await;
    let initial = order(&app).await;
    assert_eq!(
        initial["items"],
        json!([
            {"id": 1, "priority": 100},
            {"id": 2, "priority": 99},
            {"id": 3, "priority": 99},
            {"id": 4, "priority": 0},
        ])
    );

    let (status, body) = call(
        &app,
        "POST",
        "/api/channel/priority/reorder",
        Some(json!({
            "revision": initial["revision"],
            "id": 4,
            "target_id": 2,
            "position": "before",
            "preview": true,
        })),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(
        body["data"]["changes"],
        json!([
            {"id": 1, "previous": 100, "priority": 101},
            {"id": 4, "previous": 0, "priority": 100},
        ])
    );
    assert_eq!(body["data"]["items"][0]["priority"], json!(101));
    assert_eq!(order(&app).await, initial, "preview must not persist");
    assert_eq!(
        priorities(&pool, "channels").await,
        vec![(1, Some(100)), (2, Some(99)), (3, Some(99)), (4, Some(0))]
    );
}

#[tokio::test]
async fn commit_updates_channels_abilities_and_rejects_stale_clients() {
    let (app, pool, state) = setup().await;
    let initial = order(&app).await;
    let move_body = json!({
        "revision": initial["revision"],
        "id": 4,
        "target_id": 2,
        "position": "before",
    });

    let (_, preview) = call(
        &app,
        "POST",
        "/api/channel/priority/reorder",
        Some(json!({
            "revision": initial["revision"],
            "id": 4,
            "target_id": 2,
            "position": "before",
            "preview": true,
        })),
    )
    .await;
    let (_, applied) = call(
        &app,
        "POST",
        "/api/channel/priority/reorder",
        Some(move_body.clone()),
    )
    .await;
    assert_eq!(
        applied["data"], preview["data"],
        "commit answers the same order the preview promised"
    );
    assert_eq!(
        priorities(&pool, "channels").await,
        vec![(1, Some(101)), (2, Some(99)), (3, Some(99)), (4, Some(100))]
    );
    assert_eq!(
        priorities(&pool, "abilities").await,
        vec![(1, Some(101)), (2, Some(99)), (3, Some(99)), (4, Some(100))],
        "routing copy must follow the channel priority"
    );
    let cached = state.caches.read().expect("cache lock");
    let tiers: Vec<i64> = cached.abilities[&("default".to_string(), "m1".to_string())]
        .iter()
        .map(|row| row.priority.unwrap_or(0))
        .collect();
    assert!(tiers.contains(&101) && tiers.contains(&100), "{tiers:?}");
    drop(cached);

    let (status, stale) = call(
        &app,
        "POST",
        "/api/channel/priority/reorder",
        Some(move_body),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(stale["success"], json!(false));
    assert!(
        stale["message"]
            .as_str()
            .is_some_and(|m| m.contains("Refresh and try again")),
        "{stale}"
    );

    let applied_data = &applied["data"];
    let undo = json!({
        "revision": applied_data["revision"],
        "restore": applied_data["changes"].as_array().unwrap().iter().map(|change| json!({
            "id": change["id"],
            "previous": change["priority"],
            "priority": change["previous"],
        })).collect::<Vec<_>>(),
    });
    let (_, restored) = call(
        &app,
        "POST",
        "/api/channel/priority/reorder",
        Some(undo.clone()),
    )
    .await;
    assert_eq!(restored["data"]["items"], initial["items"]);
    assert_eq!(restored["data"]["revision"], initial["revision"]);

    // 撤销同样要回到原快照，重复撤销会被快照校验拒绝。
    let (_, second_undo) = call(&app, "POST", "/api/channel/priority/reorder", Some(undo)).await;
    assert_eq!(second_undo["success"], json!(false));
    assert_eq!(
        priorities(&pool, "abilities").await,
        priorities(&pool, "channels").await
    );
}

#[tokio::test]
async fn a_null_priority_moves_and_undoes_as_null() {
    let (app, pool, _) = setup().await;
    sqlx::query("UPDATE channels SET priority=NULL WHERE id=4")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE abilities SET priority=NULL WHERE channel_id=4")
        .execute(&pool)
        .await
        .unwrap();

    let initial = order(&app).await;
    assert_eq!(initial["items"][3]["priority"], Value::Null);
    let (status, applied) = call(
        &app,
        "POST",
        "/api/channel/priority/reorder",
        Some(json!({
            "revision": initial["revision"],
            "id": 4,
            "target_id": 2,
            "position": "before",
        })),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(applied["data"]["changes"][1]["previous"], Value::Null);

    let applied_data = &applied["data"];
    let undo = json!({
        "revision": applied_data["revision"],
        "restore": applied_data["changes"].as_array().unwrap().iter().map(|change| json!({
            "id": change["id"],
            "previous": change["priority"],
            "priority": change["previous"],
        })).collect::<Vec<_>>(),
    });
    let (_, restored) = call(&app, "POST", "/api/channel/priority/reorder", Some(undo)).await;
    assert_eq!(restored["data"]["items"], initial["items"]);
    assert_eq!(
        priorities(&pool, "channels").await,
        vec![(1, Some(100)), (2, Some(99)), (3, Some(99)), (4, None)]
    );
    assert_eq!(
        priorities(&pool, "abilities").await,
        priorities(&pool, "channels").await
    );
}

#[tokio::test]
async fn fix_rebuilds_abilities_from_channels() {
    let (app, pool, state) = setup().await;
    sqlx::query("DELETE FROM abilities WHERE channel_id=2")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO abilities (`group`, model, channel_id, enabled, priority, weight, tag) \
                 VALUES ('default','ghost',99,1,7,7,NULL)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE abilities SET priority=1, weight=1 WHERE channel_id=3 AND model='m1'")
        .execute(&pool)
        .await
        .unwrap();

    let (status, body) = call(&app, "POST", "/api/channel/fix", None).await;
    assert_eq!(status, 200);
    assert_eq!(body["data"], json!({ "success": 4, "fails": 0 }));

    let rows: Vec<(
        String,
        String,
        i64,
        i64,
        Option<i64>,
        Option<i64>,
        Option<String>,
    )> = sqlx::query_as(
        "SELECT `group`, model, channel_id, enabled, priority, weight, tag \
             FROM abilities ORDER BY channel_id, `group`, model",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 16, "{rows:?}");
    assert!(!rows.iter().any(|(_, model, ..)| model == "ghost"));
    let channels: Vec<(i64, Option<i64>, Option<i64>)> =
        sqlx::query_as("SELECT id, priority, weight FROM channels ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
    for (group, model, channel_id, enabled, priority, weight, tag) in &rows {
        let (_, want_priority, want_weight) = channels
            .iter()
            .find(|(id, ..)| id == channel_id)
            .expect("channel");
        assert_eq!(priority, want_priority, "priority follows the channel");
        assert_eq!(weight, want_weight, "weight follows the channel");
        assert_eq!(*enabled, 1);
        assert_eq!(tag.as_deref(), Some("a"));
        assert!(group == "default" || group == "vip", "{group}");
        assert!(model == "m1" || model == "m2", "{model}");
    }
    let cached = state.caches.read().expect("cache lock");
    assert_eq!(cached.abilities.len(), 4, "two groups x two models");
    assert!(
        cached
            .abilities
            .values()
            .all(|rows| rows.iter().all(|row| row.channel_id != 99))
    );
}
