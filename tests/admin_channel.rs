use axum::Router;
use axum::body::Body;
use axum::http::Request;
use http_body_util::BodyExt;
use sqlx::SqlitePool;
use tower::ServiceExt;

use solo_new_api_rust::state::{AppState, Caches};

async fn setup() -> (Router, SqlitePool, AppState) {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::query("CREATE TABLE users (id INTEGER PRIMARY KEY, username TEXT, password TEXT, display_name TEXT, role INTEGER, status INTEGER, `group` TEXT, auth_version INTEGER, email TEXT, deleted_at TEXT)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE tokens (id INTEGER PRIMARY KEY, user_id INTEGER, `key` TEXT, status INTEGER, name TEXT, created_time INTEGER, accessed_time INTEGER, expired_time INTEGER, remain_quota INTEGER, unlimited_quota INTEGER, model_limits_enabled INTEGER, model_limits TEXT, allow_ips TEXT, `group` TEXT, auto_groups TEXT, cross_group_retry INTEGER, used_quota INTEGER, allow_user_quota_query INTEGER, deleted_at TEXT)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE channels (id INTEGER PRIMARY KEY AUTOINCREMENT, type INTEGER, `key` TEXT, status INTEGER, name TEXT, models TEXT, `group` TEXT, base_url TEXT, priority INTEGER, weight INTEGER, channel_info TEXT, model_mapping TEXT, header_override TEXT, created_time INTEGER, test_time INTEGER, response_time INTEGER, open_ai_organization TEXT, test_model TEXT, other TEXT, balance REAL, balance_updated_time INTEGER, used_quota INTEGER, status_code_mapping TEXT, auto_ban INTEGER, other_info TEXT, tag TEXT, setting TEXT, param_override TEXT, remark TEXT, settings TEXT)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE abilities (`group` TEXT, model TEXT, channel_id INTEGER, enabled INTEGER, priority INTEGER, weight INTEGER, tag TEXT, PRIMARY KEY (`group`, model, channel_id))").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE options (`key` TEXT PRIMARY KEY, value TEXT)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO users VALUES (1,'root','x',NULL,100,1,'default',1,NULL,NULL)")
        .execute(&pool)
        .await
        .unwrap();

    let caches = Caches::load(&pool).await.unwrap();
    let cfg = solo_new_api_rust::config::Config {
        port: 0,
        db_path: ":memory:".into(),
        web_dist: "./web/dist".into(),
        jwt_secret_path: "/tmp/.jwt-test".into(),
        trust_proxy_headers: true,
        max_request_bytes: 64 * 1024 * 1024,
    };
    let state = AppState::new(cfg, pool.clone(), caches, b"test-secret".to_vec(), None);
    let app = solo_new_api_rust::routes::admin::channel::router().with_state(state.clone());
    (app, pool, state)
}

async fn call(
    app: &Router,
    method: &str,
    uri: &str,
    body: Option<serde_json::Value>,
) -> (u16, serde_json::Value) {
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
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}

