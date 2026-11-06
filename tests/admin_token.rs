use axum::body::Body;
use axum::http::Request;
use axum::Router;
use http_body_util::BodyExt;
use sqlx::SqlitePool;
use tower::ServiceExt;

use solo_new_api_rust::auth::admin::AdminContext;
use solo_new_api_rust::state::{AppState, Caches};

async fn setup() -> (Router, SqlitePool, AppState) {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::query("CREATE TABLE users (id INTEGER PRIMARY KEY, username TEXT, password TEXT, display_name TEXT, role INTEGER, status INTEGER, `group` TEXT, auth_version INTEGER, email TEXT, deleted_at TEXT)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE tokens (id INTEGER PRIMARY KEY AUTOINCREMENT, user_id INTEGER, `key` TEXT, status INTEGER, name TEXT, created_time INTEGER, accessed_time INTEGER, expired_time INTEGER, remain_quota INTEGER, unlimited_quota INTEGER, model_limits_enabled INTEGER, model_limits TEXT, allow_ips TEXT, `group` TEXT, auto_groups TEXT, cross_group_retry INTEGER, used_quota INTEGER, allow_user_quota_query INTEGER, deleted_at TEXT)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE channels (id INTEGER PRIMARY KEY, type INTEGER, `key` TEXT, status INTEGER, name TEXT, models TEXT, `group` TEXT, base_url TEXT, priority INTEGER, weight INTEGER, channel_info TEXT, model_mapping TEXT, header_override TEXT, created_time INTEGER, test_time INTEGER, response_time INTEGER, open_ai_organization TEXT, test_model TEXT, other TEXT, balance REAL, balance_updated_time INTEGER, used_quota INTEGER, status_code_mapping TEXT, auto_ban INTEGER, other_info TEXT, tag TEXT, setting TEXT, param_override TEXT, remark TEXT, settings TEXT)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE abilities (`group` TEXT, model TEXT, channel_id INTEGER, enabled INTEGER, priority INTEGER, weight INTEGER)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE options (`key` TEXT PRIMARY KEY, value TEXT)").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO users VALUES (1,'root','x',NULL,100,1,'default',1,NULL,NULL)").execute(&pool).await.unwrap();

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
    let app = solo_new_api_rust::routes::admin::token::router()
        .layer(axum::Extension(AdminContext {
            user_id: 1,
            sid: "test".into(),
        }))
        .with_state(state.clone());
    (app, pool, state)
}

async fn call(app: &Router, method: &str, uri: &str, body: Option<serde_json::Value>) -> (u16, serde_json::Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(value) => {
            builder = builder.header("content-type", "application/json");
            Body::from(serde_json::to_vec(&value).unwrap())
        }
        None => Body::empty(),
    };
    let res = app.clone().oneshot(builder.body(body).unwrap()).await.unwrap();
    let status = res.status().as_u16();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null))
}

#[tokio::test]
async fn token_crud_round_trip_and_cache_reload() {
    let (app, pool, state) = setup().await;

    let (status, body) = call(
        &app,
        "POST",
        "/api/token/",
        Some(serde_json::json!({
            "name": "cli",
            "remain_quota": 0,
            "unlimited_quota": true,
            "expired_time": -1,
            "model_limits_enabled": false,
            "model_limits": "",
            "allow_ips": "",
            "group": "default",
            "auto_groups": ["vip"],
            "cross_group_retry": true,
            "allow_user_quota_query": true
        })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["success"], true);
    let id = body["data"]["id"].as_i64().unwrap();
    let masked = body["data"]["key"].as_str().unwrap();
    assert!(masked.contains("**********"), "{masked}");
    assert_eq!(body["data"]["auto_groups"][0], "vip");

    let key: String = sqlx::query_scalar("SELECT `key` FROM tokens WHERE id=?")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(key.len(), 48);
    assert!(state.caches.read().unwrap().token_by_key(&key).is_some());

    let (_, list) = call(&app, "GET", "/api/token/?p=1&size=10", None).await;
    assert_eq!(list["data"]["total"], 1);
    assert_eq!(list["data"]["items"][0]["name"], "cli");
    assert_eq!(list["data"]["items"][0]["key"], masked);

    let (_, full) = call(&app, "POST", &format!("/api/token/{id}/key"), None).await;
    assert_eq!(full["data"]["key"], key);
    let (_, found) = call(&app, "GET", "/api/token/search?keyword=cli&p=1&size=10", None).await;
    assert_eq!(found["data"]["total"], 1);
    let (_, none) = call(&app, "GET", "/api/token/search?keyword=zzz&p=1&size=10", None).await;
    assert_eq!(none["data"]["total"], 0);

    let (status, updated) = call(
        &app,
        "PUT",
        "/api/token/",
        Some(serde_json::json!({
            "id": id, "name": "renamed", "remain_quota": 5, "unlimited_quota": false,
            "expired_time": -1, "model_limits_enabled": true, "model_limits": "m1",
            "allow_ips": "1.2.3.4", "group": "vip", "auto_groups": [],
            "cross_group_retry": false, "allow_user_quota_query": false
        })),
    )
    .await;
    assert_eq!(status, 200, "{updated}");
    assert_eq!(updated["data"]["name"], "renamed");
    assert_eq!(updated["data"]["auto_groups"], serde_json::Value::Null);
    assert_eq!(updated["data"]["remain_quota"], 5);

    let (status, _) = call(
        &app,
        "PUT",
        "/api/token/?status_only=true",
        Some(serde_json::json!({"id": id, "status": 2})),
    )
    .await;
    assert_eq!(status, 200);
    let cached = state.caches.read().unwrap().token_by_key(&key).unwrap();
    assert_eq!(cached.status, 2);

    let (status, _) = call(&app, "DELETE", &format!("/api/token/{id}/"), None).await;
    assert_eq!(status, 200);
    assert!(state.caches.read().unwrap().token_by_key(&key).is_none());
    let (_, list) = call(&app, "GET", "/api/token/?p=1&size=10", None).await;
    assert_eq!(list["data"]["total"], 0);
}

#[tokio::test]
async fn batch_delete_and_batch_keys() {
    let (app, _, _) = setup().await;
    let mut ids = Vec::new();
    for name in ["a", "b"] {
        let (_, body) = call(
            &app,
            "POST",
            "/api/token/",
            Some(serde_json::json!({"name": name})),
        )
        .await;
        ids.push(body["data"]["id"].as_i64().unwrap());
    }
    let (_, body) = call(
        &app,
        "POST",
        "/api/token/batch/keys",
        Some(serde_json::json!({"ids": ids})),
    )
    .await;
    assert_eq!(body["data"]["keys"].as_object().unwrap().len(), 2);

    let (status, _) = call(
        &app,
        "POST",
        "/api/token/batch",
        Some(serde_json::json!({"ids": ids})),
    )
    .await;
    assert_eq!(status, 200);
    let (_, list) = call(&app, "GET", "/api/token/?p=1&size=10", None).await;
    assert_eq!(list["data"]["total"], 0);
}