#[tokio::test]
async fn channel_crud_rebuilds_abilities_and_caches() {
    let (app, pool, state) = setup().await;
    let (status, body) = call(
        &app,
        "POST",
        "/api/channel",
        Some(serde_json::json!({
            "mode": "single",
            "channel": {
                "type": 1, "name": "c1", "key": "k1", "status": 1,
                "models": "m1,m2", "group": "default,vip",
                "priority": 10, "weight": 5,
                "channel_info": {"is_multi_key": false, "multi_key_size": 1, "multi_key_polling_index": 0, "multi_key_mode": "random"}
            }
        })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let id = body["data"][0].as_i64().unwrap();

    let abilities: (i64, i64) =
        sqlx::query_as("SELECT COUNT(*), SUM(enabled) FROM abilities WHERE channel_id=?")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(abilities, (4, 4));
    assert_eq!(state.caches.read().unwrap().channels[&id].name, "c1");

    let (_, list) = call(&app, "GET", "/api/channel?p=1&page_size=10", None).await;
    assert_eq!(list["data"]["total"], 1);
    assert_eq!(list["data"]["items"][0]["key"], "");
    assert_eq!(list["data"]["items"][0]["priority"], 10);

    let (_, key) = call(&app, "POST", &format!("/api/channel/{id}/key"), None).await;
    assert_eq!(key["data"]["key"], "k1");

    let (status, updated) = call(
        &app,
        "PUT",
        "/api/channel/",
        Some(serde_json::json!({
            "id": id, "name": "c2", "key": "k2", "key_mode": "replace",
            "models": "m2", "group": "default", "status": 1,
            "priority": 1, "weight": 2
        })),
    )
    .await;
    assert_eq!(status, 200, "{updated}");
    assert_eq!(updated["data"]["name"], "c2");
    let abilities: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM abilities WHERE channel_id=?")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(abilities.0, 1);
    let key: String = sqlx::query_scalar("SELECT `key` FROM channels WHERE id=?")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(key, "k2");

    let (status, _) = call(
        &app,
        "POST",
        &format!("/api/channel/{id}/status"),
        Some(serde_json::json!({"status": 0})),
    )
    .await;
    assert_eq!(status, 200);
    let enabled: i64 = sqlx::query_scalar("SELECT SUM(enabled) FROM abilities WHERE channel_id=?")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(enabled, 0);
    assert_eq!(state.caches.read().unwrap().channels[&id].status, 0);

    let (_, enabled_models) = call(&app, "GET", "/api/channel/models_enabled", None).await;
    assert_eq!(enabled_models["data"].as_array().unwrap().len(), 0);
    let (_, models) = call(&app, "GET", "/api/channel/models", None).await;
    assert!(
        models["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["id"] == "m2")
    );

    let (status, _) = call(&app, "DELETE", &format!("/api/channel/{id}"), None).await;
    assert_eq!(status, 200);
    assert!(state.caches.read().unwrap().channels.get(&id).is_none());
    let left: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM abilities WHERE channel_id=?")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left.0, 0);
}

#[tokio::test]
async fn multi_key_channel_keeps_newline_keys() {
    let (app, pool, _) = setup().await;
    let (status, body) = call(
        &app,
        "POST",
        "/api/channel",
        Some(serde_json::json!({
            "mode": "multi_to_single",
            "channel": {
                "type": 1, "name": "multi", "key": "a\nb\n", "status": 1,
                "models": "m1", "group": "default"
            }
        })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let id = body["data"][0].as_i64().unwrap();
    let (key, info): (String, String) =
        sqlx::query_as("SELECT `key`, channel_info FROM channels WHERE id=?")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(key, "a\nb");
    let info: serde_json::Value = serde_json::from_str(&info).unwrap();
    assert_eq!(info["is_multi_key"], true);
    assert_eq!(info["multi_key_size"], 2);
}

#[tokio::test]
async fn copy_channel_clones_routing_and_resets_counters() {
    let (app, pool, state) = setup().await;
    let (_, body) = call(
        &app,
        "POST",
        "/api/channel",
        Some(serde_json::json!({
            "mode": "single",
            "channel": {
                "type": 1, "name": "src", "key": "k1", "status": 1,
                "models": "m1,m2", "group": "default,vip",
                "priority": 7, "weight": 3, "balance": 12.5, "used_quota": 400,
                "test_time": 111, "response_time": 222, "tag": "t1",
                "settings": {"upstream_model_update_check_enabled": true, "keep_me": 7}
            }
        })),
    )
    .await;
    let src = body["data"][0].as_i64().unwrap();

    let (status, copied) = call(&app, "POST", &format!("/api/channel/copy/{src}"), None).await;
    assert_eq!(status, 200, "{copied}");
    let clone = copied["data"]["id"].as_i64().unwrap();
    assert_ne!(clone, src);

    let row: (String, String, i64, i64, i64, i64, f64, i64, Option<String>, String) =
        sqlx::query_as(
            "SELECT name, `key`, test_time, response_time, priority, status, balance, used_quota, tag, settings FROM channels WHERE id=?",
        )
        .bind(clone)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row.0, "src_copy");
    assert_eq!(row.1, "k1");
    assert_eq!((row.2, row.3), (0, 0));
    assert_eq!((row.4, row.5), (7, 1));
    assert_eq!((row.6, row.7), (0.0, 0));
    assert_eq!(row.8.as_deref(), Some("t1"));
    let settings: serde_json::Value = serde_json::from_str(&row.9).unwrap();
    assert_eq!(settings["upstream_model_update_check_enabled"], true);
    assert_eq!(settings["keep_me"], 7);

    let abilities: Vec<(String, String, i64, Option<i64>, Option<i64>, Option<String>)> =
        sqlx::query_as(
            "SELECT `group`, model, enabled, priority, weight, tag FROM abilities WHERE channel_id=? ORDER BY `group`, model",
        )
        .bind(clone)
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(abilities.len(), 4);
    assert!(
        abilities
            .iter()
            .all(|a| a.2 == 1 && a.3 == Some(7) && a.4 == Some(3) && a.5.as_deref() == Some("t1"))
    );

    let caches = state.caches.read().unwrap();
    assert_eq!(caches.channels[&clone].name, "src_copy");
    assert_eq!(
        caches.abilities[&("vip".to_string(), "m2".to_string())]
            .iter()
            .filter(|a| a.channel_id == clone)
            .count(),
        1
    );
    drop(caches);

    let (_, with_params) = call(
        &app,
        "POST",
        &format!("/api/channel/copy/{src}?suffix=-2&reset_balance=false"),
        None,
    )
    .await;
    let second = with_params["data"]["id"].as_i64().unwrap();
    let row: (String, f64, i64) = sqlx::query_as("SELECT name, balance, used_quota FROM channels WHERE id=?")
        .bind(second)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row.0, "src-2");
    assert_eq!((row.1, row.2), (12.5, 400));

    let (status, missing) = call(&app, "POST", "/api/channel/copy/9999", None).await;
    assert_eq!(status, 404, "{missing}");
}

#[tokio::test]
async fn group_list_returns_group_names() {
    let (_, _pool, state) = setup().await;
    sqlx::query("INSERT INTO abilities VALUES ('default','m1',1,1,0,0,'')")
        .execute(&state.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO abilities VALUES ('vip','m1',1,1,0,0,'')")
        .execute(&state.pool)
        .await
        .unwrap();
    let app = solo_new_api_rust::routes::public_router().with_state(state);
    let (status, body) = call(&app, "GET", "/api/group/", None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["success"], true);
    assert_eq!(body["data"], serde_json::json!(["default", "vip"]));
}
